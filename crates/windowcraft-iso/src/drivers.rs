// Made by ycangignacy
//! Driver export, deduplication and injection.

use crate::error::{IsoError, Result};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use windowcraft_engine::dism::{DismSession, DriverInfo};

/// Classes that must exist in `boot.wim` so Setup can see the target disk.
pub const BOOT_CLASSES: [&str; 2] = ["SCSIAdapter", "HDC"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfFacts {
    pub class: String,
    /// `(year, month, day)` from `DriverVer`.
    pub date: (u32, u32, u32),
    pub version: Vec<u32>,
}

/// Reads `Class` and `DriverVer` from INF text. Lines may be UTF-16 on disk;
/// callers pass decoded text.
pub fn parse_inf(text: &str) -> Option<InfFacts> {
    let mut class = String::new();
    let mut driver_ver = None;
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or_default().trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim().trim_matches('"'));
        if key.eq_ignore_ascii_case("Class") && class.is_empty() {
            class = value.to_owned();
        } else if key.eq_ignore_ascii_case("DriverVer") && driver_ver.is_none() {
            driver_ver = Some(value.to_owned());
        }
    }

    let driver_ver = driver_ver?;
    let (date, version) = driver_ver.split_once(',')?;
    let mut parts = date.trim().split(['/', '-']).filter_map(|part| part.trim().parse::<u32>().ok());
    let (month, day, year) = (parts.next()?, parts.next()?, parts.next()?);
    let version = version.trim().split('.').filter_map(|part| part.trim().parse().ok()).collect();
    Some(InfFacts { class, date: (year, month, day), version })
}

fn decode(bytes: &[u8]) -> String {
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverPackage {
    /// Folder holding the `.inf`, `.sys` and catalog files.
    pub directory: PathBuf,
    pub inf: PathBuf,
    pub facts: InfFacts,
}

impl DriverPackage {
    pub fn is_boot_critical(&self) -> bool {
        BOOT_CLASSES.iter().any(|class| class.eq_ignore_ascii_case(&self.facts.class))
    }

    fn key(&self) -> String {
        self.inf.file_name().map_or_else(String::new, |name| name.to_string_lossy().to_lowercase())
    }
}

/// Keeps the newest package per INF name, comparing `DriverVer` date, then
/// version.
pub fn deduplicate(packages: Vec<DriverPackage>) -> Vec<DriverPackage> {
    let mut newest: HashMap<String, DriverPackage> = HashMap::new();
    for package in packages {
        let key = package.key();
        match newest.get(&key) {
            Some(current)
                if (current.facts.date, &current.facts.version) >= (package.facts.date, &package.facts.version) => {}
            _ => {
                newest.insert(key, package);
            }
        }
    }
    let mut unique: Vec<DriverPackage> = newest.into_values().collect();
    unique.sort_by(|a, b| a.inf.cmp(&b.inf));
    unique
}

/// Splits packages into boot-image and install-image sets.
pub fn partition_by_target(packages: Vec<DriverPackage>) -> (Vec<DriverPackage>, Vec<DriverPackage>) {
    packages.into_iter().partition(DriverPackage::is_boot_critical)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination).map_err(|e| IsoError::io(destination.display().to_string(), e))?;
    let entries: Vec<_> = std::fs::read_dir(source)
        .map_err(|e| IsoError::io(source.display().to_string(), e))?
        .flatten()
        .collect();
    entries.par_iter().try_for_each(|entry| {
        let target = destination.join(entry.file_name());
        let kind = entry.file_type().map_err(|e| IsoError::io(entry.path().display().to_string(), e))?;
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).map(drop).map_err(|e| IsoError::io(target.display().to_string(), e))
        } else {
            Ok(())
        }
    })
}

fn package_from(info: &DriverInfo, export_root: &Path) -> Option<DriverPackage> {
    let directory = info.inf_path.parent()?;
    let bytes = std::fs::read(&info.inf_path).ok()?;
    let facts = parse_inf(&decode(&bytes))?;
    let folder = directory.file_name()?;
    let exported = export_root.join(folder);
    copy_tree(directory, &exported).ok()?;
    Some(DriverPackage { inf: exported.join(info.inf_path.file_name()?), directory: exported, facts })
}

