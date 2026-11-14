// Made by Nordan
//! WinGet and Chocolatey, plus a fast scan of installed programs.

use crate::download::{block_on, download, fetch_text};
use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use crate::process::{Output, run};
use crate::registry::{Hive, RegPath, RegValue, RegistryScope};
use crate::win::system32;
use rayon::prelude::*;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use windowcraft_config::App;

const INSTALL_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const POWERSHELL_ID: &str = "Microsoft.PowerShell";
/// App Installer registers itself shortly after a first logon.
const WINGET_WAIT_SECONDS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackageManager {
    #[default]
    Winget,
    Chocolatey,
}

pub fn winget_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    let path = PathBuf::from(local).join(r"Microsoft\WindowsApps\winget.exe");
    path.exists().then_some(path)
}

pub fn choco_path() -> Option<PathBuf> {
    let root = std::env::var_os("ProgramData")?;
    let path = PathBuf::from(root).join(r"chocolatey\bin\choco.exe");
    path.exists().then_some(path)
}

/// Exit codes WinGet uses for outcomes that are not failures.
fn winget_succeeded(code: i32) -> bool {
    const NO_APPLICABLE_UPDATE: u32 = 0x8A15_002B;
    const ALREADY_INSTALLED: u32 = 0x8A15_0061;
    const REBOOT_REQUIRED: i32 = 3010;
    let unsigned = code.cast_unsigned();
    code == 0 || code == REBOOT_REQUIRED || unsigned == NO_APPLICABLE_UPDATE || unsigned == ALREADY_INSTALLED
}

fn execute(program: &Path, args: &[&str], cancel: &CancelToken, log: &mut dyn FnMut(&str)) -> Result<Output> {
    run(program, args, cancel, INSTALL_TIMEOUT, |line| log(line))
}

fn fail(program: &str, output: &Output) -> EngineError {
    EngineError::Process {
        program: program.to_owned(),
        code: output.code,
        detail: output.text.lines().rev().find(|line| !line.is_empty()).unwrap_or_default().to_owned(),
    }
}

