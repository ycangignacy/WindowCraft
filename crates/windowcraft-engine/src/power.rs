// Made by Nordan
//! Power plans through PowrProf and System Restore through srclient.

use crate::dynlib::DynLib;
use crate::error::{EngineError, Result};
use crate::win::{LocalGuard, from_wide, pcwstr, wide};
use windows::Win32::System::Power::*;
use windows::Win32::System::Restore::*;
use windows::core::{BOOL, GUID, PCWSTR};

/// Template GUID shipped with Windows for Ultimate Performance.
pub const ULTIMATE_PERFORMANCE: GUID = GUID::from_u128(0xe9a42b02_d5df_448d_aa00_03f14749eb61);
const BALANCED: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
const ULTIMATE_NAME: &str = "Ultimate Performance";

fn check(status: windows::Win32::Foundation::WIN32_ERROR, operation: &'static str, target: &str) -> Result<()> {
    if status.is_ok() { Ok(()) } else { Err(EngineError::win32(operation, target, status.0)) }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PowerScheme {
    pub guid: GUID,
    pub name: String,
}

pub fn active_scheme() -> Result<GUID> {
    let mut pointer = core::ptr::null_mut();
    // SAFETY: `pointer` is a valid out pointer. The result is LocalAlloc'd.
    let status = unsafe { PowerGetActiveScheme(None, &mut pointer) };
    check(status, "PowerGetActiveScheme", "active scheme")?;
    let _guard = LocalGuard(pointer.cast());
    // SAFETY: On success the pointer references a GUID.
    Ok(unsafe { *pointer })
}

fn friendly_name(guid: &GUID) -> String {
    let mut size = 0u32;
    // SAFETY: A null buffer asks for the size.
    let _ = unsafe { PowerReadFriendlyName(None, Some(guid), None, None, None, &mut size) };
    let mut units = vec![0u16; (size as usize).div_ceil(2)];
    // SAFETY: `units` holds at least `size` bytes.
    let status =
        unsafe { PowerReadFriendlyName(None, Some(guid), None, None, Some(units.as_mut_ptr().cast()), &mut size) };
    if status.is_ok() { from_wide(&units) } else { String::new() }
}

pub fn schemes() -> Result<Vec<PowerScheme>> {
    let mut found = Vec::new();
    for index in 0u32.. {
        let mut guid = GUID::zeroed();
        let mut size = size_of::<GUID>() as u32;
        // SAFETY: `guid` provides `size` writable bytes.
        let status = unsafe {
            PowerEnumerate(None, None, None, ACCESS_SCHEME, index, Some((&raw mut guid).cast()), &mut size)
        };
        const NO_MORE_ITEMS: u32 = 259;
        if status.0 == NO_MORE_ITEMS {
            break;
        }
        check(status, "PowerEnumerate", "schemes")?;
        found.push(PowerScheme { guid, name: friendly_name(&guid) });
    }
    Ok(found)
}

/// Copies of the Ultimate Performance template.
///
/// Friendly names are localized and copies get fresh GUIDs, so the only stable
/// identity is "has the same name as the template on this system". The English
/// name stays as a fallback, and the template itself never counts as a copy.
pub fn ultimate_copies() -> Result<Vec<PowerScheme>> {
    let template_name = friendly_name(&ULTIMATE_PERFORMANCE);
    Ok(schemes()?
        .into_iter()
        .filter(|scheme| scheme.guid != ULTIMATE_PERFORMANCE)
        .filter(|scheme| scheme.name == ULTIMATE_NAME || (!template_name.is_empty() && scheme.name == template_name))
        .collect())
}

/// Duplicates the hidden template and activates the copy. Idempotent.
pub fn enable_ultimate_performance() -> Result<()> {
    if let Some(existing) = ultimate_copies()?.into_iter().next() {
        // SAFETY: `guid` is a valid scheme identifier.
        let status = unsafe { PowerSetActiveScheme(None, Some(&existing.guid)) };
        return check(status, "PowerSetActiveScheme", ULTIMATE_NAME);
    }
    let mut created = core::ptr::null_mut();
    // SAFETY: The source GUID and out pointer are valid. The result is LocalAlloc'd.
    let status = unsafe { PowerDuplicateScheme(None, &ULTIMATE_PERFORMANCE, &mut created) };
    check(status, "PowerDuplicateScheme", ULTIMATE_NAME)?;
    let _guard = LocalGuard(created.cast());
    // SAFETY: On success `created` points to the new scheme's GUID.
    let status = unsafe { PowerSetActiveScheme(None, Some(created)) };
    check(status, "PowerSetActiveScheme", ULTIMATE_NAME)
}

/// Removes every Ultimate Performance copy, moving to Balanced first because
/// the active scheme cannot be deleted.
pub fn remove_ultimate_performance() -> Result<()> {
    let active = active_scheme()?;
    for scheme in ultimate_copies()? {
        if scheme.guid == active {
            // SAFETY: BALANCED is a built-in scheme.
            let status = unsafe { PowerSetActiveScheme(None, Some(&BALANCED)) };
            check(status, "PowerSetActiveScheme", "Balanced")?;
        }
        // SAFETY: The GUID came from PowerEnumerate.
        let status = unsafe { PowerDeleteScheme(None, &scheme.guid) };
        check(status, "PowerDeleteScheme", ULTIMATE_NAME)?;
    }
    Ok(())
}

pub const RESTORE_DESCRIPTION: &str = "WindowCraft Automatic Restore Point";

/// Creates a restore point on the system drive, enabling protection first.
pub fn create_restore_point() -> Result<()> {
    type EnableSr = unsafe extern "system" fn(PCWSTR) -> u32;

    let library = DynLib::load("srclient.dll")?;
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    let root = wide(format!("{drive}\\"));
    // SAFETY: EnableSR takes one wide drive path and returns a Win32 code.
    if let Ok(enable) = unsafe { library.symbol::<EnableSr>(c"EnableSR") } {
        // SAFETY: `root` is null-terminated. Already-enabled is not an error
        // worth surfacing, because SRSetRestorePointW reports the real outcome.
        let _ = unsafe { enable(pcwstr(&root)) };
    }

    let mut description = [0u16; 256];
    for (slot, unit) in description.iter_mut().zip(RESTORE_DESCRIPTION.encode_utf16()) {
        *slot = unit;
    }
    let mut info = RESTOREPOINTINFOW {
        dwEventType: BEGIN_SYSTEM_CHANGE,
        dwRestorePtType: MODIFY_SETTINGS,
        llSequenceNumber: 0,
        szDescription: description,
    };
    let mut status = STATEMGRSTATUS::default();
    // SAFETY: Both structures are valid for the call.
    let started: BOOL = unsafe { SRSetRestorePointW(&info, &mut status) };
    if !started.as_bool() {
        let code = status.nStatus.0;
        return Err(EngineError::win32("SRSetRestorePointW", RESTORE_DESCRIPTION, code));
    }
    info.dwEventType = END_SYSTEM_CHANGE;
    info.llSequenceNumber = status.llSequenceNumber;
    // SAFETY: As above, closing the sequence opened by the first call.
    let finished: BOOL = unsafe { SRSetRestorePointW(&info, &mut status) };
    if !finished.as_bool() {
        let code = status.nStatus.0;
        return Err(EngineError::win32("SRSetRestorePointW", RESTORE_DESCRIPTION, code));
    }
    Ok(())
}

/// True when the firmware offers S4, the sleep state hibernation needs.
/// When the question cannot be answered it says yes, so powercfg decides.
pub fn firmware_supports_hibernation() -> bool {
    let mut capabilities = SYSTEM_POWER_CAPABILITIES::default();
    // SAFETY: The structure is owned here and outlives the call.
    if unsafe { GetPwrCapabilities(&mut capabilities) } { capabilities.SystemS4 } else { true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hibernation_support_can_be_asked() {
        // Any answer is valid; the call must simply work on this machine.
        let _ = firmware_supports_hibernation();
    }

    #[test]
    fn copies_never_include_builtin_plans_or_the_template() {
        const BUILT_IN: [GUID; 3] = [
            BALANCED,
            GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c),
            GUID::from_u128(0xa1841308_3541_4fab_bc81_f71556f20b4a),
        ];
        for scheme in ultimate_copies().unwrap() {
            assert!(!BUILT_IN.contains(&scheme.guid), "{} is a built-in plan", scheme.name);
            assert_ne!(scheme.guid, ULTIMATE_PERFORMANCE);
        }
    }

    #[test]
    fn active_scheme_is_listed() {
        let active = active_scheme().unwrap();
        let schemes = schemes().unwrap();
        assert!(!schemes.is_empty());
        assert!(schemes.iter().any(|scheme| scheme.guid == active));
        assert!(schemes.iter().all(|scheme| !scheme.name.is_empty()));
    }
}
