// Made by ycangignacy
//! Parallel cache scanner and cleaner.
//!
//! Reparse points are never followed: a junction inside a temp folder could
//! otherwise send a recursive delete into user data.

use crate::win::expand_env;
use rayon::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanTarget {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub bytes: u64,
    pub files: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanResult {
    pub removed_bytes: u64,
    pub removed_files: u64,
    /// Locked or protected entries that were left alone.
    pub skipped: u64,
}

fn target(id: &'static str, name: &'static str, group: &'static str, paths: &[&str]) -> CleanTarget {
    CleanTarget { id, name, group, paths: paths.iter().map(|path| (*path).to_owned()).collect() }
}

/// Every location the cleaner knows about.
pub fn targets() -> Vec<CleanTarget> {
    vec![
        target("temp-user", "User temporary files", "System", &["%TEMP%"]),
        target("temp-system", "System temporary files", "System", &[r"%SystemRoot%\Temp"]),
        target("cbs-logs", "CBS and DISM logs", "System", &[r"%SystemRoot%\Logs\CBS", r"%SystemRoot%\Logs\DISM"]),
        target("minidump", "Minidump files", "System", &[r"%SystemRoot%\Minidump"]),
        target(
            "shader-nvidia",
            "NVIDIA shader cache",
            "GPU Shader Caches",
            &[r"%LOCALAPPDATA%\NVIDIA\DXCache", r"%LOCALAPPDATA%\NVIDIA\GLCache"],
        ),
        target(
            "shader-amd",
            "AMD shader cache",
            "GPU Shader Caches",
            &[r"%LOCALAPPDATA%\AMD\DxCache", r"%LOCALAPPDATA%\AMD\GLCache", r"%LOCALAPPDATA%\AMD\DxcCache"],
        ),
        target("shader-d3d", "DirectX shader cache", "GPU Shader Caches", &[r"%LOCALAPPDATA%\D3DSCache"]),
        target(
            "cargo",
            "Cargo registry cache",
            "Developer Caches",
            &[r"%USERPROFILE%\.cargo\registry\cache", r"%USERPROFILE%\.cargo\git\db"],
        ),
        target("npm", "npm cache", "Developer Caches", &[r"%LOCALAPPDATA%\npm-cache"]),
        target("pip", "pip cache", "Developer Caches", &[r"%LOCALAPPDATA%\pip\Cache"]),
        target("nuget", "NuGet package cache", "Developer Caches", &[r"%USERPROFILE%\.nuget\packages"]),
        target(ISO_TARGET, "ISO Studio work files", "WindowCraft", &[r"%LOCALAPPDATA%\WindowCraft\iso"]),
    ]
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

fn is_directory_link(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
}

/// Sizes a directory without following links.
pub fn scan_dir(root: &Path) -> ScanResult {
    let Ok(entries) = fs::read_dir(root) else {
        return ScanResult::default();
    };
    let entries: Vec<_> = entries.flatten().collect();
    entries
        .par_iter()
        .map(|entry| {
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                return ScanResult::default();
            };
            if is_reparse_point(&metadata) {
                ScanResult::default()
            } else if metadata.is_dir() {
                scan_dir(&entry.path())
            } else {
                ScanResult { bytes: metadata.len(), files: 1 }
            }
        })
        .reduce(ScanResult::default, |a, b| ScanResult { bytes: a.bytes + b.bytes, files: a.files + b.files })
}

/// Empties a directory, keeping the directory itself. Files held open by
/// other processes are counted and skipped.
pub fn clean_dir(root: &Path) -> CleanResult {
    let Ok(entries) = fs::read_dir(root) else {
        return CleanResult::default();
    };
    let entries: Vec<_> = entries.flatten().collect();
    entries
        .par_iter()
        .map(|entry| clean_entry(&entry.path()))
        .reduce(CleanResult::default, |a, b| CleanResult {
            removed_bytes: a.removed_bytes + b.removed_bytes,
            removed_files: a.removed_files + b.removed_files,
            skipped: a.skipped + b.skipped,
        })
}

fn clean_entry(path: &Path) -> CleanResult {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return CleanResult { skipped: 1, ..CleanResult::default() };
    };
    if is_reparse_point(&metadata) {
        // Removes the link itself. Its target is never touched. `is_dir` is
        // false for junctions, so the raw attribute picks the right call.
        let removed = if is_directory_link(&metadata) { fs::remove_dir(path) } else { fs::remove_file(path) };
        return CleanResult { skipped: u64::from(removed.is_err()), ..CleanResult::default() };
    }
    if metadata.is_dir() {
        let result = clean_dir(path);
        // Fails while something is still inside, which is the wanted outcome.
        let _ = fs::remove_dir(path);
        return result;
    }
    match fs::remove_file(path) {
        Ok(()) => CleanResult { removed_bytes: metadata.len(), removed_files: 1, skipped: 0 },
        Err(_) => CleanResult { skipped: 1, ..CleanResult::default() },
    }
}