/// Finds WinGet, repairing App Installer and waiting for it when needed.
pub fn ensure_winget(cancel: &CancelToken) -> Result<PathBuf> {
    if let Some(path) = winget_path() {
        return Ok(path);
    }
    // Absent on a brand-new profile, and repairable when only unregistered.
    let _ = crate::fixes::repair_winget();
    for _ in 0..WINGET_WAIT_SECONDS {
        cancel.check()?;
        if let Some(path) = winget_path() {
            return Ok(path);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Err(EngineError::NotFound("winget.exe".to_owned()))
}

/// Installs Chocolatey through WinGet, without its `iex` bootstrap script.
pub fn bootstrap_chocolatey(cancel: &CancelToken) -> Result<()> {
    if choco_path().is_some() {
        return Ok(());
    }
    let winget = ensure_winget(cancel)?;
    let args = [
        "install", "--id", "Chocolatey.Chocolatey", "--exact", "--silent", "--accept-package-agreements",
        "--accept-source-agreements", "--disable-interactivity",
    ];
    let output = run(&winget, &args, cancel, INSTALL_TIMEOUT, |_| {})?;
    if !winget_succeeded(output.code) {
        return Err(fail("winget", &output));
    }
    if choco_path().is_some() {
        Ok(())
    } else {
        Err(EngineError::NotFound("choco.exe after installation".to_owned()))
    }
}

/// PowerShell 7 from the MSI, which must not be replaced by the Store build.
fn powershell_is_msi_based() -> bool {
    installed_programs().iter().any(|program| program.name.starts_with("PowerShell 7"))
}

pub fn install(manager: PackageManager, app: &App, cancel: &CancelToken, log: &mut dyn FnMut(&str)) -> Result<()> {
    match manager {
        PackageManager::Winget => {
            let id = app.winget.as_deref().ok_or_else(|| EngineError::NotFound(format!("WinGet id for {}", app.name)))?;
            if id == POWERSHELL_ID && powershell_is_msi_based() {
                let architecture = crate::sysinfo::query()?.architecture;
                return upgrade_powershell_msi(architecture, cancel, log);
            }
            let winget = ensure_winget(cancel)?;
            let args = [
                "install", "--id", id, "--exact", "--silent", "--accept-package-agreements",
                "--accept-source-agreements", "--disable-interactivity",
            ];
            let output = execute(&winget, &args, cancel, log)?;
            if winget_succeeded(output.code) { Ok(()) } else { Err(fail("winget", &output)) }
        }
        PackageManager::Chocolatey => {
            let id = app.choco.as_deref().ok_or_else(|| EngineError::NotFound(format!("Chocolatey id for {}", app.name)))?;
            bootstrap_chocolatey(cancel)?;
            let choco = choco_path().ok_or_else(|| EngineError::NotFound("choco.exe".to_owned()))?;
            let output = execute(&choco, &["install", id, "--yes", "--no-progress"], cancel, log)?;
            if output.code == 0 || output.code == 3010 { Ok(()) } else { Err(fail("choco", &output)) }
        }
    }
}

pub fn uninstall(manager: PackageManager, app: &App, cancel: &CancelToken, log: &mut dyn FnMut(&str)) -> Result<()> {
    match manager {
        PackageManager::Winget => {
            let id = app.winget.as_deref().ok_or_else(|| EngineError::NotFound(format!("WinGet id for {}", app.name)))?;
            let winget = ensure_winget(cancel)?;
            let args = ["uninstall", "--id", id, "--exact", "--silent", "--disable-interactivity"];
            let output = execute(&winget, &args, cancel, log)?;
            if output.code == 0 { Ok(()) } else { Err(fail("winget", &output)) }
        }
        PackageManager::Chocolatey => {
            let id = app.choco.as_deref().ok_or_else(|| EngineError::NotFound(format!("Chocolatey id for {}", app.name)))?;
            let choco = choco_path().ok_or_else(|| EngineError::NotFound("choco.exe".to_owned()))?;
            let output = execute(&choco, &["uninstall", id, "--yes"], cancel, log)?;
            if output.code == 0 { Ok(()) } else { Err(fail("choco", &output)) }
        }
    }
}

pub fn upgrade_all(manager: PackageManager, cancel: &CancelToken, log: &mut dyn FnMut(&str)) -> Result<()> {
    match manager {
        PackageManager::Winget => {
            let winget = ensure_winget(cancel)?;
            let args = [
                "upgrade", "--all", "--silent", "--accept-package-agreements", "--accept-source-agreements",
                "--disable-interactivity",
            ];
            let output = execute(&winget, &args, cancel, log)?;
            if winget_succeeded(output.code) { Ok(()) } else { Err(fail("winget", &output)) }
        }
        PackageManager::Chocolatey => {
            let choco = choco_path().ok_or_else(|| EngineError::NotFound("choco.exe".to_owned()))?;
            let output = execute(&choco, &["upgrade", "all", "--yes", "--no-progress"], cancel, log)?;
            if output.code == 0 || output.code == 3010 { Ok(()) } else { Err(fail("choco", &output)) }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledProgram {
    pub name: String,
    pub version: String,
    pub publisher: String,
}

fn read_uninstall_entry(hive: Hive, base: &str, subkey: &str) -> Option<InstalledProgram> {
    let path = RegPath { hive, subkey: format!(r"{base}\{subkey}") };
    let key = RegistryScope::System.open(&path).ok()??;
    let text = |name: &str| match key.read(name) {
        Ok(Some(RegValue::String(value) | RegValue::ExpandString(value))) => value,
        _ => String::new(),
    };
    let name = text("DisplayName");
    if name.is_empty() || matches!(key.read("SystemComponent"), Ok(Some(RegValue::DWord(1)))) {
        return None;
    }
    Some(InstalledProgram { name, version: text("DisplayVersion"), publisher: text("Publisher") })
}

/// Scans the machine and user Uninstall keys on the rayon pool.
pub fn installed_programs() -> Vec<InstalledProgram> {
    const LOCATIONS: [(Hive, &str); 3] = [
        (Hive::LocalMachine, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall"),
        (Hive::LocalMachine, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (Hive::CurrentUser, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];
    let mut programs: Vec<InstalledProgram> = LOCATIONS
        .par_iter()
        .flat_map(|(hive, base)| {
            let path = RegPath { hive: *hive, subkey: (*base).to_owned() };
            let names = RegistryScope::System
                .open(&path)
                .ok()
                .flatten()
                .and_then(|key| key.subkeys().ok())
                .unwrap_or_default();
            names.into_par_iter().filter_map(|name| read_uninstall_entry(*hive, base, &name)).collect::<Vec<_>>()
        })
        .collect();
    programs.sort_by_cached_key(|program| program.name.to_lowercase());
    programs.dedup_by(|a, b| a.name == b.name && a.version == b.version);
    programs
}

/// True when `needle` appears in `text` as whole words.
fn contains_words(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(start, found)| {
        let before = text[..start].chars().next_back();
        let after = text[start + found.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Matches a catalog app to an installed program.
///
/// Uninstall entries add vendors and versions ("Mozilla Firefox (x64 en-US)"),
/// so whole-word matching finds them, while "GitHub Desktop" is not "Git".
pub fn match_installed<'a>(app: &App, programs: &'a [InstalledProgram]) -> Option<&'a InstalledProgram> {
    let wanted = app.name.to_lowercase();
    let exact = programs.iter().find(|program| program.name.to_lowercase() == wanted);
    if exact.is_some() || wanted.chars().count() < 3 {
        return exact;
    }
    programs.iter().find(|program| contains_words(&program.name.to_lowercase(), &wanted))
}

/// Parses `winget upgrade` table output into `(id, available version)`.
pub fn parse_winget_upgrades(text: &str) -> Vec<(String, String)> {
    // Spinner output shares a line with the header, split by carriage returns.
    let lines: Vec<&str> = text.lines().map(|line| line.rsplit('\r').next().unwrap_or(line).trim_end()).collect();
    // Column titles are translated, so the dashed rule is the only stable marker.
    let is_rule = |line: &&str| line.len() >= 8 && line.chars().all(|c| c == '-');
    let Some(rule) = lines.iter().position(is_rule) else { return Vec::new() };
    let Some(header) = rule.checked_sub(1).and_then(|index| lines.get(index)) else { return Vec::new() };

    let characters: Vec<char> = header.chars().collect();
    let starts: Vec<usize> = (0..characters.len())
        .filter(|&index| !characters[index].is_whitespace() && (index == 0 || characters[index - 1].is_whitespace()))
        .collect();
    let (Some(&id_at), Some(&version_at), Some(&available_at)) = (starts.get(1), starts.get(2), starts.get(3)) else {
        return Vec::new();
    };
    let source_at = starts.get(4).copied().unwrap_or(usize::MAX);

    let mut upgrades = Vec::new();
    for line in &lines[rule + 1..] {
        // Columns are counted in characters, because names can be non-ASCII.
        let row: Vec<char> = line.chars().collect();
        if row.len() <= available_at {
            continue;
        }
        let id: String = row[id_at..version_at].iter().collect();
        let available: String = row[available_at..source_at.min(row.len())].iter().collect();
        let (id, available) = (id.trim(), available.trim());
        if !id.is_empty() && !id.contains(' ') && !available.is_empty() {
            upgrades.push((id.to_owned(), available.to_owned()));
        }
    }
    upgrades
}

/// Latest versions for pending upgrades, from `winget upgrade`.
pub fn available_upgrades(cancel: &CancelToken) -> Result<Vec<(String, String)>> {
    let winget = winget_path().ok_or_else(|| EngineError::NotFound("winget.exe".to_owned()))?;
    let args = ["upgrade", "--include-unknown", "--accept-source-agreements", "--disable-interactivity"];
    let output = run(&winget, &args, cancel, Duration::from_secs(120), |_| {})?;
    Ok(parse_winget_upgrades(&output.text))
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Picks the architecture-matched MSI from a release's assets.
fn pick_msi<'a>(assets: &'a [Asset], architecture: &str) -> Option<&'a Asset> {
    let suffix = format!("-win-{}.msi", architecture.to_lowercase());
    assets.iter().find(|asset| asset.name.to_lowercase().ends_with(&suffix))
}

/// True when `winget pin` holds a package back, which must be honored.
fn is_pinned(id: &str, cancel: &CancelToken) -> Result<bool> {
    let Some(winget) = winget_path() else { return Ok(false) };
    let output = run(&winget, &["pin", "list", "--disable-interactivity"], cancel, Duration::from_secs(30), |_| {})?;
    Ok(output.text.lines().any(|line| line.contains(id)))
}

/// Updates stable PowerShell 7 from the signed MSI on GitHub, avoiding the
/// Store MSIX and its COM/DISM activation problems.
pub fn upgrade_powershell_msi(architecture: &str, cancel: &CancelToken, log: &mut dyn FnMut(&str)) -> Result<()> {
    if is_pinned("Microsoft.PowerShell", cancel)? {
        log("Microsoft.PowerShell is pinned; leaving it unchanged");
        return Ok(());
    }
    let release: Release = block_on(async {
        let text = fetch_text("https://api.github.com/repos/PowerShell/PowerShell/releases/latest").await?;
        serde_json::from_str::<Release>(&text).map_err(|e| EngineError::Network(e.to_string()))
    })??;
    let asset = pick_msi(&release.assets, architecture)
        .ok_or_else(|| EngineError::NotFound(format!("{architecture} MSI in {}", release.tag_name)))?;

    let destination = crate::win::local_app_dir().join(&asset.name);
    log(&format!("Downloading {}", asset.name));
    block_on(download(&asset.browser_download_url, &destination, None, cancel, |_, _| {}))??;

    let trust = crate::authenticode::verify_file(&destination);
    if !trust.trusted {
        let _ = std::fs::remove_file(&destination);
        return Err(EngineError::Invalid(format!("{} failed signature verification", asset.name)));
    }

    let path = destination.display().to_string();
    let output = run(&system32("msiexec.exe"), &["/i", &path, "/qn", "/norestart"], cancel, INSTALL_TIMEOUT, |line| {
        log(line);
    });
    // The installer is about 100 MB and Windows keeps its own copy.
    let _ = std::fs::remove_file(&destination);
    let output = output?;
    if output.code == 0 || output.code == 3010 { Ok(()) } else { Err(fail("msiexec", &output)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> App {
        App {
            id: name.to_owned(),
            name: name.to_owned(),
            category: String::new(),
            description: String::new(),
            link: String::new(),
            foss: false,
            winget: None,
            choco: None,
        }
    }

    fn program(name: &str) -> InstalledProgram {
        InstalledProgram { name: name.to_owned(), version: "1.0".to_owned(), publisher: String::new() }
    }

    #[test]
    fn winget_table_is_parsed_by_column() {
        let text = "\
Name               Id                  Version  Available  Source
-----------------------------------------------------------------
Git                Git.Git             2.44.0   2.45.1     winget
Node.js LTS        OpenJS.NodeJS.LTS   20.11.0  22.2.0     winget
2 upgrades available.";
        assert_eq!(
            parse_winget_upgrades(text),
            [("Git.Git".to_owned(), "2.45.1".to_owned()), ("OpenJS.NodeJS.LTS".to_owned(), "22.2.0".to_owned())]
        );
        assert!(parse_winget_upgrades("No installed package found").is_empty());
    }

    #[test]
    fn winget_table_with_non_ascii_names_does_not_panic() {
        let text = "Name   Id      Version Available Source\n--------\nÄÖÜÄÖÜÄÖÜÄÖÜÄÖÜÄÖÜÄÖÜ x 1 2 winget";
        let _ = parse_winget_upgrades(text);
    }

    #[test]
    fn translated_winget_headers_parse_the_same() {
        let polish = "\
Nazwa              Identyfikator       Wersja   Dostępne   Źródło
-----------------------------------------------------------------
Żółć Edytor        Zolc.Edytor         1.0.0    2.0.0      winget
Git                Git.Git             2.44.0   2.45.1     winget
Dostępne uaktualnienia: 2.";
        assert_eq!(
            parse_winget_upgrades(polish),
            [("Zolc.Edytor".to_owned(), "2.0.0".to_owned()), ("Git.Git".to_owned(), "2.45.1".to_owned())]
        );
    }

    #[test]
    fn spinner_output_before_the_header_is_ignored() {
        let text = "\r   -\r   \\\rName Id Version Available Source\n--------------------\nGit  G.G 1.0     2.0       winget\n";
        assert_eq!(parse_winget_upgrades(text), [("G.G".to_owned(), "2.0".to_owned())]);
    }

    #[test]
    fn nonfatal_winget_codes_are_success() {
        assert!(winget_succeeded(0));
        assert!(winget_succeeded(3010));
        assert!(winget_succeeded(0x8A15_002B_u32.cast_signed()));
        assert!(!winget_succeeded(1));
    }

    #[test]
    fn installed_scan_returns_quickly_with_names() {
        let programs = installed_programs();
        assert!(programs.iter().all(|program| !program.name.is_empty()));
    }

    #[test]
    fn matching_uses_whole_words() {
        let programs = [
            program("7-Zip 24.07 (x64)"),
            program("Git"),
            program("Mozilla Firefox (x64 en-US)"),
            program("Google Chrome"),
        ];
        assert!(match_installed(&app("7-Zip"), &programs).is_some());
        assert!(match_installed(&app("Git"), &programs).is_some());
        assert!(match_installed(&app("Firefox"), &programs).is_some(), "the vendor prefix must not hide it");
        assert!(match_installed(&app("Chrome"), &programs).is_some());
        assert!(match_installed(&app("Vivaldi"), &programs).is_none());
    }

    #[test]
    fn a_longer_product_name_is_not_a_match() {
        let programs = [program("GitHub Desktop"), program("Gitea"), program("Notepad++")];
        assert!(match_installed(&app("Git"), &programs).is_none());
        assert!(match_installed(&app("Go"), &[program("Google Chrome")]).is_none());
    }

    #[test]
    fn msi_selection_matches_architecture() {
        let asset = |name: &str| Asset { name: name.to_owned(), browser_download_url: String::new() };
        let assets = [asset("PowerShell-7.5.0-win-x64.msi"), asset("PowerShell-7.5.0-win-arm64.msi")];
        assert_eq!(pick_msi(&assets, "ARM64").unwrap().name, "PowerShell-7.5.0-win-arm64.msi");
        assert_eq!(pick_msi(&assets, "x64").unwrap().name, "PowerShell-7.5.0-win-x64.msi");
        assert!(pick_msi(&assets, "x86").is_none());
    }
}
