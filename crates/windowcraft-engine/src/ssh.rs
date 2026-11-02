// Made by ycangignacy
//! OpenSSH Server setup.

use crate::atomicfs::atomic_write;
use crate::dism::DismSession;
use crate::error::{EngineError, Result};
use crate::security::apply_sddl;
use crate::services::{ScmBackend, ServiceBackend, start};
use crate::win::ComApartment;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::NetworkManagement::WindowsFirewall::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::core::BSTR;
use windowcraft_config::StartupType;

const CAPABILITY: &str = "OpenSSH.Server~~~~0.0.1.0";
const RULE_NAME: &str = "WindowCraft OpenSSH Server (TCP 22)";
/// Administrators and SYSTEM only, written with SIDs so it is locale-independent.
const KEYS_SDDL: &str = "D:P(A;;FA;;;S-1-5-18)(A;;FA;;;S-1-5-32-544)";

fn open_firewall() -> Result<()> {
    let _apartment = ComApartment::enter()?;
    let fail = |error: windows::core::Error| EngineError::windows("INetFwPolicy2", RULE_NAME, &error);
    // SAFETY: The CLSIDs are valid, and every call operates on live objects
    // with BSTR arguments that outlive the call.
    unsafe {
        let policy: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER).map_err(fail)?;
        let rules = policy.Rules().map_err(fail)?;
        // Re-running replaces the rule. Absence on first run is expected.
        let _ = rules.Remove(&BSTR::from(RULE_NAME));

        let rule: INetFwRule = CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER).map_err(fail)?;
        rule.SetName(&BSTR::from(RULE_NAME)).map_err(fail)?;
        rule.SetDescription(&BSTR::from("Inbound SSH managed by WindowCraft")).map_err(fail)?;
        rule.SetProtocol(NET_FW_IP_PROTOCOL_TCP.0).map_err(fail)?;
        rule.SetLocalPorts(&BSTR::from("22")).map_err(fail)?;
        rule.SetDirection(NET_FW_RULE_DIR_IN).map_err(fail)?;
        rule.SetAction(NET_FW_ACTION_ALLOW).map_err(fail)?;
        rule.SetEnabled(VARIANT_TRUE).map_err(fail)?;
        rule.SetEdgeTraversal(VARIANT_FALSE).map_err(fail)?;
        rules.Add(&rule).map_err(fail)?;
    }
    Ok(())
}

/// Removes the inbound rule. The OpenSSH capability and services stay.
pub fn remove_firewall_rule() -> Result<()> {
    let _apartment = ComApartment::enter()?;
    let fail = |error: windows::core::Error| EngineError::windows("INetFwPolicy2", RULE_NAME, &error);
    // SAFETY: The CLSID is valid, and every call operates on live objects with
    // a BSTR argument that outlives the call.
    unsafe {
        let policy: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER).map_err(fail)?;
        policy.Rules().map_err(fail)?.Remove(&BSTR::from(RULE_NAME)).map_err(fail)
    }
}

/// Merges keys, keeping existing lines first and dropping duplicates.
pub fn merge_keys(existing: &str, incoming: &str) -> String {
    let mut merged: Vec<&str> = Vec::new();
    for line in existing.lines().chain(incoming.lines()) {
        let line = line.trim();
        if !line.is_empty() && !merged.contains(&line) {
            merged.push(line);
        }
    }
    let mut output = merged.join("\r\n");
    if !output.is_empty() {
        output.push_str("\r\n");
    }
    output
}

fn migrate_keys(destination: &Path) -> Result<()> {
    let existing = std::fs::read_to_string(destination).unwrap_or_default();
    let source = std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(r".ssh\authorized_keys"));
    let incoming = source.and_then(|path| std::fs::read_to_string(path).ok()).unwrap_or_default();
    atomic_write(destination, merge_keys(&existing, &incoming).as_bytes())?;
    apply_sddl(destination, KEYS_SDDL)
}

pub fn install_server() -> Result<()> {
    DismSession::online()?.add_capability(CAPABILITY)?;
    for service in ["sshd", "ssh-agent"] {
        ScmBackend.set_startup(service, StartupType::Automatic)?;
    }
    open_firewall()?;

    let program_data = PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_owned()));
    let directory = program_data.join("ssh");
    std::fs::create_dir_all(&directory).map_err(|e| EngineError::io(directory.display().to_string(), e))?;
    migrate_keys(&directory.join("administrators_authorized_keys"))?;

    for service in ["sshd", "ssh-agent"] {
        start(service)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_merge_without_duplicates() {
        let merged = merge_keys("ssh-ed25519 AAA one\r\n", "ssh-ed25519 AAA one\nssh-rsa BBB two\n\n");
        assert_eq!(merged, "ssh-ed25519 AAA one\r\nssh-rsa BBB two\r\n");
        assert_eq!(merge_keys("", ""), "");
    }

    #[test]
    fn merge_keys_trims_whitespace_preserves_existing_order_and_drops_blank_lines() {
        let existing = "  ssh-ed25519 AAA first \r\n\t\r\nssh-rsa BBB second\nssh-ed25519 AAA first\n";
        let incoming = "ssh-rsa BBB second  \n  ssh-ecdsa CCC third\t\n   \n";
        assert_eq!(
            merge_keys(existing, incoming),
            "ssh-ed25519 AAA first\r\nssh-rsa BBB second\r\nssh-ecdsa CCC third\r\n"
        );
        assert_eq!(merge_keys("   \n\t\r\n ", "\n  \r\n"), "");
    }

    #[test]
    fn lockdown_descriptor_is_valid_and_uses_sids() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("keys");
        std::fs::write(&file, b"k").unwrap();
        assert!(KEYS_SDDL.contains("S-1-5-18") && KEYS_SDDL.contains("S-1-5-32-544"));
        // Applying it to a file this test owns is allowed: the owner keeps
        // READ_CONTROL and WRITE_DAC, which also lets the cleanup succeed.
        apply_sddl(&file, KEYS_SDDL).unwrap();
        let sddl = crate::security::dacl_sddl(&file).unwrap();
        assert!(sddl.starts_with("D:P"), "{sddl}");
        assert!(sddl.contains("SY") && sddl.contains("BA"), "{sddl}");
        apply_sddl(&file, "D:(A;;FA;;;WD)").unwrap();
    }

    #[test]
    fn migrate_keys_deduplicates_existing_destination_and_applies_lockdown_dacl() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("administrators_authorized_keys");
        std::fs::write(&destination, "  ssh-ed25519 AAA testkey  \nssh-ed25519 AAA testkey\n").unwrap();

        migrate_keys(&destination).unwrap();
        let sddl = crate::security::dacl_sddl(&destination).unwrap();
        assert!(sddl.starts_with("D:P") && sddl.contains("SY") && sddl.contains("BA"), "{sddl}");
        apply_sddl(&destination, "D:(A;;FA;;;WD)").unwrap();

        let written = std::fs::read_to_string(&destination).unwrap();
        assert_eq!(written.lines().filter(|&line| line == "ssh-ed25519 AAA testkey").count(), 1);
        assert!(written.ends_with("\r\n"));

        let missing_parent = dir.path().join("no_such_dir").join("administrators_authorized_keys");
        assert!(migrate_keys(&missing_parent).unwrap_err().is_not_found());
    }
}
