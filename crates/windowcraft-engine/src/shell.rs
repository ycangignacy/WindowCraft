// Made by ycangignacy
//! Explorer, processes, settings broadcasts and Shell launches.

use crate::error::{EngineError, Result};
use crate::win::{HandleGuard, from_wide, pcwstr, wide, wildcard_match};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{ERROR_CANCELLED, HANDLE, LPARAM, WPARAM};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
    QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SW_SHOWNORMAL, SendMessageTimeoutW, WM_SETTINGCHANGE,
};
use windows::core::{PCWSTR, PWSTR, w};

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

/// The full path of a process image, if the process can be queried.
fn image_path(handle: HANDLE) -> Option<PathBuf> {
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    // SAFETY: The buffer and its length match, and the handle has
    // PROCESS_QUERY_LIMITED_INFORMATION.
    unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut length) }
        .ok()?;
    Some(PathBuf::from(String::from_utf16_lossy(&buffer[..length as usize])))
}

fn same_folder(left: &Path, right: &Path) -> bool {
    let normal = |path: &Path| path.to_string_lossy().trim_end_matches('\\').to_lowercase();
    normal(left) == normal(right)
}

/// Ends every process, in any session, whose executable sits directly in
/// `folder`, other than this one, and waits up to `timeout` for each to exit
/// so its files can be deleted. The uninstaller runs this as SYSTEM before
/// it removes files, because a window in the user's session cannot be asked
/// to close from there. Returns how many processes were ended.
pub fn close_processes_in(folder: &Path, timeout: Duration) -> Result<usize> {
    let own_pid = std::process::id();
    let deadline = Instant::now() + timeout;
    let mut ended = Vec::new();
    for process in processes()? {
        if process.pid == own_pid || process.pid == 0 {
            continue;
        }
        let access = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE;
        // SAFETY: Opening a process has no preconditions; failure is expected
        // for protected processes and ones that already exited.
        let Ok(handle) = (unsafe { OpenProcess(access, false, process.pid) }) else { continue };
        let handle = HandleGuard(handle);
        let in_folder = image_path(handle.raw())
            .is_some_and(|image| image.parent().is_some_and(|parent| same_folder(parent, folder)));
        // SAFETY: The handle has PROCESS_TERMINATE.
        if in_folder && unsafe { TerminateProcess(handle.raw(), 1) }.is_ok() {
            ended.push(handle);
        }
    }
    for handle in &ended {
        let left = deadline.saturating_duration_since(Instant::now());
        // SAFETY: The handle has SYNCHRONIZE and stays open for the wait.
        unsafe { WaitForSingleObject(handle.raw(), u32::try_from(left.as_millis()).unwrap_or(u32::MAX)) };
    }
    Ok(ended.len())
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

/// What `launch` hands to the Shell. Control panel applets go through
/// `control.exe`: the `cplfile` type has no `open` verb (its default is
/// `cplopen`), and some cleanup tools break that association entirely.
fn launch_command(target: &str, parameters: Option<&str>) -> (String, Option<String>) {
    let is_applet = std::path::Path::new(target)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cpl"));
    if !is_applet {
        return (target.to_owned(), parameters.map(str::to_owned));
    }
    let arguments = match parameters {
        Some(extra) => format!("{target} {extra}"),
        None => target.to_owned(),
    };
    (crate::win::system32("control.exe").display().to_string(), Some(arguments))
}

/// Opens a control panel applet, MMC snap-in, program or shell URI with its
/// default verb. Declining a UAC prompt is not an error.
pub fn launch(target: &str, parameters: Option<&str>) -> Result<()> {
    let (file, arguments) = launch_command(target, parameters);
    let file = wide(&file);
    let arguments = arguments.map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
        lpFile: pcwstr(&file),
        lpParameters: arguments.as_ref().map_or_else(PCWSTR::null, |text| pcwstr(text)),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` has cbSize set, and every string it points to is
    // null-terminated and outlives the call. A null verb means the default.
    if let Err(error) = unsafe { ShellExecuteExW(&mut info) } {
        if error.code() == ERROR_CANCELLED.to_hresult() {
            return Ok(());
        }
        return Err(EngineError::windows("ShellExecuteExW", target, &error));
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
        assert!(err_plain.is_not_found(), "{err_plain}");
        assert!(matches!(
            &err_plain,
            EngineError::Win32 { operation: "ShellExecuteExW", target, .. } if target == missing
        ));

        let err_with_args = launch(missing, Some("--quiet")).unwrap_err();
        assert!(err_with_args.is_not_found(), "{err_with_args}");
        assert!(matches!(
            &err_with_args,
            EngineError::Win32 { operation: "ShellExecuteExW", target, .. } if target == missing
        ));
    }

    #[test]
    fn applets_open_through_control_exe() {
        for applet in ["main.cpl", "NCPA.CPL", r"C:\Windows\System32\mmsys.cpl"] {
            let (file, arguments) = launch_command(applet, None);
            assert!(file.to_ascii_lowercase().ends_with(r"\system32\control.exe"), "{file}");
            assert_eq!(arguments.as_deref(), Some(applet));
        }
        let (_, arguments) = launch_command("main.cpl", Some(",1"));
        assert_eq!(arguments.as_deref(), Some("main.cpl ,1"));
    }

    #[test]
    fn other_targets_pass_through_unchanged() {
        for target in ["control", "compmgmt.msc", "rstrui.exe", "shell:::{A8A91A66-3A7D-4424-8D24-04E180695C7A}"] {
            assert_eq!(launch_command(target, None), (target.to_owned(), None));
        }
        assert_eq!(launch_command("setup.exe", Some("/quiet")), ("setup.exe".to_owned(), Some("/quiet".to_owned())));
    }

    /// The child for `closes_processes_started_from_a_folder`. It does nothing
    /// unless that test starts it.
    #[test]
    #[ignore = "child process for another test"]
    fn sleeping_child() {
        if std::env::var_os("WINDOWCRAFT_SLEEPING_CHILD").is_some() {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    #[test]
    fn closes_processes_started_from_a_folder() {
        let folder = tempfile::tempdir().unwrap();
        let copy = folder.path().join("windowcraft.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
        let mut child = std::process::Command::new(&copy)
            .args(["--ignored", "--exact", "shell::tests::sleeping_child"])
            .env("WINDOWCRAFT_SLEEPING_CHILD", "1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));

        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(close_processes_in(elsewhere.path(), Duration::from_secs(5)).unwrap(), 0);
        assert!(child.try_wait().unwrap().is_none(), "a process in another folder was ended");

        let started = Instant::now();
        let mut with_slash = folder.path().as_os_str().to_owned();
        with_slash.push("\\");
        assert_eq!(close_processes_in(Path::new(&with_slash), Duration::from_secs(5)).unwrap(), 1);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!child.wait().unwrap().success());
        std::fs::remove_file(&copy).unwrap();
    }

    #[test]
    fn never_closes_itself() {
        let own = std::env::current_exe().unwrap();
        assert_eq!(close_processes_in(own.parent().unwrap(), Duration::from_secs(1)).unwrap(), 0);
    }

    /// Opens a real window on the desktop, so it only runs on request:
    /// `cargo test -p windowcraft-engine -- --ignored launch_opens_the_mouse_applet`.
    #[test]
    #[ignore = "opens Mouse Properties on the desktop"]
    fn launch_opens_the_mouse_applet() {
        launch("main.cpl", None).unwrap();
    }

    /// Every panel in the catalog must resolve to something that exists, so
    /// a renamed or removed applet shows up here before a user clicks it.
    #[test]
    fn catalog_panels_resolve_to_existing_files() {
        let system = crate::win::system32("");
        let mut targets = vec![
            "main.cpl", "ncpa.cpl", "powercfg.cpl", "appwiz.cpl", "intl.cpl", "wscui.cpl", "mmsys.cpl", "sysdm.cpl",
            "timedate.cpl", "firewall.cpl", "compmgmt.msc", "control.exe",
        ];
        // Windows Server, which CI runs on, has no System Restore.
        let version = crate::registry::RegPath::parse(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion").unwrap();
        let kind = crate::registry::RegistryScope::System.read(&version, "InstallationType").unwrap();
        if kind == Some(crate::registry::RegValue::String("Client".to_owned())) {
            targets.push("rstrui.exe");
        }
        let missing: Vec<_> = targets.into_iter().filter(|target| !system.join(target).exists()).collect();
        assert!(missing.is_empty(), "missing from {}: {missing:?}", system.display());
    }
}
