// Made by ycangignacy
//! Removes what WindowCraft itself keeps on the machine, for the uninstaller
//! and for portable users. Settings a user chose to change stay as they are.
//!
//! The uninstaller runs this as SYSTEM over folders that users can write to,
//! so nothing here may follow a link. `std::fs::remove_dir_all` removes a
//! junction or symlink itself and never descends into it, even when one is
//! swapped in during the walk, and a test pins that behaviour.

use crate::error::{EngineError, Result};
use crate::registry::{RegPath, RegValue, RegistryScope};
use crate::win::{expand_env, local_app_dir};
use crate::{dism, fixes, sysinfo, tasks};
use std::path::{Path, PathBuf};

const PROFILE_LIST: &str = r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PurgeSummary {
    pub removed: Vec<String>,
    /// Places that were checked and held nothing.
    pub absent: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl PurgeSummary {
    fn record(&mut self, item: String, outcome: Result<bool>) {
        match outcome {
            Ok(true) => self.removed.push(item),
            Ok(false) => self.absent.push(item),
            Err(error) => self.failed.push((item, error.to_string())),
        }
    }
}

/// The data folder inside one profile.
fn data_dir_of(profile: &Path) -> PathBuf {
    profile.join(r"AppData\Local\WindowCraft")
}

fn profile_paths() -> Result<Vec<PathBuf>> {
    let list = RegPath::parse(PROFILE_LIST)?;
    let Some(key) = RegistryScope::System.open(&list)? else { return Ok(Vec::new()) };
    let mut profiles = Vec::new();
    for sid in key.subkeys()? {
        let path = RegPath { hive: list.hive, subkey: format!(r"{}\{sid}", list.subkey) };
        if let Some(RegValue::String(text) | RegValue::ExpandString(text)) =
            RegistryScope::System.read(&path, "ProfileImagePath")?
        {
            profiles.push(PathBuf::from(expand_env(&text)));
        }
    }
    Ok(profiles)
}

/// Every data folder this process may remove: one per profile when elevated,
/// and only its own otherwise.
fn data_dirs() -> Result<Vec<PathBuf>> {
    let mut dirs = vec![local_app_dir()];
    if sysinfo::is_elevated() {
        dirs.extend(profile_paths()?.iter().map(|profile| data_dir_of(profile)));
    }
    dirs.sort_by_key(|dir| dir.to_string_lossy().to_lowercase());
    dirs.dedup_by_key(|dir| dir.to_string_lossy().to_lowercase());
    Ok(dirs)
}

/// Removes one data folder. Returns false when there was nothing to remove.
fn remove_data_dir(dir: &Path) -> Result<bool> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(EngineError::io(dir.display().to_string(), error)),
    }
}

/// True when ISO Studio left a mounted image behind, which a plain delete
/// cannot clear.
fn has_mounted_image(dir: &Path) -> bool {
    crate::cleaner::has_mounted_image(&dir.join("iso"))
}

/// Deletes what WindowCraft keeps for itself: its data folders, plus, when
/// elevated, the Windows Update backup folders and its scheduled tasks. Without
/// elevation it removes the current user's scheduled drift check.
/// Snapshots go too, so changes already applied can no longer be rolled back.
pub fn purge_data() -> Result<PurgeSummary> {
    let elevated = sysinfo::is_elevated();
    let dirs = data_dirs()?;
    if elevated && dirs.iter().any(|dir| has_mounted_image(dir)) {
        // Best effort. A mount that survives shows up as a failed folder below.
        let _ = dism::cleanup_mountpoints();
    }
    let mut summary = PurgeSummary::default();
    for dir in dirs {
        summary.record(dir.display().to_string(), remove_data_dir(&dir));
    }
    if elevated {
        for dir in fixes::backup_folders() {
            summary.record(dir.display().to_string(), remove_data_dir(&dir));
        }
        let task = format!(r"\{}", tasks::REGISTRY_BACKUP_TASK);
        let outcome = tasks::delete(&task);
        summary.record(format!("scheduled task {task}"), outcome);
        match tasks::delete_drift_checks() {
            Ok(names) if names.is_empty() => summary.absent.push("drift check tasks".to_owned()),
            Ok(names) => summary.removed.extend(names.into_iter().map(|name| format!("scheduled task \\{name}"))),
            Err(error) => summary.failed.push(("drift check tasks".to_owned(), error.to_string())),
        }
    } else {
        // Without elevation only this user's own scheduled check can go.
        let task = format!(r"\{}", tasks::drift_check_task());
        let outcome = tasks::delete(&task);
        summary.record(format!("scheduled task {task}"), outcome);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn junction(link: &Path, target: &Path) {
        let status = Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn removes_nested_and_read_only_contents() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("WindowCraft");
        std::fs::create_dir_all(dir.join(r"iso\media\sources")).unwrap();
        let file = dir.join(r"iso\media\sources\install.wim");
        std::fs::write(&file, b"image").unwrap();
        let mut permissions = std::fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&file, permissions).unwrap();

        assert!(remove_data_dir(&dir).unwrap());
        assert!(!dir.exists());
    }

    #[test]
    fn a_missing_folder_is_not_an_error() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!remove_data_dir(&temp.path().join("WindowCraft")).unwrap());
    }

    #[test]
    fn a_junction_inside_the_folder_is_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let dir = temp.path().join("WindowCraft");
        std::fs::create_dir(&dir).unwrap();
        junction(&dir.join("link"), &outside);

        assert!(remove_data_dir(&dir).unwrap());
        assert!(!dir.exists());
        assert_eq!(std::fs::read(outside.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn a_data_folder_replaced_by_a_junction_loses_only_the_junction() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), b"keep").unwrap();
        let dir = temp.path().join("WindowCraft");
        junction(&dir, &outside);

        assert!(remove_data_dir(&dir).unwrap());
        assert!(std::fs::symlink_metadata(&dir).is_err());
        assert_eq!(std::fs::read(outside.join("keep.txt")).unwrap(), b"keep");
    }

    #[test]
    fn data_folder_sits_under_the_local_profile_path() {
        assert_eq!(
            data_dir_of(Path::new(r"C:\Users\someone")),
            PathBuf::from(r"C:\Users\someone\AppData\Local\WindowCraft")
        );
    }

    #[test]
    fn profile_list_reads_real_entries_and_includes_this_user() {
        let profiles = profile_paths().unwrap();
        assert!(!profiles.is_empty());
        let home = PathBuf::from(std::env::var_os("USERPROFILE").unwrap());
        let home = home.to_string_lossy().to_lowercase();
        assert!(profiles.iter().any(|profile| profile.to_string_lossy().to_lowercase() == home));
    }

    #[test]
    fn mounted_image_needs_a_non_empty_mount_folder() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!has_mounted_image(temp.path()));
        let mount = temp.path().join(r"iso\mount");
        std::fs::create_dir_all(&mount).unwrap();
        assert!(!has_mounted_image(temp.path()));
        std::fs::write(mount.join("Windows"), b"").unwrap();
        assert!(has_mounted_image(temp.path()));
    }
}
