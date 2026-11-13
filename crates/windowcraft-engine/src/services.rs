// Made by WindowCraft Dev Team
//! Windows Service Control Manager.

use crate::error::{EngineError, Result};
use crate::win::{from_pcwstr, pcwstr, wide};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, ERROR_SERVICE_DOES_NOT_EXIST};
use windows::Win32::System::Services::*;
use windows::core::{BOOL, PCWSTR};
use windowcraft_config::StartupType;

/// Services the engine refuses to disable, whatever a config asks for.
/// Disabling any of these can leave Windows unbootable or unprotected.
pub const PROTECTED_SERVICES: [&str; 7] = ["RpcSs", "DcomLaunch", "LSM", "BFE", "mpssvc", "WinDefend", "ProfSvc"];

pub fn is_protected(name: &str) -> bool {
    PROTECTED_SERVICES.iter().any(|protected| protected.eq_ignore_ascii_case(name))
}

/// Refuses a change before it reaches the SCM.
pub fn guard_change(name: &str, startup: StartupType) -> Result<()> {
    if startup == StartupType::Disabled && is_protected(name) {
        return Err(EngineError::ProtectedService(name.to_owned()));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Stopped,
    Pending,
    Running,
    Paused,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    pub name: String,
    pub display_name: String,
    pub startup: StartupType,
    pub state: RunState,
    pub binary_path: String,
}

/// Backend behind the executor, swapped for an in-memory map in tests.
pub trait ServiceBackend: Send + Sync {
    fn query(&self, name: &str) -> Result<Option<ServiceInfo>>;
    fn set_startup(&self, name: &str, startup: StartupType) -> Result<()>;
    fn stop(&self, name: &str) -> Result<()>;
}

struct ScHandle(SC_HANDLE);

impl Drop for ScHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: The handle is owned by this guard and closed exactly once.
            unsafe {
                let _ = CloseServiceHandle(self.0);
            }
        }
    }
}

fn manager(access: u32) -> Result<ScHandle> {
    // SAFETY: Null machine and database names select the local active database.
    let handle = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }
        .map_err(|error| EngineError::windows("OpenSCManagerW", "Service Control Manager", &error))?;
    Ok(ScHandle(handle))
}

fn open(manager: &ScHandle, name: &str, access: u32) -> Result<Option<ScHandle>> {
    let wide_name = wide(name);
    // SAFETY: The manager handle is live and the name is null-terminated.
    match unsafe { OpenServiceW(manager.0, pcwstr(&wide_name), access) } {
        Ok(handle) => Ok(Some(ScHandle(handle))),
        Err(error) if error.code() == ERROR_SERVICE_DOES_NOT_EXIST.to_hresult() => Ok(None),
        Err(error) => Err(EngineError::windows("OpenServiceW", name, &error)),
    }
}

fn state_of(service: &ScHandle) -> RunState {
    let mut status = SERVICE_STATUS::default();
    // SAFETY: The handle has SERVICE_QUERY_STATUS and `status` is a valid out pointer.
    if unsafe { QueryServiceStatus(service.0, &mut status) }.is_err() {
        return RunState::Unknown;
    }
    match status.dwCurrentState {
        SERVICE_RUNNING => RunState::Running,
        SERVICE_STOPPED => RunState::Stopped,
        SERVICE_PAUSED => RunState::Paused,
        _ => RunState::Pending,
    }
}

