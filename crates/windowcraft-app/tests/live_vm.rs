// Made by ycangignacy
//! Live tests for a disposable VirtualBox guest. They change the real system:
//! registry, services, AppX packages, optional features, update policy, and
//! with the ISO variables set, ISO builds and a USB disk.
//!
//! Every test is ignored and also returns at once unless
//! `WINDOWCRAFT_LIVE_VM=1` is set, the process is elevated, and the BIOS
//! reports VirtualBox, so neither `cargo test --ignored` on a workstation nor
//! CI can reach them. Build with
//! `cargo test --release -p windowcraft-app --test live_vm --no-run` and run
//! the exe in the guest with `--ignored --test-threads=1 --nocapture`.
//!
//! Checks read the system through PowerShell, a different path from the one
//! WindowCraft writes through.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use windowcraft_config::{Catalog, NativeOp, Tweak};
use windowcraft_engine::jobs::{self, JobContext, JobEvent};
use windowcraft_engine::snapshot::SessionState;
use windowcraft_engine::{CancelToken, Engine, TweakAction};

fn live() -> bool {
    if std::env::var("WINDOWCRAFT_LIVE_VM").as_deref() != Ok("1") {
        println!("skipped: WINDOWCRAFT_LIVE_VM is not 1");
        return false;
    }
    if !windowcraft_engine::sysinfo::is_elevated() {
        println!("skipped: not elevated");
        return false;
    }
    let maker = ps(r"(Get-ItemProperty 'HKLM:\HARDWARE\DESCRIPTION\System\BIOS').SystemManufacturer");
    if !maker.contains("innotek") {
        println!("skipped: BIOS manufacturer is '{maker}', not VirtualBox");
        return false;
    }
    true
}

fn ps(script: &str) -> String {
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .output()
        .expect("powershell runs");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn registry_path(path: &str) -> String {
    let (hive, rest) = path.split_once('\\').expect("hive");
    let root = match hive {
        "HKLM" => "HKEY_LOCAL_MACHINE",
        "HKCU" => "HKEY_CURRENT_USER",
        other => panic!("unexpected hive {other}"),
    };
    format!("Registry::{root}\\{rest}")
}

/// A DWORD read through PowerShell, `None` when the value is absent.
fn dword(path: &str, name: &str) -> Option<u32> {
    let text = ps(&format!(
        "$v = Get-ItemProperty -LiteralPath '{}' -Name '{name}' -ErrorAction SilentlyContinue; if ($v) {{ $v.'{name}' }}",
        registry_path(path)
    ));
    text.parse().ok()
}

fn service_start(name: &str) -> Option<u32> {
    dword(&format!(r"HKLM\SYSTEM\CurrentControlSet\Services\{name}"), "Start")
}

fn feature_state(name: &str) -> String {
    ps(&format!("(Get-WindowsOptionalFeature -Online -FeatureName '{name}' -WarningAction SilentlyContinue).State"))
}

fn engine() -> Engine {
    Engine::system(Arc::new(Catalog::load().expect("catalog"))).expect("engine on the live system")
}

fn tweak<'a>(engine: &'a Engine, id: &str) -> &'a Tweak {
    engine.catalog.tweak(id).unwrap_or_else(|| panic!("tweak {id}"))
}

fn state(engine: &Engine, session: u64) -> SessionState {
    engine.store.session(session).expect("store").expect("session").state
}

fn run(engine: &Engine, label: &str, items: &[(&Tweak, TweakAction)]) -> u64 {
    let report = engine.run(label, items, &CancelToken::new(), |_, name| println!("  applying {name}")).expect("run");
    assert!(report.failed.is_empty(), "failed: {:?}", report.failed);
    report.session
}

/// Runs a pipeline step the way the app does, on a job thread with a context.
fn job(name: &str, body: impl FnOnce(&JobContext) -> windowcraft_engine::Result<String> + Send + 'static) -> String {
    let mut handle = jobs::spawn(name, body);
    let mut outcome = None;
    while let Some(event) = handle.events.blocking_recv() {
        match event {
            JobEvent::Finished { outcome: done } => outcome = Some(done),
            JobEvent::Progress { step, .. } => println!("  {step}"),
        }
    }
    outcome.expect("job finished").unwrap_or_else(|error| panic!("{name} failed: {error}"))
}

