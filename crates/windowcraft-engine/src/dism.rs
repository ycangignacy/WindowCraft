// Made by ycangignacy
//! DISM API bindings.
//!
//! The DLL is loaded at runtime because the import library ships only with the
//! ADK. Every heap object and session has a guard so an early return cannot
//! leak it, and `DismShutdown` runs when the last user finishes.

use crate::dynlib::DynLib;
use crate::error::{EngineError, Result};
use crate::win::{from_pcwstr, pcwstr, wide};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use windows::Win32::Foundation::{HANDLE, SYSTEMTIME};
use windows::core::{BOOL, HRESULT, PCWSTR};

type Session = u32;
type Progress = Option<unsafe extern "system" fn(u32, u32, *mut c_void)>;

const ONLINE_IMAGE: &str = "DISM_{53BFAE52-B167-4E2F-A258-0A37B57FF845}";
const LOG_ERRORS: i32 = 0;
const PACKAGE_NONE: i32 = 0;
const IDENTIFIER_INDEX: i32 = 0;
const MOUNT_READ_WRITE: u32 = 0;
const COMMIT_IMAGE: u32 = 0;
const DISCARD_IMAGE: u32 = 1;
const RESERVED_DISABLED: u32 = 0;
const RESERVED_ENABLED: u32 = 1;
/// HRESULT for "a reboot is required", which counts as success here.
const S_REBOOT_REQUIRED: i32 = 3010;

#[repr(C, packed)]
struct DismString {
    value: PCWSTR,
}

#[repr(C, packed)]
pub struct DismDriverPackage {
    published_name: PCWSTR,
    original_file_name: PCWSTR,
    in_box: BOOL,
    catalog_file: PCWSTR,
    class_name: PCWSTR,
    class_guid: PCWSTR,
    class_description: PCWSTR,
    boot_critical: BOOL,
    driver_signature: i32,
    provider_name: PCWSTR,
    date: SYSTEMTIME,
    major_version: u32,
    minor_version: u32,
    build: u32,
    revision: u32,
}

/// Leading fields of `DismFeatureInfo`. Only the state is read, and DISM
/// allocates the whole structure, so the tail can be left out.
#[repr(C, packed)]
struct DismFeatureInfo {
    feature_name: PCWSTR,
    feature_state: i32,
}

/// `DismPackageFeatureState` values that mean the feature is on or turning on.
const STATE_INSTALLED: i32 = 4;
const STATE_INSTALL_PENDING: i32 = 5;
const STATE_PARTIALLY_INSTALLED: i32 = 7;

/// The parts of a driver package entry the driver export reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverInfo {
    pub inf_path: PathBuf,
    pub in_box: bool,
}

struct Api {
    library: DynLib,
    shutdown: unsafe extern "system" fn() -> HRESULT,
    open_session: unsafe extern "system" fn(PCWSTR, PCWSTR, PCWSTR, *mut Session) -> HRESULT,
    close_session: unsafe extern "system" fn(Session) -> HRESULT,
    delete: unsafe extern "system" fn(*const c_void) -> HRESULT,
    last_error: unsafe extern "system" fn(*mut *mut DismString) -> HRESULT,
}

static SHARED: Mutex<Weak<Api>> = Mutex::new(Weak::new());

impl Api {
    /// Shares one initialized runtime, since DISM allows only one per process.
    fn acquire() -> Result<Arc<Self>> {
        let mut shared = SHARED.lock().map_err(|_| EngineError::Invalid("DISM runtime lock poisoned".into()))?;
        if let Some(api) = shared.upgrade() {
            return Ok(api);
        }
        let library = DynLib::load("dismapi.dll")?;
        type Initialize = unsafe extern "system" fn(i32, PCWSTR, PCWSTR) -> HRESULT;
        // SAFETY: Every signature below is copied from DismApi.h.
        let (initialize, shutdown, open_session, close_session, delete, last_error) = unsafe {
            (
                library.symbol::<Initialize>(c"DismInitialize")?,
                library.symbol(c"DismShutdown")?,
                library.symbol(c"DismOpenSession")?,
                library.symbol(c"DismCloseSession")?,
                library.symbol(c"DismDelete")?,
                library.symbol(c"DismGetLastErrorMessage")?,
            )
        };
        // SAFETY: Null log and scratch paths select the defaults.
        unsafe { initialize(LOG_ERRORS, PCWSTR::null(), PCWSTR::null()) }
            .ok()
            .map_err(|error| EngineError::windows("DismInitialize", "DISM", &error))?;
        let api = Arc::new(Self { library, shutdown, open_session, close_session, delete, last_error });
        *shared = Arc::downgrade(&api);
        Ok(api)
    }

