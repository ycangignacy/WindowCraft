// Made by ycangignacy
//! Operations shared by the GUI and the CLI.

use crate::i18n::{Language, Msg, format_1, format_2, t};
use std::collections::BTreeMap;
use windowcraft_config::{
    Catalog, NativeOp, PresetName, Profile, ResolvedProfile, ServiceRule, Tweak, TweakGroup, TweakKind,
};
use windowcraft_engine::error::Result;
use windowcraft_engine::jobs::CancelToken;
use windowcraft_engine::packages::{self, PackageManager};
use windowcraft_engine::{Detection, Engine, EngineError, TweakAction, dns};

/// Past this size the log rolls over to one `.old` file.
const LOG_LIMIT_BYTES: u64 = 1024 * 1024;

/// Appends a finished job to the log. A log failure never fails the job.
pub fn append_log(path: &std::path::Path, label: &str, lines: &[String]) {
    use std::io::Write;
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > LOG_LIMIT_BYTES) {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let time = windowcraft_engine::snapshot::unix_now();
        let _ = writeln!(file, "[{time}] {label}");
        for line in lines {
            let _ = writeln!(file, "  {line}");
        }
    }
}

/// Selection that turns a preset into a profile.
pub fn preset_profile(catalog: &Catalog, preset: PresetName) -> Profile {
    let mut profile = Profile::new();
    profile.tweaks = catalog.preset(preset).to_vec();
    profile
}

/// Wraps custom rules so they get the same snapshot and protected-service
/// guard as any catalog entry.
fn custom_services_tweak(rules: &[ServiceRule]) -> Tweak {
    Tweak {
        id: "custom-services".to_owned(),
        legacy_id: String::new(),
        name: "Custom service rules".to_owned(),
        description: String::new(),
        group: TweakGroup::Advanced,
        kind: TweakKind::Check,
        registry: Vec::new(),
        services: rules.to_vec(),
        prepare: Vec::new(),
        apply: Vec::new(),
        undo: Vec::new(),
        options: Vec::new(),
        notice: None,
        irreversible: false,
        exclusive: None,
    }
}