const SYSTEM: &str = r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System";
const PERSONALIZE: &str = r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const ADVANCED: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
const DWM: &str = r"HKLM\SOFTWARE\Microsoft\Windows\Dwm";

#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t01_tweaks_apply_and_roll_back() {
    if !live() {
        return;
    }
    let engine = engine();
    let watched = [
        (SYSTEM, "EnableActivityFeed"),
        (SYSTEM, "PublishUserActivities"),
        (SYSTEM, "UploadUserActivities"),
        (PERSONALIZE, "AppsUseLightTheme"),
        (ADVANCED, "HideFileExt"),
        (DWM, "OverlayTestMode"),
    ];
    let before: Vec<Option<u32>> = watched.iter().map(|(path, name)| dword(path, name)).collect();
    println!("before: {before:?}");

    let items = [
        (tweak(&engine, "activity"), TweakAction::Apply),
        (tweak(&engine, "dark-mode"), TweakAction::Set(true)),
        (tweak(&engine, "show-ext"), TweakAction::Set(true)),
        (tweak(&engine, "multiplane-overlay"), TweakAction::Select("Disabled (Compatibility)".to_owned())),
    ];
    let session = run(&engine, "VM tweaks", &items);
    let after: Vec<Option<u32>> = watched.iter().map(|(path, name)| dword(path, name)).collect();
    println!("after:  {after:?}");
    assert_eq!(after, [Some(1), Some(0), Some(0), Some(0), Some(0), Some(5)]);

    // A single change first, as the per-change undo button does it.
    let overlay = engine
        .store
        .changes(session)
        .unwrap()
        .into_iter()
        .find(|record| record.change.target().ends_with("OverlayTestMode"))
        .expect("OverlayTestMode record");
    engine.rollback_change(session, overlay.index).unwrap();
    assert_eq!(dword(DWM, "OverlayTestMode"), before[5], "single change undo");

    engine.rollback_session(session).unwrap();
    let restored: Vec<Option<u32>> = watched.iter().map(|(path, name)| dword(path, name)).collect();
    println!("rolled back: {restored:?}");
    assert_eq!(restored, before);
    assert_eq!(state(&engine, session), SessionState::RolledBack);
}

#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t02_drift_is_found_repaired_and_baseline_kept() {
    if !live() {
        return;
    }
    let engine = engine();
    let before = dword(SYSTEM, "EnableActivityFeed");
    let session = run(&engine, "VM drift", &[(tweak(&engine, "activity"), TweakAction::Apply)]);
    assert!(engine.detect_drift().unwrap().is_empty(), "no drift right after apply");

    let set = ps(&format!(
        "Set-ItemProperty -LiteralPath '{}' -Name EnableActivityFeed -Value 0 -Type DWord; 'ok'",
        registry_path(SYSTEM)
    ));
    assert_eq!(set, "ok");
    let drift = engine.detect_drift().unwrap();
    println!("drift: {drift:?}");
    assert_eq!(drift.len(), 1);
    assert_eq!(drift[0].tweak_id, "activity");

    assert_eq!(engine.repair_drift().unwrap(), 1);
    assert_eq!(dword(SYSTEM, "EnableActivityFeed"), Some(1));
    assert!(engine.detect_drift().unwrap().is_empty(), "no drift after repair");

    engine.rollback_session(session).unwrap();
    assert_eq!(dword(SYSTEM, "EnableActivityFeed"), before, "original baseline");
}

#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t03_services_apply_and_roll_back() {
    if !live() {
        return;
    }
    let engine = engine();
    let names = ["CscService", "DiagTrack", "MapsBroker", "StorSvc", "SharedAccess"];
    let before: Vec<Option<u32>> = names.iter().map(|name| service_start(name)).collect();
    println!("before: {before:?}");
    let session = run(&engine, "VM services", &[(tweak(&engine, "services"), TweakAction::Apply)]);
    let after: Vec<Option<u32>> = names.iter().map(|name| service_start(name)).collect();
    println!("after:  {after:?}");
    // Disabled 4, Manual 3. CscService (Offline Files) is absent on Home and
    // optional in the catalog, so it stays absent there.
    let offline_files = before[0].map(|_| 4);
    assert_eq!(after, [offline_files, Some(4), Some(3), Some(3), Some(4)]);
    engine.rollback_session(session).unwrap();
    let restored: Vec<Option<u32>> = names.iter().map(|name| service_start(name)).collect();
    assert_eq!(restored, before);
}

