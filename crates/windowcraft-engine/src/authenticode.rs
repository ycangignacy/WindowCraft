// Made by WindowCraft Dev Team
//! Authenticode signature verification and publisher lookup.

use crate::win::{from_wide, pcwstr, wide};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Security::Cryptography::Catalog::*;
use windows::Win32::Security::WinTrust::*;
use windows::core::{GUID, PCWSTR, w};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trust {
    pub trusted: bool,
    pub publisher: Option<String>,
}

/// Verifies a file, embedded signature first, then the system catalogs.
///
/// Most inbox Windows binaries carry no embedded signature at all: they are
/// signed through catalogs, so stopping at the first check calls them unsigned.
/// No UI and no network revocation checks, so scans stay fast and work offline.
pub fn verify_file(path: &Path) -> Trust {
    let embedded = verify_embedded(path);
    if embedded.trusted { embedded } else { verify_catalog(path).unwrap_or(embedded) }
}

fn verify_embedded(path: &Path) -> Trust {
    let file_path = wide(path);
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: pcwstr(&file_path),
        ..Default::default()
    };
    run_trust(WTD_CHOICE_FILE, WINTRUST_DATA_0 { pFile: &raw mut file })
}

struct CatalogAdmin(isize);

impl Drop for CatalogAdmin {
    fn drop(&mut self) {
        // SAFETY: The context is owned by this guard and released once.
        let _ = unsafe { CryptCATAdminReleaseContext(self.0, 0) };
    }
}

struct CatalogEntry<'a>(&'a CatalogAdmin, isize);

impl Drop for CatalogEntry<'_> {
    fn drop(&mut self) {
        // SAFETY: The entry came from this admin context and is released once.
        let _ = unsafe { CryptCATAdminReleaseCatalogContext(self.0.0, self.1, 0) };
    }
}

fn verify_catalog(path: &Path) -> Option<Trust> {
    let file = std::fs::File::open(path).ok()?;
    let handle = HANDLE(file.as_raw_handle());

    let mut raw_admin = 0;
    // SAFETY: The out pointer is valid and the algorithm name is static.
    unsafe { CryptCATAdminAcquireContext2(&mut raw_admin, None, w!("SHA256"), None, None) }.ok()?;
    let admin = CatalogAdmin(raw_admin);

    let mut size = 0u32;
    // SAFETY: A null buffer asks for the size, so the reported error is expected.
    let _ = unsafe { CryptCATAdminCalcHashFromFileHandle2(admin.0, handle, &mut size, None, None) };
    if size == 0 {
        return None;
    }
    let mut hash = vec![0u8; size as usize];
    // SAFETY: `hash` holds `size` writable bytes.
    unsafe { CryptCATAdminCalcHashFromFileHandle2(admin.0, handle, &mut size, Some(hash.as_mut_ptr()), None) }.ok()?;

    // SAFETY: The context is live and `hash` outlives the call.
    let entry = unsafe { CryptCATAdminEnumCatalogFromHash(admin.0, &hash, None, None) };
    if entry == 0 {
        return None;
    }
    let entry = CatalogEntry(&admin, entry);

    let mut info = CATALOG_INFO { cbStruct: size_of::<CATALOG_INFO>() as u32, ..Default::default() };
    // SAFETY: `info` has cbStruct set and the entry is live.
    unsafe { CryptCATCatalogInfoFromContext(entry.1, &mut info, 0) }.ok()?;

    let member = wide(path);
    let tag: Vec<u16> = crate::download::hex(&hash).to_uppercase().encode_utf16().chain(Some(0)).collect();
    let mut catalog = WINTRUST_CATALOG_INFO {
        cbStruct: size_of::<WINTRUST_CATALOG_INFO>() as u32,
        pcwszCatalogFilePath: PCWSTR(info.wszCatalogFile.as_ptr()),
        pcwszMemberTag: pcwstr(&tag),
        pcwszMemberFilePath: pcwstr(&member),
        hMemberFile: handle,
        hCatAdmin: admin.0,
        ..Default::default()
    };
    debug_assert!(!from_wide(&info.wszCatalogFile).is_empty());
    let trust = run_trust(WTD_CHOICE_CATALOG, WINTRUST_DATA_0 { pCatalog: &raw mut catalog });
    trust.trusted.then_some(trust)
}

/// Runs one verification and always closes its state.
fn run_trust(choice: WINTRUST_DATA_UNION_CHOICE, subject: WINTRUST_DATA_0) -> Trust {
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: choice,
        Anonymous: subject,
        dwStateAction: WTD_STATEACTION_VERIFY,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    };
    let mut action: GUID = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let invalid_window = HWND(core::ptr::dangling_mut::<core::ffi::c_void>().wrapping_sub(2));
    // SAFETY: `data` and the subject outlive the call, and INVALID_HANDLE_VALUE
    // as the window requests no UI.
    let status = unsafe { WinVerifyTrust(invalid_window, &mut action, (&raw mut data).cast()) };

    let publisher = (status == 0).then(|| publisher_from_state(&data)).flatten();

    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: Releases the state opened by the verify call above, on every path.
    let _ = unsafe { WinVerifyTrust(invalid_window, &mut action, (&raw mut data).cast()) };
    Trust { trusted: status == 0, publisher }
}

fn publisher_from_state(data: &WINTRUST_DATA) -> Option<String> {
    use windows::Win32::Security::Cryptography::{CERT_NAME_SIMPLE_DISPLAY_TYPE, CertGetNameStringW};
    // SAFETY: The state handle is valid until the close call that runs after
    // this function, and the helper functions tolerate a failed chain.
    unsafe {
        let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
        if provider.is_null() {
            return None;
        }
        let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
        if signer.is_null() {
            return None;
        }
        let certificate = WTHelperGetProvCertFromChain(signer, 0);
        if certificate.is_null() {
            return None;
        }
        let mut name = [0u16; 256];
        let length = CertGetNameStringW((*certificate).pCert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut name));
        (length > 1).then(|| crate::win::from_wide(&name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win::system32;

    #[test]
    fn unsigned_file_is_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.exe");
        std::fs::write(&path, b"MZ").unwrap();
        let trust = verify_file(&path);
        assert!(!trust.trusted);
        assert_eq!(trust.publisher, None);
    }

    #[test]
    fn missing_file_is_not_trusted() {
        assert!(!verify_file(Path::new(r"C:\no\such\file.exe")).trusted);
    }

    #[test]
    fn catalog_signed_system_binaries_are_trusted() {
        let trust = verify_file(&system32("cmd.exe"));
        assert!(trust.trusted, "cmd.exe is signed through a system catalog");
        let publisher = trust.publisher.unwrap_or_default();
        assert!(publisher.contains("Microsoft"), "{publisher}");
    }

    #[test]
    fn repeated_verification_does_not_crash() {
        for _ in 0..3 {
            verify_file(&system32("notepad.exe"));
        }
    }
}