/// Applies the DNS selection. `Fastest` benchmarks first.
pub fn apply_dns(catalog: &Catalog, selection: &str, lang: Language) -> Result<String> {
    use windowcraft_engine::dns_bench::{Target, benchmark, fastest};
    use windowcraft_engine::download::block_on;
    match selection {
        "Default" | "DHCP" => {
            dns::reset_to_dhcp()?;
            Ok(t(lang, Msg::DnsReturnedToDhcp).to_owned())
        }
        "Fastest" => {
            let targets: Vec<Target> =
                catalog.dns.iter().filter(|provider| provider.benchmark).filter_map(Target::from_provider).collect();
            let results = block_on(benchmark(targets, 5))??;
            let winner = fastest(&results).ok_or_else(|| EngineError::Network("no resolver answered".to_owned()))?;
            let provider = catalog.dns_provider(winner).ok_or_else(|| EngineError::NotFound(winner.to_owned()))?;
            dns::set_provider(provider)?;
            Ok(format_1(t(lang, Msg::DnsSetFastest), winner))
        }
        name => {
            let provider = catalog.dns_provider(name).ok_or_else(|| EngineError::NotFound(name.to_owned()))?;
            let adapters = dns::set_provider(provider)?;
            Ok(format_2(t(lang, Msg::DnsSetOnAdapters), name, &adapters.to_string()))
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProfileSummary {
    pub applied: usize,
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
}

/// Runs a resolved profile end to end.
pub fn run_profile(
    engine: &Engine,
    resolved: &ResolvedProfile,
    manager: PackageManager,
    cancel: &CancelToken,
    mut progress: impl FnMut(f32, &str),
) -> Result<ProfileSummary> {
    let profile = &resolved.profile;
    let catalog = engine.catalog.clone();
    let custom = (!profile.services.is_empty()).then(|| custom_services_tweak(&profile.services));

    let mut items: Vec<(&Tweak, TweakAction)> = Vec::new();
    for id in &profile.tweaks {
        if let Some(tweak) = catalog.tweak(id) {
            items.push((tweak, TweakAction::Apply));
        }
    }
    for (id, enabled) in &profile.toggles {
        if let Some(tweak) = catalog.tweak(id) {
            items.push((tweak, TweakAction::Set(*enabled)));
        }
    }
    for (id, option) in &profile.segmented {
        if let Some(tweak) = catalog.tweak(id) {
            items.push((tweak, TweakAction::Select(option.clone())));
        }
    }
    if let Some(tweak) = &custom {
        items.push((tweak, TweakAction::Apply));
    }

    let total = (items.len() + profile.features.len() + profile.appx_remove.len() + profile.apps.len()).max(1) as f32;
    let report = engine.run("Profile", &items, cancel, |index, name| progress(index as f32 / total, name))?;
    let mut summary =
        ProfileSummary { applied: report.applied.len(), failed: report.failed, cancelled: report.cancelled };
    let mut completed = items.len();

    let mut step = |summary: &mut ProfileSummary, name: &str, outcome: Result<()>| {
        progress(completed as f32 / total, name);
        completed += 1;
        match outcome {
            Ok(()) => summary.applied += 1,
            Err(EngineError::Cancelled) => summary.cancelled = true,
            Err(error) => summary.failed.push((name.to_owned(), error.to_string())),
        }
    };

    for id in &profile.features {
        if let Some(feature) = catalog.feature(id) {
            step(&mut summary, &feature.name, engine.apply_feature(feature, None, cancel).map(drop));
        }
    }
    for id in &profile.appx_remove {
        if let Some(package) = catalog.appx_package(id) {
            let op = NativeOp::RemoveAppx { pattern: package.package_id.clone() };
            step(&mut summary, &package.name, engine.native.run(&op, cancel));
        }
    }
    for id in &profile.apps {
        if let Some(app) = catalog.app(id) {
            step(&mut summary, &app.name, packages::install(manager, app, cancel, &mut |_| {}));
        }
    }
    if let Some(selection) = &profile.dns {
        step(&mut summary, "DNS", apply_dns(&catalog, selection, Language::En).map(drop));
    }
    if let Some(id) = &profile.update_mode
        && let Some(mode) = catalog.update_mode(id)
    {
        // Unattended runs may not enable a mode that demands confirmation.
        let outcome = windowcraft_engine::updates::require_confirmation(mode, false)
            .and_then(|()| engine.apply_update_mode(mode).map(drop));
        step(&mut summary, &mode.name, outcome);
    }
    Ok(summary)
}

/// Live state of every detectable tweak.
pub fn detect_all(engine: &Engine) -> BTreeMap<String, Detection> {
    let mut states = BTreeMap::new();
    for tweak in &engine.catalog.tweaks {
        if let Ok(detection) = engine.detect(tweak) {
            states.insert(tweak.id.clone(), detection);
        }
    }
    for tweak in windowcraft_engine::gaming::display_toggles() {
        if let Ok(detection) = engine.detect(&tweak) {
            states.insert(tweak.id.clone(), detection);
        }
    }
    states
}

/// Selected segmented options, read back from the registry.
pub fn detect_options(engine: &Engine) -> BTreeMap<String, String> {
    let mut options = BTreeMap::new();
    for tweak in engine.catalog.tweaks.iter().filter(|tweak| tweak.kind == TweakKind::Segmented) {
        if let Ok(Some(option)) = engine.current_option(tweak) {
            options.insert(tweak.id.clone(), option);
        }
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use windowcraft_config::{StartupType, import_any};
    use windowcraft_engine::executor::RecordingRunner;
    use windowcraft_engine::services::MemoryServices;
    use windowcraft_engine::testing::Sandbox;
    use windowcraft_engine::{RegPath, RegValue, SnapshotStore};

    struct Rig {
        engine: Engine,
        sandbox: Sandbox,
        _dir: tempfile::TempDir,
    }

    fn rig(name: &str) -> Rig {
        let sandbox = Sandbox::new(name);
        let dir = tempfile::tempdir().unwrap();
        let catalog = Arc::new(Catalog::load().unwrap());
        let mut table = Vec::new();
        for tweak in &catalog.tweaks {
            for rule in &tweak.services {
                table.push((rule.name.clone(), rule.original));
            }
        }
        let borrowed: Vec<(&str, StartupType)> = table.iter().map(|(name, kind)| (name.as_str(), *kind)).collect();
        let engine = Engine {
            scope: sandbox.scope.clone(),
            services: Arc::new(MemoryServices::with(&borrowed)),
            native: Arc::new(RecordingRunner::default()),
            store: Arc::new(SnapshotStore::open(&dir.path().join("a.redb")).unwrap()),
            catalog,
            ram_kb: 1 << 24,
            os_build: "1.1".to_owned(),
        };
        Rig { engine, sandbox, _dir: dir }
    }

    #[test]
    fn the_log_appends_and_rolls_over() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("windowcraft.log");
        append_log(&path, "First", &["line one".to_owned()]);
        append_log(&path, "Second", &[]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("First") && text.contains("  line one") && text.contains("Second"));

        std::fs::write(&path, vec![b'x'; LOG_LIMIT_BYTES as usize + 1]).unwrap();
        append_log(&path, "Third", &[]);
        assert!(dir.path().join("windowcraft.log.old").exists());
        assert!(std::fs::metadata(&path).unwrap().len() < 100);
    }

    #[test]
    fn presets_map_to_profiles() {
        let catalog = Catalog::load().unwrap();
        assert_eq!(preset_profile(&catalog, PresetName::Standard).tweaks.len(), 12);
    }

    #[test]
    fn profile_runs_every_section_in_the_sandbox() {
        let rig = rig("runprofile");
        let source = r#"{"version":1,
            "tweaks":["activity"],
            "toggles":{"show-ext":true},
            "segmented":{"multiplane-overlay":"Fully Disabled"},
            "features":["wsl"],
            "appx_remove":["microsoft-windows-feedback-hub"]}"#;
        let resolved = import_any(source, &rig.engine.catalog).unwrap();
        let mut steps = 0;
        let summary =
            run_profile(&rig.engine, &resolved, PackageManager::Winget, &CancelToken::new(), |_, _| steps += 1)
                .unwrap();
        assert!(summary.failed.is_empty(), "{:?}", summary.failed);
        assert_eq!(summary.applied, 5);
        assert_eq!(steps, 5);

        let policy = RegPath::parse(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System").unwrap();
        assert_eq!(rig.sandbox.scope.read(&policy, "EnableActivityFeed").unwrap(), Some(RegValue::DWord(1)));
    }

    #[test]
    fn malformed_config_cannot_disable_a_protected_service() {
        let rig = rig("protectedprofile");
        let source = r#"{"version":1,"services":[{"name":"RpcSs","startup":"Disabled","original":"Automatic"}]}"#;
        let resolved = import_any(source, &rig.engine.catalog).unwrap();
        let summary =
            run_profile(&rig.engine, &resolved, PackageManager::Winget, &CancelToken::new(), |_, _| {}).unwrap();
        assert_eq!(summary.applied, 0);
        assert!(summary.failed[0].1.contains("protected"), "{:?}", summary.failed);
    }

    #[test]
    fn unattended_runs_refuse_disable_all_updates() {
        let rig = rig("confirmprofile");
        let resolved = import_any(r#"{"version":1,"update_mode":"disabled"}"#, &rig.engine.catalog).unwrap();
        let summary =
            run_profile(&rig.engine, &resolved, PackageManager::Winget, &CancelToken::new(), |_, _| {}).unwrap();
        assert_eq!(summary.failed.len(), 1);
    }

    #[test]
    fn detection_covers_catalog_and_display_toggles() {
        let rig = rig("detectall");
        let states = detect_all(&rig.engine);
        assert_eq!(states.len(), rig.engine.catalog.tweaks.len() + 3);
        assert_eq!(detect_options(&rig.engine).get("multiplane-overlay").map(String::as_str), Some("Enabled"));
    }
}