#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t04_appx_removal_for_all_users() {
    if !live() {
        return;
    }
    let engine = engine();
    let package = engine.catalog.appx_package("microsoft-windows-feedback-hub").expect("catalog entry");
    let query = format!(
        "@(Get-AppxPackage -AllUsers '*{0}*').Count + @(Get-AppxProvisionedPackage -Online | Where-Object DisplayName -like '*{0}*').Count",
        package.package_id
    );
    let present = ps(&query);
    println!("installed and provisioned before: {present}");
    if present == "0" {
        println!("skipped: Feedback Hub is not on this image");
        return;
    }
    engine.native.run(&NativeOp::RemoveAppx { pattern: package.package_id.clone() }, &CancelToken::new()).unwrap();
    assert_eq!(ps(&query), "0", "removed for all users and deprovisioned");
}

#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t05_features_roll_back_only_what_was_off() {
    if !live() {
        return;
    }
    let engine = engine();
    let platform = "VirtualMachinePlatform";
    let linux = "Microsoft-Windows-Subsystem-Linux";
    println!("before: {platform}={} {linux}={}", feature_state(platform), feature_state(linux));
    assert_eq!(feature_state(linux), "Disabled", "the test needs WSL off at the start");

    // One feature on before, so the rollback has to leave it.
    ps(&format!(
        "Enable-WindowsOptionalFeature -Online -FeatureName {platform} -NoRestart -All -WarningAction SilentlyContinue | Out-Null"
    ));
    let enabled = feature_state(platform);
    println!("{platform} switched on outside WindowCraft: {enabled}");
    assert!(enabled == "Enabled" || enabled == "EnablePending", "{enabled}");

    let wsl = engine.catalog.feature("wsl").expect("wsl").clone();
    let session = engine.apply_feature(&wsl, None, &CancelToken::new()).unwrap();
    let on = feature_state(linux);
    println!("after apply: {linux}={on}");
    assert!(on == "Enabled" || on == "EnablePending", "{on}");
    assert!(engine.irreversible_in_session(session).unwrap().is_empty());

    engine.rollback_session(session).unwrap();
    let (kept, off) = (feature_state(platform), feature_state(linux));
    println!("after rollback: {platform}={kept} {linux}={off}");
    assert!(off == "Disabled" || off == "DisablePending", "{linux} must be off again: {off}");
    assert!(kept == "Enabled" || kept == "EnablePending", "{platform} was on before and must stay: {kept}");
    assert_eq!(state(&engine, session), SessionState::RolledBack);

    let backup = engine.catalog.feature("reg-backup").expect("reg-backup").clone();
    let config = r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Configuration Manager";
    let value_before = dword(config, "EnablePeriodicBackup");
    let session = engine.apply_feature(&backup, None, &CancelToken::new()).unwrap();
    assert_eq!(dword(config, "EnablePeriodicBackup"), Some(1));
    assert_eq!(ps("@(Get-ScheduledTask -TaskName AutoRegBackup -ErrorAction SilentlyContinue).Count"), "1");
    engine.rollback_session(session).unwrap();
    assert_eq!(dword(config, "EnablePeriodicBackup"), value_before);
    assert_eq!(ps("@(Get-ScheduledTask -TaskName AutoRegBackup -ErrorAction SilentlyContinue).Count"), "0");
}

