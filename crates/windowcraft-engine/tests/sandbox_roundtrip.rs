// Made by ycangignacy
//! End-to-end engine tests against real registry calls, confined to
//! `HKCU\Software\WindowCraft_TestSandbox` and an in-memory service table.

use std::sync::Arc;
use windowcraft_config::{Catalog, NativeOp, Profile, StartupType, Tweak, TweakKind, import_any};
use windowcraft_engine::executor::RecordingRunner;
use windowcraft_engine::services::MemoryServices;
use windowcraft_engine::snapshot::{Change, SessionState};
use windowcraft_engine::testing::Sandbox;
use windowcraft_engine::*;

struct Rig {
    engine: Engine,
    services: Arc<MemoryServices>,
    native: Arc<RecordingRunner>,
    sandbox: Sandbox,
    _dir: tempfile::TempDir,
}

fn rig(name: &str) -> Rig {
    let sandbox = Sandbox::new(name);
    let dir = tempfile::tempdir().unwrap();
    let catalog = Arc::new(Catalog::load().unwrap());
    let mut table: Vec<(&str, StartupType)> = Vec::new();
    for tweak in &catalog.tweaks {
        for rule in &tweak.services {
            table.push((rule.name.as_str(), rule.original));
        }
    }
    let services = Arc::new(MemoryServices::with(&table));
    let native = Arc::new(RecordingRunner::default());
    let engine = Engine {
        scope: sandbox.scope.clone(),
        services: services.clone(),
        native: native.clone(),
        store: Arc::new(SnapshotStore::open(&dir.path().join("t.redb")).unwrap()),
        catalog,
        ram_kb: 16 * 1024 * 1024,
        os_build: "26300.1000".to_owned(),
    };
    Rig { engine, services, native, sandbox, _dir: dir }
}

fn tweak<'a>(rig: &'a Rig, id: &str) -> &'a Tweak {
    rig.engine.catalog.tweak(id).unwrap_or_else(|| panic!("no tweak {id}"))
}

fn dword(rig: &Rig, path: &str, name: &str) -> Option<RegValue> {
    rig.sandbox.scope.read(&RegPath::parse(path).unwrap(), name).unwrap()
}

const SYSTEM_POLICY: &str = r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System";

#[test]
fn full_lifecycle_apply_detect_snapshot_drift_rollback() {
    let rig = rig("lifecycle");
    let scope = &rig.sandbox.scope;
    let activity = tweak(&rig, "activity").clone();
    let cancel = CancelToken::new();

    // A pre-existing value must come back exactly, not a catalog default.
    let policy = RegPath::parse(SYSTEM_POLICY).unwrap();
    scope.write(&policy, "EnableActivityFeed", &RegValue::DWord(7)).unwrap();

    assert_eq!(rig.engine.detect(&activity).unwrap(), Detection::NotApplied);

    // Apply
    let report = rig.engine.run("Lifecycle", &[(&activity, TweakAction::Apply)], &cancel, |_, _| {}).unwrap();
    assert_eq!(report.applied, ["activity"]);
    assert!(report.failed.is_empty());

    // Detect
    assert_eq!(rig.engine.detect(&activity).unwrap(), Detection::Applied);
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), Some(RegValue::DWord(1)));

    // Snapshot holds the true pre-change values
    let records = rig.engine.store.changes(report.session).unwrap();
    assert_eq!(records.len(), 3);
    assert!(records.iter().any(|record| matches!(
        &record.change,
        Change::Registry { name, before: Some(RegValue::DWord(7)), .. } if name == "EnableActivityFeed"
    )));
    assert_eq!(rig.engine.store.session(report.session).unwrap().unwrap().state, SessionState::Committed);

    // Simulate a Windows Update resetting one value
    assert!(rig.engine.detect_drift().unwrap().is_empty());
    scope.write(&policy, "PublishUserActivities", &RegValue::DWord(1)).unwrap();

    // Detect drift
    let drifted = rig.engine.detect_drift().unwrap();
    assert_eq!(drifted.len(), 1);
    assert_eq!(drifted[0].tweak_id, "activity");
    assert_eq!(drifted[0].entries[0].1, "0");
    assert_eq!(drifted[0].entries[0].2, "1");
    assert_eq!(rig.engine.detect(&activity).unwrap(), Detection::Partial { matched: 2, total: 3 });

    // Repair keeps the original undo point
    assert_eq!(rig.engine.repair_drift().unwrap(), 1);
    assert!(rig.engine.detect_drift().unwrap().is_empty());

    // Rollback restores the pre-existing value, and removes values that were absent
    rig.engine.rollback_session(report.session).unwrap();
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), Some(RegValue::DWord(7)));
    assert_eq!(dword(&rig, SYSTEM_POLICY, "PublishUserActivities"), None);
    assert_eq!(dword(&rig, SYSTEM_POLICY, "UploadUserActivities"), None);
    assert_eq!(rig.engine.detect(&activity).unwrap(), Detection::NotApplied);
    assert_eq!(rig.engine.store.session(report.session).unwrap().unwrap().state, SessionState::RolledBack);
    assert!(rig.engine.store.applied().unwrap().is_empty());
}