fn delayed_auto_start(service: &ScHandle) -> bool {
    let mut info = SERVICE_DELAYED_AUTO_START_INFO::default();
    let mut needed = 0;
    // SAFETY: `info` is exactly the structure this info level writes.
    let result = unsafe {
        QueryServiceConfig2W(
            service.0,
            SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
            Some(core::slice::from_raw_parts_mut(
                (&raw mut info).cast::<u8>(),
                size_of::<SERVICE_DELAYED_AUTO_START_INFO>(),
            )),
            &mut needed,
        )
    };
    result.is_ok() && info.fDelayedAutostart.as_bool()
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ScmBackend;

impl ScmBackend {
    fn query_config(service: &ScHandle, name: &str) -> Result<ServiceInfo> {
        let mut needed = 0u32;
        // SAFETY: A null buffer of size zero asks for the required size.
        let probe = unsafe { QueryServiceConfigW(service.0, None, 0, &mut needed) };
        if let Err(error) = probe
            && error.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult()
        {
            return Err(EngineError::windows("QueryServiceConfigW", name, &error));
        }

        // u64 storage keeps the buffer aligned for the structure's pointers.
        let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
        let config = storage.as_mut_ptr().cast::<QUERY_SERVICE_CONFIGW>();
        let size = needed;
        // SAFETY: `storage` holds at least `size` writable, aligned bytes.
        unsafe { QueryServiceConfigW(service.0, Some(config), size, &mut needed) }
            .map_err(|error| EngineError::windows("QueryServiceConfigW", name, &error))?;

        // SAFETY: The call above initialized the structure, and its string
        // pointers reference `storage`, which is alive for this block.
        let (start, display_name, binary_path) = unsafe {
            (
                (*config).dwStartType,
                from_pcwstr(PCWSTR((*config).lpDisplayName.0)),
                from_pcwstr(PCWSTR((*config).lpBinaryPathName.0)),
            )
        };

        let startup = match start {
            SERVICE_AUTO_START if delayed_auto_start(service) => StartupType::AutomaticDelayedStart,
            SERVICE_AUTO_START => StartupType::Automatic,
            SERVICE_DEMAND_START => StartupType::Manual,
            SERVICE_DISABLED => StartupType::Disabled,
            SERVICE_BOOT_START => StartupType::Boot,
            SERVICE_SYSTEM_START => StartupType::System,
            _ => StartupType::Manual,
        };
        Ok(ServiceInfo { name: name.to_owned(), display_name, startup, state: state_of(service), binary_path })
    }

    /// Non-Microsoft services for the Autoruns view.
    pub fn enumerate(&self) -> Result<Vec<ServiceInfo>> {
        let manager = manager(SC_MANAGER_ENUMERATE_SERVICE)?;
        let mut needed = 0;
        let mut count = 0;
        let mut resume = 0;
        // SAFETY: A null buffer asks for the required size.
        let probe = unsafe {
            EnumServicesStatusExW(
                manager.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                None,
                &mut needed,
                &mut count,
                Some(&mut resume),
                PCWSTR::null(),
            )
        };
        if let Err(error) = probe
            && error.code() != ERROR_MORE_DATA.to_hresult()
        {
            return Err(EngineError::windows("EnumServicesStatusExW", "services", &error));
        }

        let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
        // SAFETY: `storage` spans `needed` bytes, and that size is the one requested.
        let bytes = unsafe { core::slice::from_raw_parts_mut(storage.as_mut_ptr().cast::<u8>(), needed as usize) };
        let mut remaining = 0;
        // SAFETY: `bytes` is writable and out pointers are valid.
        unsafe {
            EnumServicesStatusExW(
                manager.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                Some(bytes),
                &mut remaining,
                &mut count,
                Some(&mut resume),
                PCWSTR::null(),
            )
        }
        .map_err(|error| EngineError::windows("EnumServicesStatusExW", "services", &error))?;

        let first = storage.as_ptr().cast::<ENUM_SERVICE_STATUS_PROCESSW>();
        let mut names = Vec::with_capacity(count as usize);
        for index in 0..count as usize {
            // SAFETY: The API wrote `count` entries at the start of `storage`.
            names.push(unsafe { from_pcwstr(PCWSTR((*first.add(index)).lpServiceName.0)) });
        }

        let mut services = Vec::with_capacity(names.len());
        for name in names {
            if let Ok(Some(service)) = open(&manager, &name, SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS)
                && let Ok(info) = Self::query_config(&service, &name)
            {
                services.push(info);
            }
        }
        Ok(services)
    }
}

impl ServiceBackend for ScmBackend {
    fn query(&self, name: &str) -> Result<Option<ServiceInfo>> {
        let manager = manager(SC_MANAGER_CONNECT)?;
        let Some(service) = open(&manager, name, SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS)? else {
            return Ok(None);
        };
        Self::query_config(&service, name).map(Some)
    }

    fn set_startup(&self, name: &str, startup: StartupType) -> Result<()> {
        guard_change(name, startup)?;
        let manager = manager(SC_MANAGER_CONNECT)?;
        let service = open(&manager, name, SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG)?
            .ok_or_else(|| EngineError::NotFound(format!("service {name}")))?;

        let start = match startup {
            StartupType::Automatic | StartupType::AutomaticDelayedStart => SERVICE_AUTO_START,
            StartupType::Manual => SERVICE_DEMAND_START,
            StartupType::Disabled => SERVICE_DISABLED,
            StartupType::Boot => SERVICE_BOOT_START,
            StartupType::System => SERVICE_SYSTEM_START,
        };
        // SAFETY: Every optional string is null, which means "leave unchanged".
        unsafe {
            ChangeServiceConfigW(
                service.0,
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                start,
                SERVICE_ERROR(SERVICE_NO_CHANGE),
                PCWSTR::null(),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
        }
        .map_err(|error| EngineError::windows("ChangeServiceConfigW", name, &error))?;

        if matches!(startup, StartupType::Automatic | StartupType::AutomaticDelayedStart) {
            let mut info = SERVICE_DELAYED_AUTO_START_INFO {
                fDelayedAutostart: BOOL::from(startup == StartupType::AutomaticDelayedStart),
            };
            // SAFETY: `info` matches the info level and outlives the call.
            unsafe {
                ChangeServiceConfig2W(
                    service.0,
                    SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                    Some((&raw mut info).cast()),
                )
            }
            .map_err(|error| EngineError::windows("ChangeServiceConfig2W", name, &error))?;
        }
        Ok(())
    }

    fn stop(&self, name: &str) -> Result<()> {
        let manager = manager(SC_MANAGER_CONNECT)?;
        let Some(service) = open(&manager, name, SERVICE_STOP | SERVICE_QUERY_STATUS)? else {
            return Ok(());
        };
        if state_of(&service) == RunState::Stopped {
            return Ok(());
        }
        let mut status = SERVICE_STATUS::default();
        // SAFETY: The handle has SERVICE_STOP and `status` is a valid out pointer.
        let result = unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) };
        if let Err(error) = result {
            const NOT_ACTIVE: u32 = 1062;
            if error.code() != windows::core::HRESULT::from_win32(NOT_ACTIVE) {
                return Err(EngineError::windows("ControlService", name, &error));
            }
        }
        for _ in 0..100 {
            if state_of(&service) == RunState::Stopped {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Err(EngineError::Invalid(format!("service {name} did not stop within 10 seconds")))
    }
}

/// Starts a service, tolerating one that is already running.
pub fn start(name: &str) -> Result<()> {
    let manager = manager(SC_MANAGER_CONNECT)?;
    let service = open(&manager, name, SERVICE_START)?
        .ok_or_else(|| EngineError::NotFound(format!("service {name}")))?;
    // SAFETY: The handle has SERVICE_START, and no arguments are passed.
    if let Err(error) = unsafe { StartServiceW(service.0, None) } {
        const ALREADY_RUNNING: u32 = 1056;
        if error.code() != windows::core::HRESULT::from_win32(ALREADY_RUNNING) {
            return Err(EngineError::windows("StartServiceW", name, &error));
        }
    }
    Ok(())
}

/// In-memory service table for sandboxed runs.
#[derive(Debug, Default)]
pub struct MemoryServices {
    table: Mutex<HashMap<String, StartupType>>,
    /// Services that accept a write and keep their old start type, as
    /// Windows Update Medic does for wuauserv and BITS.
    pinned: Mutex<Vec<String>>,
}

impl MemoryServices {
    pub fn with(entries: &[(&str, StartupType)]) -> Self {
        let table = entries.iter().map(|(name, startup)| (name.to_lowercase(), *startup)).collect();
        Self { table: Mutex::new(table), pinned: Mutex::new(Vec::new()) }
    }

    pub fn pin(&self, name: &str) {
        if let Ok(mut pinned) = self.pinned.lock() {
            pinned.push(name.to_lowercase());
        }
    }

    pub fn startup(&self, name: &str) -> Option<StartupType> {
        self.table.lock().ok()?.get(&name.to_lowercase()).copied()
    }

    pub fn force(&self, name: &str, startup: StartupType) {
        if let Ok(mut table) = self.table.lock() {
            table.insert(name.to_lowercase(), startup);
        }
    }
}

impl ServiceBackend for MemoryServices {
    fn query(&self, name: &str) -> Result<Option<ServiceInfo>> {
        Ok(self.startup(name).map(|startup| ServiceInfo {
            name: name.to_owned(),
            display_name: name.to_owned(),
            startup,
            state: RunState::Stopped,
            binary_path: String::new(),
        }))
    }

    fn set_startup(&self, name: &str, startup: StartupType) -> Result<()> {
        guard_change(name, startup)?;
        if self.startup(name).is_none() {
            return Err(EngineError::NotFound(format!("service {name}")));
        }
        if self.pinned.lock().is_ok_and(|pinned| pinned.contains(&name.to_lowercase())) {
            return Ok(());
        }
        self.force(name, startup);
        Ok(())
    }

    fn stop(&self, _name: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_services_are_refused_case_insensitively() {
        for name in PROTECTED_SERVICES {
            assert!(matches!(guard_change(name, StartupType::Disabled), Err(EngineError::ProtectedService(_))));
            assert!(guard_change(&name.to_uppercase(), StartupType::Disabled).is_err());
            assert!(guard_change(&name.to_lowercase(), StartupType::Disabled).is_err());
            assert!(guard_change(name, StartupType::Manual).is_ok(), "only disabling is blocked");
        }
        assert!(guard_change("DiagTrack", StartupType::Disabled).is_ok());
    }

    #[test]
    fn real_backend_refuses_before_touching_the_scm() {
        let error = ScmBackend.set_startup("WinDefend", StartupType::Disabled).unwrap_err();
        assert!(matches!(error, EngineError::ProtectedService(_)));
    }

    #[test]
    fn memory_backend_applies_the_same_guard() {
        let services = MemoryServices::with(&[("RpcSs", StartupType::Automatic)]);
        assert!(services.set_startup("rpcss", StartupType::Disabled).is_err());
        assert_eq!(services.startup("RPCSS"), Some(StartupType::Automatic));
    }

    #[test]
    fn query_reads_a_real_service_without_elevation() {
        let info = ScmBackend.query("RpcSs").unwrap().expect("RpcSs always exists");
        assert_eq!(info.state, RunState::Running);
        assert!(!info.display_name.is_empty());
        assert!(ScmBackend.query("WindowCraftNoSuchService").unwrap().is_none());
    }

    #[test]
    fn enumerate_returns_services() {
        let services = ScmBackend.enumerate().unwrap();
        assert!(services.len() > 50);
    }
}