#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t06_update_modes_apply_and_roll_back() {
    if !live() {
        return;
    }
    let engine = engine();
    let policy = r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate";
    let au = r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU";
    let services = ["BITS", "wuauserv", "UsoSvc"];
    let tasks = "@(Get-ScheduledTask -TaskPath '\\Microsoft\\Windows\\WindowsUpdate\\' | Where-Object State -eq 'Disabled').Count";
    let starts = || services.iter().map(|name| service_start(name)).collect::<Vec<_>>();
    let before = (dword(policy, "DeferQualityUpdatesPeriodInDays"), dword(au, "NoAutoUpdate"), starts());
    println!("before: {before:?}, disabled update tasks: {}", ps(tasks));

    let mode = |id: &str| engine.catalog.update_mode(id).unwrap_or_else(|| panic!("mode {id}")).clone();
    let security = engine.apply_update_mode(&mode("security")).unwrap();
    assert_eq!(dword(policy, "DeferQualityUpdatesPeriodInDays"), Some(4));
    assert_eq!(dword(policy, "DeferFeatureUpdatesPeriodInDays"), Some(365));
    assert_eq!(dword(policy, "ExcludeWUDriversInQualityUpdate"), Some(1));
    assert_eq!(engine.current_update_mode().unwrap(), "security");

    let disabled = engine.apply_update_mode(&mode("disabled")).unwrap();
    assert_eq!(dword(au, "NoAutoUpdate"), Some(1));
    assert_eq!(starts(), [Some(4), Some(4), Some(4)]);
    let off = ps(tasks);
    println!("disabled update tasks after Disable ALL: {off}");
    assert_ne!(off, "0");
    assert_eq!(engine.current_update_mode().unwrap(), "disabled");

    engine.rollback_session(disabled).unwrap();
    assert_eq!(engine.current_update_mode().unwrap(), "security", "rollback returns to the earlier mode");
    assert_eq!(dword(au, "NoAutoUpdate"), before.1);
    assert_eq!(starts(), before.2);

    engine.rollback_session(security).unwrap();
    let after = (dword(policy, "DeferQualityUpdatesPeriodInDays"), dword(au, "NoAutoUpdate"), starts());
    println!("after both rollbacks: {after:?}, disabled update tasks: {}", ps(tasks));
    assert_eq!(after, before);
}

#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t07_bitlocker_decrypt_on_this_volume() {
    if !live() {
        return;
    }
    let engine = engine();
    let status = "(Get-BitLockerVolume -MountPoint $env:SystemDrive).VolumeStatus";
    println!("before: {}", ps(status));
    let decrypt = tweak(&engine, "disable-bit-locker");
    assert!(decrypt.irreversible);
    let report = engine.run("VM BitLocker", &[(decrypt, TweakAction::Apply)], &CancelToken::new(), |_, _| {}).unwrap();
    println!("failed: {:?}", report.failed);
    let after = ps(status);
    println!("after: {after}");
    assert!(after == "FullyDecrypted" || after == "DecryptionInProgress", "{after}");
}

