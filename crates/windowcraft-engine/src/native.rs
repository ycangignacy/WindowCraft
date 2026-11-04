// Made by ycangignacy
//! Maps catalog operations onto the native adapters.

use crate::atomicfs::{atomic_write, hosts_with, hosts_without, sanitize_blocklist};
use crate::cleaner::clean_dir;
use crate::dism::{DismSession, enable_online_feature};
use crate::download::{block_on, download, fetch_text_pinned};
use crate::error::{EngineError, Result};
use crate::executor::NativeRunner;
use crate::jobs::CancelToken;
use crate::process::{run, run_ok};
use crate::registry::{RegPath, RegistryScope};
use crate::security::{AclGuard, SID_ADMINISTRATORS, SID_EVERYONE, add_deny, clear_deny, delete_mask, rights_mask};
use crate::win::{expand_env, system32};
use crate::{appx, dns, fixes, packages, power, shell, sysinfo, tasks, wmi};
use std::path::{Path, PathBuf};
use std::time::Duration;
use windowcraft_config::{AclRights, App, NativeOp};

const SHORT: Duration = Duration::from_secs(120);
const LONG: Duration = Duration::from_secs(60 * 60);
/// The CTT profile at a reviewed commit. Update both values together after
/// reading the new version: it runs in every PowerShell 7 session.
const PROFILE_URL: &str = "https://raw.githubusercontent.com/ChrisTitusTech/powershell-profile/daa7ba227573de2a97739f15762f2ec231817925/Microsoft.PowerShell_profile.ps1";
const PROFILE_SHA256: &str = "e249c4f838b72fcaea0742c1922b6542567b42afcf11a9462d7bbc0f163e1ade";

/// Runs against the live system.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

fn hosts_path() -> PathBuf {
    system32(r"drivers\etc\hosts")
}

fn block_directory(path: &Path) -> Result<()> {
    if path.exists() {
        clean_dir(path);
    } else {
        std::fs::create_dir_all(path).map_err(|e| EngineError::io(path.display().to_string(), e))?;
    }
    add_deny(path, SID_EVERYONE, rights_mask(AclRights::Write))
}

fn find_edge_setup() -> Option<PathBuf> {
    let root = std::env::var_os("ProgramFiles(x86)")?;
    let application = PathBuf::from(root).join(r"Microsoft\Edge\Application");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(application)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(r"Installer\setup.exe"))
        .filter(|path| path.exists())
        .collect();
    candidates.sort();
    candidates.pop()
}

fn remove_edge(cancel: &CancelToken) -> Result<()> {
    let Some(setup) = find_edge_setup() else {
        return Ok(());
    };
    // A file at the legacy Edge path unlocks the official uninstaller.
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    let marker = PathBuf::from(root).join(r"SystemApps\Microsoft.MicrosoftEdge_8wekyb3d8bbwe\MicrosoftEdge.exe");
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).map_err(|e| EngineError::io(parent.display().to_string(), e))?;
    }
    std::fs::write(&marker, b"").map_err(|e| EngineError::io(marker.display().to_string(), e))?;
    let arguments = ["--uninstall", "--system-level", "--force-uninstall", "--delete-profile"];
    run(&setup, &arguments, cancel, LONG, |_| {})?;
    Ok(())
}

fn onedrive_setup() -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("SystemRoot")?);
    [root.join(r"System32\OneDriveSetup.exe"), root.join(r"SysWOW64\OneDriveSetup.exe")]
        .into_iter()
        .find(|path| path.exists())
}

