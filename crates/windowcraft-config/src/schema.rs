// Made by ycangignacy
//! Declarative catalog entries. Unknown keys are rejected everywhere so a typo
//! in a custom config fails loudly instead of silently doing nothing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Sentinel meaning "this value should not exist".
pub const REMOVE_ENTRY: &str = "<RemoveEntry>";
/// Value token expanded to installed RAM in KB at apply time.
pub const RAM_KB_TOKEN: &str = "{ram_kb}";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct App {
    pub id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub link: String,
    pub foss: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winget: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choco: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
/// Value types the catalog writes. Live values of other types (REG_EXPAND_SZ,
/// REG_MULTI_SZ) are still read and restored through `RegValue` in the engine.
pub enum RegKind {
    String,
    DWord,
    QWord,
    Binary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RegistryEntry {
    pub path: String,
    pub name: String,
    pub kind: RegKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_on: Option<bool>,
    /// Per-option values for segmented controls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum StartupType {
    Automatic,
    AutomaticDelayedStart,
    Manual,
    Disabled,
    Boot,
    System,
}

impl StartupType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::AutomaticDelayedStart => "Automatic (Delayed)",
            Self::Manual => "Manual",
            Self::Disabled => "Disabled",
            Self::Boot => "Boot",
            Self::System => "System",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceRule {
    pub name: String,
    pub startup: StartupType,
    pub original: StartupType,
    /// Absent services are skipped rather than reported as failures.
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AclRights {
    Full,
    Write,
}

/// Every operation a catalog entry may perform that is not a plain registry,
/// service or task change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeOp {
    RestartExplorer,
    RefreshShell,
    BroadcastEnvironment,
    KillProcesses { pattern: String },
    RemoveAppx { pattern: String },
    MarkAppxEndOfLife { pattern: String },
    Hibernate { enabled: bool },
    AclDeny { path: String, sid: String, rights: AclRights },
    AclClearDeny { path: String, sid: String },
    BlockDirectory { path: String },
    UnblockDirectory { path: String },
    TempCleanup,
    DiskCleanup,
    RestorePoint,
    BitlockerDecrypt,
    RemoveEdge,
    RemoveOnedrive,
    WingetInstall { id: String },
    ReservedStorage { enabled: bool },
    /// `url` must name a fixed commit, and `sha256` is the hash of that file.
    HostsBlock { section: String, url: String, sha256: String },
    HostsUnblock { section: String },
    RegDeleteTree { path: String },
    Teredo { enabled: bool },
    Ipv6Binding { enabled: bool },
    DownloadAndRun {
        url: String,
        file: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<String>,
    },
    UltimatePerformance { enabled: bool },
    DisableFeature { name: String },
    RequireKbAbsent { kb: String },
    BootMenuPolicy { legacy: bool },
    NfsConfigure,
    CreateRegistryBackupTask,
    RemoveRegistryBackupTask,
    FixWindowsUpdate { aggressive: bool },
    FixNetwork,
    FixSystemScan,
    FixWinget,
    FixNtp,
    PowershellProfile { install: bool },
    SshServer,
    RemoveSshFirewallRule,
    Launch { target: String },
}

impl NativeOp {
    /// Human-readable summary shown in the X-Ray Inspector.
    pub fn describe(&self) -> String {
        match self {
            Self::RestartExplorer => "Restart explorer.exe".into(),
            Self::RefreshShell => "Broadcast WM_SETTINGCHANGE (ImmersiveColorSet)".into(),
            Self::BroadcastEnvironment => "Broadcast WM_SETTINGCHANGE (Environment)".into(),
            Self::KillProcesses { pattern } => format!("TerminateProcess matching {pattern}"),
            Self::RemoveAppx { pattern } => format!("PackageManager.RemovePackageAsync {pattern} (all users)"),
            Self::MarkAppxEndOfLife { pattern } => format!("AppxAllUserStore\\EndOfLife entry for {pattern}"),
            Self::Hibernate { enabled } => format!("powercfg /hibernate {}", if *enabled { "on" } else { "off" }),
            Self::AclDeny { path, sid, rights } => format!("SetNamedSecurityInfoW DENY {rights:?} for {sid} on {path}"),
            Self::AclClearDeny { path, sid } => format!("SetNamedSecurityInfoW remove DENY for {sid} on {path}"),
            Self::BlockDirectory { path } => format!("Purge {path} and DENY Write for S-1-1-0"),
            Self::UnblockDirectory { path } => format!("Remove DENY for S-1-1-0 on {path}"),
            Self::TempCleanup => "Parallel delete of %TEMP% and %SystemRoot%\\Temp, skipping locked files".into(),
            Self::DiskCleanup => "cleanmgr /VERYLOWDISK, then DISM /StartComponentCleanup /ResetBase".into(),
            Self::RestorePoint => "SRSetRestorePointW \"WindowCraft Automatic Restore Point\"".into(),
            Self::BitlockerDecrypt => "Win32_EncryptableVolume.Decrypt on the system drive".into(),
            Self::RemoveEdge => "Edge setup.exe --uninstall --system-level --force-uninstall".into(),
            Self::RemoveOnedrive => "OneDriveSetup.exe /uninstall under a deterministic ACL guard".into(),
            Self::WingetInstall { id } => format!("winget install {id}"),
            Self::ReservedStorage { enabled } => format!("DismSetReservedStorageState {enabled}"),
            Self::HostsBlock { section, .. } => format!("Atomic write of fenced {section} block in drivers\\etc\\hosts"),
            Self::HostsUnblock { section } => format!("Atomic removal of fenced {section} block in drivers\\etc\\hosts"),
            Self::RegDeleteTree { path } => format!("RegDeleteTreeW {path}"),
            Self::Teredo { enabled } => format!("netsh interface teredo set state {}", if *enabled { "default" } else { "disabled" }),
            Self::Ipv6Binding { enabled } => format!("INetCfg binding ms_tcpip6 = {enabled}"),
            Self::DownloadAndRun { url, file, .. } => format!("Download {url} to %LocalAppData%\\WindowCraft\\{file} and launch"),
            Self::UltimatePerformance { enabled } => format!("PowerDuplicateScheme e9a42b02-d5df-448d-aa00-03f14749eb61 = {enabled}"),
            Self::DisableFeature { name } => format!("DismDisableFeature {name}"),
            Self::RequireKbAbsent { kb } => format!("Check Component Based Servicing for {kb}"),
            Self::BootMenuPolicy { legacy } => format!("bcdedit /set bootmenupolicy {}", if *legacy { "legacy" } else { "standard" }),
            Self::NfsConfigure => "nfsadmin client config fileaccess=755".into(),
            Self::CreateRegistryBackupTask => "ITaskFolder::RegisterTaskDefinition AutoRegBackup at 00:30".into(),
            Self::RemoveRegistryBackupTask => "ITaskFolder::DeleteTask AutoRegBackup".into(),
            Self::FixWindowsUpdate { aggressive } => format!("Windows Update reset ({})", if *aggressive { "aggressive" } else { "standard" }),
            Self::FixNetwork => "netsh winsock reset, netsh int ip reset".into(),
            Self::FixSystemScan => "chkdsk /scan /perf, sfc /scannow, dism /restorehealth".into(),
            Self::FixWinget => "RegisterPackageByFamilyNameAsync Microsoft.DesktopAppInstaller".into(),
            Self::FixNtp => "w32time peers = pool.ntp.org,0x8, resync".into(),
            Self::PowershellProfile { install } => format!("PowerShell 7 profile {}", if *install { "install" } else { "remove" }),
            Self::SshServer => "DismAddCapability OpenSSH.Server, firewall rule, SID-based ACL".into(),
            Self::RemoveSshFirewallRule => "INetFwRules::Remove WindowCraft OpenSSH Server (TCP 22)".into(),
            Self::Launch { target } => format!("ShellExecuteW {target}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TweakGroup {
    Essential,
    Advanced,
    Preferences,
    Performance,
}

impl TweakGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::Essential => "Essential",
            Self::Advanced => "Advanced",
            Self::Preferences => "Preferences",
            Self::Performance => "Performance Plans",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TweakKind {
    /// Queued with a checkbox and applied in a batch.
    Check,
    /// Applied immediately.
    Toggle,
    /// N-state control with one registry value per option.
    Segmented,
    /// Runs once on click.
    Action,
    /// DNS provider picker.
    Dns,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Tweak {
    pub id: String,
    /// WinUtil's id for the same item, filled from `winutil-import.json`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub legacy_id: String,
    pub name: String,
    pub description: String,
    pub group: TweakGroup,
    pub kind: TweakKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<RegistryEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceRule>,
    /// Runs before registry writes, for example clearing a key to recreate it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prepare: Vec<NativeOp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apply: Vec<NativeOp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undo: Vec<NativeOp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ChoiceOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub irreversible: bool,
    /// Entries in one group write the same value, so the last one applied wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclusive: Option<String>,
}

impl Tweak {
    /// Registry state can be compared against the live system.
    pub fn is_detectable(&self) -> bool {
        !self.registry.is_empty() || !self.services.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FeatureKind {
    Feature,
    Fix,
    Panel,
    Profile,
    Remote,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub id: String,
    /// WinUtil's id for the same item, filled from `winutil-import.json`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub legacy_id: String,
    pub name: String,
    pub description: String,
    pub kind: FeatureKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<RegistryEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apply: Vec<NativeOp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undo: Vec<NativeOp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ChoiceOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// A rollback leaves part of this feature in place.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub irreversible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppxPackage {
    pub id: String,
    /// WinUtil's id for the same item, filled from `winutil-import.json`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub legacy_id: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub package_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DnsProvider {
    pub id: String,
    pub primary: String,
    pub secondary: String,
    pub primary6: String,
    pub secondary6: String,
    pub doh_template: String,
    pub benchmark: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Presets {
    pub minimal: Vec<String>,
    pub standard: Vec<String>,
    pub advanced: Vec<String>,
    pub appx_default: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresetName {
    Minimal,
    Standard,
    Advanced,
}

impl std::str::FromStr for PresetName {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "minimal" => Ok(Self::Minimal),
            "standard" => Ok(Self::Standard),
            "advanced" => Ok(Self::Advanced),
            other => Err(format!("unknown preset '{other}'; expected minimal, standard or advanced")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskFolderRule {
    pub path: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UpdateMode {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub confirm: bool,
    #[serde(default)]
    pub purge_software_distribution: bool,
    pub registry: Vec<RegistryEntry>,
    pub services: Vec<ServiceRule>,
    pub task_folders: Vec<TaskFolderRule>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_schema_types_populate_defaults_and_roundtrip_cleanly() {
        let tweak_json = r#"{
            "id": "sample",
            "legacy_id": "WPFSample",
            "name": "Sample",
            "description": "Desc",
            "group": "essential",
            "kind": "check"
        }"#;
        let tweak: Tweak = serde_json::from_str(tweak_json).unwrap();
        assert!(tweak.registry.is_empty());
        assert!(tweak.services.is_empty());
        assert!(tweak.prepare.is_empty());
        assert!(tweak.apply.is_empty());
        assert!(tweak.undo.is_empty());
        assert!(tweak.options.is_empty());
        assert!(tweak.notice.is_none());
        assert!(!tweak.irreversible);
        assert!(tweak.exclusive.is_none());
        let serialized = serde_json::to_string(&tweak).unwrap();
        assert!(!serialized.contains("irreversible"), "false irreversible flag must be skipped");
        assert!(!serialized.contains("registry"), "empty vecs must be skipped");
        assert_eq!(serde_json::from_str::<Tweak>(&serialized).unwrap(), tweak);

        let rule: ServiceRule =
            serde_json::from_str(r#"{"name":"DiagTrack","startup":"Disabled","original":"Automatic"}"#).unwrap();
        assert!(!rule.optional);
        assert_eq!(serde_json::from_str::<ServiceRule>(&serde_json::to_string(&rule).unwrap()).unwrap(), rule);

        let feature: Feature = serde_json::from_str(
            r#"{"id":"f","legacy_id":"WPFF","name":"F","description":"D","kind":"feature"}"#,
        )
        .unwrap();
        assert!(feature.features.is_empty() && feature.registry.is_empty() && feature.target.is_none());
        assert_eq!(serde_json::from_str::<Feature>(&serde_json::to_string(&feature).unwrap()).unwrap(), feature);

        let mode: UpdateMode = serde_json::from_str(
            r#"{"id":"m","name":"M","description":"D","registry":[],"services":[],"task_folders":[]}"#,
        )
        .unwrap();
        assert!(!mode.confirm && !mode.purge_software_distribution);
        assert_eq!(serde_json::from_str::<UpdateMode>(&serde_json::to_string(&mode).unwrap()).unwrap(), mode);
    }

    #[test]
    fn tweak_detectability_requires_registry_or_service_rules() {
        let mut tweak: Tweak = serde_json::from_str(
            r#"{"id":"a","legacy_id":"L","name":"N","description":"D","group":"advanced","kind":"action","apply":[{"op":"temp_cleanup"}]}"#,
        )
        .unwrap();
        assert!(!tweak.is_detectable());

        tweak.services.push(ServiceRule {
            name: "wuauserv".to_owned(),
            startup: StartupType::Manual,
            original: StartupType::Manual,
            optional: false,
        });
        assert!(tweak.is_detectable());

        tweak.services.clear();
        tweak.registry.push(RegistryEntry {
            path: r"HKCU\Software\Test".to_owned(),
            name: "V".to_owned(),
            kind: RegKind::DWord,
            value: Some("1".to_owned()),
            original: None,
            default_on: None,
            values: None,
        });
        assert!(tweak.is_detectable());
    }

    #[test]
    fn native_ops_roundtrip_with_tagged_snake_case_and_describe_effects() {
        let ops = [
            (NativeOp::Hibernate { enabled: true }, "powercfg /hibernate on"),
            (NativeOp::Hibernate { enabled: false }, "powercfg /hibernate off"),
            (
                NativeOp::AclDeny { path: r"C:\test".to_owned(), sid: "S-1-1-0".to_owned(), rights: AclRights::Full },
                r"SetNamedSecurityInfoW DENY Full for S-1-1-0 on C:\test",
            ),
            (
                NativeOp::DownloadAndRun {
                    url: "https://example.com/a.exe".to_owned(),
                    file: "a.exe".to_owned(),
                    args: None,
                },
                r"Download https://example.com/a.exe to %LocalAppData%\WindowCraft\a.exe and launch",
            ),
            (NativeOp::FixWindowsUpdate { aggressive: true }, "Windows Update reset (aggressive)"),
            (NativeOp::BootMenuPolicy { legacy: true }, "bcdedit /set bootmenupolicy legacy"),
            (NativeOp::PowershellProfile { install: false }, "PowerShell 7 profile remove"),
        ];
        for (op, expected_description) in ops {
            let text = serde_json::to_string(&op).unwrap();
            assert!(text.contains("\"op\":"), "{text}");
            assert_eq!(serde_json::from_str::<NativeOp>(&text).unwrap(), op);
            assert_eq!(op.describe(), expected_description);
        }
    }

    #[test]
    fn unknown_fields_are_rejected_across_catalog_types() {
        assert!(serde_json::from_str::<App>(
            r#"{"id":"a","name":"A","category":"C","description":"D","link":"https://x","foss":true,"extra":1}"#
        )
        .is_err());
        assert!(serde_json::from_str::<RegistryEntry>(
            r#"{"path":"HKCU\\A","name":"B","kind":"DWord","typo":"1"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<ServiceRule>(
            r#"{"name":"S","startup":"Manual","original":"Manual","unknown":true}"#
        )
        .is_err());
        assert!(serde_json::from_str::<NativeOp>(
            r#"{"op":"kill_processes","pattern":"explorer.exe","extra":1}"#
        )
        .is_err());
        assert!(serde_json::from_str::<AppxPackage>(
            r#"{"id":"a","legacy_id":"l","name":"n","category":"c","description":"d","package_id":"p","bad":1}"#
        )
        .is_err());
    }

    #[test]
    fn preset_names_parse_case_insensitively_and_reject_unknown_values() {
        assert_eq!("minimal".parse::<PresetName>().unwrap(), PresetName::Minimal);
        assert_eq!("STANDARD".parse::<PresetName>().unwrap(), PresetName::Standard);
        assert_eq!("AdVaNcEd".parse::<PresetName>().unwrap(), PresetName::Advanced);
        assert!("extreme".parse::<PresetName>().unwrap_err().contains("unknown preset 'extreme'"));
    }
}