pub fn expanded_paths(target: &CleanTarget) -> Vec<PathBuf> {
    target
        .paths
        .iter()
        .map(|path| expand_env(path))
        // An unexpanded token would resolve relative to the working directory.
        .filter(|path| !path.contains('%'))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect()
}

pub fn scan(target: &CleanTarget) -> ScanResult {
    let paths = expanded_paths(target);
    paths.par_iter().map(|path| scan_dir(path)).reduce(ScanResult::default, |a, b| ScanResult {
        bytes: a.bytes + b.bytes,
        files: a.files + b.files,
    })
}

/// Id of the target that holds ISO Studio work files.
pub const ISO_TARGET: &str = "windowcraft-iso";

fn has_mounted_image(iso_dir: &Path) -> bool {
    fs::read_dir(iso_dir.join("mount")).is_ok_and(|mut entries| entries.next().is_some())
}

pub fn clean(target: &CleanTarget) -> CleanResult {
    let paths = expanded_paths(target);
    // Deleting inside a mounted WIM edits the image, so ISO Studio work
    // files stay while an image is mounted there.
    if target.id == ISO_TARGET && paths.iter().any(|path| has_mounted_image(path)) {
        return CleanResult { skipped: 1, ..CleanResult::default() };
    }
    paths.par_iter().map(|path| clean_dir(path)).reduce(CleanResult::default, |a, b| CleanResult {
        removed_bytes: a.removed_bytes + b.removed_bytes,
        removed_files: a.removed_files + b.removed_files,
        skipped: a.skipped + b.skipped,
    })
}