/// Uninstalls OneDrive with the user's files shielded by a temporary DENY.
/// The guard restores the original ACL on success, failure, timeout or panic.
fn remove_onedrive(cancel: &CancelToken) -> Result<()> {
    let folder = std::env::var_os("OneDrive").map(PathBuf::from).filter(|path| path.is_dir());
    let guard = folder.as_deref().map(|path| AclGuard::deny(path, SID_ADMINISTRATORS, delete_mask())).transpose()?;

    if let Some(setup) = onedrive_setup() {
        run_ok(&setup, &["/uninstall"], cancel, Duration::from_secs(15 * 60))?;
    }
    shell::kill_matching("FileCoAuth")?;
    shell::kill_matching("explorer")?;

    let mut leftovers = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        leftovers.push(PathBuf::from(local).join(r"Microsoft\OneDrive"));
    }
    if let Some(data) = std::env::var_os("ProgramData") {
        leftovers.push(PathBuf::from(data).join("Microsoft OneDrive"));
    }
    for path in leftovers {
        clean_dir(&path);
        // Anything still open stays behind and is harmless.
        let _ = std::fs::remove_dir(&path);
    }

    if let Some(guard) = guard {
        guard.restore()?;
    }

    // Only an empty folder is removed, never one that holds user files.
    if let Some(path) = &folder
        && std::fs::remove_dir(path).is_ok()
    {
        let environment = RegPath::parse(r"HKCU\Environment")?;
        RegistryScope::System.write_opt(&environment, "OneDrive", None)?;
    }
    shell::restart_explorer()
}

fn install_profile(cancel: &CancelToken) -> Result<()> {
    let target = powershell_profile_path()?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| EngineError::io(parent.display().to_string(), e))?;
    }
    block_on(download(PROFILE_URL, &target.with_extension("wc-new"), Some(PROFILE_SHA256), cancel, |_, _| {}))??;
    let content =
        std::fs::read(target.with_extension("wc-new")).map_err(|e| EngineError::io(target.display().to_string(), e))?;
    atomic_write(&target, &content)?;
    let _ = std::fs::remove_file(target.with_extension("wc-new"));
    Ok(())
}

fn uninstall_profile() -> Result<()> {
    let target = powershell_profile_path()?;
    let backup = target.with_file_name(format!(
        "{}.bak",
        target.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    ));
    if backup.exists() {
        std::fs::rename(&backup, &target).map_err(|e| EngineError::io(target.display().to_string(), e))
    } else {
        match std::fs::remove_file(&target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(EngineError::io(target.display().to_string(), error)),
        }
    }
}

fn powershell_profile_path() -> Result<PathBuf> {
    use windows::Win32::UI::Shell::{FOLDERID_Documents, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    // SAFETY: The GUID is a static and no token is passed. The returned string
    // is CoTaskMem-allocated and freed below.
    let path = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None) }
        .map_err(|error| EngineError::windows("SHGetKnownFolderPath", "Documents", &error))?;
    // SAFETY: On success `path` is a valid null-terminated string.
    let text = unsafe { path.to_string() }.unwrap_or_default();
    // SAFETY: Frees the allocation made by SHGetKnownFolderPath, exactly once.
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(path.0.cast())) };
    if text.is_empty() {
        return Err(EngineError::NotFound("Documents folder".to_owned()));
    }
    Ok(PathBuf::from(text).join(r"PowerShell\Microsoft.PowerShell_profile.ps1"))
}

fn winget_install(id: &str, cancel: &CancelToken) -> Result<()> {
    let app = App {
        id: id.to_owned(),
        name: id.to_owned(),
        category: String::new(),
        description: String::new(),
        link: String::new(),
        foss: false,
        winget: Some(id.to_owned()),
        choco: None,
    };
    packages::install(packages::PackageManager::Winget, &app, cancel, &mut |_| {})
}