    fn check(&self, status: HRESULT, operation: &'static str, target: &str) -> Result<()> {
        // DISM reports a pending restart in both the plain and the HRESULT form.
        if status.is_ok() || status.0 == S_REBOOT_REQUIRED || status == HRESULT::from_win32(3010) {
            return Ok(());
        }
        let mut text: *mut DismString = core::ptr::null_mut();
        // SAFETY: `text` is a valid out pointer. The result is released below.
        let detail = if unsafe { (self.last_error)(&mut text) }.is_ok() && !text.is_null() {
            // SAFETY: On success `text` points to a DismString, read unaligned
            // because the structure is packed.
            let value = unsafe { core::ptr::read_unaligned(&raw const (*text).value) };
            // SAFETY: `value` is a valid string owned by `text`.
            let message = unsafe { from_pcwstr(value) };
            // SAFETY: `text` was allocated by DISM and is released once.
            let _ = unsafe { (self.delete)(text.cast()) };
            message
        } else {
            String::new()
        };
        let detail = describe(detail, status);
        Err(EngineError::Win32 { operation, target: target.to_owned(), code: status.0.cast_unsigned(), message: detail })
    }
}

/// DISM's own text, then the system's text for the code. Many CBS codes have
/// neither, so the last resort points at the log that names the cause.
fn describe(dism_text: String, status: HRESULT) -> String {
    let dism_text = dism_text.trim();
    if !dism_text.is_empty() {
        return dism_text.to_owned();
    }
    let system_text = status.message();
    let system_text = system_text.trim();
    if !system_text.is_empty() {
        return system_text.to_owned();
    }
    let windows = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    format!("DISM gave no description; the cause is in {windows}\\Logs\\DISM\\dism.log. Windows may be installing updates, so restart and try again")
}

impl Drop for Api {
    fn drop(&mut self) {
        // Held across shutdown so a concurrent `acquire` cannot initialize a
        // second runtime while this one is tearing down.
        let _lock = SHARED.lock();
        // SAFETY: The last session closed before this drop, since each one
        // holds an Arc to this value.
        let _ = unsafe { (self.shutdown)() };
    }
}

/// An open servicing session, closed on every exit path.
pub struct DismSession {
    api: Arc<Api>,
    session: Session,
}

impl Drop for DismSession {
    fn drop(&mut self) {
        // SAFETY: The session is valid and closed exactly once.
        let _ = unsafe { (self.api.close_session)(self.session) };
    }
}

impl DismSession {
    pub fn online() -> Result<Self> {
        Self::open(ONLINE_IMAGE)
    }

    /// Opens a session on a mounted image directory.
    pub fn image(mount: &Path) -> Result<Self> {
        Self::open(&mount.display().to_string())
    }

    fn open(image: &str) -> Result<Self> {
        let api = Api::acquire()?;
        let wide_image = wide(image);
        let mut session = 0;
        // SAFETY: The image string is null-terminated and the out pointer is valid.
        let status =
            unsafe { (api.open_session)(pcwstr(&wide_image), PCWSTR::null(), PCWSTR::null(), &mut session) };
        api.check(status, "DismOpenSession", image)?;
        Ok(Self { api, session })
    }

    pub fn enable_feature(&self, name: &str) -> Result<()> {
        type Enable = unsafe extern "system" fn(
            Session, PCWSTR, PCWSTR, i32, BOOL, *const PCWSTR, u32, BOOL, HANDLE, Progress, *mut c_void,
        ) -> HRESULT;
        // SAFETY: The signature matches DismEnableFeature in DismApi.h.
        let enable = unsafe { self.api.library.symbol::<Enable>(c"DismEnableFeature")? };
        let wide_name = wide(name);
        // SAFETY: The name outlives the call. Sources are empty, EnableAll
        // pulls in parent features, and no cancel event or callback is used.
        let status = unsafe {
            enable(
                self.session,
                pcwstr(&wide_name),
                PCWSTR::null(),
                PACKAGE_NONE,
                BOOL::from(false),
                core::ptr::null(),
                0,
                BOOL::from(true),
                HANDLE::default(),
                None,
                core::ptr::null_mut(),
            )
        };
        self.api.check(status, "DismEnableFeature", name)
    }

