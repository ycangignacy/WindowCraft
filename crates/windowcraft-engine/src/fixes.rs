// Made by WindowCraft Dev Team
//! Repair workflows.

use crate::cleaner;
use crate::dynlib::DynLib;
use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use crate::process::{run, run_ok};
use crate::registry::{RegPath, RegistryScope};
use crate::services::{ScmBackend, ServiceBackend, start};
use crate::win::system32;
use std::path::PathBuf;
use std::time::Duration;
use windows::core::HRESULT;

const SHORT: Duration = Duration::from_secs(120);
const LONG: Duration = Duration::from_secs(3 * 60 * 60);

const UPDATE_SERVICES: [&str; 4] = ["BITS", "wuauserv", "appidsvc", "cryptsvc"];
const UPDATE_DLLS: [&str; 15] = [
    "atl.dll", "urlmon.dll", "mshtml.dll", "jscript.dll", "vbscript.dll", "msxml3.dll", "msxml6.dll",
    "wuapi.dll", "wuaueng.dll", "wups.dll", "wups2.dll", "qmgr.dll", "qmgrprxy.dll", "wucltux.dll", "wuwebv.dll",
];

fn windows_dir(child: &str) -> PathBuf {
    PathBuf::from(std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned())).join(child)
}

/// Folders `rename_aside` may have left. They can reach several gigabytes.
pub fn backup_folders() -> [PathBuf; 3] {
    [
        windows_dir(r"SoftwareDistribution\Download.wcbak"),
        windows_dir(r"SoftwareDistribution\DataStore.wcbak"),
        system32("catroot2.wcbak"),
    ]
}

fn rename_aside(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let backup = path.with_extension("wcbak");
    if backup.exists() {
        cleaner::clean_dir(&backup);
        // A busy leftover only stops the rename below, which then reports it.
        let _ = std::fs::remove_dir(&backup);
    }
    std::fs::rename(path, &backup).map_err(|error| EngineError::io(path.display().to_string(), error))
}

/// Calls `DllRegisterServer` in-process, without spawning regsvr32. Libraries
/// missing on this build are skipped, because not all exist on every SKU.
fn register_dll(name: &str) {
    type Register = unsafe extern "system" fn() -> HRESULT;
    if let Ok(library) = DynLib::load(name) {
        // SAFETY: DllRegisterServer has this signature by COM convention.
        if let Ok(register) = unsafe { library.symbol::<Register>(c"DllRegisterServer") } {
            // SAFETY: `library` stays loaded for the call. Registration
            // failures on inbox DLLs are non-fatal for the reset.
            let _ = unsafe { register() };
        }
    }
}

/// Stops `services`, runs `body` and starts again every service it stopped,
/// also when a stop, `body` or cancellation ends the work early. A failed
/// start is added to the first error, or returned when there was none.
fn with_services_stopped<T>(
    services: &[&str],
    cancel: &CancelToken,
    stop: impl Fn(&str) -> Result<()>,
    start: impl Fn(&str) -> Result<()>,
    body: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let mut stopped = Vec::new();
    let mut outcome = Ok(());
    for service in services {
        if let Err(error) = cancel.check() {
            outcome = Err(error);
            break;
        }
        // A failed stop can still leave the service stopping, so it is
        // started again too.
        stopped.push(*service);
        if let Err(error) = stop(service) {
            outcome = Err(error);
            break;
        }
    }
    let result = outcome.and_then(|()| body());
    let mut failed = Vec::new();
    for service in stopped {
        if let Err(error) = start(service) {
            failed.push(format!("{service}: {error}"));
        }
    }
    if failed.is_empty() {
        return result;
    }
    let restart = format!("Starting the update services failed ({})", failed.join("; "));
    Err(match result {
        Ok(_) => EngineError::Message(format!("{restart}.")),
        Err(error) => EngineError::Message(format!("{error}. {restart}.")),
    })
}

