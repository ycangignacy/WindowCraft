// Made by ycangignacy
//! Explorer, processes, settings broadcasts and Shell launches.

use crate::error::{EngineError, Result};
use crate::win::{HandleGuard, from_wide, pcwstr, wide, wildcard_match};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, SendMessageTimeoutW, WM_SETTINGCHANGE,
};
use windows::core::{PCWSTR, w};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub name: String,
}

pub fn processes() -> Result<Vec<ProcessEntry>> {
    // SAFETY: Valid flags, and the handle moves into a guard immediately.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|error| EngineError::windows("CreateToolhelp32Snapshot", "processes", &error))?;
    let snapshot = HandleGuard(snapshot);

    let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
    let mut found = Vec::new();
    // SAFETY: `entry` has dwSize set and the snapshot handle is live.
    let mut more = unsafe { Process32FirstW(snapshot.raw(), &mut entry) }.is_ok();
    while more {
        found.push(ProcessEntry { pid: entry.th32ProcessID, name: from_wide(&entry.szExeFile) });
        // SAFETY: As above.
        more = unsafe { Process32NextW(snapshot.raw(), &mut entry) }.is_ok();
    }
    Ok(found)
}

/// Terminates processes by wildcard, ignoring the `.exe` suffix. Processes
/// that exit first or cannot be opened are skipped.
pub fn kill_matching(pattern: &str) -> Result<usize> {
    let own_pid = std::process::id();
    let mut killed = 0;
    for process in processes()? {
        let stem = process.name.strip_suffix(".exe").unwrap_or(&process.name);
        if process.pid == own_pid || !wildcard_match(pattern, stem) {
            continue;
        }
        // SAFETY: Opening a process has no preconditions; failure is expected
        // for protected processes.
        if let Ok(handle) = unsafe { OpenProcess(PROCESS_TERMINATE, false, process.pid) } {
            let handle = HandleGuard(handle);
            // Exit code 1 reads as a clean shell exit and Winlogon would not
            // restart Explorer, so this uses the same code as Stop-Process.
            // SAFETY: The handle has PROCESS_TERMINATE.
            if unsafe { TerminateProcess(handle.raw(), u32::MAX) }.is_ok() {
                killed += 1;
            }
        }
    }
    Ok(killed)
}

/// Restarts the shell. Winlogon respawns Explorer after a non-zero exit, and
/// this starts one explicitly if it does not appear.
pub fn restart_explorer() -> Result<()> {
    kill_matching("explorer")?;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if processes()?.iter().any(|process| process.name.eq_ignore_ascii_case("explorer.exe")) {
            return Ok(());
        }
    }
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
    std::process::Command::new(format!(r"{root}\explorer.exe"))
        .spawn()
        .map(drop)
        .map_err(|error| EngineError::io("explorer.exe", error))
}

fn broadcast(area: PCWSTR) {
    let mut result = 0;
    // SAFETY: `area` is a static null-terminated string, and the abort flag
    // and timeout stop a hung window from blocking this thread.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(area.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            1_000,
            Some(&mut result),
        );
    }
}

/// Tells running apps that the color theme changed.
pub fn refresh_theme() {
    broadcast(w!("ImmersiveColorSet"));
}

/// Tells running apps that environment variables changed.
pub fn broadcast_environment() {
    broadcast(w!("Environment"));
}

/// Opens a control panel applet, MMC snap-in or shell URI.
pub fn launch(target: &str, parameters: Option<&str>) -> Result<()> {
    let file = wide(target);
    let arguments = parameters.map(wide);
    let arguments_pointer = arguments.as_ref().map_or_else(PCWSTR::null, |text| pcwstr(text));
    // SAFETY: All strings are null-terminated and outlive the call.
    let result = unsafe {
        ShellExecuteW(None, w!("open"), pcwstr(&file), arguments_pointer, PCWSTR::null(), SW_SHOWNORMAL)
    };
    if result.0 as usize <= 32 {
        return Err(EngineError::win32("ShellExecuteW", target, result.0 as u32));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_this_process() {
        let list = processes().unwrap();
        let own_pid = std::process::id();
        let current = list.iter().find(|process| process.pid == own_pid).unwrap();
        assert!(!current.name.is_empty());
        assert!(list.iter().all(|process| !process.name.is_empty()));
    }

    #[test]
    fn never_kills_itself() {
        let name = std::env::current_exe().unwrap();
        let stem = name.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(kill_matching(&stem).unwrap(), 0);
        assert_eq!(kill_matching(&format!("{stem}.exe")).unwrap(), 0);
    }

    #[test]
    fn unmatched_pattern_kills_nothing() {
        assert_eq!(kill_matching("windowcraft-no-such-process-*").unwrap(), 0);
        assert_eq!(kill_matching("").unwrap(), 0);
    }

    #[test]
    fn launch_reports_win32_error_for_missing_target_with_and_without_parameters() {
        let missing = r"C:\WindowCraft_NoSuchBinary_98765.exe";
        let err_plain = launch(missing, None).unwrap_err();
        assert!(err_plain.is_not_found());
        assert!(matches!(
            &err_plain,
            EngineError::Win32 { operation: "ShellExecuteW", target, .. } if target == missing
        ));

        let err_with_args = launch(missing, Some("--quiet")).unwrap_err();
        assert!(err_with_args.is_not_found());
        assert!(matches!(
            &err_with_args,
            EngineError::Win32 { operation: "ShellExecuteW", target, .. } if target == missing
        ));
    }
}
