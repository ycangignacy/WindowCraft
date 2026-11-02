// Made by ycangignacy
//! Operating system facts used by the UI, reports and guards.

use crate::error::{EngineError, Result};
use crate::registry::{RegPath, RegValue, RegistryScope};
use crate::win::{HandleGuard, wide};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation};
use windows::Win32::System::SystemInformation::{
    GetNativeSystemInfo, GetPhysicallyInstalledSystemMemory, PROCESSOR_ARCHITECTURE_AMD64, PROCESSOR_ARCHITECTURE_ARM64,
    SYSTEM_INFO,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

const VERSION_KEY: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemInfo {
    pub product: String,
    pub display_version: String,
    pub build: String,
    pub ubr: u32,
    pub architecture: &'static str,
    pub ram_kb: u64,
    pub logical_processors: u32,
    pub elevated: bool,
}

impl SystemInfo {
    /// `26300.1234`, which changes when a cumulative update lands.
    pub fn full_build(&self) -> String {
        format!("{}.{}", self.build, self.ubr)
    }
}

fn text(key: &crate::registry::Key, name: &str) -> String {
    match key.read(name) {
        Ok(Some(RegValue::String(value))) => value,
        _ => String::new(),
    }
}

/// `ProductName` still says "Windows 10" on Windows 11, so the build decides.
fn product_name(registry_value: String, build: &str) -> String {
    if build.parse::<u32>().is_ok_and(|build| build >= 22_000) {
        registry_value.replacen("Windows 10", "Windows 11", 1)
    } else {
        registry_value
    }
}

pub fn query() -> Result<SystemInfo> {
    let path = RegPath::parse(VERSION_KEY)?;
    let key = RegistryScope::System
        .open(&path)?
        .ok_or_else(|| EngineError::NotFound(VERSION_KEY.to_owned()))?;
    let ubr = match key.read("UBR")? {
        Some(RegValue::DWord(value)) => value,
        _ => 0,
    };
    let mut native = SYSTEM_INFO::default();
    // SAFETY: `native` is a valid out pointer.
    unsafe { GetNativeSystemInfo(&mut native) };
    // SAFETY: GetNativeSystemInfo always fills the architecture union member.
    let architecture = match unsafe { native.Anonymous.Anonymous.wProcessorArchitecture } {
        PROCESSOR_ARCHITECTURE_AMD64 => "x64",
        PROCESSOR_ARCHITECTURE_ARM64 => "ARM64",
        _ => "unknown",
    };
    let build = text(&key, "CurrentBuildNumber");
    Ok(SystemInfo {
        product: product_name(text(&key, "ProductName"), &build),
        display_version: text(&key, "DisplayVersion"),
        build,
        ubr,
        architecture,
        ram_kb: installed_ram_kb(),
        logical_processors: native.dwNumberOfProcessors,
        elevated: is_elevated(),
    })
}

pub fn installed_ram_kb() -> u64 {
    let mut kilobytes = 0;
    // SAFETY: `kilobytes` is a valid out pointer.
    if unsafe { GetPhysicallyInstalledSystemMemory(&mut kilobytes) }.is_ok() { kilobytes } else { 0 }
}

pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    // SAFETY: The pseudo-handle is valid and `token` is an out pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }
    let token = HandleGuard(token);
    let mut elevation = TOKEN_ELEVATION::default();
    let mut size = 0;
    // SAFETY: `elevation` is exactly the structure this class writes.
    let result = unsafe {
        GetTokenInformation(
            token.raw(),
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
    };
    result.is_ok() && elevation.TokenIsElevated != 0
}

/// Starts a new elevated instance. The UAC prompt belongs to the user.
pub fn relaunch_elevated(arguments: &str) -> Result<()> {
    let executable = std::env::current_exe().map_err(|e| EngineError::io("current executable", e))?;
    let file = wide(&executable);
    let parameters = wide(arguments);
    // SAFETY: All strings are null-terminated and outlive the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(file.as_ptr()),
            PCWSTR(parameters.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as usize <= 32 {
        return Err(EngineError::NeedsElevation("relaunch".to_owned()));
    }
    Ok(())
}

/// Reports whether a hotfix is present, from Component Based Servicing.
pub fn kb_installed(kb: &str) -> Result<bool> {
    let path = RegPath::parse(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\Packages")?;
    let needle = kb.to_ascii_uppercase();
    Ok(match RegistryScope::System.open(&path)? {
        Some(key) => key.subkeys()?.iter().any(|name| name.to_ascii_uppercase().contains(&needle)),
        None => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_system_facts() {
        let info = query().unwrap();
        assert!(info.product.contains("Windows"));
        assert!(!info.build.is_empty());
        assert_eq!(info.full_build(), format!("{}.{}", info.build, info.ubr));
        assert!(info.ram_kb > 0);
        assert!(info.logical_processors > 0);
        assert_ne!(info.architecture, "unknown");
    }

    #[test]
    fn product_name_follows_the_build() {
        assert_eq!(product_name("Windows 10 Home".into(), "26300"), "Windows 11 Home");
        assert_eq!(product_name("Windows 10 Pro".into(), "22000"), "Windows 11 Pro");
        assert_eq!(product_name("Windows 10 Pro".into(), "21999"), "Windows 10 Pro");
        assert_eq!(product_name("Windows 10 Pro".into(), "19045"), "Windows 10 Pro");
        assert_eq!(product_name("Windows 10 Enterprise".into(), ""), "Windows 10 Enterprise");
        assert_eq!(product_name("Windows 10 Enterprise".into(), "not-a-number"), "Windows 10 Enterprise");
        assert_eq!(product_name("Windows Server 2022".into(), "26100"), "Windows Server 2022");
        assert_eq!(
            product_name("Windows 10 / Windows 10".into(), "26100"),
            "Windows 11 / Windows 10"
        );
    }

    #[test]
    fn text_reads_string_values_and_ignores_missing_or_non_string_entries() {
        let path = RegPath::parse(VERSION_KEY).unwrap();
        let key = RegistryScope::System.open(&path).unwrap().unwrap();
        assert!(!text(&key, "CurrentBuildNumber").is_empty());
        assert_eq!(text(&key, "UBR"), "");
        assert_eq!(text(&key, "WindowCraft_MissingValue"), "");
    }

    #[test]
    fn unknown_kb_is_absent() {
        assert!(!kb_installed("KB0000001").unwrap());
        assert!(!kb_installed("kb0000001").unwrap());
    }
}