#[test]
fn applying_twice_keeps_the_true_original_for_undo() {
    let rig = rig("reapply");
    let scope = &rig.sandbox.scope;
    let activity = tweak(&rig, "activity").clone();
    let cancel = CancelToken::new();
    let policy = RegPath::parse(SYSTEM_POLICY).unwrap();
    scope.write(&policy, "EnableActivityFeed", &RegValue::DWord(7)).unwrap();

    let first = rig.engine.run("First", &[(&activity, TweakAction::Apply)], &cancel, |_, _| {}).unwrap();
    let second = rig.engine.run("Second", &[(&activity, TweakAction::Apply)], &cancel, |_, _| {}).unwrap();
    assert_ne!(first.session, second.session);

    let rows = rig.engine.inspect(&activity, &TweakAction::Apply).unwrap();
    assert_eq!(rows[0].undo, "7", "the inspector must show the true original");

    rig.engine.undo_tweak(&activity).unwrap();
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), Some(RegValue::DWord(7)));
    assert_eq!(dword(&rig, SYSTEM_POLICY, "PublishUserActivities"), None);
    assert!(rig.engine.store.applied_state("activity").unwrap().is_none());
}

#[test]
fn rolling_back_only_a_reapply_keeps_the_tweak_tracked() {
    let rig = rig("reapplyrollback");
    let activity = tweak(&rig, "activity").clone();
    let cancel = CancelToken::new();
    let first = rig.engine.run("First", &[(&activity, TweakAction::Apply)], &cancel, |_, _| {}).unwrap();
    let second = rig.engine.run("Second", &[(&activity, TweakAction::Apply)], &cancel, |_, _| {}).unwrap();

    rig.engine.rollback_session(second.session).unwrap();
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), Some(RegValue::DWord(1)), "still applied");
    let state = rig.engine.store.applied_state("activity").unwrap().unwrap();
    assert_eq!(state.session, first.session);

    rig.engine.rollback_session(first.session).unwrap();
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), None);
    assert!(rig.engine.store.applied_state("activity").unwrap().is_none());
}

#[test]
fn an_interrupted_session_can_still_be_rolled_back() {
    let rig = rig("interrupted");
    let scope = &rig.sandbox.scope;
    let activity = tweak(&rig, "activity").clone();
    let policy = RegPath::parse(SYSTEM_POLICY).unwrap();
    scope.write(&policy, "EnableActivityFeed", &RegValue::DWord(7)).unwrap();

    // A crash leaves the session open with its writes already on disk.
    let session = rig.engine.store.begin_session("Crashed").unwrap();
    rig.engine.apply_tweak(session, &activity, &TweakAction::Apply, &CancelToken::new()).unwrap();
    assert_eq!(rig.engine.store.session(session).unwrap().unwrap().state, SessionState::Open);

    rig.engine.rollback_session(session).unwrap();
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), Some(RegValue::DWord(7)));
}