/// Removes everything in `%TEMP%` and `%SystemRoot%\Temp` that is not in use.
pub fn clean_temporary_files() -> CleanResult {
    let mut total = CleanResult::default();
    for target in targets().iter().filter(|target| target.id.starts_with("temp-")) {
        let result = clean(target);
        total.removed_bytes += result.removed_bytes;
        total.removed_files += result.removed_files;
        total.skipped += result.skipped;
    }
    total
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanCandidate {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Lists `%AppData%` and `%LocalAppData%` folders with no matching installed
/// program. Names are not proof, so results are never preselected for removal.
pub fn orphan_candidates(installed_names: &[String]) -> Vec<OrphanCandidate> {
    const KEEP: [&str; 6] = ["microsoft", "packages", "temp", "windowcraft", "programs", "nvidia"];
    let lowered: Vec<String> = installed_names.iter().map(|name| name.to_lowercase()).collect();
    let mut roots = Vec::new();
    for variable in ["APPDATA", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(variable) {
            roots.push(PathBuf::from(root));
        }
    }

    let mut directories = Vec::new();
    for root in roots {
        let Ok(entries) = fs::read_dir(root) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            let known = KEEP.contains(&name.as_str())
                || lowered.iter().any(|installed| installed.contains(&name) || name.contains(installed.as_str()));
            if !known && entry.path().is_dir() {
                directories.push(entry.path());
            }
        }
    }
    directories
        .into_par_iter()
        .map(|path| {
            let bytes = scan_dir(&path).bytes;
            OrphanCandidate { path, bytes }
        })
        .collect()
}

/// Removes a reviewed orphan folder. Only direct children of the two AppData
/// roots qualify, so a stale or edited path can never reach anything else.
pub fn remove_orphan(path: &Path) -> CleanResult {
    let allowed = ["APPDATA", "LOCALAPPDATA"]
        .iter()
        .filter_map(std::env::var_os)
        .any(|root| path.parent() == Some(Path::new(&root)));
    if !allowed {
        return CleanResult { skipped: 1, ..CleanResult::default() };
    }
    let result = clean_dir(path);
    // Fails while a locked file remains, which leaves the rest in place.
    let _ = fs::remove_dir(path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::os::windows::process::CommandExt;

    fn populate(root: &Path) {
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("one.txt"), [0u8; 100]).unwrap();
        fs::write(root.join("a/two.txt"), [0u8; 200]).unwrap();
        fs::write(root.join("a/b/three.txt"), [0u8; 300]).unwrap();
    }

    #[test]
    fn scan_counts_every_file() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        assert_eq!(scan_dir(dir.path()), ScanResult { bytes: 600, files: 3 });
    }

    #[test]
    fn clean_empties_but_keeps_the_root() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        let result = clean_dir(dir.path());
        assert_eq!(result.removed_bytes, 600);
        assert_eq!(result.removed_files, 3);
        assert_eq!(result.skipped, 0);
        assert!(dir.path().exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn locked_files_are_skipped_gracefully() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        let locked = dir.path().join("locked.bin");
        fs::write(&locked, [1u8; 10]).unwrap();
        let handle = fs::OpenOptions::new().read(true).share_mode(0).open(&locked).unwrap();

        let result = clean_dir(dir.path());
        assert_eq!(result.removed_files, 3);
        assert_eq!(result.skipped, 1);
        assert!(locked.exists());
        drop::<File>(handle);
    }

    #[test]
    fn junctions_are_never_followed() {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let outside = tempfile::tempdir().unwrap();
        let precious = outside.path().join("precious.txt");
        fs::write(&precious, b"user data").unwrap();

        let cache = tempfile::tempdir().unwrap();
        let junction = cache.path().join("link");
        let status = std::process::Command::new(crate::win::system32("cmd.exe"))
            .args(["/c", "mklink", "/J"])
            .arg(&junction)
            .arg(outside.path())
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());

        assert_eq!(scan_dir(cache.path()).bytes, 0, "scan must not cross the junction");
        clean_dir(cache.path());
        assert!(precious.exists(), "the junction target must survive");
        assert!(!junction.exists(), "the link itself is removed");
    }

    #[test]
    fn orphan_removal_refuses_paths_outside_appdata() {
        let dir = tempfile::tempdir().unwrap();
        populate(dir.path());
        let result = remove_orphan(dir.path());
        assert_eq!(result.skipped, 1);
        assert_eq!(result.removed_files, 0);
        assert!(dir.path().join("one.txt").exists());
    }

    #[test]
    fn missing_directories_are_empty_results() {
        assert_eq!(scan_dir(Path::new(r"C:\WindowCraft\none")), ScanResult::default());
        assert_eq!(clean_dir(Path::new(r"C:\WindowCraft\none")), CleanResult::default());
    }

    #[test]
    fn unexpanded_paths_are_dropped() {
        let target = target("x", "x", "x", &[r"%WC_UNSET_VAR%\Cache", r"relative\path"]);
        assert!(expanded_paths(&target).is_empty());
    }

    #[test]
    fn every_target_uses_environment_rooted_paths() {
        let ids: Vec<_> = targets().iter().map(|target| target.id).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len());
        for target in targets() {
            assert!(target.paths.iter().all(|path| path.starts_with('%')));
        }
    }

    #[test]
    fn targets_contains_single_iso_target_ending_with_windowcraft_iso() {
        let iso_entries: Vec<_> = targets().into_iter().filter(|target| target.id == ISO_TARGET).collect();
        assert_eq!(iso_entries.len(), 1);
        assert_eq!(iso_entries[0].paths.len(), 1);
        assert!(iso_entries[0].paths[0].ends_with(r"WindowCraft\iso"));
    }

    #[test]
    fn a_mounted_image_blocks_only_the_iso_target() {
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("mount");
        fs::create_dir_all(&mount).unwrap();
        fs::write(mount.join("mounted.wim"), [1u8; 100]).unwrap();

        let sibling = dir.path().join("other.txt");
        fs::write(&sibling, [2u8; 200]).unwrap();

        let iso_target = target(ISO_TARGET, "ISO Studio work files", "WindowCraft", &[dir.path().to_str().unwrap()]);
        let target_result = clean(&iso_target);
        assert_eq!(target_result.skipped, 1);
        assert_eq!(target_result.removed_files, 0);
        assert_eq!(target_result.removed_bytes, 0);
        assert!(sibling.exists());
        assert!(mount.join("mounted.wim").exists());

        // Any other cache with a folder named mount is cleaned as before.
        let cache = target("npm", "npm cache", "Developer Caches", &[dir.path().to_str().unwrap()]);
        assert_eq!(clean(&cache).skipped, 0);
        assert!(!sibling.exists());
    }

    #[test]
    fn empty_mount_subfolder_does_not_block_cleaning() {
        let dir = tempfile::tempdir().unwrap();
        let mount = dir.path().join("mount");
        fs::create_dir_all(&mount).unwrap();

        let sibling = dir.path().join("other.txt");
        fs::write(&sibling, [2u8; 200]).unwrap();

        let dir_result = clean_dir(dir.path());
        assert_eq!(dir_result.removed_files, 1);
        assert_eq!(dir_result.removed_bytes, 200);
        assert_eq!(dir_result.skipped, 0);
        assert!(!sibling.exists());

        let dir2 = tempfile::tempdir().unwrap();
        let mount2 = dir2.path().join("mount");
        fs::create_dir_all(&mount2).unwrap();
        let sibling2 = dir2.path().join("other.txt");
        fs::write(&sibling2, [2u8; 200]).unwrap();

        let iso_target = target(ISO_TARGET, "ISO Studio work files", "WindowCraft", &[dir2.path().to_str().unwrap()]);
        let target_result = clean(&iso_target);
        assert_eq!(target_result.removed_files, 1);
        assert_eq!(target_result.removed_bytes, 200);
        assert_eq!(target_result.skipped, 0);
        assert!(!sibling2.exists());
    }
}
