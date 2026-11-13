// Made by WindowCraft Dev Team
//! `autounattend.xml`, `ei.cfg` and first-boot provisioning generation.

use serde::{Deserialize, Serialize};
use windowcraft_config::{Catalog, REMOVE_ENTRY, RegKind, RegistryEntry};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerOptions {
    /// Image index pinned in `/IMAGE/INDEX`.
    pub image_index: u32,
    /// `Professional`, `Core` and so on, as written to `ei.cfg`.
    pub edition_id: String,
    pub architecture: String,
    pub bypass_requirements: bool,
    pub local_account: bool,
    pub dark_mode: bool,
    pub disable_device_encryption: bool,
    pub pause_updates_during_oobe: bool,
    /// Runs the bundled profile at first logon.
    #[serde(default)]
    pub run_profile_at_logon: bool,
}

impl Default for AnswerOptions {
    fn default() -> Self {
        Self {
            image_index: 6,
            edition_id: "Professional".to_owned(),
            architecture: "amd64".to_owned(),
            bypass_requirements: true,
            local_account: true,
            dark_mode: true,
            disable_device_encryption: true,
            pause_updates_during_oobe: true,
            run_profile_at_logon: false,
        }
    }
}

pub fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Where `$OEM$` content lands on the installed system.
const BUNDLE_DIRECTORY: &str = r"%WINDIR%\Setup\Scripts\WindowCraft";

const LAB_CONFIG: [&str; 5] =
    ["BypassTPMCheck", "BypassSecureBootCheck", "BypassCPUCheck", "BypassRAMCheck", "BypassStorageCheck"];

fn run_synchronous(commands: &[String]) -> String {
    let mut body = String::new();
    for (index, command) in commands.iter().enumerate() {
        body.push_str(&format!(
            "        <RunSynchronousCommand wcm:action=\"add\">\n          <Order>{}</Order>\n          <Path>{}</Path>\n        </RunSynchronousCommand>\n",
            index + 1,
            escape_xml(command)
        ));
    }
    format!("      <RunSynchronous>\n{body}      </RunSynchronous>\n")
}

fn component(name: &str, architecture: &str, body: &str) -> String {
    format!(
        "    <component name=\"{name}\" processorArchitecture=\"{architecture}\" publicKeyToken=\"31bf3856ad364e35\" language=\"neutral\" versionScope=\"nonSxS\">\n{body}    </component>\n"
    )
}

