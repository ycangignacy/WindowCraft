// Made by WindowCraft Dev Team
//! Unattended command line.

use crate::actions::{preset_profile, run_profile};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use windowcraft_config::{Catalog, PresetName, ResolvedProfile, import_any};
use windowcraft_engine::jobs::CancelToken;
use windowcraft_engine::packages::PackageManager;
use windowcraft_engine::{Engine, purge, report};

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(
    name = "windowcraft",
    version,
    about = "Windows 11 administration, debloating and ISO mastering",
    after_help = "Made by WindowCraft Dev Team"
)]
pub struct Cli {
    /// Preset to load: minimal, standard or advanced
    #[arg(long, conflicts_with = "config")]
    pub preset: Option<PresetArg>,
    /// WindowCraft profile or WinUtil JSON file
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Apply the loaded preset or profile, then exit
    #[arg(long)]
    pub run: bool,
    /// Report tweaks reset since they were applied
    #[arg(long)]
    pub check_drift: bool,
    /// With --check-drift, re-apply what drifted
    #[arg(long, requires = "check_drift")]
    pub fix: bool,
    /// Write a privacy-safe environment report
    #[arg(long, value_name = "PATH")]
    pub export_report: Option<PathBuf>,
    /// Module to open: applications, tweaks, system, updates, iso, timemachine, inspector, extensions or settings
    #[arg(long, value_name = "MODULE")]
    pub open: Option<StartPage>,
    /// Open the X-Ray Inspector on a tweak id
    #[arg(long, value_name = "TWEAK")]
    pub inspect: Option<String>,
    /// Delete WindowCraft's own data, including Time Machine snapshots. Changes already applied stay and can no longer be rolled back
    #[arg(long, conflicts_with_all = ["run", "check_drift", "export_report"])]
    pub purge_data: bool,
    /// End every WindowCraft process started from this folder. The uninstaller runs it before it removes files
    #[arg(long, conflicts_with_all = ["run", "check_drift", "export_report", "purge_data"])]
    pub close_running: bool,
    /// Run by the scheduled task from Settings: checks for reverted tweaks and shows a notification
    #[arg(long, hide = true, conflicts_with_all = ["run", "check_drift", "export_report", "purge_data", "close_running"])]
    pub scheduled_check: bool,
}

/// Newtype so clap can parse the app's page enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartPage(pub crate::app::Page);

impl std::str::FromStr for StartPage {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

/// Newtype so clap can parse the config crate's enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetArg(pub PresetName);

impl std::str::FromStr for PresetArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse().map(Self)
    }
}

impl Cli {
    /// True when any flag asks for headless work.
    pub fn is_headless(&self) -> bool {
        self.run
            || self.check_drift
            || self.export_report.is_some()
            || self.purge_data
            || self.close_running
            || self.scheduled_check
    }
}

/// Loads the selection named on the command line, if any.
pub fn load_selection(cli: &Cli, catalog: &Catalog) -> Result<Option<ResolvedProfile>, String> {
    if cli.config.is_some() || cli.preset.is_some() { load_profile(cli, catalog).map(Some) } else { Ok(None) }
}

fn load_profile(cli: &Cli, catalog: &Catalog) -> Result<ResolvedProfile, String> {
    if let Some(path) = &cli.config {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        return import_any(&text, catalog).map_err(|e| e.to_string());
    }
    if let Some(preset) = cli.preset {
        return preset_profile(catalog, preset.0).resolve(catalog).map_err(|e| e.to_string());
    }
    Err("--run needs --preset or --config".to_owned())
}

