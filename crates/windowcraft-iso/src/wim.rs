// Made by WindowCraft Dev Team
//! `wimgapi.dll` bindings with RAII handles.

use crate::error::{IsoError, Result};
use std::ffi::c_void;
use std::path::Path;
use windows::Win32::Foundation::{GetLastError, HANDLE, HLOCAL, LocalFree};
use windows::core::{BOOL, PCWSTR};
use windowcraft_engine::dynlib::DynLib;
use windowcraft_engine::win::{from_wide, pcwstr, wide};

const WIM_GENERIC_READ: u32 = 0x8000_0000;
const WIM_GENERIC_WRITE: u32 = 0x4000_0000;
const WIM_OPEN_EXISTING: u32 = 3;
const WIM_CREATE_ALWAYS: u32 = 2;
const WIM_COMPRESS_LZX: u32 = 2;

type CreateFile = unsafe extern "system" fn(PCWSTR, u32, u32, u32, u32, *mut u32) -> HANDLE;
type CloseHandle = unsafe extern "system" fn(HANDLE) -> BOOL;
type SetTemporaryPath = unsafe extern "system" fn(HANDLE, PCWSTR) -> BOOL;
type GetImageCount = unsafe extern "system" fn(HANDLE) -> u32;
type LoadImage = unsafe extern "system" fn(HANDLE, u32) -> HANDLE;
type GetImageInformation = unsafe extern "system" fn(HANDLE, *mut *mut c_void, *mut u32) -> BOOL;
type SplitFile = unsafe extern "system" fn(HANDLE, PCWSTR, *mut i64, u32) -> BOOL;
type ExportImage = unsafe extern "system" fn(HANDLE, HANDLE, u32) -> BOOL;

fn last_error(operation: &'static str, target: &str) -> IsoError {
    // SAFETY: Reads the calling thread's last-error value.
    let code = unsafe { GetLastError() };
    IsoError::Wim { operation, target: target.to_owned(), code: code.0 }
}

struct Api {
    library: DynLib,
    close: CloseHandle,
}

impl Api {
    fn load() -> Result<Self> {
        let library = DynLib::load("wimgapi.dll")?;
        // SAFETY: The signature matches wimgapi.h.
        let close = unsafe { library.symbol::<CloseHandle>(c"WIMCloseHandle")? };
        Ok(Self { library, close })
    }
}

/// An open image inside a WIM.
pub struct WimImage<'a> {
    wim: &'a WimFile,
    handle: HANDLE,
}

impl Drop for WimImage<'_> {
    fn drop(&mut self) {
        // SAFETY: The handle is owned by this guard and closed exactly once.
        let _ = unsafe { (self.wim.api.close)(self.handle) };
    }
}

/// An open WIM or ESD, closed on every exit path.
pub struct WimFile {
    api: Api,
    handle: HANDLE,
    label: String,
}

// SAFETY: WIM handles are process-wide and not tied to the opening thread.
unsafe impl Send for WimFile {}

impl Drop for WimFile {
    fn drop(&mut self) {
        // SAFETY: The handle is owned by this guard and closed exactly once.
        let _ = unsafe { (self.api.close)(self.handle) };
    }
}

