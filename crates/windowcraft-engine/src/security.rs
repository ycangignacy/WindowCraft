// Made by ycangignacy
//! File and directory ACLs through the Win32 security APIs.
//!
//! All principals are SIDs, so behaviour does not change with the system locale.

use crate::error::{EngineError, Result};
use crate::win::{LocalGuard, from_pcwstr, pcwstr, wide};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::GENERIC_ALL;
use windows::Win32::Security::Authorization::*;
use windows::Win32::Security::*;
use windows::Win32::Storage::FileSystem::{
    DELETE, FILE_APPEND_DATA, FILE_DELETE_CHILD, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FILE_WRITE_EA,
};
use windows::core::{BOOL, PCWSTR, PWSTR};
use windowcraft_config::AclRights;

pub const SID_EVERYONE: &str = "S-1-1-0";
pub const SID_ADMINISTRATORS: &str = "S-1-5-32-544";

const ACCESS_DENIED_TYPE: u8 = 1;

fn name_of(path: &Path) -> Vec<u16> {
    wide(path)
}

struct Sid(LocalGuard);

impl Sid {
    fn parse(text: &str) -> Result<Self> {
        let wide_sid = wide(text);
        let mut sid = PSID::default();
        // SAFETY: The string is null-terminated, and `sid` is a valid out
        // pointer. Ownership of the allocation moves into the guard.
        unsafe { ConvertStringSidToSidW(pcwstr(&wide_sid), &mut sid) }
            .map_err(|error| EngineError::windows("ConvertStringSidToSidW", text, &error))?;
        Ok(Self(LocalGuard(sid.0)))
    }

    fn psid(&self) -> PSID {
        PSID(self.0.0)
    }
}

/// A file's security descriptor, freed on every exit path.
struct Descriptor {
    guard: LocalGuard,
    dacl: *mut ACL,
}

fn read_dacl(path: &Path) -> Result<Descriptor> {
    let name = name_of(path);
    let mut dacl = core::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: `name` is null-terminated and all out pointers are valid.
    // The returned descriptor is LocalAlloc'd and owned by the guard below.
    let status = unsafe {
        GetNamedSecurityInfoW(
            pcwstr(&name),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut dacl),
            None,
            &mut descriptor,
        )
    };
    if status.is_err() {
        return Err(EngineError::win32("GetNamedSecurityInfoW", path.display().to_string(), status.0));
    }
    Ok(Descriptor { guard: LocalGuard(descriptor.0), dacl })
}

fn write_dacl(path: &Path, dacl: *const ACL, information: OBJECT_SECURITY_INFORMATION) -> Result<()> {
    let name = name_of(path);
    // SAFETY: `name` is null-terminated and `dacl` is valid or null, which the
    // API documents as an empty-security DACL only when the flag asks for it.
    let status = unsafe {
        SetNamedSecurityInfoW(pcwstr(&name), SE_FILE_OBJECT, information, None, None, Some(dacl), None)
    };
    if status.is_err() {
        return Err(EngineError::win32("SetNamedSecurityInfoW", path.display().to_string(), status.0));
    }
    Ok(())
}

/// Text form of a path's DACL.
pub fn dacl_sddl(path: &Path) -> Result<String> {
    let descriptor = read_dacl(path)?;
    let mut text = PWSTR::null();
    // SAFETY: The descriptor is live. The output string is LocalAlloc'd and
    // moved into a guard immediately after the call.
    unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            PSECURITY_DESCRIPTOR(descriptor.guard.0),
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            None,
        )
    }
    .map_err(|error| EngineError::windows("ConvertSecurityDescriptorToStringSecurityDescriptorW", path.display().to_string(), &error))?;
    let _guard = LocalGuard::pwstr(text);
    // SAFETY: On success `text` is a valid null-terminated string.
    Ok(unsafe { from_pcwstr(PCWSTR(text.0)) })
}

/// Applies a DACL given as SDDL text, for example a lockdown of
/// `administrators_authorized_keys`.
pub fn apply_sddl(path: &Path, sddl: &str) -> Result<()> {
    let wide_sddl = wide(sddl);
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: The input is null-terminated. The output descriptor is
    // LocalAlloc'd and moved into a guard immediately after the call.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(pcwstr(&wide_sddl), SDDL_REVISION_1, &mut descriptor, None)
    }
    .map_err(|error| EngineError::windows("ConvertStringSecurityDescriptorToSecurityDescriptorW", sddl, &error))?;
    let _guard = LocalGuard(descriptor.0);

    let (mut present, mut defaulted) = (BOOL::default(), BOOL::default());
    let mut dacl = core::ptr::null_mut();
    // SAFETY: The descriptor is live and all out pointers are valid.
    unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted) }
        .map_err(|error| EngineError::windows("GetSecurityDescriptorDacl", sddl, &error))?;

    let protected = sddl.split('(').next().is_some_and(|flags| flags.contains('P'));
    let flag = if protected { PROTECTED_DACL_SECURITY_INFORMATION } else { UNPROTECTED_DACL_SECURITY_INFORMATION };
    write_dacl(path, dacl, DACL_SECURITY_INFORMATION | flag)
}

