// Made by Nordan
//! Loads optional system DLLs from System32 only.
//!
//! `LOAD_LIBRARY_SEARCH_SYSTEM32` blocks DLL planting next to the portable
//! executable, which matters because it usually runs elevated.

use crate::error::{EngineError, Result};
use crate::win::{pcwstr, wide};
use std::ffi::CStr;
use windows::Win32::Foundation::{FreeLibrary, HMODULE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
use windows::core::PCSTR;

pub struct DynLib {
    module: HMODULE,
    name: String,
}

// SAFETY: Module handles are process-wide and usable from any thread.
unsafe impl Send for DynLib {}
// SAFETY: As above, and this type exposes no interior mutation.
unsafe impl Sync for DynLib {}

impl DynLib {
    pub fn load(name: &str) -> Result<Self> {
        let wide_name = wide(name);
        // SAFETY: The name is null-terminated and the file handle must be None.
        let module = unsafe { LoadLibraryExW(pcwstr(&wide_name), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map_err(|error| EngineError::windows("LoadLibraryExW", name, &error))?;
        Ok(Self { module, name: name.to_owned() })
    }

    /// Resolves an exported function.
    ///
    /// # Safety
    /// `T` must be an `unsafe extern "system" fn` type exactly matching the
    /// export's real signature. Calling a mismatched pointer is undefined.
    pub unsafe fn symbol<T: Copy>(&self, name: &CStr) -> Result<T> {
        const { assert!(size_of::<T>() == size_of::<usize>(), "T must be a function pointer") };
        // SAFETY: The module is loaded and the name is null-terminated.
        let address = unsafe { GetProcAddress(self.module, PCSTR(name.as_ptr().cast())) }
            .ok_or_else(|| EngineError::Unsupported(format!("{} is missing {name:?}", self.name)))?;
        // SAFETY: T is pointer-sized (checked above) and the caller vouches
        // for the signature.
        Ok(unsafe { core::mem::transmute_copy(&address) })
    }
}

impl Drop for DynLib {
    fn drop(&mut self) {
        // SAFETY: The module is owned by this value and released exactly once.
        // Function pointers must not outlive it, which callers keep to by
        // holding the DynLib alongside them.
        unsafe {
            let _ = FreeLibrary(self.module);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_system_library_and_resolves_exports() {
        type GetTickCount = unsafe extern "system" fn() -> u32;
        let kernel = DynLib::load("kernel32.dll").unwrap();
        // SAFETY: GetTickCount takes nothing and returns a DWORD.
        let tick = unsafe { kernel.symbol::<GetTickCount>(c"GetTickCount") }.unwrap();
        // SAFETY: The signature matches and `kernel` is alive.
        assert!(unsafe { tick() } > 0);
        // SAFETY: The export does not exist, so the pointer is never produced.
        assert!(unsafe { kernel.symbol::<GetTickCount>(c"NoSuchExport") }.is_err());
    }

    #[test]
    fn missing_library_is_an_error() {
        assert!(DynLib::load("windowcraft-no-such.dll").is_err());
    }
}