impl WimFile {
    fn open_with(path: &Path, access: u32, disposition: u32, compression: u32) -> Result<Self> {
        let api = Api::load()?;
        let label = path.display().to_string();
        // SAFETY: The signature matches wimgapi.h.
        let create = unsafe { api.library.symbol::<CreateFile>(c"WIMCreateFile")? };
        let wide_path = wide(path);
        let mut created = 0;
        // SAFETY: The path is null-terminated and `created` is an out pointer.
        let handle = unsafe { create(pcwstr(&wide_path), access, disposition, 0, compression, &mut created) };
        if handle.0.is_null() {
            return Err(last_error("WIMCreateFile", &label));
        }
        let file = Self { api, handle, label };
        file.set_temporary_path(&std::env::temp_dir())?;
        Ok(file)
    }

    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with(path, WIM_GENERIC_READ, WIM_OPEN_EXISTING, 0)
    }

    pub fn create(path: &Path) -> Result<Self> {
        Self::open_with(path, WIM_GENERIC_WRITE | WIM_GENERIC_READ, WIM_CREATE_ALWAYS, WIM_COMPRESS_LZX)
    }

    fn set_temporary_path(&self, path: &Path) -> Result<()> {
        // SAFETY: The signature matches wimgapi.h.
        let set = unsafe { self.api.library.symbol::<SetTemporaryPath>(c"WIMSetTemporaryPath")? };
        let wide_path = wide(path);
        // SAFETY: The handle is live and the path outlives the call.
        if !unsafe { set(self.handle, pcwstr(&wide_path)) }.as_bool() {
            return Err(last_error("WIMSetTemporaryPath", &self.label));
        }
        Ok(())
    }

    pub fn image_count(&self) -> Result<u32> {
        // SAFETY: The signature matches wimgapi.h.
        let count = unsafe { self.api.library.symbol::<GetImageCount>(c"WIMGetImageCount")? };
        // SAFETY: The handle is live.
        Ok(unsafe { count(self.handle) })
    }

    fn load_image(&self, index: u32) -> Result<WimImage<'_>> {
        // SAFETY: The signature matches wimgapi.h.
        let load = unsafe { self.api.library.symbol::<LoadImage>(c"WIMLoadImage")? };
        // SAFETY: The handle is live.
        let handle = unsafe { load(self.handle, index) };
        if handle.0.is_null() {
            return Err(last_error("WIMLoadImage", &self.label));
        }
        Ok(WimImage { wim: self, handle })
    }

    /// Reads the XML metadata of one image.
    pub fn image_xml(&self, index: u32) -> Result<String> {
        let image = self.load_image(index)?;
        // SAFETY: The signature matches wimgapi.h.
        let get = unsafe { self.api.library.symbol::<GetImageInformation>(c"WIMGetImageInformation")? };
        let (mut buffer, mut size) = (core::ptr::null_mut(), 0u32);
        // SAFETY: Both out pointers are valid. The buffer is LocalAlloc'd.
        if !unsafe { get(image.handle, &mut buffer, &mut size) }.as_bool() {
            return Err(last_error("WIMGetImageInformation", &self.label));
        }
        struct Guard(*mut c_void);
        impl Drop for Guard {
            fn drop(&mut self) {
                // SAFETY: Allocated by wimgapi with LocalAlloc and freed once.
                unsafe {
                    LocalFree(Some(HLOCAL(self.0)));
                }
            }
        }
        let _guard = Guard(buffer);
        // SAFETY: The API wrote `size` bytes of UTF-16 text at `buffer`.
        let units = unsafe { core::slice::from_raw_parts(buffer.cast::<u16>(), size as usize / 2) };
        Ok(from_wide(units.strip_prefix(&[0xFEFF]).unwrap_or(units)))
    }

    pub fn images(&self) -> Result<Vec<ImageInfo>> {
        (1..=self.image_count()?).map(|index| Ok(ImageInfo::parse(index, &self.image_xml(index)?))).collect()
    }

    /// Splits into `.swm` chunks no larger than `part_bytes`.
    pub fn split(&self, destination: &Path, part_bytes: u64) -> Result<()> {
        // SAFETY: The signature matches wimgapi.h.
        let split = unsafe { self.api.library.symbol::<SplitFile>(c"WIMSplitFile")? };
        let path = wide(destination);
        let mut size = i64::try_from(part_bytes).map_err(|_| IsoError::Invalid("part size".to_owned()))?;
        // SAFETY: The path and size outlive the call.
        if !unsafe { split(self.handle, pcwstr(&path), &mut size, 0) }.as_bool() {
            return Err(last_error("WIMSplitFile", &self.label));
        }
        Ok(())
    }

    /// Copies one image into another WIM, which also converts ESD to WIM.
    pub fn export_image(&self, index: u32, destination: &Self) -> Result<()> {
        let image = self.load_image(index)?;
        // SAFETY: The signature matches wimgapi.h.
        let export = unsafe { self.api.library.symbol::<ExportImage>(c"WIMExportImage")? };
        // SAFETY: Both handles are live.
        if !unsafe { export(image.handle, destination.handle, 0) }.as_bool() {
            return Err(last_error("WIMExportImage", &self.label));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInfo {
    pub index: u32,
    pub name: String,
    pub edition: String,
    pub architecture: String,
    pub build: String,
}

fn tag(xml: &str, name: &str) -> String {
    let (open, close) = (format!("<{name}>"), format!("</{name}>"));
    xml.find(&open)
        .and_then(|start| {
            let rest = &xml[start + open.len()..];
            rest.find(&close).map(|end| rest[..end].trim().to_owned())
        })
        .unwrap_or_default()
}

impl ImageInfo {
    pub fn parse(index: u32, xml: &str) -> Self {
        let architecture = match tag(xml, "ARCH").as_str() {
            "9" => "x64",
            "12" => "ARM64",
            "0" => "x86",
            _ => "unknown",
        };
        Self {
            index,
            name: tag(xml, "NAME"),
            edition: tag(xml, "EDITIONID"),
            architecture: architecture.to_owned(),
            build: tag(xml, "BUILD"),
        }
    }

    pub fn is_windows_11(&self) -> bool {
        self.name.contains("Windows 11") || self.build.parse::<u32>().is_ok_and(|build| build >= 22_000)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    type CaptureImage = unsafe extern "system" fn(HANDLE, PCWSTR, u32) -> HANDLE;

    /// Captures a folder so the real wimgapi calls can be exercised.
    pub(crate) fn capture(wim: &WimFile, source: &Path) -> bool {
        // SAFETY: The signature matches wimgapi.h.
        let capture = unsafe { wim.api.library.symbol::<CaptureImage>(c"WIMCaptureImage") }.unwrap();
        let path = wide(source);
        // SAFETY: The handle is live and the path outlives the call.
        let image = unsafe { capture(wim.handle, pcwstr(&path), 0) };
        if image.0.is_null() {
            return false;
        }
        // SAFETY: The image handle came from the capture above.
        let _ = unsafe { (wim.api.close)(image) };
        true
    }

    #[test]
    fn real_wim_files_roundtrip_through_wimgapi() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("content");
        std::fs::create_dir(&source).unwrap();
        // Incompressible, so the image stays larger than one split part.
        let mut state = 0x2545_F491_u32;
        let noise: Vec<u8> = (0..8 * 1024 * 1024)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (state >> 24) as u8
            })
            .collect();
        std::fs::write(source.join("setup.exe"), noise).unwrap();

        let original = dir.path().join("install.wim");
        {
            let wim = WimFile::create(&original).unwrap();
            if !capture(&wim, &source) {
                // SAFETY: Reads the last-error value of the failed capture.
                let code = unsafe { GetLastError() }.0;
                eprintln!("skipped: capture needs backup privileges here (error {code})");
                assert!(matches!(code, 5 | 1314), "only missing privileges may skip this test, got {code}");
                return;
            }
        }

        let wim = WimFile::open(&original).unwrap();
        assert_eq!(wim.image_count().unwrap(), 1);
        let xml = wim.image_xml(1).unwrap();
        assert!(xml.contains("<IMAGE"), "{xml}");
        assert_eq!(wim.images().unwrap().len(), 1);

        let exported = dir.path().join("exported.wim");
        wim.export_image(1, &WimFile::create(&exported).unwrap()).unwrap();
        assert_eq!(WimFile::open(&exported).unwrap().image_count().unwrap(), 1);

        let parts = dir.path().join("install.swm");
        wim.split(&parts, 2 * 1024 * 1024).unwrap();
        assert!(parts.exists());
        assert!(dir.path().join("install2.swm").exists(), "the image must span several parts");
    }

    const SAMPLE: &str = "<IMAGE INDEX=\"6\"><NAME>Windows 11 Pro</NAME><WINDOWS><ARCH>9</ARCH>\
        <EDITIONID>Professional</EDITIONID><VERSION><BUILD>26100</BUILD></VERSION></WINDOWS></IMAGE>";

    #[test]
    fn parses_image_metadata() {
        let info = ImageInfo::parse(6, SAMPLE);
        assert_eq!(info.name, "Windows 11 Pro");
        assert_eq!(info.edition, "Professional");
        assert_eq!(info.architecture, "x64");
        assert!(info.is_windows_11());
    }

    #[test]
    fn rejects_older_windows() {
        let xml = "<NAME>Windows 10 Pro</NAME><BUILD>19045</BUILD>";
        assert!(!ImageInfo::parse(1, xml).is_windows_11());
    }

    #[test]
    fn opening_a_missing_wim_is_an_error() {
        assert!(WimFile::open(Path::new(r"C:\no\such.wim")).is_err());
    }

    #[test]
    fn opening_garbage_is_an_error_and_releases_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake.wim");
        std::fs::write(&path, b"not a wim").unwrap();
        assert!(WimFile::open(&path).is_err());
        std::fs::remove_file(&path).unwrap();
    }
}