/// Adds an explicit DENY entry.
pub fn add_deny(path: &Path, sid: &str, mask: u32) -> Result<()> {
    let descriptor = read_dacl(path)?;
    let sid = Sid::parse(sid)?;
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: mask,
        grfAccessMode: DENY_ACCESS,
        // Inheritance flags only mean something on a directory.
        grfInheritance: if path.is_dir() { SUB_CONTAINERS_AND_OBJECTS_INHERIT } else { NO_INHERITANCE },
        Trustee: TRUSTEE_W {
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
            ptstrName: PWSTR(sid.psid().0.cast()),
            ..TRUSTEE_W::default()
        },
    };
    let mut merged = core::ptr::null_mut();
    // SAFETY: The entry and old DACL are valid for the call. The merged ACL
    // is LocalAlloc'd and moved into a guard immediately after.
    let status = unsafe { SetEntriesInAclW(Some(&[entry]), Some(descriptor.dacl), &mut merged) };
    if status.is_err() {
        return Err(EngineError::win32("SetEntriesInAclW", path.display().to_string(), status.0));
    }
    let _merged = LocalGuard(merged.cast());
    write_dacl(path, merged, DACL_SECURITY_INFORMATION)
}

/// Removes every explicit DENY entry for `sid`, leaving others intact.
pub fn clear_deny(path: &Path, sid: &str) -> Result<usize> {
    let descriptor = read_dacl(path)?;
    let sid = Sid::parse(sid)?;
    if descriptor.dacl.is_null() {
        return Ok(0);
    }

    // SAFETY: `dacl` points into the live descriptor and is non-null.
    let count = u32::from(unsafe { (*descriptor.dacl).AceCount });
    let mut removed = 0;
    for index in (0..count).rev() {
        let mut ace = core::ptr::null_mut();
        // SAFETY: `index` is below AceCount, and deletion runs high to low so
        // earlier indexes stay valid.
        unsafe { GetAce(descriptor.dacl, index, &mut ace) }
            .map_err(|error| EngineError::windows("GetAce", path.display().to_string(), &error))?;
        let ace = ace.cast::<ACCESS_DENIED_ACE>();
        // SAFETY: GetAce returned a pointer to a complete ACE. The header is
        // common to all ACE types and read before the type is trusted.
        let kind = unsafe { (*ace).Header.AceType };
        if kind != ACCESS_DENIED_TYPE {
            continue;
        }
        // SAFETY: For an access-denied ACE, the SID starts at `SidStart`.
        let entry_sid = PSID(unsafe { (&raw const (*ace).SidStart).cast_mut().cast() });
        // SAFETY: Both SIDs are valid for the call.
        if unsafe { EqualSid(entry_sid, sid.psid()) }.is_ok() {
            // SAFETY: The ACL is writable memory owned by the descriptor guard.
            unsafe { DeleteAce(descriptor.dacl, index) }
                .map_err(|error| EngineError::windows("DeleteAce", path.display().to_string(), &error))?;
            removed += 1;
        }
    }
    if removed > 0 {
        write_dacl(path, descriptor.dacl, DACL_SECURITY_INFORMATION)?;
    }
    Ok(removed)
}

pub fn rights_mask(rights: AclRights) -> u32 {
    match rights {
        AclRights::Full => GENERIC_ALL.0,
        // Not GENERIC_WRITE: it maps to SYNCHRONIZE and READ_CONTROL as well,
        // and denying those stops anyone opening the folder at all.
        AclRights::Write => FILE_WRITE_DATA.0 | FILE_APPEND_DATA.0 | FILE_WRITE_EA.0 | FILE_WRITE_ATTRIBUTES.0,
    }
}

/// Mask that stops a folder or its children being deleted.
pub fn delete_mask() -> u32 {
    DELETE.0 | FILE_DELETE_CHILD.0
}

/// Applies a DENY and guarantees the original DACL comes back.
///
/// The restore lives in `Drop`, so it runs on early returns, on timeouts and
/// while a panic unwinds. The release profile keeps unwinding on for this.
#[must_use = "dropping the guard restores the ACL immediately"]
pub struct AclGuard {
    path: PathBuf,
    original_sddl: String,
    restored: bool,
}

impl AclGuard {
    pub fn deny(path: &Path, sid: &str, mask: u32) -> Result<Self> {
        let original_sddl = dacl_sddl(path)?;
        add_deny(path, sid, mask)?;
        Ok(Self { path: path.to_path_buf(), original_sddl, restored: false })
    }

    /// Restores now and reports failure, which `Drop` cannot do.
    pub fn restore(mut self) -> Result<()> {
        self.restored = true;
        apply_sddl(&self.path, &self.original_sddl)
    }
}