/// "1 item", "2 items". The CLI speaks English only.
fn counted(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// A folder gets the report under a fixed name inside it.
fn report_path(path: &std::path::Path) -> PathBuf {
    if path.is_dir() { path.join("windowcraft-report.json") } else { path.to_path_buf() }
}

fn run(cli: &Cli) -> Result<(), String> {
    if cli.close_running {
        return close_running();
    }
    if cli.scheduled_check {
        return crate::background::run();
    }
    // Before the engine, which would recreate the data folder.
    if cli.purge_data {
        return purge();
    }

    // Failing here beats one "access denied" per tweak.
    if (cli.run || cli.fix) && !windowcraft_engine::sysinfo::is_elevated() {
        return Err("this command changes system settings: run it from an elevated terminal".to_owned());
    }
    let catalog = Arc::new(Catalog::load()?);
    let engine = Engine::system(catalog.clone()).map_err(|e| e.to_string())?;

    if let Some(path) = &cli.export_report {
        let path = &report_path(path);
        let report = report::build(&engine).map_err(|e| e.to_string())?;
        let text = report::to_json(&report).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        println!("Report written to {}", path.display());
    }

    if cli.check_drift {
        let drifted = engine.detect_drift().map_err(|e| e.to_string())?;
        match Engine::drift_summary(&drifted) {
            None => println!("No drift detected."),
            Some(summary) => {
                println!("{summary}");
                for drift in &drifted {
                    println!("  {}", drift.tweak_name);
                    for (target, expected, live) in &drift.entries {
                        println!("    {target}: expected {expected}, found {live}");
                    }
                }
                if cli.fix {
                    let repaired = engine.repair_drift().map_err(|e| e.to_string())?;
                    println!("Re-applied {}.", counted(repaired, "tweak", "tweaks"));
                }
            }
        }
    }

    if cli.run {
        let resolved = load_profile(cli, &catalog)?;
        let summary = run_profile(&engine, &resolved, PackageManager::Winget, &CancelToken::new(), |fraction, step| {
            println!("[{:>3.0}%] {step}", fraction * 100.0);
        })
        .map_err(|e| e.to_string())?;
        println!("Applied {}.", counted(summary.applied, "item", "items"));
        if !summary.failed.is_empty() {
            for (name, reason) in &summary.failed {
                eprintln!("Failed: {name}: {reason}");
            }
            return Err(format!("{} failed", counted(summary.failed.len(), "item", "items")));
        }
    }
    Ok(())
}

fn close_running() -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let folder = executable.parent().ok_or("the executable has no folder")?;
    let ended = windowcraft_engine::shell::close_processes_in(folder, std::time::Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    println!("WindowCraft processes closed in {}: {ended}", folder.display());
    Ok(())
}

fn purge() -> Result<(), String> {
    let summary = purge::purge_data().map_err(|e| e.to_string())?;
    for item in &summary.removed {
        println!("Removed {item}");
    }
    for item in &summary.absent {
        println!("Nothing at {item}");
    }
    if summary.failed.is_empty() {
        return Ok(());
    }
    for (item, reason) in &summary.failed {
        eprintln!("Failed: {item}: {reason}");
    }
    Err("some data could not be removed: close WindowCraft and try again".to_owned())
}

pub fn execute(cli: &Cli) -> ExitCode {
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("windowcraft").chain(arguments.iter().copied()))
    }

    #[test]
    fn documented_invocations_parse() {
        let cli = parse(&["--preset", "standard", "--run"]).unwrap();
        assert_eq!(cli.preset, Some(PresetArg(PresetName::Standard)));
        assert!(cli.run && cli.is_headless());

        let cli = parse(&["--config", r"C:\path\to\config.json", "--run"]).unwrap();
        assert_eq!(cli.config, Some(PathBuf::from(r"C:\path\to\config.json")));

        let cli = parse(&["--check-drift", "--fix"]).unwrap();
        assert!(cli.check_drift && cli.fix);

        let cli = parse(&["--export-report", r"C:\report.json"]).unwrap();
        assert_eq!(cli.export_report, Some(PathBuf::from(r"C:\report.json")));
    }

    #[test]
    fn counts_take_the_singular_for_one_only() {
        assert_eq!(counted(1, "item", "items"), "1 item");
        assert_eq!(counted(0, "item", "items"), "0 items");
        assert_eq!(counted(21, "tweak", "tweaks"), "21 tweaks");
    }

    #[test]
    fn a_report_folder_gets_a_file_name() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(report_path(dir.path()), dir.path().join("windowcraft-report.json"));
        let file = dir.path().join("mine.json");
        assert_eq!(report_path(&file), file);
    }

    #[test]
    fn no_arguments_opens_the_interface() {
        assert!(!parse(&[]).unwrap().is_headless());
    }

    #[test]
    fn deep_links_open_the_interface() {
        let cli = parse(&["--open", "tweaks", "--inspect", "telemetry"]).unwrap();
        assert_eq!(cli.open, Some(StartPage(crate::app::Page::Tweaks)));
        assert_eq!(cli.inspect.as_deref(), Some("telemetry"));
        assert!(!cli.is_headless());
        assert!(parse(&["--open", "nowhere"]).is_err());
    }

    #[test]
    fn invalid_combinations_are_rejected() {
        assert!(parse(&["--fix"]).is_err(), "--fix needs --check-drift");
        assert!(parse(&["--preset", "turbo"]).is_err());
        assert!(parse(&["--preset", "standard", "--config", "x.json"]).is_err());
    }

    #[test]
    fn a_config_without_run_is_loaded_for_review() {
        let catalog = Catalog::load().unwrap();
        let cli = parse(&["--preset", "minimal"]).unwrap();
        assert!(!cli.is_headless());
        assert_eq!(load_selection(&cli, &catalog).unwrap().unwrap().profile.tweaks.len(), 6);
        assert!(load_selection(&parse(&[]).unwrap(), &catalog).unwrap().is_none());
    }

    #[test]
    fn run_without_a_selection_is_an_error() {
        let catalog = Catalog::load().unwrap();
        let cli = parse(&["--run"]).unwrap();
        assert!(load_profile(&cli, &catalog).is_err());
    }

    #[test]
    fn purge_data_is_headless_and_stands_alone() {
        assert!(parse(&["--purge-data"]).unwrap().is_headless());
        assert!(parse(&["--purge-data", "--run", "--preset", "minimal"]).is_err());
        assert!(parse(&["--purge-data", "--check-drift"]).is_err());
        assert!(parse(&["--purge-data", "--export-report", "out.json"]).is_err());
    }

    #[test]
    fn close_running_is_headless_and_stands_alone() {
        assert!(parse(&["--close-running"]).unwrap().is_headless());
        assert!(parse(&["--close-running", "--purge-data"]).is_err());
        assert!(parse(&["--close-running", "--run", "--preset", "minimal"]).is_err());
    }

    #[test]
    fn the_scheduled_check_is_headless_and_stands_alone() {
        assert!(parse(&["--scheduled-check"]).unwrap().is_headless());
        assert!(parse(&["--scheduled-check", "--purge-data"]).is_err());
        assert!(parse(&["--scheduled-check", "--check-drift"]).is_err());
    }

    #[test]
    fn config_files_load_from_disk() {
        let catalog = Catalog::load().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"WPFTweaks":["WPFTweaksActivity"]}"#).unwrap();
        let cli = Cli {
            preset: None,
            config: Some(path),
            run: true,
            check_drift: false,
            fix: false,
            export_report: None,
            open: None,
            inspect: None,
            purge_data: false,
            close_running: false,
            scheduled_check: false,
        };
        assert_eq!(load_profile(&cli, &catalog).unwrap().profile.tweaks, ["activity"]);
    }

    #[test]
    fn all_module_names_and_presets_parse_case_insensitively() {
        use crate::app::Page;
        let modules = [
            ("applications", Page::Applications),
            ("apps", Page::Applications),
            ("APPS", Page::Applications),
            ("tweaks", Page::Tweaks),
            ("system", Page::System),
            ("updates", Page::Updates),
            ("iso", Page::Iso),
            ("extensions", Page::Extensions),
            ("misc", Page::Extensions),
            ("timemachine", Page::TimeMachine),
            ("TimeMachine", Page::TimeMachine),
            ("inspector", Page::Inspector),
        ];
        for (arg, expected) in modules {
            let cli = parse(&["--open", arg]).unwrap();
            assert_eq!(cli.open, Some(StartPage(expected)), "for --open {arg}");
        }

        let presets = [
            ("MINIMAL", PresetName::Minimal),
            ("Standard", PresetName::Standard),
            ("advanced", PresetName::Advanced),
            ("ADVANCED", PresetName::Advanced),
        ];
        for (arg, expected) in presets {
            let cli = parse(&["--preset", arg]).unwrap();
            assert_eq!(cli.preset, Some(PresetArg(expected)), "for --preset {arg}");
        }
    }

    #[test]
    fn missing_malformed_or_unknown_id_config_files_report_clear_errors() {
        let catalog = Catalog::load().unwrap();
        let dir = tempfile::tempdir().unwrap();

        let missing = dir.path().join("missing.json");
        let cli = parse(&["--config", missing.to_str().unwrap()]).unwrap();
        let err = load_selection(&cli, &catalog).unwrap_err();
        assert!(err.contains("missing.json"), "{err}");

        let malformed = dir.path().join("bad.json");
        std::fs::write(&malformed, "{not valid json").unwrap();
        let cli = parse(&["--config", malformed.to_str().unwrap()]).unwrap();
        let err = load_selection(&cli, &catalog).unwrap_err();
        assert!(err.contains("not valid JSON"), "{err}");

        let unknown = dir.path().join("unknown.json");
        std::fs::write(&unknown, r#"{"version":1,"tweaks":["nonexistent-tweak"]}"#).unwrap();
        let cli = parse(&["--config", unknown.to_str().unwrap()]).unwrap();
        let err = load_selection(&cli, &catalog).unwrap_err();
        assert!(err.contains("nonexistent-tweak"), "{err}");
    }

    #[test]
    fn headless_detection_and_unexpected_arguments_behave_consistently() {
        let drift_only = parse(&["--check-drift"]).unwrap();
        assert!(drift_only.is_headless());
        assert!(!drift_only.fix);

        let config_review = parse(&["--config", "review.json", "--open", "tweaks"]).unwrap();
        assert!(!config_review.is_headless());

        let config_and_report = parse(&["--config", "review.json", "--export-report", "out.json"]).unwrap();
        assert!(config_and_report.is_headless());

        assert!(parse(&["positional"]).is_err());
        assert!(parse(&["--no-such-flag"]).is_err());

        if !windowcraft_engine::sysinfo::is_elevated() {
            let run_cli = parse(&["--preset", "minimal", "--run"]).unwrap();
            assert!(run(&run_cli).unwrap_err().contains("elevated terminal"));
            let fix_cli = parse(&["--check-drift", "--fix"]).unwrap();
            assert!(run(&fix_cli).unwrap_err().contains("elevated terminal"));
        }
    }
}