    pub fn disable_feature(&self, name: &str) -> Result<()> {
        type Disable =
            unsafe extern "system" fn(Session, PCWSTR, PCWSTR, BOOL, HANDLE, Progress, *mut c_void) -> HRESULT;
        // SAFETY: The signature matches DismDisableFeature in DismApi.h.
        let disable = unsafe { self.api.library.symbol::<Disable>(c"DismDisableFeature")? };
        let wide_name = wide(name);
        // SAFETY: The name outlives the call and optional arguments are null.
        let status = unsafe {
            disable(
                self.session,
                pcwstr(&wide_name),
                PCWSTR::null(),
                BOOL::from(false),
                HANDLE::default(),
                None,
                core::ptr::null_mut(),
            )
        };
        self.api.check(status, "DismDisableFeature", name)
    }

    /// Whether an optional feature is enabled, pending enable, or partly
    /// installed. All three count as on, so a rollback leaves them alone.
    pub fn feature_enabled(&self, name: &str) -> Result<bool> {
        type Get = unsafe extern "system" fn(Session, PCWSTR, PCWSTR, i32, *mut *mut DismFeatureInfo) -> HRESULT;
        // SAFETY: The signature matches DismGetFeatureInfo in DismApi.h.
        let get = unsafe { self.api.library.symbol::<Get>(c"DismGetFeatureInfo")? };
        let wide_name = wide(name);
        let mut info: *mut DismFeatureInfo = core::ptr::null_mut();
        // SAFETY: The name outlives the call, the package identifier is
        // unused, and `info` is a valid out pointer released below.
        let status = unsafe { get(self.session, pcwstr(&wide_name), PCWSTR::null(), PACKAGE_NONE, &mut info) };
        self.api.check(status, "DismGetFeatureInfo", name)?;
        if info.is_null() {
            return Err(EngineError::Invalid(format!("DISM returned no information for feature {name}")));
        }
        // SAFETY: On success `info` points to a packed DismFeatureInfo.
        let state = unsafe { core::ptr::read_unaligned(&raw const (*info).feature_state) };
        // SAFETY: `info` was allocated by DISM and is released once.
        let _ = unsafe { (self.api.delete)(info.cast()) };
        Ok(is_enabled_state(state))
    }

    pub fn add_capability(&self, name: &str) -> Result<()> {
        type Add = unsafe extern "system" fn(
            Session, PCWSTR, BOOL, *const PCWSTR, u32, HANDLE, Progress, *mut c_void,
        ) -> HRESULT;
        // SAFETY: The signature matches DismAddCapability in DismApi.h.
        let add = unsafe { self.api.library.symbol::<Add>(c"DismAddCapability")? };
        let wide_name = wide(name);
        // SAFETY: The name outlives the call and optional arguments are null.
        let status = unsafe {
            add(
                self.session,
                pcwstr(&wide_name),
                BOOL::from(false),
                core::ptr::null(),
                0,
                HANDLE::default(),
                None,
                core::ptr::null_mut(),
            )
        };
        self.api.check(status, "DismAddCapability", name)
    }

    pub fn set_reserved_storage(&self, enabled: bool) -> Result<()> {
        type Set = unsafe extern "system" fn(Session, u32) -> HRESULT;
        // SAFETY: The signature matches the documented export.
        let set = unsafe { self.api.library.symbol::<Set>(c"DismSetReservedStorageState")? };
        let state = if enabled { RESERVED_ENABLED } else { RESERVED_DISABLED };
        // SAFETY: The session is valid.
        let status = unsafe { set(self.session, state) };
        self.api.check(status, "DismSetReservedStorageState", "online image")
    }

    pub fn add_driver(&self, inf: &Path) -> Result<()> {
        type Add = unsafe extern "system" fn(Session, PCWSTR, BOOL) -> HRESULT;
        // SAFETY: The signature matches DismAddDriver in DismApi.h.
        let add = unsafe { self.api.library.symbol::<Add>(c"DismAddDriver")? };
        let path = wide(inf);
        // SAFETY: The path outlives the call.
        let status = unsafe { add(self.session, pcwstr(&path), BOOL::from(false)) };
        self.api.check(status, "DismAddDriver", &inf.display().to_string())
    }