pub fn reset_windows_update(aggressive: bool, cancel: &CancelToken, mut log: impl FnMut(&str)) -> Result<()> {
    let backend = ScmBackend;
    log("Stopping update services");
    with_services_stopped(&UPDATE_SERVICES, cancel, |name| backend.stop(name), start, || {
        reset_with_services_stopped(aggressive, cancel, &mut log)
    })?;

    log("Triggering update detection");
    let _ = run(&system32("usoclient.exe"), &["StartScan"], cancel, SHORT, |_| {})?;
    Ok(())
}

fn reset_with_services_stopped(aggressive: bool, cancel: &CancelToken, log: &mut dyn FnMut(&str)) -> Result<()> {
    log("Removing BITS job queue");
    let downloader = PathBuf::from(std::env::var("ProgramData").unwrap_or_else(|_| r"C:\ProgramData".to_owned()))
        .join(r"Microsoft\Network\Downloader");
    if let Ok(entries) = std::fs::read_dir(&downloader) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.starts_with("qmgr") && name.ends_with(".dat") {
                // Files recreated by a racing service are harmless.
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    log("Renaming SoftwareDistribution download cache");
    rename_aside(&windows_dir(r"SoftwareDistribution\Download"))?;
    if aggressive {
        log("Renaming DataStore and Catroot2");
        rename_aside(&windows_dir(r"SoftwareDistribution\DataStore"))?;
        rename_aside(&system32("catroot2"))?;
    }

    log("Clearing WSUS client identity");
    let path = RegPath::parse(r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate")?;
    for name in ["AccountDomainSid", "PingID", "SusClientId", "SusClientIDValidation"] {
        RegistryScope::System.write_opt(&path, name, None)?;
    }

    log("Re-registering update components");
    for name in UPDATE_DLLS {
        cancel.check()?;
        register_dll(name);
    }

    log("Resetting WinSock");
    run_ok(&system32("netsh.exe"), &["winsock", "reset"], cancel, SHORT)?;

    log("Starting update services");
    Ok(())
}

pub fn reset_network(cancel: &CancelToken) -> Result<()> {
    let netsh = system32("netsh.exe");
    run_ok(&netsh, &["winsock", "reset"], cancel, SHORT)?;
    run_ok(&netsh, &["int", "ipv4", "reset"], cancel, SHORT)?;
    run_ok(&netsh, &["int", "ipv6", "reset"], cancel, SHORT)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanOutcome {
    Clean,
    Repaired,
    RepairedRebootRequired,
    NeedsAttention,
}

/// Maps a repair tool's exit code. `0` is clean, `1` is fixed, `2` and `3010`
/// need a restart, and anything else needs a person.
pub fn interpret_exit_code(code: i32) -> ScanOutcome {
    match code {
        0 => ScanOutcome::Clean,
        1 => ScanOutcome::Repaired,
        2 | 3010 => ScanOutcome::RepairedRebootRequired,
        _ => ScanOutcome::NeedsAttention,
    }
}

/// Three-stage scan: disk, then system files, then the component store.
pub fn system_scan(cancel: &CancelToken, mut progress: impl FnMut(f32, &str)) -> Result<Vec<(String, ScanOutcome)>> {
    // Unelevated, sfc exits with 1, which would read as "repaired".
    if !crate::sysinfo::is_elevated() {
        return Err(EngineError::NeedsElevation("the system corruption scan".to_owned()));
    }
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned());
    let stages: [(&str, PathBuf, Vec<&str>); 3] = [
        ("chkdsk /scan /perf", system32("chkdsk.exe"), vec![&drive, "/scan", "/perf"]),
        ("sfc /scannow", system32("sfc.exe"), vec!["/scannow"]),
        (
            "dism /online /cleanup-image /restorehealth",
            system32("Dism.exe"),
            vec!["/online", "/cleanup-image", "/restorehealth"],
        ),
    ];
    let mut results = Vec::with_capacity(stages.len());
    for (index, (label, program, arguments)) in stages.iter().enumerate() {
        let base = index as f32 / stages.len() as f32;
        progress(base, label);
        let output = run(program, arguments, cancel, LONG, |line| {
            let percent = line.split('%').next().and_then(|t| t.rsplit([' ', '[']).next()?.parse::<f32>().ok());
            let local = percent.map_or(0.0, |value| value / 100.0);
            progress(base + local / stages.len() as f32, line);
        })?;
        results.push(((*label).to_owned(), interpret_exit_code(output.code)));
    }
    Ok(results)
}

impl ScanOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::Repaired => "repaired",
            Self::RepairedRebootRequired => "repaired, restart required",
            Self::NeedsAttention => "needs attention",
        }
    }
}

