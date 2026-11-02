// Made by ycangignacy
//! Startup entries across Run keys, Startup folders, tasks and services.
//!
//! Disabling never deletes anything: Run entries use the same `StartupApproved`
//! flag as Task Manager, so they can always be turned back on.

use crate::authenticode::verify_file;
use crate::error::{EngineError, Result};
use crate::executor::Engine;
use crate::registry::{Hive, RegPath, RegValue, RegistryScope};
use crate::services::{ScmBackend, ServiceBackend, ServiceInfo, guard_change};
use crate::snapshot::Change;
use crate::win::expand_env;
use rayon::prelude::*;
use std::path::PathBuf;
use windowcraft_config::StartupType;

const RUN_KEYS: [(Hive, &str); 4] = [
    (Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run"),
    (Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\RunOnce"),
    (Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\Run"),
    (Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\RunOnce"),
];
const APPROVED_RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const APPROVED_FOLDER: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder";
const ENABLED_FLAG: u8 = 0x02;
const DISABLED_FLAG: u8 = 0x03;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySource {
    RunKey,
    StartupFolder,
    Task,
    Service,
}

impl EntrySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::RunKey => "Run key",
            Self::StartupFolder => "Startup folder",
            Self::Task => "Scheduled task",
            Self::Service => "Service",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutorunEntry {
    pub name: String,
    pub source: EntrySource,
    pub location: String,
    pub command: String,
    /// Program to verify when `command` is not itself a command line.
    pub binary: Option<String>,
    pub enabled: bool,
    pub publisher: Option<String>,
    pub signed: bool,
}

/// Extracts the executable path from a command line.
pub fn executable_of(command: &str) -> Option<PathBuf> {
    let expanded = expand_env(command.trim());
    let text = expanded.trim();
    if let Some(rest) = text.strip_prefix('"') {
        return rest.split('"').next().map(PathBuf::from);
    }
    let lower = text.to_lowercase();
    if let Some(position) = lower.find(".exe") {
        return Some(PathBuf::from(&text[..position + 4]));
    }
    text.split_whitespace().next().map(PathBuf::from)
}

fn is_enabled_flag(value: Option<&RegValue>) -> bool {
    match value {
        Some(RegValue::Binary(bytes)) => bytes.first().is_none_or(|flag| flag % 2 == 0),
        _ => true,
    }
}

fn run_entries() -> Vec<AutorunEntry> {
    let mut entries = Vec::new();
    for (hive, subkey) in RUN_KEYS {
        let path = RegPath { hive, subkey: subkey.to_owned() };
        let Ok(Some(key)) = RegistryScope::System.open(&path) else { continue };
        let approved = RegPath { hive, subkey: APPROVED_RUN.to_owned() };
        for name in key.value_names().unwrap_or_default() {
            let Ok(Some(RegValue::String(command) | RegValue::ExpandString(command))) = key.read(&name) else {
                continue;
            };
            let flag = RegistryScope::System.read(&approved, &name).ok().flatten();
            entries.push(AutorunEntry {
                enabled: is_enabled_flag(flag.as_ref()),
                name,
                source: EntrySource::RunKey,
                location: path.display(),
                command,
                binary: None,
                publisher: None,
                signed: false,
            });
        }
    }
    entries
}

fn folder_entries() -> Vec<AutorunEntry> {
    let mut folders = Vec::new();
    if let Some(root) = std::env::var_os("APPDATA") {
        folders.push(PathBuf::from(root).join(r"Microsoft\Windows\Start Menu\Programs\Startup"));
    }
    if let Some(root) = std::env::var_os("ProgramData") {
        folders.push(PathBuf::from(root).join(r"Microsoft\Windows\Start Menu\Programs\Startup"));
    }
    let mut entries = Vec::new();
    for folder in folders {
        let Ok(items) = std::fs::read_dir(&folder) else { continue };
        for item in items.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            entries.push(AutorunEntry {
                name,
                source: EntrySource::StartupFolder,
                location: folder.display().to_string(),
                command: item.path().display().to_string(),
                binary: None,
                enabled: true,
                publisher: None,
                signed: false,
            });
        }
    }
    entries
}

fn task_entries() -> Vec<AutorunEntry> {
    let Ok(tasks) = crate::tasks::list_folder("\\") else {
        return Vec::new();
    };
    tasks
        .into_iter()
        .map(|task| AutorunEntry {
            name: task.name,
            source: EntrySource::Task,
            location: "Task Scheduler".to_owned(),
            command: task.path,
            binary: Some(task.executable.unwrap_or_default()),
            enabled: task.enabled,
            publisher: None,
            signed: false,
        })
        .collect()
}

fn is_microsoft_service(service: &ServiceInfo) -> bool {
    let lower = service.binary_path.to_lowercase();
    lower.contains(r"\windows\system32\svchost.exe") || lower.contains(r"\windows\system32\")
}

fn service_entries() -> Vec<AutorunEntry> {
    let Ok(services) = ScmBackend.enumerate() else {
        return Vec::new();
    };
    services
        .into_iter()
        .filter(|service| !is_microsoft_service(service))
        .filter(|service| {
            matches!(service.startup, StartupType::Automatic | StartupType::AutomaticDelayedStart | StartupType::Disabled)
        })
        .map(|service| AutorunEntry {
            enabled: service.startup != StartupType::Disabled,
            location: "Services".to_owned(),
            name: service.name,
            source: EntrySource::Service,
            command: service.binary_path,
            binary: None,
            publisher: None,
            signed: false,
        })
        .collect()
}

fn verify(mut entry: AutorunEntry) -> AutorunEntry {
    let target = entry.binary.as_deref().unwrap_or(&entry.command);
    if target.is_empty() {
        entry.publisher = Some("COM handler".to_owned());
    } else if let Some(path) = executable_of(target) {
        let trust = verify_file(&path);
        entry.signed = trust.trusted;
        entry.publisher = trust.publisher;
    }
    entry
}

/// Scans all sources in parallel and verifies publishers on the rayon pool.
pub fn scan() -> Vec<AutorunEntry> {
    let (run, (folders, (tasks, services))) = rayon::join(
        run_entries,
        || rayon::join(folder_entries, || rayon::join(task_entries, service_entries)),
    );
    let mut entries: Vec<AutorunEntry> = run.into_iter().chain(folders).chain(tasks).chain(services).collect();
    entries = entries.into_par_iter().map(verify).collect();
    entries.sort_by_cached_key(|entry| entry.name.to_lowercase());
    entries
}

/// `StartupApproved` value: a flag byte, reserved bytes and a timestamp.
fn approved_bytes(enabled: bool) -> Vec<u8> {
    let mut bytes = vec![0u8; 12];
    bytes[0] = if enabled { ENABLED_FLAG } else { DISABLED_FLAG };
    bytes
}

fn approved_location(entry: &AutorunEntry) -> Option<RegPath> {
    match entry.source {
        EntrySource::RunKey => {
            let hive = if entry.location.starts_with("HKLM") { Hive::LocalMachine } else { Hive::CurrentUser };
            Some(RegPath { hive, subkey: APPROVED_RUN.to_owned() })
        }
        EntrySource::StartupFolder => Some(RegPath { hive: Hive::CurrentUser, subkey: APPROVED_FOLDER.to_owned() }),
        EntrySource::Task | EntrySource::Service => None,
    }
}

/// Toggles an entry through a snapshot, so Time Machine can reverse it.
pub fn set_enabled_tracked(engine: &Engine, entry: &AutorunEntry, enabled: bool) -> Result<u64> {
    let change = match entry.source {
        EntrySource::RunKey | EntrySource::StartupFolder => {
            let path = approved_location(entry).ok_or_else(|| EngineError::NotFound(entry.name.clone()))?;
            Change::Registry {
                path: path.display(),
                name: entry.name.clone(),
                before: RegistryScope::System.read(&path, &entry.name)?,
                after: Some(RegValue::Binary(approved_bytes(enabled))),
            }
        }
        EntrySource::Task => Change::Task { path: entry.command.clone(), before: entry.enabled, after: enabled },
        EntrySource::Service => {
            let after = if enabled { StartupType::Automatic } else { StartupType::Disabled };
            guard_change(&entry.name, after)?;
            let before = ScmBackend.query(&entry.name)?.map(|info| info.startup);
            Change::Service { name: entry.name.clone(), before, after }
        }
    };
    let verb = if enabled { "Enable" } else { "Disable" };
    engine.tracked(&format!("{verb} startup entry: {}", entry.name), "autoruns", change, || set_enabled(entry, enabled))
}

/// Turns an entry on or off without removing it.
pub fn set_enabled(entry: &AutorunEntry, enabled: bool) -> Result<()> {
    match entry.source {
        EntrySource::RunKey | EntrySource::StartupFolder => {
            let path = approved_location(entry).ok_or_else(|| EngineError::NotFound(entry.name.clone()))?;
            RegistryScope::System.write(&path, &entry.name, &RegValue::Binary(approved_bytes(enabled)))
        }
        EntrySource::Task => crate::tasks::set_enabled(&entry.command, enabled),
        EntrySource::Service => {
            let startup = if enabled { StartupType::Automatic } else { StartupType::Disabled };
            guard_change(&entry.name, startup)?;
            ScmBackend.set_startup(&entry.name, startup)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_executables_from_command_lines() {
        assert_eq!(executable_of(r#""C:\Program Files\App\a.exe" --tray"#), Some(PathBuf::from(r"C:\Program Files\App\a.exe")));
        assert_eq!(executable_of(r"C:\Tools\b.exe /silent"), Some(PathBuf::from(r"C:\Tools\b.exe")));
        assert_eq!(executable_of(r"C:\Program Files\X\c.EXE -a"), Some(PathBuf::from(r"C:\Program Files\X\c.EXE")));
        assert_eq!(executable_of("   "), None);
    }

    #[test]
    fn startup_approved_flags() {
        assert!(is_enabled_flag(None));
        assert!(is_enabled_flag(Some(&RegValue::Binary(approved_bytes(true)))));
        assert!(!is_enabled_flag(Some(&RegValue::Binary(approved_bytes(false)))));
        assert_eq!(approved_bytes(false).len(), 12);
    }

    #[test]
    fn scan_runs_without_elevation() {
        let entries = scan();
        assert!(entries.iter().all(|entry| !entry.name.is_empty()));
    }

    #[test]
    fn protected_services_cannot_be_disabled_from_autoruns() {
        let entry = AutorunEntry {
            name: "WinDefend".into(),
            source: EntrySource::Service,
            location: String::new(),
            command: String::new(),
            binary: None,
            enabled: true,
            publisher: None,
            signed: true,
        };
        assert!(set_enabled(&entry, false).is_err());
    }
}