    /// Lists third-party drivers in the image. The DISM array is copied and
    /// released before returning.
    pub fn drivers(&self) -> Result<Vec<DriverInfo>> {
        type Get = unsafe extern "system" fn(Session, BOOL, *mut *mut DismDriverPackage, *mut u32) -> HRESULT;
        // SAFETY: The signature matches DismGetDrivers in DismApi.h.
        let get = unsafe { self.api.library.symbol::<Get>(c"DismGetDrivers")? };
        let mut packages: *mut DismDriverPackage = core::ptr::null_mut();
        let mut count = 0u32;
        // SAFETY: Both out pointers are valid.
        let status = unsafe { get(self.session, BOOL::from(false), &mut packages, &mut count) };
        self.api.check(status, "DismGetDrivers", "image")?;

        struct Array<'a>(&'a Api, *mut DismDriverPackage);
        impl Drop for Array<'_> {
            fn drop(&mut self) {
                if !self.1.is_null() {
                    // SAFETY: The array came from DISM and is released once.
                    let _ = unsafe { (self.0.delete)(self.1.cast()) };
                }
            }
        }
        let _array = Array(&self.api, packages);

        let mut drivers = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            // SAFETY: DISM wrote `count` packed entries. Fields are read by
            // value without forming references to unaligned data.
            let info = unsafe {
                let entry = packages.add(index);
                let read = |pointer: *const PCWSTR| from_pcwstr(core::ptr::read_unaligned(pointer));
                DriverInfo {
                    inf_path: PathBuf::from(read(&raw const (*entry).original_file_name)),
                    in_box: core::ptr::read_unaligned(&raw const (*entry).in_box).as_bool(),
                }
            };
            drivers.push(info);
        }
        Ok(drivers)
    }
}

fn is_enabled_state(state: i32) -> bool {
    matches!(state, STATE_INSTALLED | STATE_INSTALL_PENDING | STATE_PARTIALLY_INSTALLED)
}

/// Enables an optional feature on the running system.
pub fn enable_online_feature(name: &str) -> Result<()> {
    DismSession::online()?.enable_feature(name)
}

/// Disables an optional feature on the running system. The payload stays,
/// so enabling it again needs no download.
pub fn disable_online_feature(name: &str) -> Result<()> {
    DismSession::online()?.disable_feature(name)
}

/// Whether an optional feature is on in the running system.
pub fn online_feature_enabled(name: &str) -> Result<bool> {
    DismSession::online()?.feature_enabled(name)
}

/// A mounted WIM. The image is discarded on drop unless committed, so an
/// aborted build can never leave a stale mount or a half-written image.
pub struct MountedImage {
    api: Arc<Api>,
    path: PathBuf,
    active: bool,
}

type Unmount = unsafe extern "system" fn(PCWSTR, u32, HANDLE, Progress, *mut c_void) -> HRESULT;

impl MountedImage {
    pub fn mount(image: &Path, index: u32, directory: &Path) -> Result<Self> {
        type Mount = unsafe extern "system" fn(
            PCWSTR, PCWSTR, u32, PCWSTR, i32, u32, HANDLE, Progress, *mut c_void,
        ) -> HRESULT;
        let api = Api::acquire()?;
        // SAFETY: The signature matches DismMountImage in DismApi.h.
        let mount = unsafe { api.library.symbol::<Mount>(c"DismMountImage")? };
        let (wide_image, wide_directory) = (wide(image), wide(directory));
        // SAFETY: Paths outlive the call and optional arguments are null.
        let status = unsafe {
            mount(
                pcwstr(&wide_image),
                pcwstr(&wide_directory),
                index,
                PCWSTR::null(),
                IDENTIFIER_INDEX,
                MOUNT_READ_WRITE,
                HANDLE::default(),
                None,
                core::ptr::null_mut(),
            )
        };
        api.check(status, "DismMountImage", &image.display().to_string())?;
        Ok(Self { api, path: directory.to_path_buf(), active: true })
    }

    fn unmount(&mut self, flags: u32) -> Result<()> {
        // SAFETY: The signature matches DismUnmountImage in DismApi.h.
        let unmount = unsafe { self.api.library.symbol::<Unmount>(c"DismUnmountImage")? };
        let path = wide(&self.path);
        // SAFETY: The path outlives the call and optional arguments are null.
        let status = unsafe { unmount(pcwstr(&path), flags, HANDLE::default(), None, core::ptr::null_mut()) };
        self.active = false;
        self.api.check(status, "DismUnmountImage", &self.path.display().to_string())
    }

