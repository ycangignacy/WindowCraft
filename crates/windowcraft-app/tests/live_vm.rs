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

/// raw.githubusercontent.com answers on port 443 within five seconds.
fn online() -> bool {
    use std::net::ToSocketAddrs;
    ("raw.githubusercontent.com", 443).to_socket_addrs().ok().and_then(|mut addresses| addresses.next()).is_some_and(
        |address| std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(5)).is_ok(),
    )
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
        "HKU" => "HKEY_USERS",
        "HKCR" => "HKEY_CLASSES_ROOT",
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
    let (number, identity) = (target.number, target.identity.clone());
    job("usb", move |context| {
        pipeline::export_usb(&mut session, &work, &catalog, number, &identity, context)
            .map_err(|error| windowcraft_engine::EngineError::Invalid(error.to_string()))?;
        Ok("written".to_owned())
    });
    let layout = ps(&format!(
        "$d = Get-Disk -Number {number}; $p = Get-Partition -DiskNumber {number} | Where-Object DriveLetter; $v = $p | Get-Volume; \"$($d.PartitionStyle) $($v.FileSystem) $($v.FileSystemLabel) $(Test-Path \"$($p.DriveLetter):\\autounattend.xml\") $(Test-Path \"$($p.DriveLetter):\\sources\\boot.wim\")\""
    ));
    println!("USB layout: {layout}");
    assert_eq!(layout, "GPT FAT32 WIN11 True True");
}

/// The WindowCraft PowerShell profile from the System page: installed over a
/// profile of the user's own, kept as a backup, removed again.
#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t10_powershell_profile_install_and_remove() {
    if !live() {
        return;
    }
    let engine = engine();
    let path = ps("[Environment]::GetFolderPath('MyDocuments')") + r"\PowerShell\Microsoft.PowerShell_profile.ps1";
    let path = PathBuf::from(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# the user's own profile\n").unwrap();

    let install = engine.catalog.feature("install-psprofile").expect("install-psprofile").clone();
    let remove = engine.catalog.feature("uninstall-psprofile").expect("uninstall-psprofile").clone();
    engine.apply_feature(&install, None, &CancelToken::new()).unwrap();
    engine.apply_feature(&install, None, &CancelToken::new()).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# WindowCraft PowerShell profile"), "profile written");
    let backup = path.with_file_name("Microsoft.PowerShell_profile.ps1.bak");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "# the user's own profile\n", "backup kept after two installs");

    // The profile has to load and its prompt and commands have to run.
    let loaded = ps(&format!(
        ". '{}'; $p = prompt; $u = uptime; $w = which powershell; \"ok $($p.Length -gt 0) $($u -match 'days') $($w -match 'powershell')\"",
        path.display()
    ));
    println!("loaded: {loaded}");
    assert_eq!(loaded, "ok True True True");

    engine.apply_feature(&remove, None, &CancelToken::new()).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# the user's own profile\n", "the user's profile is back");
    assert!(!backup.exists());
    std::fs::remove_file(&path).unwrap();
}

/// The scheduled check as Settings creates it: registered for this user
/// without elevation, run once by Task Scheduler, recorded, removed again.
#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t11_scheduled_check_from_settings() {
    use windowcraft_app::settings::{self, CheckInterval, Settings};
    if !live() {
        return;
    }
    let program = PathBuf::from(r"C:\WC\bin\windowcraft.exe");
    assert!(program.exists(), "the kit holds windowcraft.exe");
    {
        // A reverted tweak for the check to find.
        let engine = engine();
        run(&engine, "VM drift for the scheduled check", &[(tweak(&engine, "activity"), TweakAction::Apply)]);
    }
    ps(r"Set-ItemProperty 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\System' -Name EnableActivityFeed -Value 0 -Type DWord");

    let chosen = Settings { interval: CheckInterval::Weekly, after_updates: true, ..Settings::default() };
    settings::save(&chosen).unwrap();
    settings::apply_schedule_for(&chosen, "WindowCraft live test", &program).unwrap();
    let name = windowcraft_engine::tasks::drift_check_task();
    let facts = ps(&format!(
        "$t = Get-ScheduledTask -TaskName '{name}'; \"$($t.Principal.RunLevel) $($t.Principal.LogonType) $($t.Triggers.Count) $($t.Actions[0].Arguments)\""
    ));
    println!("task: {facts}");
    assert_eq!(facts, "Limited Interactive 2 --scheduled-check");

    ps(&format!("Start-ScheduledTask -TaskName '{name}'"));
    let mut recorded = false;
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if settings::load().last_check > 0 {
            recorded = true;
            break;
        }
    }
    // The run waits on its notification or message box; it is not needed any more.
    ps("Get-Process windowcraft -ErrorAction SilentlyContinue | Stop-Process -Force");
    assert!(recorded, "the task run recorded its check in settings.json");
    let after = settings::load();
    assert_eq!(after.interval, CheckInterval::Weekly);
    assert!(!after.last_build.is_empty());
    assert_eq!(dword(SYSTEM, "EnableActivityFeed"), Some(0), "the check changes nothing");

    let off = Settings { interval: CheckInterval::Off, after_updates: false, ..after };
    settings::save(&off).unwrap();
    settings::apply_schedule_for(&off, "WindowCraft live test", &program).unwrap();
    assert_eq!(ps(&format!("@(Get-ScheduledTask -TaskName '{name}' -ErrorAction SilentlyContinue).Count")), "0");

    let engine = engine();
    assert_eq!(engine.repair_drift().unwrap(), 1);
    assert_eq!(dword(SYSTEM, "EnableActivityFeed"), Some(1));
}