impl Drop for AclGuard {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        for attempt in 0..3 {
            match apply_sddl(&self.path, &self.original_sddl) {
                Ok(()) => return,
                Err(error) if error.is_not_found() => return,
                Err(_) if attempt < 2 => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(error) => {
                    // The folder stays protected rather than exposed, so the
                    // failure is surfaced for manual repair.
                    eprintln!("failed to restore ACL on {}: {error}", self.path.display());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn directory() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn dacl_can_be_read_and_reapplied() {
        let dir = directory();
        let before = dacl_sddl(dir.path()).unwrap();
        assert!(before.starts_with("D:"));
        apply_sddl(dir.path(), &before).unwrap();
        assert_eq!(dacl_sddl(dir.path()).unwrap(), before);
    }

    #[test]
    fn deny_blocks_deletion_and_clear_removes_only_that_sid() {
        let dir = directory();
        let child = dir.path().join("keep.txt");
        fs::write(&child, b"data").unwrap();

        add_deny(dir.path(), SID_EVERYONE, delete_mask()).unwrap();
        assert!(dacl_sddl(dir.path()).unwrap().contains("(D;"));
        assert!(fs::remove_file(&child).is_err(), "deny rule must stop the delete");

        assert_eq!(clear_deny(dir.path(), SID_ADMINISTRATORS).unwrap(), 0);
        assert!(dacl_sddl(dir.path()).unwrap().contains("(D;"));

        assert!(clear_deny(dir.path(), SID_EVERYONE).unwrap() >= 1);
        assert!(!dacl_sddl(dir.path()).unwrap().contains("(D;"));
        fs::remove_file(&child).unwrap();
    }

    #[test]
    fn write_deny_blocks_writes_but_keeps_the_folder_readable() {
        let dir = directory();
        let existing = dir.path().join("kept.txt");
        fs::write(&existing, b"data").unwrap();

        add_deny(dir.path(), SID_EVERYONE, rights_mask(AclRights::Write)).unwrap();
        assert!(fs::write(dir.path().join("new.txt"), b"x").is_err(), "writes must be blocked");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "the folder must stay listable");

        assert!(clear_deny(dir.path(), SID_EVERYONE).unwrap() >= 1);
        fs::write(dir.path().join("new.txt"), b"x").unwrap();
    }

    #[test]
    fn deny_on_a_single_file_applies_without_inheritance_flags() {
        let dir = directory();
        let file = dir.path().join("store.db");
        fs::write(&file, b"db").unwrap();
        add_deny(&file, SID_EVERYONE, rights_mask(AclRights::Full)).unwrap();
        assert!(fs::read(&file).is_err(), "the deny must take effect on a file");
        clear_deny(&file, SID_EVERYONE).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"db");
    }

    #[test]
    fn guard_restores_on_scope_exit() {
        let dir = directory();
        let before = dacl_sddl(dir.path()).unwrap();
        {
            let _guard = AclGuard::deny(dir.path(), SID_EVERYONE, delete_mask()).unwrap();
            assert_ne!(dacl_sddl(dir.path()).unwrap(), before);
        }
        assert_eq!(dacl_sddl(dir.path()).unwrap(), before);
    }

    #[test]
    fn guard_restores_while_a_panic_unwinds() {
        let dir = directory();
        let before = dacl_sddl(dir.path()).unwrap();
        let path = dir.path().to_path_buf();
        let outcome = std::panic::catch_unwind(move || {
            let _guard = AclGuard::deny(&path, SID_EVERYONE, delete_mask()).unwrap();
            panic!("simulated uninstaller crash");
        });
        assert!(outcome.is_err());
        assert_eq!(dacl_sddl(dir.path()).unwrap(), before);
    }

    #[test]
    fn guard_restores_on_early_error_return() {
        fn work(path: &Path) -> Result<()> {
            let _guard = AclGuard::deny(path, SID_EVERYONE, delete_mask())?;
            Err(EngineError::Process { program: "OneDriveSetup.exe".into(), code: 1, detail: "failed".into() })
        }
        let dir = directory();
        let before = dacl_sddl(dir.path()).unwrap();
        assert!(work(dir.path()).is_err());
        assert_eq!(dacl_sddl(dir.path()).unwrap(), before);
    }

    #[test]
    fn explicit_restore_reports_success() {
        let dir = directory();
        let before = dacl_sddl(dir.path()).unwrap();
        AclGuard::deny(dir.path(), SID_EVERYONE, delete_mask()).unwrap().restore().unwrap();
        assert_eq!(dacl_sddl(dir.path()).unwrap(), before);
    }

    #[test]
    fn invalid_sid_is_an_error_and_changes_nothing() {
        let dir = directory();
        let before = dacl_sddl(dir.path()).unwrap();
        assert!(add_deny(dir.path(), "not-a-sid", delete_mask()).is_err());
        assert_eq!(dacl_sddl(dir.path()).unwrap(), before);
    }

    #[test]
    fn missing_path_reports_not_found() {
        let error = dacl_sddl(Path::new(r"C:\WindowCraft\does\not\exist")).unwrap_err();
        assert!(error.is_not_found(), "{error}");
    }
}
