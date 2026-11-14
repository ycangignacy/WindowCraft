// Made by Nordan
//! Read-only ISO mounting through the virtual disk API.

use crate::error::{IsoError, Result};
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{HANDLE, WIN32_ERROR};
use windows::Win32::Storage::FileSystem::QueryDosDeviceW;
use windows::Win32::Storage::Vhd::*;
use windows::core::{GUID, PWSTR};
use windowcraft_engine::EngineError;
use windowcraft_engine::win::{HandleGuard, from_wide, pcwstr, wide};

const VENDOR_MICROSOFT: GUID = GUID::from_u128(0xec984aec_a0f9_47e9_901f_71415a66345b);
const DEVICE_ISO: u32 = 1;

fn check(status: WIN32_ERROR, operation: &'static str, path: &Path) -> Result<()> {
    if status.is_ok() {
        Ok(())
    } else {
        Err(IsoError::Engine(EngineError::win32(operation, path.display().to_string(), status.0)))
    }
}

/// A mounted ISO, detached on every exit path.
pub struct MountedIso {
    handle: HandleGuard,
    root: PathBuf,
}

impl MountedIso {
    pub fn attach(path: &Path) -> Result<Self> {
        let storage = VIRTUAL_STORAGE_TYPE { DeviceId: DEVICE_ISO, VendorId: VENDOR_MICROSOFT };
        let wide_path = wide(path);
        let mut raw = HANDLE::default();
        // SAFETY: All pointers reference locals that outlive the call.
        let status = unsafe {
            OpenVirtualDisk(
                &storage,
                pcwstr(&wide_path),
                VIRTUAL_DISK_ACCESS_READ,
                OPEN_VIRTUAL_DISK_FLAG_NONE,
                None,
                &mut raw,
            )
        };
        check(status, "OpenVirtualDisk", path)?;
        let handle = HandleGuard(raw);

        // A drive letter is required so the volume can be found below.
        // SAFETY: The handle is live and optional parameters are None.
        let status =
            unsafe { AttachVirtualDisk(handle.raw(), None, ATTACH_VIRTUAL_DISK_FLAG_READ_ONLY, 0, None, None) };
        check(status, "AttachVirtualDisk", path)?;

        let mut iso = Self { handle, root: PathBuf::new() };
        iso.root = iso.locate(path)?;
        Ok(iso)
    }

    fn locate(&self, path: &Path) -> Result<PathBuf> {
        let mut units = [0u16; 260];
        let mut size = size_of_val(&units) as u32;
        // SAFETY: The buffer holds `size` bytes.
        let status = unsafe { GetVirtualDiskPhysicalPath(self.handle.raw(), &mut size, PWSTR(units.as_mut_ptr())) };
        check(status, "GetVirtualDiskPhysicalPath", path)?;
        let device = from_wide(&units);
        let number = device.rsplit("CDROM").next().unwrap_or_default().to_owned();
        let wanted = format!(r"\Device\CdRom{number}");

        for _ in 0..50 {
            for letter in 'A'..='Z' {
                let name = wide(format!("{letter}:"));
                let mut target = [0u16; 260];
                // SAFETY: The name is null-terminated and the buffer outlives the call.
                let written = unsafe { QueryDosDeviceW(pcwstr(&name), Some(&mut target)) };
                if written > 0 && from_wide(&target).eq_ignore_ascii_case(&wanted) {
                    return Ok(PathBuf::from(format!(r"{letter}:\")));
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(IsoError::Invalid("the mounted ISO received no drive letter".to_owned()))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Drop for MountedIso {
    fn drop(&mut self) {
        // SAFETY: The handle is live. A detach failure leaves the ISO mounted
        // read-only, which is harmless and clears when the handle closes.
        let _ = unsafe { DetachVirtualDisk(self.handle.raw(), DETACH_VIRTUAL_DISK_FLAG_NONE, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_iso_is_an_error() {
        assert!(MountedIso::attach(Path::new(r"C:\no\such.iso")).is_err());
    }

    #[test]
    fn check_maps_ok_access_denied_and_not_found_win32_codes() {
        let iso_path = Path::new(r"C:\test\win11.iso");
        assert!(check(WIN32_ERROR(0), "OpenVirtualDisk", iso_path).is_ok());

        let denied = check(WIN32_ERROR(5), "AttachVirtualDisk", iso_path).unwrap_err();
        assert!(matches!(
            denied,
            IsoError::Engine(EngineError::NeedsElevation(detail))
                if detail == r"AttachVirtualDisk on C:\test\win11.iso"
        ));

        let missing = check(WIN32_ERROR(2), "OpenVirtualDisk", iso_path).unwrap_err();
        assert!(matches!(&missing, IsoError::Engine(err) if err.is_not_found()));
    }

    #[test]
    fn mounted_iso_exposes_root_and_reports_errors_for_invalid_handle_or_non_iso_file() {
        let dummy = MountedIso { handle: HandleGuard(HANDLE::default()), root: PathBuf::from(r"E:\") };
        assert_eq!(dummy.root(), Path::new(r"E:\"));

        let locate_err = dummy.locate(Path::new(r"C:\dummy.iso")).unwrap_err();
        assert!(matches!(
            locate_err,
            IsoError::Engine(EngineError::Win32 { operation: "GetVirtualDiskPhysicalPath", .. })
        ));
        drop(dummy);

        let dir = tempfile::tempdir().unwrap();
        let fake_iso = dir.path().join("invalid.iso");
        std::fs::write(&fake_iso, b"not an iso image").unwrap();
        let attach_err = MountedIso::attach(&fake_iso).err().unwrap();
        assert!(matches!(
            attach_err,
            IsoError::Engine(EngineError::Win32 { operation: "OpenVirtualDisk", .. })
        ));
    }
}