/// Copies third-party drivers from the driver store into `export_root`.
pub fn export_system_drivers(export_root: &Path) -> Result<Vec<DriverPackage>> {
    let drivers: Vec<DriverInfo> = DismSession::online()?.drivers()?;
    let packages = drivers
        .par_iter()
        .filter(|info| !info.in_box)
        .filter_map(|info| package_from(info, export_root))
        .collect();
    Ok(deduplicate(packages))
}

/// Injects packages into a mounted image.
pub fn inject(mount: &Path, packages: &[DriverPackage]) -> Result<usize> {
    let session = DismSession::image(mount)?;
    let mut injected = 0;
    for package in packages {
        session.add_driver(&package.inf)?;
        injected += 1;
    }
    Ok(injected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, class: &str, date: (u32, u32, u32), version: &[u32]) -> DriverPackage {
        DriverPackage {
            directory: PathBuf::from(name),
            inf: PathBuf::from(format!(r"C:\export\{name}")),
            facts: InfFacts { class: class.to_owned(), date, version: version.to_vec() },
        }
    }

    #[test]
    fn parses_inf_metadata() {
        let inf = "[Version]\r\nSignature = \"$Windows NT$\"\r\nClass=SCSIAdapter ; storage\r\n\
                   ClassGuid={4D36E97B-E325-11CE-BFC1-08002BE10318}\r\nDriverVer = 04/12/2024, 10.1.20.3\r\n";
        let facts = parse_inf(inf).unwrap();
        assert_eq!(facts.class, "SCSIAdapter");
        assert_eq!(facts.date, (2024, 4, 12));
        assert_eq!(facts.version, [10, 1, 20, 3]);
    }

    #[test]
    fn inf_without_driver_ver_is_rejected() {
        assert!(parse_inf("[Version]\nClass=Net\n").is_none());
        assert!(parse_inf("DriverVer = garbage").is_none());
    }

    #[test]
    fn comments_do_not_leak_into_values() {
        let facts = parse_inf(";Class=Wrong\nClass=Net\nDriverVer=01/02/2023,1.0\n").unwrap();
        assert_eq!(facts.class, "Net");
    }

    #[test]
    fn utf16_inf_files_decode() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("Class=Net".encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode(&bytes), "Class=Net");
    }

    #[test]
    fn deduplication_keeps_the_newest_date_then_version() {
        let packages = vec![
            package("net.inf", "Net", (2022, 1, 1), &[1, 0]),
            package("net.inf", "Net", (2024, 1, 1), &[1, 0]),
            package("net.inf", "Net", (2023, 1, 1), &[9, 0]),
            package("gpu.inf", "Display", (2024, 1, 1), &[1, 0]),
            package("gpu.inf", "Display", (2024, 1, 1), &[2, 0]),
        ];
        let unique = deduplicate(packages);
        assert_eq!(unique.len(), 2);
        let net = unique.iter().find(|p| p.facts.class == "Net").unwrap();
        assert_eq!(net.facts.date, (2024, 1, 1));
        let gpu = unique.iter().find(|p| p.facts.class == "Display").unwrap();
        assert_eq!(gpu.facts.version, [2, 0]);
    }

    #[test]
    fn storage_drivers_go_to_the_boot_image() {
        let packages = vec![
            package("nvme.inf", "SCSIAdapter", (2024, 1, 1), &[1]),
            package("sata.inf", "hdc", (2024, 1, 1), &[1]),
            package("net.inf", "Net", (2024, 1, 1), &[1]),
        ];
        let (boot, install) = partition_by_target(packages);
        assert_eq!(boot.len(), 2);
        assert_eq!(install.len(), 1);
    }

    #[test]
    fn copy_tree_preserves_structure() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::create_dir(source.path().join("sub")).unwrap();
        std::fs::write(source.path().join("a.inf"), b"a").unwrap();
        std::fs::write(source.path().join("sub/b.sys"), b"b").unwrap();
        copy_tree(source.path(), &target.path().join("pkg")).unwrap();
        assert_eq!(std::fs::read(target.path().join("pkg/sub/b.sys")).unwrap(), b"b");
    }
}
