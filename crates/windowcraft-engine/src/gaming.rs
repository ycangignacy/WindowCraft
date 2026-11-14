// Made by Nordan
//! MSI mode, interrupt priority, GPU settings and timer resolution.

use crate::dynlib::DynLib;
use crate::error::{EngineError, Result};
use crate::executor::Engine;
use crate::registry::{Hive, RegPath, RegValue, RegistryScope};
use windowcraft_config::{RegKind, RegistryEntry, Tweak, TweakGroup, TweakKind};

const PCI_ROOT: &str = r"SYSTEM\CurrentControlSet\Enum\PCI";
const INTERRUPT_MANAGEMENT: &str = r"Device Parameters\Interrupt Management";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Gpu,
    Nvme,
    Network,
    Usb,
}

impl DeviceClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gpu => "GPU",
            Self::Nvme => "NVMe",
            Self::Network => "Network",
            Self::Usb => "USB",
        }
    }

    /// Device instances carry `ClassGUID`, not a class name.
    fn from_class_guid(guid: &str, name: &str, service: &str) -> Option<Self> {
        match guid.to_ascii_lowercase().as_str() {
            "{4d36e968-e325-11ce-bfc1-08002be10318}" => Some(Self::Gpu),
            "{4d36e972-e325-11ce-bfc1-08002be10318}" => Some(Self::Network),
            "{36fc9e60-c465-11cf-8056-444553540000}" => Some(Self::Usb),
            "{4d36e97b-e325-11ce-bfc1-08002be10318}" | "{4d36e96a-e325-11ce-bfc1-08002be10318}"
                if service.eq_ignore_ascii_case("stornvme") || name.to_lowercase().contains("nvm") =>
            {
                Some(Self::Nvme)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptPriority {
    Undefined = 0,
    Low = 1,
    Normal = 2,
    High = 3,
}

impl InterruptPriority {
    pub const ALL: [Self; 4] = [Self::Undefined, Self::Low, Self::Normal, Self::High];

    fn from_value(value: u32) -> Self {
        match value {
            1 => Self::Low,
            2 => Self::Normal,
            3 => Self::High,
            _ => Self::Undefined,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PciDevice {
    pub instance: String,
    pub name: String,
    pub class: DeviceClass,
    pub msi_enabled: bool,
    pub priority: InterruptPriority,
}

impl PciDevice {
    fn msi_key(&self) -> String {
        format!(r"{PCI_ROOT}\{}\{INTERRUPT_MANAGEMENT}\MessageSignaledInterruptProperties", self.instance)
    }

    fn affinity_key(&self) -> String {
        format!(r"{PCI_ROOT}\{}\{INTERRUPT_MANAGEMENT}\Affinity Policy", self.instance)
    }
}

fn string_of(key: &crate::registry::Key, name: &str) -> String {
    match key.read(name) {
        Ok(Some(RegValue::String(value))) => value,
        Ok(Some(RegValue::MultiString(items))) => items.into_iter().next().unwrap_or_default(),
        _ => String::new(),
    }
}

/// `@oem1.inf,%name%;Friendly Name` holds the text after the last semicolon.
fn friendly(text: &str) -> &str {
    text.rsplit(';').next().unwrap_or(text)
}

fn read_device(scope: &RegistryScope, instance: String) -> Option<PciDevice> {
    let path = RegPath { hive: Hive::LocalMachine, subkey: format!(r"{PCI_ROOT}\{instance}") };
    let key = scope.open(&path).ok()??;
    let description = string_of(&key, "DeviceDesc");
    let name = friendly(&description).to_owned();
    let class = DeviceClass::from_class_guid(&string_of(&key, "ClassGUID"), &name, &string_of(&key, "Service"))?;

    let mut device = PciDevice { instance, name, class, msi_enabled: false, priority: InterruptPriority::Undefined };
    let msi = RegPath { hive: Hive::LocalMachine, subkey: device.msi_key() };
    device.msi_enabled = matches!(scope.read(&msi, "MSISupported"), Ok(Some(RegValue::DWord(1))));
    let affinity = RegPath { hive: Hive::LocalMachine, subkey: device.affinity_key() };
    if let Ok(Some(RegValue::DWord(value))) = scope.read(&affinity, "DevicePriority") {
        device.priority = InterruptPriority::from_value(value);
    }
    Some(device)
}

/// Lists GPUs, NVMe controllers, NICs and USB host controllers.
pub fn pci_devices(scope: &RegistryScope) -> Result<Vec<PciDevice>> {
    let root = RegPath { hive: Hive::LocalMachine, subkey: PCI_ROOT.to_owned() };
    let Some(key) = scope.open(&root)? else {
        return Ok(Vec::new());
    };
    let mut devices = Vec::new();
    for hardware in key.subkeys()? {
        let path = RegPath { hive: Hive::LocalMachine, subkey: format!(r"{PCI_ROOT}\{hardware}") };
        let Some(hardware_key) = scope.open(&path)? else { continue };
        for instance in hardware_key.subkeys()? {
            if let Some(device) = read_device(scope, format!(r"{hardware}\{instance}")) {
                devices.push(device);
            }
        }
    }
    Ok(devices)
}

/// Sets MSI mode through a snapshot.
pub fn set_msi(engine: &Engine, device: &PciDevice, enabled: bool) -> Result<u64> {
    let path = RegPath { hive: Hive::LocalMachine, subkey: device.msi_key() };
    let label = format!("MSI mode: {}", device.name);
    engine.write_tracked(&label, "msi-mode", &path, "MSISupported", RegValue::DWord(u32::from(enabled)))
}

/// Sets interrupt priority through a snapshot.
pub fn set_priority(engine: &Engine, device: &PciDevice, priority: InterruptPriority) -> Result<u64> {
    let path = RegPath { hive: Hive::LocalMachine, subkey: device.affinity_key() };
    let label = format!("Interrupt priority: {}", device.name);
    engine.write_tracked(&label, "msi-mode", &path, "DevicePriority", RegValue::DWord(priority as u32))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimerResolution {
    pub coarsest_ms: f64,
    pub finest_ms: f64,
    pub current_ms: f64,
}

/// Reads the kernel timer resolution. All values come in 100ns units.
pub fn timer_resolution() -> Result<TimerResolution> {
    type Query = unsafe extern "system" fn(*mut u32, *mut u32, *mut u32) -> i32;
    let ntdll = DynLib::load("ntdll.dll")?;
    // SAFETY: NtQueryTimerResolution takes three PULONG out parameters.
    let query = unsafe { ntdll.symbol::<Query>(c"NtQueryTimerResolution")? };
    let (mut coarsest, mut finest, mut current) = (0u32, 0u32, 0u32);
    // SAFETY: All three pointers are valid for writes.
    let status = unsafe { query(&mut coarsest, &mut finest, &mut current) };
    if status != 0 {
        return Err(EngineError::Win32 {
            operation: "NtQueryTimerResolution",
            target: "kernel timer".to_owned(),
            code: status.cast_unsigned(),
            message: "NTSTATUS failure".to_owned(),
        });
    }
    let milliseconds = |value: u32| f64::from(value) / 10_000.0;
    Ok(TimerResolution {
        coarsest_ms: milliseconds(coarsest),
        finest_ms: milliseconds(finest),
        current_ms: milliseconds(current),
    })
}

fn entry(path: &str, name: &str, kind: RegKind, on: &str, off: &str) -> RegistryEntry {
    RegistryEntry {
        path: path.to_owned(),
        name: name.to_owned(),
        kind,
        value: Some(on.to_owned()),
        original: Some(off.to_owned()),
        default_on: None,
        values: None,
    }
}

fn toggle(id: &str, name: &str, description: &str, registry: Vec<RegistryEntry>) -> Tweak {
    Tweak {
        id: id.to_owned(),
        legacy_id: String::new(),
        name: name.to_owned(),
        description: description.to_owned(),
        group: TweakGroup::Performance,
        kind: TweakKind::Toggle,
        registry,
        services: Vec::new(),
        prepare: Vec::new(),
        apply: Vec::new(),
        undo: Vec::new(),
        options: Vec::new(),
        notice: None,
        irreversible: false,
        exclusive: None,
    }
}

/// Display-pipeline toggles, defined as tweaks so snapshots and drift apply.
pub fn display_toggles() -> Vec<Tweak> {
    const GAME_CONFIG: &str = r"HKCU\System\GameConfigStore";
    const REMOVE: &str = windowcraft_config::REMOVE_ENTRY;
    vec![
        toggle(
            "hags",
            "Hardware-Accelerated GPU Scheduling",
            "HwSchMode. Takes effect after a restart.",
            vec![entry(r"HKLM\SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "HwSchMode", RegKind::DWord, "2", "1")],
        ),
        toggle(
            "vrr",
            "Variable Refresh Rate",
            "Per-user DirectX VRR optimization.",
            vec![entry(
                r"HKCU\Software\Microsoft\DirectX\UserGpuPreferences",
                "DirectXUserGlobalSettings",
                RegKind::String,
                "VRROptimizeEnable=1;",
                "VRROptimizeEnable=0;",
            )],
        ),
        toggle(
            "fullscreen-optimizations",
            "Fullscreen Optimizations",
            "GameConfigStore FSE behavior flags.",
            vec![
                entry(GAME_CONFIG, "GameDVR_FSEBehaviorMode", RegKind::DWord, "0", "2"),
                entry(GAME_CONFIG, "GameDVR_HonorUserFSEBehaviorMode", RegKind::DWord, REMOVE, "1"),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Sandbox;

    const DISPLAY: &str = "{4d36e968-e325-11ce-bfc1-08002be10318}";
    const NET: &str = "{4D36E972-E325-11CE-BFC1-08002BE10318}";
    const STORAGE: &str = "{4d36e97b-e325-11ce-bfc1-08002be10318}";
    const SYSTEM: &str = "{4d36e97d-e325-11ce-bfc1-08002be10318}";

    /// Seeds the values a real device instance has. Windows has no `Class`
    /// value under `Enum\PCI`, so the fixture must not invent one.
    fn seed_device(scope: &RegistryScope, instance: &str, class_guid: &str, description: &str) {
        let path = RegPath { hive: Hive::LocalMachine, subkey: format!(r"{PCI_ROOT}\{instance}") };
        scope.write(&path, "ClassGUID", &RegValue::String(class_guid.to_owned())).unwrap();
        scope.write(&path, "DeviceDesc", &RegValue::String(description.to_owned())).unwrap();
    }

    #[test]
    fn enumerates_only_supported_classes_and_toggles_msi() {
        let sandbox = Sandbox::new("msimode");
        let scope = &sandbox.scope;
        seed_device(scope, r"VEN_10DE&DEV_2484\4&1", DISPLAY, "@oem9.inf,%nv%;NVIDIA GeForce RTX 3070");
        seed_device(scope, r"VEN_144D&DEV_A80A\4&2", STORAGE, "Standard NVM Express Controller");
        seed_device(scope, r"VEN_8086&DEV_15F3\4&3", NET, "Intel Ethernet");
        seed_device(scope, r"VEN_1022&DEV_1234\4&4", SYSTEM, "PCI Bridge");

        let mut devices = pci_devices(scope).unwrap();
        devices.sort_by_key(|device| device.class.label());
        assert_eq!(devices.len(), 3, "the PCI bridge is out of scope");

        let gpu = devices.iter().find(|device| device.class == DeviceClass::Gpu).unwrap();
        assert_eq!(gpu.name, "NVIDIA GeForce RTX 3070");
        assert!(!gpu.msi_enabled);

        let dir = tempfile::tempdir().unwrap();
        let engine = Engine {
            scope: scope.clone(),
            services: std::sync::Arc::new(crate::services::MemoryServices::default()),
            native: std::sync::Arc::new(crate::executor::RecordingRunner::default()),
            store: std::sync::Arc::new(crate::SnapshotStore::open(&dir.path().join("g.redb")).unwrap()),
            catalog: std::sync::Arc::new(windowcraft_config::Catalog::load().unwrap()),
            ram_kb: 0,
            os_build: String::new(),
        };
        let msi_session = set_msi(&engine, gpu, true).unwrap();
        set_priority(&engine, gpu, InterruptPriority::High).unwrap();
        let updated = pci_devices(scope).unwrap();
        let tuned = updated.iter().find(|device| device.class == DeviceClass::Gpu).unwrap();
        assert!(tuned.msi_enabled);
        assert_eq!(tuned.priority, InterruptPriority::High);

        // Both controls land in the timeline and roll back to the true value.
        assert_eq!(engine.store.sessions().unwrap().len(), 2);
        engine.rollback_session(msi_session).unwrap();
        let reverted = pci_devices(scope).unwrap();
        let reverted = reverted.iter().find(|device| device.class == DeviceClass::Gpu).unwrap();
        assert!(!reverted.msi_enabled);
        assert_eq!(reverted.priority, InterruptPriority::High, "other sessions stay applied");
    }

    #[test]
    fn reads_real_timer_resolution() {
        let timer = timer_resolution().unwrap();
        assert!(timer.finest_ms > 0.0 && timer.finest_ms <= timer.coarsest_ms);
        assert!(timer.current_ms >= timer.finest_ms - 0.001);
    }

    /// Counts the GPU, network and USB instances straight from the registry
    /// and expects the scan to classify each of them. A machine may have none:
    /// a GitHub runner once had a single PCI entry of another class.
    #[test]
    fn real_device_scan_finds_what_the_registry_holds() {
        const ALWAYS: [&str; 3] = [
            "{4d36e968-e325-11ce-bfc1-08002be10318}",
            "{4d36e972-e325-11ce-bfc1-08002be10318}",
            "{36fc9e60-c465-11cf-8056-444553540000}",
        ];
        let scope = RegistryScope::System;
        let root = RegPath { hive: Hive::LocalMachine, subkey: PCI_ROOT.to_owned() };
        let mut expected = 0;
        if let Some(key) = scope.open(&root).unwrap() {
            for hardware in key.subkeys().unwrap() {
                let path = RegPath { hive: Hive::LocalMachine, subkey: format!(r"{PCI_ROOT}\{hardware}") };
                let Some(hardware_key) = scope.open(&path).unwrap() else { continue };
                for instance in hardware_key.subkeys().unwrap() {
                    let path = RegPath { hive: Hive::LocalMachine, subkey: format!(r"{PCI_ROOT}\{hardware}\{instance}") };
                    let Some(instance_key) = scope.open(&path).ok().flatten() else { continue };
                    let guid = string_of(&instance_key, "ClassGUID").to_ascii_lowercase();
                    expected += usize::from(ALWAYS.contains(&guid.as_str()));
                }
            }
        }
        let devices = pci_devices(&scope).unwrap();
        let found = devices
            .iter()
            .filter(|device| matches!(device.class, DeviceClass::Gpu | DeviceClass::Network | DeviceClass::Usb))
            .count();
        assert_eq!(found, expected, "GPU, network and USB instances in the registry against the scan");
    }

    #[test]
    fn display_toggles_are_valid() {
        for tweak in display_toggles() {
            for entry in &tweak.registry {
                windowcraft_config::validate::check_registry_entry(&tweak.id, entry).unwrap();
            }
        }
    }
}