impl NativeRunner for SystemRunner {
    fn run(&self, op: &NativeOp, cancel: &CancelToken) -> Result<()> {
        cancel.check()?;
        match op {
            NativeOp::RestartExplorer => shell::restart_explorer(),
            NativeOp::RefreshShell => {
                shell::refresh_theme();
                Ok(())
            }
            NativeOp::BroadcastEnvironment => {
                shell::broadcast_environment();
                Ok(())
            }
            NativeOp::KillProcesses { pattern } => shell::kill_matching(pattern).map(drop),
            NativeOp::RemoveAppx { pattern } => appx::remove(pattern, cancel).map(drop),
            NativeOp::MarkAppxEndOfLife { pattern } => appx::mark_end_of_life(pattern),
            NativeOp::Hibernate { enabled } => {
                let state = if *enabled { "on" } else { "off" };
                run_ok(&system32("powercfg.exe"), &["/hibernate", state], cancel, SHORT).map(drop)
            }
            // A target that does not exist has nothing to protect.
            NativeOp::AclDeny { path, sid, rights } => {
                match add_deny(Path::new(&expand_env(path)), sid, rights_mask(*rights)) {
                    Err(error) if error.is_not_found() => Ok(()),
                    other => other,
                }
            }
            NativeOp::AclClearDeny { path, sid } => match clear_deny(Path::new(&expand_env(path)), sid) {
                Err(error) if error.is_not_found() => Ok(()),
                other => other.map(drop),
            },
            NativeOp::BlockDirectory { path } => block_directory(Path::new(&expand_env(path))),
            NativeOp::UnblockDirectory { path } => {
                clear_deny(Path::new(&expand_env(path)), SID_EVERYONE).map(drop)
            }
            NativeOp::TempCleanup => {
                crate::cleaner::clean_temporary_files();
                Ok(())
            }
            NativeOp::DiskCleanup => {
                let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
                run(&system32("cleanmgr.exe"), &["/d", &drive, "/VERYLOWDISK"], cancel, LONG, |_| {})?;
                let arguments = ["/online", "/Cleanup-Image", "/StartComponentCleanup", "/ResetBase"];
                run_ok(&system32("Dism.exe"), &arguments, cancel, LONG).map(drop)
            }
            NativeOp::RestorePoint => power::create_restore_point(),
            NativeOp::BitlockerDecrypt => wmi::decrypt_system_volume(),
            NativeOp::RemoveEdge => remove_edge(cancel),
            NativeOp::RemoveOnedrive => remove_onedrive(cancel),
            NativeOp::WingetInstall { id } => winget_install(id, cancel),
            NativeOp::ReservedStorage { enabled } => DismSession::online()?.set_reserved_storage(*enabled),
            NativeOp::HostsBlock { section, url, sha256 } => {
                let list = sanitize_blocklist(&block_on(fetch_text_pinned(url, sha256))??);
                if list.is_empty() {
                    return Err(EngineError::Invalid(format!("block list at {url} held no valid entries")));
                }
                let path = hosts_path();
                let current = std::fs::read_to_string(&path).map_err(|e| EngineError::io(path.display().to_string(), e))?;
                atomic_write(&path, hosts_with(&current, section, &list).as_bytes())?;
                dns::flush_cache();
                Ok(())
            }
            NativeOp::HostsUnblock { section } => {
                let path = hosts_path();
                let current = std::fs::read_to_string(&path).map_err(|e| EngineError::io(path.display().to_string(), e))?;
                atomic_write(&path, hosts_without(&current, section).as_bytes())?;
                dns::flush_cache();
                Ok(())
            }
            NativeOp::RegDeleteTree { path } => RegistryScope::System.delete_tree(&RegPath::parse(path)?),
            NativeOp::Teredo { enabled } => {
                let state = if *enabled { "default" } else { "disabled" };
                let arguments = ["interface", "teredo", "set", "state", state];
                run_ok(&system32("netsh.exe"), &arguments, cancel, SHORT).map(drop)
            }
            NativeOp::Ipv6Binding { enabled } => wmi::set_ipv6_binding(*enabled),
            NativeOp::DownloadAndRun { url, file, args } => {
                let destination = crate::win::local_app_dir().join(file);
                block_on(download(url, &destination, None, cancel, |_, _| {}))??;
                if !crate::authenticode::verify_file(&destination).trusted {
                    return Err(EngineError::Invalid(format!("{file} failed signature verification")));
                }
                shell::launch(&destination.display().to_string(), args.as_deref())
            }
            NativeOp::UltimatePerformance { enabled } => {
                if *enabled { power::enable_ultimate_performance() } else { power::remove_ultimate_performance() }
            }
            NativeOp::DisableFeature { name } => match DismSession::online()?.disable_feature(name) {
                Err(error) if error.is_not_found() => Ok(()),
                other => other,
            },
            NativeOp::RequireKbAbsent { kb } => {
                if sysinfo::kb_installed(kb)? {
                    Err(EngineError::Unsupported(format!("{kb} is installed, which removes this option")))
                } else {
                    Ok(())
                }
            }
            NativeOp::BootMenuPolicy { legacy } => {
                let policy = if *legacy { "legacy" } else { "standard" };
                let arguments = ["/set", "bootmenupolicy", policy];
                run_ok(&system32("bcdedit.exe"), &arguments, cancel, SHORT).map(drop)
            }
            NativeOp::NfsConfigure => {
                let admin = system32("nfsadmin.exe");
                run(&admin, &["client", "stop"], cancel, SHORT, |_| {})?;
                run(&admin, &["client", "start"], cancel, SHORT, |_| {})?;
                let arguments = ["client", "localhost", "config", "fileaccess=755", "SecFlavors=+sys", "-krb5", "-krb5i"];
                run_ok(&admin, &arguments, cancel, SHORT).map(drop)
            }
            NativeOp::CreateRegistryBackupTask => tasks::register_daily(
                tasks::REGISTRY_BACKUP_TASK,
                "Create System Registry Backups",
                "schtasks",
                r#"/run /i /tn "\Microsoft\Windows\Registry\RegIdleBackup""#,
                "2024-01-01T00:30:00",
            ),
            NativeOp::FixWindowsUpdate { aggressive } => fixes::reset_windows_update(*aggressive, cancel, |_| {}),
            NativeOp::FixNetwork => fixes::reset_network(cancel),
            NativeOp::FixSystemScan => {
                let results = fixes::system_scan(cancel, |_, _| {})?;
                fixes::require_clean(&results)
            }
            NativeOp::FixWinget => fixes::repair_winget(),
            NativeOp::FixNtp => fixes::configure_ntp(cancel),
            NativeOp::PowershellProfile { install } => {
                if *install { install_profile(cancel) } else { uninstall_profile() }
            }
            NativeOp::SshServer => crate::ssh::install_server(),
            NativeOp::RemoveSshFirewallRule => crate::ssh::remove_firewall_rule(),
            NativeOp::RemoveRegistryBackupTask => {
                tasks::delete(&format!(r"\{}", tasks::REGISTRY_BACKUP_TASK)).map(drop)
            }
            NativeOp::Launch { target } => shell::launch(target, None),
        }
    }