/// Needs `WINDOWCRAFT_LIVE_ISO` (a Windows 11 ISO) and `WINDOWCRAFT_LIVE_OUT`
/// (where the built ISO goes). Network access for oscdimg through WinGet.
#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t08_iso_studio_build() {
    if !live() {
        return;
    }
    let (Ok(source), Ok(output)) = (std::env::var("WINDOWCRAFT_LIVE_ISO"), std::env::var("WINDOWCRAFT_LIVE_OUT")) else {
        println!("skipped: WINDOWCRAFT_LIVE_ISO or WINDOWCRAFT_LIVE_OUT not set");
        return;
    };
    use windowcraft_iso::pipeline;
    let catalog = Arc::new(Catalog::load().unwrap());
    let work = windowcraft_engine::win::local_app_dir().join("iso");
    let options = windowcraft_iso::AnswerOptions {
        bypass_requirements: true,
        local_account: true,
        dark_mode: true,
        disable_device_encryption: true,
        pause_updates_during_oobe: true,
        ..windowcraft_iso::AnswerOptions::default()
    };
    let mut session = pipeline::StudioSession::new(PathBuf::from(&source), options);
    let mut profile = windowcraft_config::Profile::new();
    profile.tweaks = vec!["activity".to_owned()];
    session.profile = Some(profile);

    let (work_job, catalog_job, output_job) = (work.clone(), catalog.clone(), PathBuf::from(&output));
    let summary = job("iso", move |context| {
        let wrap = |error: windowcraft_iso::IsoError| windowcraft_engine::EngineError::Invalid(error.to_string());
        let images = pipeline::extract(&mut session, &work_job, context).map_err(wrap)?;
        let image = pipeline::default_edition(&images).expect("an edition").clone();
        println!("  editions: {}, chosen {} {}", images.len(), image.index, image.name);
        pipeline::select_edition(&mut session.options, &image);
        pipeline::configure(&mut session, &work_job, &catalog_job, context).map_err(wrap)?;
        pipeline::export_iso(&mut session, &work_job, &output_job, context).map_err(wrap)?;
        Ok("built".to_owned())
    });
    println!("{summary}");

    let media = work.join("media");
    let answer = std::fs::read_to_string(media.join("autounattend.xml")).expect("autounattend.xml");
    for needed in ["BypassTPMCheck", "BypassSecureBootCheck", "BypassNRO", "PreventDeviceEncryption", "/IMAGE/INDEX"] {
        assert!(answer.contains(needed), "autounattend.xml lacks {needed}");
    }
    for forbidden in ["DiskConfiguration", "WillWipeDisk"] {
        assert!(!answer.contains(forbidden), "autounattend.xml must not contain {forbidden}");
    }
    let scripts = media.join(r"sources\$OEM$\$$\Setup\Scripts");
    assert!(scripts.join("SetupComplete.cmd").exists());
    assert!(scripts.join(r"WindowCraft\profile.json").exists());
    assert!(scripts.join(r"WindowCraft\windowcraft.exe").exists());
    assert!(!media.join(r"sources\PID.txt").exists());
    assert_eq!(ps("@(Get-WindowsImage -Mounted).Count"), "0", "no WIM left mounted");
    let volume = ps(&format!(
        "$m = Mount-DiskImage -ImagePath '{output}' -PassThru; $l = ($m | Get-Volume).FileSystemLabel; Dismount-DiskImage -ImagePath '{output}' | Out-Null; $l"
    ));
    println!("built ISO mounts with label '{volume}'");
    assert!(!volume.is_empty());
}

/// Needs `WINDOWCRAFT_LIVE_USB_MODEL`: the model string of a throwaway disk on
/// the USB bus. Runs after t08, from its staged media.
#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t09_usb_guard_and_write() {
    if !live() {
        return;
    }
    let Ok(model) = std::env::var("WINDOWCRAFT_LIVE_USB_MODEL") else {
        println!("skipped: WINDOWCRAFT_LIVE_USB_MODEL not set");
        return;
    };
    use windowcraft_iso::{pipeline, usb};
    let all = usb::disks();
    let eligible = usb::eligible_disks();
    println!("disks: {:?}", all.iter().map(|d| (d.number, d.model.clone(), d.bus.label())).collect::<Vec<_>>());
    println!("eligible: {:?}", eligible.iter().map(|d| (d.number, d.model.clone())).collect::<Vec<_>>());
    assert!(eligible.iter().all(|disk| !disk.is_system), "the system disk is never offered");
    let target = eligible.iter().find(|disk| disk.model.trim() == model.trim()).expect("the throwaway USB disk is offered");
    assert_eq!(eligible.len(), 1, "only the throwaway disk is offered");

    let catalog = Arc::new(Catalog::load().unwrap());
    let work = windowcraft_engine::win::local_app_dir().join("iso");
    let mut session = pipeline::StudioSession::resume(&work).expect("session from t08");
    let (number, model_job) = (target.number, target.model.clone());
    job("usb", move |context| {
        pipeline::export_usb(&mut session, &work, &catalog, number, &model_job, context)
            .map_err(|error| windowcraft_engine::EngineError::Invalid(error.to_string()))?;
        Ok("written".to_owned())
    });
    let layout = ps(&format!(
        "$d = Get-Disk -Number {number}; $p = Get-Partition -DiskNumber {number} | Where-Object DriveLetter; $v = $p | Get-Volume; \"$($d.PartitionStyle) $($v.FileSystem) $($v.FileSystemLabel) $(Test-Path \"$($p.DriveLetter):\\autounattend.xml\") $(Test-Path \"$($p.DriveLetter):\\sources\\boot.wim\")\""
    ));
    println!("USB layout: {layout}");
    assert_eq!(layout, "GPT FAT32 WIN11 True True");
}