#[test]
fn a_rollback_names_what_it_cannot_restore() {
    let rig = rig("irreversiblenames");
    let ai = tweak(&rig, "windows-ai").clone();
    rig.services.force("WSAIFabricSvc", StartupType::Manual);
    let report = rig.engine.run("AI", &[(&ai, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(rig.engine.irreversible_in_session(report.session).unwrap(), std::slice::from_ref(&ai.name));
}

#[test]
fn a_failing_service_takes_the_update_policy_back_with_it() {
    let rig = rig("updatefault");
    // UsoSvc is missing from nothing here: BITS is present, then a protected
    // rule forces the failure after the registry was already written.
    rig.services.force("BITS", StartupType::Manual);
    rig.services.force("RpcSs", StartupType::Automatic);
    let mut mode = rig.engine.catalog.update_mode("disabled").unwrap().clone();
    mode.services.push(windowcraft_config::ServiceRule {
        name: "RpcSs".to_owned(),
        startup: StartupType::Disabled,
        original: StartupType::Automatic,
        optional: false,
    });
    assert!(rig.engine.apply_update_mode(&mode).is_err());

    let au = r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU";
    assert_eq!(dword(&rig, au, "NoAutoUpdate"), None, "the policy must not stay half applied");
    assert_eq!(rig.services.startup("BITS"), Some(StartupType::Manual));
}

#[test]
fn every_catalog_tweak_applies_and_rolls_back_in_the_sandbox() {
    let rig = rig("everytweak");
    let cancel = CancelToken::new();
    for tweak in rig.engine.catalog.tweaks.clone() {
        let action = match tweak.kind {
            TweakKind::Toggle => TweakAction::Set(true),
            TweakKind::Segmented => TweakAction::Select(tweak.options[1].label.clone()),
            _ => TweakAction::Apply,
        };
        let report = rig.engine.run(&tweak.name, &[(&tweak, action)], &cancel, |_, _| {}).unwrap();
        assert!(report.failed.is_empty(), "{}: {:?}", tweak.id, report.failed);
        let detection = rig.engine.detect(&tweak).unwrap();
        if tweak.is_detectable() {
            assert_eq!(detection, Detection::Applied, "{}", tweak.id);
        }
        rig.engine.rollback_session(report.session).unwrap();
    }
    // Every tweak was reverted, so the sandboxed registry holds no leftovers.
    for tweak in &rig.engine.catalog.tweaks {
        for entry in tweak.registry.iter() {
            let path = RegPath::parse(&entry.path).unwrap();
            assert_eq!(rig.sandbox.scope.read(&path, &entry.name).unwrap(), None, "{}", tweak.id);
        }
    }
    for tweak in &rig.engine.catalog.tweaks {
        for rule in &tweak.services {
            assert_eq!(rig.services.startup(&rule.name), Some(rule.original), "{}", rule.name);
        }
    }
}

#[test]
fn single_key_rollback_leaves_the_rest_applied() {
    let rig = rig("singlekey");
    let activity = tweak(&rig, "activity").clone();
    let report =
        rig.engine.run("One", &[(&activity, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();
    let records = rig.engine.store.changes(report.session).unwrap();
    rig.engine.rollback_change(report.session, records[1].index).unwrap();

    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), Some(RegValue::DWord(1)));
    assert_eq!(dword(&rig, SYSTEM_POLICY, "PublishUserActivities"), None);
    assert!(rig.engine.store.changes(report.session).unwrap()[1].rolled_back);
    assert!(rig.engine.rollback_change(report.session, 999).is_err());
}

#[test]
fn toggles_switch_both_ways_with_catalog_values() {
    let rig = rig("toggle");
    let toggle = tweak(&rig, "show-ext").clone();
    let cancel = CancelToken::new();
    let session = rig.engine.store.begin_session("toggle").unwrap();
    let path = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";

    rig.engine.apply_tweak(session, &toggle, &TweakAction::Set(true), &cancel).unwrap();
    assert_eq!(dword(&rig, path, "HideFileExt"), Some(RegValue::DWord(0)));
    assert_eq!(rig.engine.detect(&toggle).unwrap(), Detection::Applied);

    rig.engine.apply_tweak(session, &toggle, &TweakAction::Set(false), &cancel).unwrap();
    assert_eq!(dword(&rig, path, "HideFileExt"), Some(RegValue::DWord(1)));
    assert_eq!(rig.engine.detect(&toggle).unwrap(), Detection::NotApplied);
    assert!(rig.engine.store.applied_state("show-ext").unwrap().is_none());

    let ran = rig.native.ran.lock().unwrap();
    assert_eq!(ran.iter().filter(|op| **op == NativeOp::RestartExplorer).count(), 2);
}

#[test]
fn segmented_control_writes_and_detects_each_state() {
    let rig = rig("segmented");
    let mpo = tweak(&rig, "multiplane-overlay").clone();
    let cancel = CancelToken::new();
    let session = rig.engine.store.begin_session("mpo").unwrap();
    let dwm = r"HKLM\SOFTWARE\Microsoft\Windows\Dwm";
    let graphics = r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers";

    assert_eq!(rig.engine.current_option(&mpo).unwrap().as_deref(), Some("Enabled"));

    let fully = TweakAction::Select("Fully Disabled".to_owned());
    rig.engine.apply_tweak(session, &mpo, &fully, &cancel).unwrap();
    assert_eq!(dword(&rig, dwm, "OverlayTestMode"), Some(RegValue::DWord(5)));
    assert_eq!(dword(&rig, graphics, "DisableOverlays"), Some(RegValue::DWord(1)));
    assert_eq!(rig.engine.current_option(&mpo).unwrap().as_deref(), Some("Fully Disabled"));

    let compatibility = TweakAction::Select("Disabled (Compatibility)".to_owned());
    rig.engine.apply_tweak(session, &mpo, &compatibility, &cancel).unwrap();
    assert_eq!(dword(&rig, graphics, "DisableOverlays"), None);
    assert_eq!(rig.engine.current_option(&mpo).unwrap().as_deref(), Some("Disabled (Compatibility)"));
}

#[test]
fn ram_token_and_binary_values_are_written_correctly() {
    let rig = rig("tokens");
    let cancel = CancelToken::new();
    let session = rig.engine.store.begin_session("t").unwrap();

    rig.engine.apply_tweak(session, tweak(&rig, "svc-host-splitting"), &TweakAction::Apply, &cancel).unwrap();
    assert_eq!(
        dword(&rig, r"HKLM\SYSTEM\CurrentControlSet\Control", "SvcHostSplitThresholdInKB"),
        Some(RegValue::DWord(16 * 1024 * 1024))
    );

    rig.engine.apply_tweak(session, tweak(&rig, "display"), &TweakAction::Apply, &cancel).unwrap();
    assert_eq!(
        dword(&rig, r"HKCU\Control Panel\Desktop", "UserPreferencesMask"),
        Some(RegValue::Binary(vec![144, 18, 3, 128, 16, 0, 0, 0]))
    );
}

#[test]
fn failure_midway_reverts_the_whole_tweak() {
    let rig = rig("fault");
    let hibernate = tweak(&rig, "hiber").clone();
    let path = r"HKLM\System\CurrentControlSet\Control\Session Manager\Power";
    rig.sandbox.scope.write(&RegPath::parse(path).unwrap(), "HibernateEnabled", &RegValue::DWord(1)).unwrap();

    *rig.native.fail_on.lock().unwrap() = Some(NativeOp::Hibernate { enabled: false });
    let report =
        rig.engine.run("Fault", &[(&hibernate, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();

    assert_eq!(report.failed.len(), 1);
    assert!(report.applied.is_empty());
    assert_eq!(dword(&rig, path, "HibernateEnabled"), Some(RegValue::DWord(1)), "value must be restored");
    assert_eq!(rig.engine.store.session(report.session).unwrap().unwrap().state, SessionState::Failed);
    assert!(rig.engine.store.applied_state("hiber").unwrap().is_none());
}

#[test]
fn one_failure_does_not_stop_the_batch() {
    let rig = rig("batch");
    let (hibernate, activity) = (tweak(&rig, "hiber").clone(), tweak(&rig, "activity").clone());
    *rig.native.fail_on.lock().unwrap() = Some(NativeOp::Hibernate { enabled: false });
    let items = [(&hibernate, TweakAction::Apply), (&activity, TweakAction::Apply)];
    let report = rig.engine.run("Batch", &items, &CancelToken::new(), |_, _| {}).unwrap();
    assert_eq!(report.applied, ["activity"]);
    assert_eq!(report.failed[0].0, "hiber");
    assert_eq!(rig.engine.store.session(report.session).unwrap().unwrap().state, SessionState::Committed);
}

#[test]
fn cancellation_stops_between_tweaks_and_keeps_a_valid_session() {
    let rig = rig("cancel");
    let activity = tweak(&rig, "activity").clone();
    let cancel = CancelToken::new();
    cancel.cancel();
    let report = rig.engine.run("Cancelled", &[(&activity, TweakAction::Apply)], &cancel, |_, _| {}).unwrap();
    assert!(report.cancelled);
    assert!(report.applied.is_empty());
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), None);
}

#[test]
fn protected_service_in_custom_config_writes_nothing() {
    let rig = rig("protected");
    let mut malicious = tweak(&rig, "activity").clone();
    malicious.services.push(windowcraft_config::ServiceRule {
        name: "windefend".to_owned(),
        startup: StartupType::Disabled,
        original: StartupType::Automatic,
        optional: false,
    });
    rig.services.force("WinDefend", StartupType::Automatic);

    let report =
        rig.engine.run("Bad", &[(&malicious, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();
    assert_eq!(report.failed.len(), 1);
    assert!(report.failed[0].1.contains("protected"));
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), None, "no partial apply");
    assert_eq!(rig.services.startup("WinDefend"), Some(StartupType::Automatic));
}

#[test]
fn undo_without_snapshot_falls_back_to_catalog_values() {
    let rig = rig("undofallback");
    let scope = &rig.sandbox.scope;
    let policy = RegPath::parse(SYSTEM_POLICY).unwrap();
    for name in ["EnableActivityFeed", "PublishUserActivities", "UploadUserActivities"] {
        scope.write(&policy, name, &RegValue::DWord(1)).unwrap();
    }
    rig.engine.undo_tweak(tweak(&rig, "activity")).unwrap();
    assert_eq!(dword(&rig, SYSTEM_POLICY, "EnableActivityFeed"), None);
}

#[test]
fn irreversible_tweaks_refuse_automatic_undo() {
    let rig = rig("irreversible");
    let error = rig.engine.undo_tweak(tweak(&rig, "disable-bit-locker")).unwrap_err();
    assert!(matches!(error, EngineError::Unsupported(_)));
}

#[test]
fn inspector_rows_show_current_target_and_undo() {
    let rig = rig("inspect");
    let wpbt = tweak(&rig, "wpbt").clone();
    let rows = rig.engine.inspect(&wpbt, &TweakAction::Apply).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].current, "(not set)");
    assert_eq!(rows[0].desired, "1");
    assert_eq!(rows[0].undo, "(remove value)");
    assert!(!rows[0].matches);

    let report = rig.engine.run("W", &[(&wpbt, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();
    assert!(report.failed.is_empty());
    let rows = rig.engine.inspect(&wpbt, &TweakAction::Apply).unwrap();
    assert_eq!(rows[0].current, "1");
    assert_eq!(rows[0].undo, "(not set)", "undo must come from the snapshot");
    assert!(rows[0].matches);

    let services = rig.engine.inspect(tweak(&rig, "location"), &TweakAction::Apply).unwrap();
    let service = services.iter().find(|row| row.target == "Service lfsvc").unwrap();
    assert_eq!((service.current.as_str(), service.desired.as_str()), ("Manual", "Disabled"));
}

#[test]
fn drift_on_a_service_is_detected_and_reported_after_update() {
    let mut rig = rig("servicedrift");
    let location = tweak(&rig, "location").clone();
    rig.engine.run("L", &[(&location, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();
    assert_eq!(rig.services.startup("lfsvc"), Some(StartupType::Disabled));

    rig.services.force("lfsvc", StartupType::Manual);
    rig.engine.os_build = "26300.2000".to_owned();

    let drifted = rig.engine.detect_drift().unwrap();
    assert_eq!(drifted.len(), 1);
    assert!(drifted[0].after_update);
    assert_eq!(
        Engine::drift_summary(&drifted).unwrap(),
        "1 applied tweak was reset by a recent Windows Update"
    );
    assert_eq!(rig.engine.repair_drift().unwrap(), 1);
    assert_eq!(rig.services.startup("lfsvc"), Some(StartupType::Disabled));
    assert!(Engine::drift_summary(&[]).is_none());
}

#[test]
fn preset_runs_end_to_end() {
    let rig = rig("preset");
    let catalog = rig.engine.catalog.clone();
    let items: Vec<(&Tweak, TweakAction)> = catalog
        .presets
        .standard
        .iter()
        .map(|id| (catalog.tweak(id).unwrap(), TweakAction::Apply))
        .collect();
    let mut steps = 0;
    let report = rig.engine.run("Standard", &items, &CancelToken::new(), |_, _| steps += 1).unwrap();
    assert_eq!(report.applied.len(), 13, "{:?}", report.failed);
    assert_eq!(steps, 13);
    assert!(rig.native.ran.lock().unwrap().contains(&NativeOp::RestorePoint));
}

#[test]
fn profile_with_winutil_ids_resolves_for_execution() {
    let rig = rig("profile");
    let catalog = rig.engine.catalog.clone();
    let resolved = import_any(r#"{"WPFTweaks":["WPFTweaksActivity","WPFToggleDarkMode"]}"#, &catalog).unwrap();
    let profile: Profile = resolved.profile;
    assert_eq!(profile.tweaks, ["activity"]);
    assert_eq!(profile.toggles.get("dark-mode"), Some(&true));
}

#[test]
fn features_enable_through_dism_and_snapshot_registry() {
    let rig = rig("features");
    let wsl = rig.engine.catalog.feature("wsl").unwrap().clone();
    rig.engine.apply_feature(&wsl, None, &CancelToken::new()).unwrap();
    assert_eq!(
        *rig.native.features.lock().unwrap(),
        ["VirtualMachinePlatform", "Microsoft-Windows-Subsystem-Linux"]
    );

    let backup = rig.engine.catalog.feature("reg-backup").unwrap().clone();
    let session = rig.engine.apply_feature(&backup, None, &CancelToken::new()).unwrap();
    assert_eq!(rig.engine.store.changes(session).unwrap().len(), 3, "two values and the task");
    assert!(rig.native.ran.lock().unwrap().contains(&NativeOp::CreateRegistryBackupTask));

    let update = rig.engine.catalog.feature("update").unwrap().clone();
    rig.engine.apply_feature(&update, Some("Aggressive"), &CancelToken::new()).unwrap();
    assert!(rig.native.ran.lock().unwrap().contains(&NativeOp::FixWindowsUpdate { aggressive: true }));
}

#[test]
fn update_modes_are_snapshotted_and_keep_wsus_settings() {
    let rig = rig("updates");
    let services = &rig.services;
    for name in ["BITS", "wuauserv", "UsoSvc"] {
        services.force(name, StartupType::Manual);
    }
    let policy = RegPath::parse(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate").unwrap();
    let wsus = RegValue::String("https://wsus.corp.example".to_owned());
    rig.sandbox.scope.write(&policy, "WUServer", &wsus).unwrap();

    assert_eq!(rig.engine.current_update_mode().unwrap(), "default");

    let security = rig.engine.catalog.update_mode("security").unwrap().clone();
    let session = rig.engine.apply_update_mode(&security).unwrap();
    assert_eq!(rig.engine.current_update_mode().unwrap(), "security");
    assert_eq!(rig.sandbox.scope.read(&policy, "DeferQualityUpdatesPeriodInDays").unwrap(), Some(RegValue::DWord(4)));
    assert_eq!(rig.sandbox.scope.read(&policy, "WUServer").unwrap(), Some(wsus.clone()), "WSUS must survive");

    let disabled = rig.engine.catalog.update_mode("disabled").unwrap().clone();
    assert!(windowcraft_engine::updates::require_confirmation(&disabled, false).is_err());
    rig.engine.apply_update_mode(&disabled).unwrap();
    assert_eq!(rig.engine.current_update_mode().unwrap(), "disabled");
    assert_eq!(services.startup("wuauserv"), Some(StartupType::Disabled));

    let default = rig.engine.catalog.update_mode("default").unwrap().clone();
    rig.engine.apply_update_mode(&default).unwrap();
    assert_eq!(rig.engine.current_update_mode().unwrap(), "default");
    assert_eq!(services.startup("wuauserv"), Some(StartupType::Manual));
    assert_eq!(rig.sandbox.scope.read(&policy, "WUServer").unwrap(), Some(wsus));

    rig.engine.rollback_session(session).unwrap();
}

#[test]
fn concurrent_batches_do_not_corrupt_the_store() {
    let rig = Arc::new(rig("concurrent"));
    let mut workers = Vec::new();
    for worker in 0..6 {
        let rig = Arc::clone(&rig);
        workers.push(std::thread::spawn(move || {
            let ids = ["activity", "wpbt", "consumer-features", "delivery-optimization"];
            let id = ids[worker % ids.len()];
            let tweak = rig.engine.catalog.tweak(id).unwrap();
            for _ in 0..10 {
                let report =
                    rig.engine.run(id, &[(tweak, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();
                assert!(report.failed.is_empty());
                rig.engine.store.sessions().unwrap();
                rig.engine.detect_drift().unwrap();
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(rig.engine.store.sessions().unwrap().len(), 60);
}

fn session_state(rig: &Rig, session: u64) -> SessionState {
    rig.engine.store.sessions().unwrap().iter().find(|entry| entry.id == session).unwrap().state
}

#[test]
fn rolling_back_the_registry_backup_feature_deletes_its_task() {
    let rig = rig("featureundo");
    let feature = rig.engine.catalog.feature("reg-backup").unwrap().clone();
    let session = rig.engine.apply_feature(&feature, None, &CancelToken::new()).unwrap();
    assert!(rig.engine.irreversible_in_session(session).unwrap().is_empty());

    rig.engine.rollback_session(session).unwrap();
    assert_eq!(session_state(&rig, session), SessionState::RolledBack);
    assert_eq!(
        *rig.native.ran.lock().unwrap(),
        [NativeOp::CreateRegistryBackupTask, NativeOp::RemoveRegistryBackupTask]
    );
}

#[test]
fn rolling_back_openssh_removes_the_rule_and_reports_the_rest_as_kept() {
    let rig = rig("featuressh");
    let feature = rig.engine.catalog.feature("sshserver").unwrap().clone();
    let session = rig.engine.apply_feature(&feature, None, &CancelToken::new()).unwrap();
    assert!(rig.engine.irreversible_in_session(session).unwrap().contains(&feature.name));

    rig.engine.rollback_session(session).unwrap();
    assert_eq!(session_state(&rig, session), SessionState::PartlyRolledBack);
    assert_eq!(*rig.native.ran.lock().unwrap(), [NativeOp::SshServer, NativeOp::RemoveSshFirewallRule]);
}

#[test]
fn rolling_back_windows_features_disables_them_newest_first() {
    let rig = rig("featuredism");
    let feature = rig.engine.catalog.feature("wsl").unwrap().clone();
    assert!(!feature.features.is_empty());
    let session = rig.engine.apply_feature(&feature, None, &CancelToken::new()).unwrap();
    assert!(rig.engine.irreversible_in_session(session).unwrap().is_empty());

    rig.engine.rollback_session(session).unwrap();
    assert_eq!(session_state(&rig, session), SessionState::RolledBack);
    assert_eq!(
        *rig.native.disabled_features.lock().unwrap(),
        ["Microsoft-Windows-Subsystem-Linux", "VirtualMachinePlatform"]
    );
}

#[test]
fn rolling_back_leaves_features_that_were_already_on() {
    let rig = rig("featurekeep");
    rig.native.features_on.lock().unwrap().push("VirtualMachinePlatform".to_owned());
    let feature = rig.engine.catalog.feature("wsl").unwrap().clone();
    let session = rig.engine.apply_feature(&feature, None, &CancelToken::new()).unwrap();

    rig.engine.rollback_session(session).unwrap();
    assert_eq!(session_state(&rig, session), SessionState::RolledBack);
    assert_eq!(*rig.native.disabled_features.lock().unwrap(), ["Microsoft-Windows-Subsystem-Linux"]);
}

#[test]
fn rolling_back_with_failed_feature_query_disables_nothing() {
    let rig = rig("featurequeryfail");
    rig.native.feature_query_fails.store(true, std::sync::atomic::Ordering::Relaxed);
    let feature = rig.engine.catalog.feature("wsl").unwrap().clone();
    let session = rig.engine.apply_feature(&feature, None, &CancelToken::new()).unwrap();

    rig.engine.rollback_session(session).unwrap();
    assert_eq!(session_state(&rig, session), SessionState::RolledBack);
    assert!(rig.native.disabled_features.lock().unwrap().is_empty());
}

#[test]
fn feature_steps_recorded_by_0_1_0_still_count_as_kept() {
    let rig = rig("featurelegacy");
    let session = rig.engine.store.begin_session("WSL").unwrap();
    // 0.1.0 wrote this record without the feature state.
    let legacy: Change = serde_json::from_str(
        r#"{"kind":"Native","description":"Enable Windows feature Microsoft-Windows-Subsystem-Linux"}"#,
    )
    .unwrap();
    rig.engine.store.record(session, "wsl", &[legacy]).unwrap();
    rig.engine.store.finish_session(session, SessionState::Committed).unwrap();

    let name = rig.engine.catalog.feature("wsl").unwrap().name.clone();
    assert_eq!(rig.engine.irreversible_in_session(session).unwrap(), [name]);
    rig.engine.rollback_session(session).unwrap();
    assert_eq!(session_state(&rig, session), SessionState::PartlyRolledBack);
    assert!(rig.native.disabled_features.lock().unwrap().is_empty());
}

#[test]
fn registry_only_tweaks_still_roll_back_fully() {
    let rig = rig("fullrollback");
    let activity = tweak(&rig, "activity").clone();
    let report = rig.engine.run("t", &[(&activity, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();

    rig.engine.rollback_session(report.session).unwrap();
    assert_eq!(session_state(&rig, report.session), SessionState::RolledBack);
}