/// The Group Policy value since 1.1.0. Up to 1.0.3 the tweak wrote the
/// Settings store under CurrentVersion\Policies, which DiagTrack writes back
/// after a delete, so a rollback could not remove it (KB-15).
const ALLOW_TELEMETRY: (&str, &str) = (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection", "AllowTelemetry");
const SETTINGS_STORE: &str = r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection";

/// The Standard preset, which also contains Minimal, applied and rolled back.
#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t12_standard_preset_applies_and_rolls_back() {
    if !live() {
        return;
    }
    let engine = engine();
    let items: Vec<(&Tweak, TweakAction)> =
        engine.catalog.presets.standard.iter().map(|id| (tweak(&engine, id), TweakAction::Apply)).collect();
    assert_eq!(items.len(), 13);
    let watched = [
        (SYSTEM, "PublishUserActivities"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization", "DODownloadMode"),
        (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableWindowsConsumerFeatures"),
    ];
    let read = || watched.iter().map(|(path, name)| dword(path, name)).collect::<Vec<_>>();
    let before = read();
    let session = run(&engine, "VM Standard preset", &items);
    let applied = read();
    println!("before {before:?} applied {applied:?}");
    assert_eq!(applied[0], Some(0));
    assert_eq!(dword(ALLOW_TELEMETRY.0, ALLOW_TELEMETRY.1), Some(0));
    let restore_points = ps("@(Get-ComputerRestorePoint -ErrorAction SilentlyContinue).Count");
    println!("restore points: {restore_points}");
    assert_ne!(restore_points, "0", "the preset starts with a restore point");

    engine.rollback_session(session).unwrap();
    assert_eq!(read(), before);
    let final_state = state(&engine, session);
    println!("state after rollback: {final_state:?}");
    assert!(matches!(final_state, SessionState::RolledBack | SessionState::PartlyRolledBack));
}

/// The telemetry tweak alone: every value it writes is gone or back after a
/// rollback, also 20 s later, and the Settings store is never touched.
#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t13_telemetry_rolls_back_every_value() {
    if !live() {
        return;
    }
    let engine = engine();
    let telemetry = tweak(&engine, "telemetry");
    let entries: Vec<(String, String)> = telemetry
        .registry
        .iter()
        .filter(|e| e.kind == windowcraft_config::RegKind::DWord)
        .map(|e| (e.path.clone(), e.name.clone()))
        .collect();
    let read = || entries.iter().map(|(path, name)| dword(path, name)).collect::<Vec<_>>();
    let store_before = dword(SETTINGS_STORE, "AllowTelemetry");
    let before = read();
    let session = run(&engine, "VM telemetry", &[(telemetry, TweakAction::Apply)]);
    assert_eq!(dword(ALLOW_TELEMETRY.0, ALLOW_TELEMETRY.1), Some(0), "the policy is set");
    let applied = read();
    engine.rollback_session(session).unwrap();
    let now = read();
    std::thread::sleep(std::time::Duration::from_secs(20));
    let later = read();
    for (index, (path, name)) in entries.iter().enumerate() {
        let mark = if before[index] == now[index] && now[index] == later[index] { "same" } else { "DIFFERENT" };
        println!("{mark}  {path}\\{name}: before {:?} applied {:?} rollback {:?} 20 s later {:?}", before[index], applied[index], now[index], later[index]);
    }
    assert_eq!(now, before, "right after the rollback");
    assert_eq!(later, before, "20 s after the rollback");
    assert_eq!(dword(SETTINGS_STORE, "AllowTelemetry"), store_before, "the Settings store is left alone");
}

/// Any value as text through PowerShell, `None` when absent. Binary values
/// come back as their bytes joined with spaces.
fn value(path: &str, name: &str) -> Option<String> {
    let text = ps(&format!(
        "$v = Get-ItemProperty -LiteralPath '{}' -Name '{name}' -ErrorAction SilentlyContinue; if ($null -ne $v) {{ '=' + ((@($v.'{name}') | ForEach-Object {{ \"$_\" }}) -join ' ') }}",
        registry_path(path)
    ));
    text.strip_prefix('=').map(str::to_owned)
}

/// Every tweak that can be undone, one at a time: applied, rolled back, and
/// after 10 s every registry value and service it touches is as before.
/// Tweaks that remove apps, delete files or decrypt drives are left out.
#[test]
#[ignore = "changes the live system; VirtualBox guest only"]
fn t14_every_reversible_tweak_rolls_back_cleanly() {
    use windowcraft_config::TweakKind;
    if !live() {
        return;
    }
    const ONE_WAY: [&str; 9] = [
        "widget",
        "remove-edge",
        "remove-one-drive",
        "disable-bit-locker",
        "windows-ai",
        "disk-cleanup",
        "delete-temp-files",
        "restore-point",
        "revert-start-menu",
    ];
    let engine = engine();
    // Tweaks that download a list (block-adobe-net) cannot run while the VM
    // is offline, and t14 runs offline so Windows Update stays quiet.
    let online = online();
    let downloads = |tweak: &windowcraft_config::Tweak| {
        serde_json::to_string(&tweak.apply).is_ok_and(|json| json.contains("\"url\":\"http"))
    };
    let mut problems = Vec::new();
    let mut skipped = Vec::new();
    let mut checked = 0;
    for tweak in &engine.catalog.tweaks {
        let action = match tweak.kind {
            TweakKind::Check if !ONE_WAY.contains(&tweak.id.as_str()) => TweakAction::Apply,
            TweakKind::Toggle => TweakAction::Set(true),
            _ => continue,
        };
        if !online && downloads(tweak) {
            println!("  {} skipped: it downloads a list and the network is off", tweak.id);
            skipped.push(tweak.id.clone());
            continue;
        }
        let values: Vec<(String, String)> = tweak.registry.iter().map(|e| (e.path.clone(), e.name.clone())).collect();
        let services: Vec<String> = tweak.services.iter().map(|rule| rule.name.clone()).collect();
        let read = || {
            (
                values.iter().map(|(path, name)| value(path, name)).collect::<Vec<_>>(),
                services.iter().map(|name| service_start(name)).collect::<Vec<_>>(),
            )
        };
        let before = read();
        let report = engine.run(&format!("VM sweep {}", tweak.id), &[(tweak, action)], &CancelToken::new(), |_, _| {}).unwrap();
        if !report.failed.is_empty() {
            problems.push(format!("{}: apply failed: {:?}", tweak.id, report.failed));
            continue;
        }
        if let Err(error) = engine.rollback_session(report.session) {
            problems.push(format!("{}: rollback failed: {error}", tweak.id));
            continue;
        }
        std::thread::sleep(std::time::Duration::from_secs(10));
        let after = read();
        for (index, (path, name)) in values.iter().enumerate() {
            if before.0[index] != after.0[index] {
                problems.push(format!("{}: {path}\\{name} was {:?}, after rollback {:?}", tweak.id, before.0[index], after.0[index]));
            }
        }
        for (index, name) in services.iter().enumerate() {
            if before.1[index] != after.1[index] {
                problems.push(format!("{}: service {name} start was {:?}, after rollback {:?}", tweak.id, before.1[index], after.1[index]));
            }
        }
        checked += 1;
        println!("  {} checked: {} values, {} services", tweak.id, values.len(), services.len());
    }
    println!("checked {checked} tweaks, skipped {} without network: {skipped:?}", skipped.len());
    for problem in &problems {
        println!("PROBLEM {problem}");
    }
    assert!(checked > 50, "only {checked} tweaks were checked");
    assert!(problems.is_empty(), "{} problems, listed above", problems.len());
}