/// Generates the answer file. Disks are never configured here, so Setup still
/// asks where to install and a stray ISO cannot wipe a machine unattended.
pub fn autounattend(options: &AnswerOptions) -> String {
    let arch = escape_xml(&options.architecture);
    let mut pe = String::new();
    if options.bypass_requirements {
        let commands: Vec<String> = LAB_CONFIG
            .iter()
            .map(|name| format!(r#"reg add "HKLM\SYSTEM\Setup\LabConfig" /v {name} /t REG_DWORD /d 1 /f"#))
            .collect();
        pe.push_str(&component("Microsoft-Windows-Setup", &arch, &format!(
            "{}      <ImageInstall>\n        <OSImage>\n          <InstallFrom>\n            <MetaData wcm:action=\"add\">\n              <Key>/IMAGE/INDEX</Key>\n              <Value>{}</Value>\n            </MetaData>\n          </InstallFrom>\n        </OSImage>\n      </ImageInstall>\n      <UserData>\n        <AcceptEula>true</AcceptEula>\n      </UserData>\n",
            run_synchronous(&commands),
            options.image_index
        )));
    } else {
        pe.push_str(&component("Microsoft-Windows-Setup", &arch, &format!(
            "      <ImageInstall>\n        <OSImage>\n          <InstallFrom>\n            <MetaData wcm:action=\"add\">\n              <Key>/IMAGE/INDEX</Key>\n              <Value>{}</Value>\n            </MetaData>\n          </InstallFrom>\n        </OSImage>\n      </ImageInstall>\n",
            options.image_index
        )));
    }

    let mut specialize_commands = Vec::new();
    if options.local_account {
        specialize_commands.push(
            r#"reg add "HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\OOBE" /v BypassNRO /t REG_DWORD /d 1 /f"#.to_owned(),
        );
    }
    if options.disable_device_encryption {
        specialize_commands.push(
            r#"reg add "HKLM\SYSTEM\CurrentControlSet\Control\BitLocker" /v PreventDeviceEncryption /t REG_DWORD /d 1 /f"#.to_owned(),
        );
    }
    if options.pause_updates_during_oobe {
        specialize_commands.push(
            r#"reg add "HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU" /v NoAutoUpdate /t REG_DWORD /d 1 /f"#.to_owned(),
        );
    }
    let specialize = if specialize_commands.is_empty() {
        String::new()
    } else {
        format!(
            "  <settings pass=\"specialize\">\n{}  </settings>\n",
            component("Microsoft-Windows-Deployment", &arch, &run_synchronous(&specialize_commands))
        )
    };

    let theme = if options.dark_mode {
        "      <Themes>\n        <WindowColor>Automatic</WindowColor>\n        <SystemUsesLightTheme>false</SystemUsesLightTheme>\n        <UWPAppsUseLightTheme>false</UWPAppsUseLightTheme>\n      </Themes>\n"
    } else {
        ""
    };
    let accounts = if options.local_account {
        "        <HideOnlineAccountScreens>true</HideOnlineAccountScreens>\n        <HideWirelessSetupInOOBE>false</HideWirelessSetupInOOBE>\n"
    } else {
        ""
    };
    // WinGet exists only once a user has logged on, so a profile that installs
    // applications cannot run from SetupComplete, which runs as SYSTEM.
    let first_logon = if options.run_profile_at_logon {
        let apply = format!(
            r#"cmd /c ""{BUNDLE_DIRECTORY}\windowcraft.exe" --config "{BUNDLE_DIRECTORY}\profile.json" --run""#
        );
        // No uninstaller exists here, so the bundle cleans up after its one run.
        let purge = format!(r#"cmd /c ""{BUNDLE_DIRECTORY}\windowcraft.exe" --purge-data""#);
        let remove = format!(r#"cmd /c rd /s /q "{BUNDLE_DIRECTORY}""#);
        let commands: String = [("Apply WindowCraft profile", apply), ("Remove WindowCraft data", purge), ("Remove WindowCraft", remove)]
            .iter()
            .enumerate()
            .map(|(index, (description, command))| {
                format!(
                    "        <SynchronousCommand wcm:action=\"add\">\n          <Order>{}</Order>\n          <CommandLine>{}</CommandLine>\n          <Description>{description}</Description>\n        </SynchronousCommand>\n",
                    index + 1,
                    escape_xml(command)
                )
            })
            .collect();
        format!("      <FirstLogonCommands>\n{commands}      </FirstLogonCommands>\n")
    } else {
        String::new()
    };
    let oobe = component(
        "Microsoft-Windows-Shell-Setup",
        &arch,
        &format!("{theme}{first_logon}      <OOBE>\n        <HideEULAPage>true</HideEULAPage>\n{accounts}        <ProtectYourPC>3</ProtectYourPC>\n      </OOBE>\n"),
    );

    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<unattend xmlns=\"urn:schemas-microsoft-com:unattend\" xmlns:wcm=\"http://schemas.microsoft.com/WMIConfig/2002/State\">\n  <settings pass=\"windowsPE\">\n{pe}  </settings>\n{specialize}  <settings pass=\"oobeSystem\">\n{oobe}  </settings>\n</unattend>\n"
    )
}

/// `sources\ei.cfg`. Removing `PID.txt` as well stops an OEM key in firmware
/// from forcing the Home edition.
pub fn ei_cfg(edition_id: &str) -> String {
    format!("[EditionID]\r\n{edition_id}\r\n[Channel]\r\nRetail\r\n[VL]\r\n0\r\n")
}

/// AppX packages removed on first boot: the default list plus Teams.
pub fn bloat_packages(catalog: &Catalog) -> Vec<String> {
    let mut packages: Vec<String> = catalog
        .presets
        .appx_default
        .iter()
        .filter_map(|id| catalog.appx_package(id))
        .map(|package| package.package_id.clone())
        .collect();
    packages.push("MSTeams".to_owned());
    packages
}

/// Tweaks whose registry values are baked in for the first logon.
const PRIVACY_TWEAKS: [&str; 10] = [
    "telemetry",
    "activity",
    "consumer-features",
    "location",
    "delivery-optimization",
    "wpbt",
    "prevent-device-metadata-from-network",
    "edge-debloat",
    "disable-explorer-auto-discovery",
    "windows-ai",
];

/// Group Policy values with no equivalent catalog tweak.
/// AllowTelemetry comes from the telemetry tweak itself.
const POLICY_BASELINE: [(&str, &str, u32); 9] = [
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection", "DoNotShowFeedbackNotifications", 1),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\AdvertisingInfo", "DisabledByGroupPolicy", 1),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\AppCompat", "DisableInventory", 1),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\AppCompat", "AITEnable", 0),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableSoftLanding", 1),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent", "DisableCloudOptimizedContent", 1),
    (r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search", "AllowCortana", 0),
    (r"HKLM\SOFTWARE\Policies\Microsoft\SQMClient\Windows", "CEIPEnable", 0),
    (r"HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting", "Disabled", 1),
];

/// Scheduled telemetry tasks removed on first boot.
pub const TELEMETRY_TASKS: [&str; 7] = [
    r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
    r"\Microsoft\Windows\Customer Experience Improvement Program\UsbCeip",
    r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser",
    r"\Microsoft\Windows\Application Experience\ProgramDataUpdater",
    r"\Microsoft\Windows\Autochk\Proxy",
    r"\Microsoft\Windows\DiskDiagnostic\Microsoft-Windows-DiskDiagnosticDataCollector",
    r"\Microsoft\Windows\WaaSMedic\PerformRemediation",
];

fn reg_add(entry: &RegistryEntry) -> Option<String> {
    let value = entry.value.as_deref()?;
    if value == REMOVE_ENTRY || value.starts_with('{') {
        return None;
    }
    let kind = match entry.kind {
        RegKind::DWord => "REG_DWORD",
        RegKind::QWord => "REG_QWORD",
        RegKind::String => "REG_SZ",
        RegKind::Binary => "REG_BINARY",
    };
    let name = if entry.name.is_empty() { "/ve".to_owned() } else { format!("/v \"{}\"", entry.name) };
    Some(format!("reg add \"{}\" {name} /t {kind} /d \"{value}\" /f", entry.path))
}

/// Registry commands for the privacy baseline.
pub fn privacy_commands(catalog: &Catalog) -> Vec<String> {
    let mut commands: Vec<String> = PRIVACY_TWEAKS
        .iter()
        .filter_map(|id| catalog.tweak(id))
        .flat_map(|tweak| tweak.registry.iter())
        .filter_map(reg_add)
        .collect();
    for (path, name, value) in POLICY_BASELINE {
        commands.push(format!("reg add \"{path}\" /v \"{name}\" /t REG_DWORD /d \"{value}\" /f"));
    }
    commands
}

/// First-boot script staged at `sources\$OEM$\$$\Setup\Scripts\SetupComplete.cmd`.
///
/// Per-user values go into the Default profile hive, which every new account
/// is copied from. `HKU\.DEFAULT` would be wrong: that is the logon screen's
/// profile and no user ever inherits it.
pub fn setup_complete(catalog: &Catalog, options: &AnswerOptions) -> String {
    let mut script = String::from("@echo off\r\nrem Generated by WindowCraft ISO Studio.\r\n");
    for package in bloat_packages(catalog) {
        script.push_str(&format!(
            "powershell -NoProfile -Command \"Get-AppxProvisionedPackage -Online | Where-Object DisplayName -like '{package}*' | Remove-AppxProvisionedPackage -Online\"\r\n"
        ));
    }
    script.push_str("if exist \"%SystemRoot%\\System32\\OneDriveSetup.exe\" \"%SystemRoot%\\System32\\OneDriveSetup.exe\" /uninstall\r\n");
    let (per_user, machine): (Vec<String>, Vec<String>) =
        privacy_commands(catalog).into_iter().partition(|command| command.contains("\"HKCU\\"));
    for command in machine {
        script.push_str(&command);
        script.push_str("\r\n");
    }
    script.push_str("reg load \"HKLM\\WC_DEFAULT\" \"%SystemDrive%\\Users\\Default\\NTUSER.DAT\"\r\n");
    for command in per_user {
        script.push_str(&command.replace("\"HKCU\\", "\"HKLM\\WC_DEFAULT\\"));
        script.push_str("\r\n");
    }
    // Windows 11 26H2 ignores the <Themes> values in the answer file, so dark
    // mode also goes into the hive every new account is copied from.
    if options.dark_mode {
        for name in ["AppsUseLightTheme", "SystemUsesLightTheme"] {
            script.push_str(&format!(
                "reg add \"HKLM\\WC_DEFAULT\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\" /v {name} /t REG_DWORD /d 0 /f\r\n"
            ));
        }
    }
    script.push_str("reg unload \"HKLM\\WC_DEFAULT\"\r\n");
    for task in TELEMETRY_TASKS {
        script.push_str(&format!("schtasks /Change /TN \"{task}\" /Disable\r\n"));
    }
    script.push_str("reg delete \"HKLM\\SOFTWARE\\Policies\\Microsoft\\Windows\\WindowsUpdate\\AU\" /v NoAutoUpdate /f\r\n");
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::load().unwrap()
    }

    #[test]
    fn answer_file_pins_the_image_and_bypasses_checks() {
        let xml = autounattend(&AnswerOptions { image_index: 6, ..AnswerOptions::default() });
        assert!(xml.contains("<Key>/IMAGE/INDEX</Key>"));
        assert!(xml.contains("<Value>6</Value>"));
        for name in LAB_CONFIG {
            assert!(xml.contains(name), "{name}");
        }
        assert!(xml.contains("BypassNRO"));
        assert!(xml.contains("<SystemUsesLightTheme>false</SystemUsesLightTheme>"));
        assert!(xml.contains("PreventDeviceEncryption"));
    }

    #[test]
    fn answer_file_never_configures_disks() {
        let xml = autounattend(&AnswerOptions::default());
        assert!(!xml.contains("DiskConfiguration"), "Setup must always ask where to install");
        assert!(!xml.contains("WillWipeDisk"));
    }

    #[test]
    fn options_can_turn_every_change_off() {
        let xml = autounattend(&AnswerOptions {
            bypass_requirements: false,
            local_account: false,
            dark_mode: false,
            disable_device_encryption: false,
            pause_updates_during_oobe: false,
            run_profile_at_logon: false,
            ..AnswerOptions::default()
        });
        assert!(!xml.contains("LabConfig"));
        assert!(!xml.contains("BypassNRO"));
        assert!(!xml.contains("Themes"));
        assert!(!xml.contains("pass=\"specialize\""));
        assert!(xml.contains("/IMAGE/INDEX"));
    }

    #[test]
    fn output_is_balanced_xml() {
        let xml = autounattend(&AnswerOptions::default());
        for element in ["unattend", "settings", "component", "RunSynchronous", "OOBE", "ImageInstall"] {
            let open = xml.matches(&format!("<{element}>")).count() + xml.matches(&format!("<{element} ")).count();
            let close = xml.matches(&format!("</{element}>")).count();
            assert_eq!(open, close, "{element}");
        }
        assert!(xml.contains("&quot;"), "quotes in commands must be escaped");
    }

    #[test]
    fn hostile_values_are_escaped() {
        assert_eq!(escape_xml("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
        let xml = autounattend(&AnswerOptions { architecture: "x\"><evil".to_owned(), ..AnswerOptions::default() });
        assert!(!xml.contains("<evil"));
    }

    #[test]
    fn edition_config_is_retail() {
        let cfg = ei_cfg("Professional");
        assert!(cfg.starts_with("[EditionID]\r\nProfessional\r\n"));
        assert!(cfg.contains("[Channel]\r\nRetail"));
    }

    #[test]
    fn removes_the_default_packages_and_teams() {
        let packages = bloat_packages(&catalog());
        assert_eq!(packages.len(), catalog().presets.appx_default.len() + 1);
        assert!(packages.iter().any(|package| package == "MSTeams"));
    }

    #[test]
    fn privacy_baseline_has_fifty_values() {
        let commands = privacy_commands(&catalog());
        assert!(commands.len() >= 50, "only {}", commands.len());
        assert!(commands.iter().any(|command| command.contains("AllowTelemetry")));
        assert!(commands.iter().all(|command| !command.contains("<RemoveEntry>")));
    }

    #[test]
    fn bundled_profiles_run_at_first_logon_with_escaped_quotes() {
        let xml = autounattend(&AnswerOptions { run_profile_at_logon: true, ..AnswerOptions::default() });
        assert!(xml.contains("<FirstLogonCommands>"));
        let apply = xml.find("windowcraft.exe&quot; --config").unwrap();
        let purge = xml.find("windowcraft.exe&quot; --purge-data").unwrap();
        let remove = xml.find("rd /s /q").unwrap();
        assert!(apply < purge && purge < remove, "the bundle must run before it removes itself");
        assert!(!autounattend(&AnswerOptions::default()).contains("FirstLogonCommands"));
    }

    #[test]
    fn first_boot_script_covers_every_stage() {
        let catalog = catalog();
        let script = setup_complete(&catalog, &AnswerOptions::default());
        assert!(script.contains("OneDriveSetup.exe\" /uninstall"));
        assert!(script.contains("Customer Experience Improvement Program"));
        assert!(script.contains("Compatibility Appraiser"));
        assert!(script.contains("WaaSMedic"));
        assert!(!script.contains("windowcraft.exe"), "WinGet is unavailable while this runs as SYSTEM");
        assert!(script.contains("reg delete \"HKLM\\SOFTWARE\\Policies\\Microsoft\\Windows\\WindowsUpdate\\AU\""));
    }

    #[test]
    fn dark_mode_reaches_the_default_profile() {
        let dark = setup_complete(&catalog(), &AnswerOptions { dark_mode: true, ..AnswerOptions::default() });
        let unload = dark.find("reg unload").unwrap();
        let theme = dark.find(r#"Themes\Personalize" /v AppsUseLightTheme"#).expect("dark mode in the Default hive");
        assert!(theme < unload);
        assert!(dark.contains("/v SystemUsesLightTheme /t REG_DWORD /d 0"));
        let light = setup_complete(&catalog(), &AnswerOptions { dark_mode: false, ..AnswerOptions::default() });
        assert!(!light.contains("AppsUseLightTheme"));
    }

    #[test]
    fn per_user_values_target_the_default_profile() {
        let script = setup_complete(&catalog(), &AnswerOptions::default());
        assert!(!script.contains("\"HKCU\\"), "HKCU here would be SYSTEM's own hive");
        assert!(!script.contains(".DEFAULT"), "nobody inherits the logon screen's profile");
        let load = script.find("reg load").unwrap();
        let write = script.find("HKLM\\WC_DEFAULT\\").unwrap();
        let unload = script.find("reg unload").unwrap();
        assert!(load < write && write < unload, "the hive must be mounted around its writes");
    }
}