    fn enable_feature(&self, name: &str, _cancel: &CancelToken) -> Result<()> {
        enable_online_feature(name)
    }

    fn disable_feature(&self, name: &str, _cancel: &CancelToken) -> Result<()> {
        // A feature that no longer exists is as good as off.
        match crate::dism::disable_online_feature(name) {
            Err(error) if error.is_not_found() => Ok(()),
            other => other,
        }
    }

    fn feature_enabled(&self, name: &str) -> Result<bool> {
        crate::dism::online_feature_enabled(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reaches GitHub, so it only runs with `--ignored`. A failure means the
    /// pinned commit or its hash no longer match.
    #[tokio::test]
    #[ignore = "needs network access"]
    async fn pinned_powershell_profile_matches_its_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.ps1");
        download(PROFILE_URL, &path, Some(PROFILE_SHA256), &CancelToken::new(), |_, _| {}).await.unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("function Update-Profile"));
    }

    /// Fetches every pinned hosts block list in the catalog. Needs network.
    #[tokio::test]
    #[ignore = "needs network access"]
    async fn pinned_block_lists_match_their_hashes() {
        let catalog = windowcraft_config::Catalog::load().unwrap();
        let mut checked = 0;
        for op in catalog.tweaks.iter().flat_map(|tweak| tweak.apply.iter()) {
            if let NativeOp::HostsBlock { url, sha256, .. } = op {
                let list = sanitize_blocklist(&crate::download::fetch_text_pinned(url, sha256).await.unwrap());
                assert!(!list.is_empty(), "{url}");
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}
