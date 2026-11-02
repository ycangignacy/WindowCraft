// Made by ycangignacy
//! Small RAII wrappers shared across the native adapters.

use crate::error::{EngineError, Result};
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::core::{PCWSTR, PWSTR};

/// Null-terminated UTF-16 for a Win32 call.
pub fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

pub fn pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}

/// Lossy conversion for buffers that may or may not be null-terminated.
pub fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..end]).to_string_lossy().into_owned()
}

/// Reads a null-terminated string owned by the OS.
///
/// # Safety
/// `pointer` must be null or point to a valid null-terminated UTF-16 string.
pub unsafe fn from_pcwstr(pointer: PCWSTR) -> String {
    if pointer.is_null() {
        return String::new();
    }
    // SAFETY: Guaranteed by the caller.
    unsafe { String::from_utf16_lossy(pointer.as_wide()) }
}

/// Frees a `LocalAlloc` buffer on every exit path.
pub struct LocalGuard(pub *mut core::ffi::c_void);

impl Drop for LocalGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: The pointer came from an API documented to allocate with
            // LocalAlloc, and this guard is its sole owner.
            unsafe {
                LocalFree(Some(HLOCAL(self.0)));
            }
        }
    }
}

impl LocalGuard {
    pub fn pwstr(value: PWSTR) -> Self {
        Self(value.0.cast())
    }
}

/// Closes a kernel handle on every exit path.
pub struct HandleGuard(pub HANDLE);

impl HandleGuard {
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0.0.is_null() {
            // SAFETY: The handle is owned by this guard and closed exactly once.
            // A close failure cannot be acted on while dropping.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

// SAFETY: Kernel handles are process-wide and may be closed from any thread.
unsafe impl Send for HandleGuard {}

/// Balances `CoInitializeEx` with `CoUninitialize` for the current thread.
pub struct ComApartment {
    owns: bool,
    // COM apartments are per-thread, so this guard must not cross threads.
    _not_send: std::marker::PhantomData<*const ()>,
}

impl ComApartment {
    pub fn enter() -> Result<Self> {
        // SAFETY: The reserved parameter is None and the call has no other
        // preconditions.
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        const RPC_E_CHANGED_MODE: i32 = 0x8001_0106_u32.cast_signed();
        if result.0 == RPC_E_CHANGED_MODE {
            // The thread already has an apartment owned by someone else, so
            // this guard must not uninitialize it.
            return Ok(Self { owns: false, _not_send: std::marker::PhantomData });
        }
        result.ok().map_err(|error| EngineError::windows("CoInitializeEx", "COM", &error))?;
        Ok(Self { owns: true, _not_send: std::marker::PhantomData })
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.owns {
            // SAFETY: Balances the successful CoInitializeEx in `enter` on the
            // same thread, which `_not_send` enforces.
            unsafe { CoUninitialize() };
        }
    }
}

/// Absolute path to a System32 binary. Elevated code must never resolve tools
/// through `PATH`, because an attacker-writable entry would run as admin.
pub fn system32(file: &str) -> PathBuf {
    let mut buffer = [0u16; 260];
    // SAFETY: The buffer is valid for its full length.
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    let root = if length == 0 || length >= buffer.len() {
        PathBuf::from(r"C:\Windows\System32")
    } else {
        PathBuf::from(OsString::from_wide(&buffer[..length]))
    };
    root.join(file)
}

/// Expands `%NAME%` tokens from the process environment.
pub fn expand_env(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        output.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => output.push_str(&value),
                    Err(_) => {
                        output.push('%');
                        output.push_str(name);
                        output.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                output.push('%');
                rest = after;
            }
        }
    }
    output.push_str(rest);
    output
}

pub fn local_app_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("WindowCraft")
}

/// Wildcard match supporting `*` and `?`, case-insensitively.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let (mut p, mut t, mut star, mut mark) = (0, 0, None, 0);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(position) = star {
            p = position + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard_match("*Widget*", "msedgewebview_Widgets"));
        assert!(wildcard_match("*copilot*", "Microsoft.Copilot"));
        assert!(wildcard_match("Microsoft.Xbox?", "microsoft.xboxa"));
        assert!(wildcard_match("exact", "EXACT"));
        assert!(!wildcard_match("exact", "exactly"));
        assert!(!wildcard_match("Micro*Edge", "MicrosoftEdgeUpdate"));
        assert!(wildcard_match("*", ""));
    }

    #[test]
    fn env_expansion_keeps_unknown_tokens() {
        assert_eq!(expand_env("%WC_DEFINITELY_UNSET%\\x"), "%WC_DEFINITELY_UNSET%\\x");
        assert_eq!(expand_env("50% done"), "50% done");
        let root = std::env::var("SystemRoot").unwrap();
        assert_eq!(expand_env("%SystemRoot%\\Temp"), format!("{root}\\Temp"));
    }

    #[test]
    fn wide_roundtrip() {
        assert_eq!(from_wide(&wide("Grüße")), "Grüße");
        assert_eq!(from_wide(&[65, 66]), "AB");
    }

    #[test]
    fn system32_is_absolute() {
        let path = system32("cmd.exe");
        assert!(path.is_absolute());
        assert!(path.exists());
    }

    #[test]
    fn com_apartment_is_balanced_and_nestable() {
        let outer = ComApartment::enter().unwrap();
        let inner = ComApartment::enter().unwrap();
        drop(inner);
        drop(outer);
    }
}