    pub fn commit(mut self) -> Result<()> {
        self.unmount(COMMIT_IMAGE)
    }
}

impl Drop for MountedImage {
    fn drop(&mut self) {
        if self.active {
            // A failed discard is cleaned up by cleanup_mountpoints on next run.
            let _ = self.unmount(DISCARD_IMAGE);
        }
    }
}

/// Clears mounts orphaned by an earlier crash.
pub fn cleanup_mountpoints() -> Result<()> {
    type Cleanup = unsafe extern "system" fn() -> HRESULT;
    let api = Api::acquire()?;
    // SAFETY: The signature matches DismCleanupMountpoints in DismApi.h.
    let cleanup = unsafe { api.library.symbol::<Cleanup>(c"DismCleanupMountpoints")? };
    // SAFETY: The function takes no arguments.
    let status = unsafe { cleanup() };
    api.check(status, "DismCleanupMountpoints", "mount points")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    // Per thread, because every test that builds a fake Api also drops one.
    thread_local! {
        static CLOSED_SESSIONS: Cell<u32> = const { Cell::new(0) };
        static SHUTDOWN_CALLS: Cell<usize> = const { Cell::new(0) };
    }

    extern "system" fn fake_shutdown() -> HRESULT {
        SHUTDOWN_CALLS.set(SHUTDOWN_CALLS.get() + 1);
        HRESULT(0)
    }

    extern "system" fn fake_open_session(
        _image: PCWSTR,
        _windows_dir: PCWSTR,
        _system_drive: PCWSTR,
        _session: *mut Session,
    ) -> HRESULT {
        HRESULT(0)
    }

    extern "system" fn fake_close_session(session: Session) -> HRESULT {
        CLOSED_SESSIONS.set(CLOSED_SESSIONS.get() + session);
        HRESULT(0)
    }

    extern "system" fn fake_delete(_buffer: *const c_void) -> HRESULT {
        HRESULT(0)
    }

    extern "system" fn fake_last_error_empty(_message: *mut *mut DismString) -> HRESULT {
        HRESULT(0x8000_4005_u32.cast_signed())
    }

    fn fake_api() -> Arc<Api> {
        Arc::new(Api {
            library: DynLib::load("kernel32.dll").unwrap(),
            shutdown: fake_shutdown,
            open_session: fake_open_session,
            close_session: fake_close_session,
            delete: fake_delete,
            last_error: fake_last_error_empty,
        })
    }

    #[test]
    fn packed_layout_matches_the_header() {
        // 8-byte pointers and 4-byte BOOL/enum with no padding at any boundary.
        assert_eq!(size_of::<DismDriverPackage>(), 8 * 7 + 4 * 3 + 16 + 4 * 4);
        assert_eq!(align_of::<DismDriverPackage>(), 1);
        assert_eq!(size_of::<DismString>(), size_of::<PCWSTR>());
        assert_eq!(align_of::<DismString>(), 1);
        assert_eq!(core::mem::offset_of!(DismDriverPackage, catalog_file), 20);
        assert_eq!(core::mem::offset_of!(DismDriverPackage, provider_name), 60);
        assert_eq!(core::mem::offset_of!(DismDriverPackage, major_version), 84);
        assert_eq!(core::mem::offset_of!(DismFeatureInfo, feature_state), 8);
    }

    /// Reads feature states from the running system. Read-only, and it runs
    /// only when elevated, which in practice means the CI runner.
    #[test]
    fn online_feature_state_reads_without_changing_anything() {
        if !crate::sysinfo::is_elevated() {
            return;
        }
        let session = DismSession::online().unwrap();
        // Present in every Windows 11 and Server 2025 image, on or off.
        session.feature_enabled("NetFx3").unwrap();
        let missing = session.feature_enabled("WindowCraft-No-Such-Feature").unwrap_err();
        assert!(missing.is_not_found(), "{missing}");
    }

    #[test]
    fn only_installed_pending_and_partial_states_count_as_enabled() {
        // DismPackageFeatureState: NotPresent 0, UninstallPending 1, Staged 2,
        // Removed 3, Installed 4, InstallPending 5, Superseded 6, PartiallyInstalled 7.
        let enabled: Vec<i32> = (0..=7).filter(|state| is_enabled_state(*state)).collect();
        assert_eq!(enabled, [4, 5, 7]);
    }

    #[test]
    fn check_treats_ok_and_both_reboot_required_forms_as_success() {
        let api = fake_api();
        assert!(api.check(HRESULT(0), "DismEnableFeature", "TelnetClient").is_ok());
        assert!(api.check(HRESULT(1), "DismEnableFeature", "TelnetClient").is_ok());
        assert!(api.check(HRESULT(S_REBOOT_REQUIRED), "DismEnableFeature", "Hyper-V").is_ok());
        assert!(api.check(HRESULT::from_win32(3010), "DismAddDriver", "oem0.inf").is_ok());
    }

    #[test]
    fn check_maps_failing_hresult_with_operation_target_and_code() {
        let api = fake_api();
        let cbs_unknown = HRESULT(0x800F_080C_u32.cast_signed());
        let error = api.check(cbs_unknown, "DismEnableFeature", "Recall").unwrap_err();
        assert!(error.is_not_found());
        assert!(matches!(
            &error,
            EngineError::Win32 { operation: "DismEnableFeature", target, code: 0x800F_080C, .. }
                if target == "Recall"
        ));
    }

    #[test]
    fn describe_never_leaves_the_message_empty() {
        let reserved = HRESULT(0x800F_0975_u32.cast_signed());
        assert_eq!(describe("  The image is busy.\r\n".to_owned(), reserved), "The image is busy.");
        let access = HRESULT::from_win32(5);
        assert_eq!(describe("   ".to_owned(), access), access.message().trim());
        let fallback = describe(String::new(), reserved);
        assert!(!fallback.trim().is_empty());
        if reserved.message().trim().is_empty() {
            assert!(fallback.contains(r"Logs\DISM\dism.log"), "{fallback}");
        }
    }

    #[test]
    fn dropping_sessions_closes_each_handle_before_api_shutdown() {
        let api = fake_api();
        let first = DismSession { api: api.clone(), session: 10 };
        let second = DismSession { api: api.clone(), session: 25 };
        drop(api);

        drop(first);
        assert_eq!(CLOSED_SESSIONS.get(), 10);
        assert_eq!(SHUTDOWN_CALLS.get(), 0);

        drop(second);
        assert_eq!(CLOSED_SESSIONS.get(), 35);
        assert_eq!(SHUTDOWN_CALLS.get(), 1);
    }

    #[test]
    fn session_methods_report_missing_dism_symbols_without_touching_system() {
        let session = DismSession { api: fake_api(), session: 1 };

        let err = session.enable_feature("NetFx3").unwrap_err();
        assert!(matches!(&err, EngineError::Unsupported(msg) if msg.contains("DismEnableFeature")));

        let err = session.disable_feature("NetFx3").unwrap_err();
        assert!(matches!(&err, EngineError::Unsupported(msg) if msg.contains("DismDisableFeature")));

        let err = session.feature_enabled("NetFx3").unwrap_err();
        assert!(matches!(&err, EngineError::Unsupported(msg) if msg.contains("DismGetFeatureInfo")));

        let err = session.add_capability("OpenSSH.Server~~~~0.0.1.0").unwrap_err();
        assert!(matches!(&err, EngineError::Unsupported(msg) if msg.contains("DismAddCapability")));

        let err = session.set_reserved_storage(false).unwrap_err();
        assert!(matches!(&err, EngineError::Unsupported(msg) if msg.contains("DismSetReservedStorageState")));

        let err = session.add_driver(Path::new(r"C:\missing.inf")).unwrap_err();
        assert!(matches!(&err, EngineError::Unsupported(msg) if msg.contains("DismAddDriver")));

        let err = session.drivers().unwrap_err();
        assert!(matches!(&err, EngineError::Unsupported(msg) if msg.contains("DismGetDrivers")));
    }

    #[test]
    fn mounted_image_exposes_mount_path_and_attempts_unmount_on_commit_or_drop() {
        let mount_dir = PathBuf::from(r"C:\WindowCraft_MountTest");
        let mounted = MountedImage { api: fake_api(), path: mount_dir.clone(), active: true };
        assert_eq!(mounted.path, mount_dir);

        let err = mounted.commit().unwrap_err();
        assert!(matches!(
            &err,
            EngineError::Unsupported(msg) if msg.contains("DismUnmountImage")
        ));
    }
}