/// One line per stage, for the user.
pub fn scan_summary(results: &[(String, ScanOutcome)]) -> String {
    results.iter().map(|(stage, outcome)| format!("{stage}: {}", outcome.label())).collect::<Vec<_>>().join("; ")
}

/// Turns a stage that could not finish cleanly into an error.
pub fn require_clean(results: &[(String, ScanOutcome)]) -> Result<()> {
    if results.iter().any(|(_, outcome)| *outcome == ScanOutcome::NeedsAttention) {
        return Err(EngineError::Invalid(scan_summary(results)));
    }
    Ok(())
}

/// Re-registers App Installer, which provides `winget.exe`.
pub fn repair_winget() -> Result<()> {
    crate::appx::register_family("Microsoft.DesktopAppInstaller_8wekyb3d8bbwe")
}

pub fn configure_ntp(cancel: &CancelToken) -> Result<()> {
    let time = system32("w32tm.exe");
    start("w32time").or_else(|error| if error.is_not_found() { Err(error) } else { Ok(()) })?;
    run_ok(
        &time,
        &["/config", "/manualpeerlist:pool.ntp.org,0x8", "/syncfromflags:manual", "/reliable:no", "/update"],
        cancel,
        SHORT,
    )?;
    run_ok(&time, &["/resync", "/force"], cancel, SHORT)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const NAMES: [&str; 3] = ["a", "b", "c"];

    fn recorded(log: &RefCell<Vec<String>>, prefix: &str, name: &str) -> Result<()> {
        log.borrow_mut().push(format!("{prefix} {name}"));
        Ok(())
    }

    #[test]
    fn services_come_back_after_success() {
        let log = RefCell::new(Vec::new());
        let result = with_services_stopped(
            &NAMES,
            &CancelToken::new(),
            |name| recorded(&log, "stop", name),
            |name| recorded(&log, "start", name),
            || Ok(5),
        );
        assert_eq!(result.unwrap(), 5);
        assert_eq!(log.into_inner(), ["stop a", "stop b", "stop c", "start a", "start b", "start c"]);
    }

    #[test]
    fn services_come_back_when_the_work_fails() {
        let started = RefCell::new(Vec::new());
        let result: Result<()> = with_services_stopped(
            &NAMES,
            &CancelToken::new(),
            |_| Ok(()),
            |name| recorded(&started, "start", name),
            || Err(EngineError::Invalid("rename".into())),
        );
        assert!(matches!(result, Err(EngineError::Invalid(_))));
        assert_eq!(started.into_inner(), ["start a", "start b", "start c"]);
    }

    #[test]
    fn cancelling_while_stopping_starts_what_was_stopped() {
        let cancel = CancelToken::new();
        let started = RefCell::new(Vec::new());
        let ran = RefCell::new(false);
        let result: Result<()> = with_services_stopped(
            &NAMES,
            &cancel,
            |name| {
                if name == "b" {
                    cancel.cancel();
                }
                Ok(())
            },
            |name| recorded(&started, "start", name),
            || {
                *ran.borrow_mut() = true;
                Ok(())
            },
        );
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(!ran.into_inner());
        assert_eq!(started.into_inner(), ["start a", "start b"]);
    }

    #[test]
    fn a_failed_start_is_added_to_the_first_error() {
        let result: Result<()> = with_services_stopped(
            &NAMES,
            &CancelToken::new(),
            |_| Ok(()),
            |name| if name == "b" { Err(EngineError::NotFound("b".into())) } else { Ok(()) },
            || Err(EngineError::Invalid("rename".into())),
        );
        let text = result.unwrap_err().to_string();
        assert!(text.contains("rename") && text.contains("b: not found"), "{text}");
    }

    #[test]
    fn exit_codes_are_interpreted_explicitly() {
        assert_eq!(interpret_exit_code(0), ScanOutcome::Clean);
        assert_eq!(interpret_exit_code(1), ScanOutcome::Repaired);
        assert_eq!(interpret_exit_code(2), ScanOutcome::RepairedRebootRequired);
        assert_eq!(interpret_exit_code(3010), ScanOutcome::RepairedRebootRequired);
        assert_eq!(interpret_exit_code(87), ScanOutcome::NeedsAttention);
        assert_eq!(interpret_exit_code(-1), ScanOutcome::NeedsAttention);
    }

    #[test]
    fn a_failed_stage_is_reported_instead_of_swallowed() {
        let results = vec![
            ("sfc".to_owned(), ScanOutcome::Repaired),
            ("dism".to_owned(), ScanOutcome::NeedsAttention),
        ];
        let error = require_clean(&results).unwrap_err().to_string();
        assert!(error.contains("dism: needs attention") && error.contains("sfc: repaired"), "{error}");
        assert!(require_clean(&results[..1]).is_ok());
    }

    #[test]
    fn scan_outcome_labels_and_summary_format_every_stage_state() {
        let stages = vec![
            ("chkdsk".to_owned(), ScanOutcome::Clean),
            ("sfc".to_owned(), ScanOutcome::Repaired),
            ("dism".to_owned(), ScanOutcome::RepairedRebootRequired),
        ];
        assert_eq!(
            scan_summary(&stages),
            "chkdsk: clean; sfc: repaired; dism: repaired, restart required"
        );
        assert!(require_clean(&stages).is_ok());
        assert_eq!(scan_summary(&[]), "");
        assert!(require_clean(&[]).is_ok());
    }

    #[test]
    fn backup_folders_match_what_rename_aside_creates() {
        let renamed = windows_dir(r"SoftwareDistribution\Download").with_extension("wcbak");
        assert!(backup_folders().contains(&renamed));
        assert!(backup_folders().contains(&system32("catroot2").with_extension("wcbak")));
    }

    #[test]
    fn windows_dir_joins_child_under_system_root() {
        let path = windows_dir(r"SoftwareDistribution\Download");
        assert!(path.is_absolute());
        assert!(path.ends_with(r"SoftwareDistribution\Download"));
    }

    #[test]
    fn rename_aside_ignores_missing_target_and_replaces_existing_wcbak_directory() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("Download");
        let backup = temp.path().join("Download.wcbak");

        rename_aside(&target).unwrap();
        assert!(!backup.exists());

        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("first.cab"), b"old").unwrap();
        rename_aside(&target).unwrap();
        assert!(!target.exists());
        assert_eq!(std::fs::read(backup.join("first.cab")).unwrap(), b"old");

        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("second.cab"), b"new").unwrap();
        rename_aside(&target).unwrap();
        assert!(!target.exists());
        assert!(!backup.join("first.cab").exists());
        assert_eq!(std::fs::read(backup.join("second.cab")).unwrap(), b"new");
    }

    #[test]
    fn register_dll_tolerates_missing_and_non_com_libraries() {
        register_dll("windowcraft_nonexistent_12345.dll");
        register_dll("kernel32.dll");
    }
}
