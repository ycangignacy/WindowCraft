// Made by ycangignacy
//! WindowCraft profiles and legacy WinUtil JSON compatibility.

use crate::catalog::Catalog;
use crate::schema::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

pub const PROFILE_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("the file is not valid JSON: {0}")]
    Malformed(String),
    #[error("the profile does not match the expected shape: {0}")]
    Shape(String),
    #[error("unknown key(s) in WinUtil config: {0}")]
    UnknownKeys(String),
    #[error("unknown {kind} '{id}'")]
    UnknownId { kind: &'static str, id: String },
    #[error("profile version {0} is newer than this build supports")]
    Version(u32),
    #[error("tweaks '{0}' and '{1}' exclude each other; keep one of them")]
    Exclusive(String, String),
    #[error("the profile has no \"version\" field; add \"version\": 1 at the top")]
    MissingVersion,
}

/// Parses JSON written by any editor. PowerShell 5.1 and older Notepad start
/// UTF-8 files with a byte order mark, which serde_json does not accept.
pub fn parse_json(text: &str) -> serde_json::Result<Value> {
    serde_json::from_str(text.strip_prefix('\u{FEFF}').unwrap_or(text))
}

/// Extended WindowCraft configuration. Every field is optional except the
/// version so hand-written profiles stay short.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tweaks: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub toggles: BTreeMap<String, bool>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub segmented: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub appx_remove: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_mode: Option<String>,
    /// Extra service rules. The engine still enforces its protected list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceRule>,
}

/// Ids checked against the catalog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedProfile {
    pub profile: Profile,
}

impl Profile {
    pub fn new() -> Self {
        Self { version: PROFILE_VERSION, ..Self::default() }
    }

    pub fn parse(source: &str) -> Result<Self, ProfileError> {
        let value = parse_json(source).map_err(|e| ProfileError::Malformed(e.to_string()))?;
        Self::from_value(value)
    }

    fn from_value(value: Value) -> Result<Self, ProfileError> {
        let profile: Self = serde_json::from_value(value).map_err(|e| ProfileError::Shape(e.to_string()))?;
        if profile.version > PROFILE_VERSION {
            return Err(ProfileError::Version(profile.version));
        }
        Ok(profile)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// Confirms every reference exists before anything touches the system.
    pub fn resolve(self, catalog: &Catalog) -> Result<ResolvedProfile, ProfileError> {
        let check = |known: bool, kind: &'static str, id: &str| {
            known.then_some(()).ok_or_else(|| ProfileError::UnknownId { kind, id: id.to_owned() })
        };
        for id in self.tweaks.iter().chain(self.toggles.keys()).chain(self.segmented.keys()) {
            check(catalog.tweak(id).is_some(), "tweak", id)?;
        }
        for (position, id) in self.tweaks.iter().enumerate() {
            if let Some(other) = self.tweaks[position + 1..].iter().find(|other| catalog.exclusive_peers(id).any(|peer| peer == *other)) {
                return Err(ProfileError::Exclusive(id.clone(), other.clone()));
            }
        }
        for (id, option) in &self.segmented {
            let known = catalog.tweak(id).is_some_and(|tweak| tweak.options.iter().any(|o| &o.label == option));
            check(known, "option", option)?;
        }
        for id in &self.features {
            check(catalog.feature(id).is_some(), "feature", id)?;
        }
        for id in &self.apps {
            check(catalog.app(id).is_some(), "application", id)?;
        }
        for id in &self.appx_remove {
            check(catalog.appx_package(id).is_some(), "AppX package", id)?;
        }
        if let Some(mode) = &self.update_mode {
            check(catalog.update_mode(mode).is_some(), "update mode", mode)?;
        }
        if let Some(dns) = &self.dns {
            let known = matches!(dns.as_str(), "Default" | "DHCP" | "Fastest") || catalog.dns_provider(dns).is_some();
            check(known, "DNS provider", dns)?;
        }
        Ok(ResolvedProfile { profile: self })
    }
}

/// Keys WinUtil writes in exported configs.
const WINUTIL_KEYS: [&str; 7] = ["WPFTweaks", "WPFFeature", "WPFInstall", "Install", "WPFToggle", "WPFAppx", "WPFAppxPackages"];

fn strings(value: &Value) -> Vec<String> {
    value.as_array().map_or_else(Vec::new, |items| {
        items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect()
    })
}

/// Converts a legacy WinUtil config, mapping ids through `legacy_id`.
fn from_winutil(map: &serde_json::Map<String, Value>, catalog: &Catalog) -> Result<Profile, ProfileError> {
    let unknown: Vec<&str> =
        map.keys().map(String::as_str).filter(|key| !WINUTIL_KEYS.contains(key)).collect();
    if !unknown.is_empty() {
        return Err(ProfileError::UnknownKeys(unknown.join(", ")));
    }

    let mut profile = Profile::new();
    for legacy in map.get("WPFTweaks").map(strings).unwrap_or_default() {
        let tweak = catalog
            .tweak_by_legacy(&legacy)
            .ok_or_else(|| ProfileError::UnknownId { kind: "tweak", id: legacy.clone() })?;
        match tweak.kind {
            TweakKind::Toggle => {
                profile.toggles.insert(tweak.id.clone(), true);
            }
            _ => profile.tweaks.push(tweak.id.clone()),
        }
    }
    for legacy in map.get("WPFToggle").map(strings).unwrap_or_default() {
        let tweak = catalog
            .tweak_by_legacy(&legacy)
            .ok_or_else(|| ProfileError::UnknownId { kind: "toggle", id: legacy.clone() })?;
        profile.toggles.insert(tweak.id.clone(), true);
    }
    for legacy in map.get("WPFFeature").map(strings).unwrap_or_default() {
        let feature = catalog
            .features
            .iter()
            .find(|feature| feature.legacy_id == legacy)
            .ok_or_else(|| ProfileError::UnknownId { kind: "feature", id: legacy.clone() })?;
        profile.features.push(feature.id.clone());
    }

    // `WPFInstall` holds WPFInstall<id> names; `Install` holds package objects.
    for name in map.get("WPFInstall").map(strings).unwrap_or_default() {
        let id = name.strip_prefix("WPFInstall").unwrap_or(&name);
        let app = catalog
            .app(id)
            .ok_or_else(|| ProfileError::UnknownId { kind: "application", id: id.to_owned() })?;
        profile.apps.push(app.id.clone());
    }
    for item in map.get("Install").and_then(Value::as_array).into_iter().flatten() {
        let winget = item.get("winget").and_then(Value::as_str);
        let choco = item.get("choco").and_then(Value::as_str);
        let found = catalog.apps.iter().find(|app| {
            (winget.is_some() && app.winget.as_deref() == winget) || (choco.is_some() && app.choco.as_deref() == choco)
        });
        if let Some(app) = found.filter(|app| !profile.apps.contains(&app.id)) {
            profile.apps.push(app.id.clone());
        }
    }
    for key in ["WPFAppx", "WPFAppxPackages"] {
        for legacy in map.get(key).map(strings).unwrap_or_default() {
            let package = catalog
                .appx
                .iter()
                .find(|package| package.legacy_id == legacy)
                .ok_or_else(|| ProfileError::UnknownId { kind: "AppX package", id: legacy.clone() })?;
            profile.appx_remove.push(package.id.clone());
        }
    }
    Ok(profile)
}

/// Accepts either a WindowCraft profile or a legacy WinUtil config.
pub fn import_any(source: &str, catalog: &Catalog) -> Result<ResolvedProfile, ProfileError> {
    let value = parse_json(source).map_err(|e| ProfileError::Malformed(e.to_string()))?;
    let Value::Object(map) = &value else {
        return Err(ProfileError::Shape("the top level must be an object".to_owned()));
    };
    // Keys of a WindowCraft profile, so one that only lacks its version is not
    // read as a WinUtil config.
    const PROFILE_KEYS: [&str; 9] =
        ["tweaks", "toggles", "segmented", "features", "apps", "appx_remove", "dns", "update_mode", "services"];
    let profile = if map.contains_key("version") {
        Profile::from_value(value)?
    } else if map.keys().any(|key| PROFILE_KEYS.contains(&key.as_str())) {
        return Err(ProfileError::MissingVersion);
    } else {
        from_winutil(map, catalog)?
    };
    profile.resolve(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::load().unwrap()
    }

    #[test]
    fn roundtrips_a_populated_profile_through_json_and_resolves() {
        let catalog = catalog();
        let mut profile = Profile::new();
        profile.tweaks = vec!["activity".to_owned(), "telemetry".to_owned()];
        profile.toggles.insert("dark-mode".to_owned(), true);
        profile.segmented.insert("multiplane-overlay".to_owned(), "Fully Disabled".to_owned());
        profile.features = vec!["wsl".to_owned()];
        profile.apps = vec!["7zip".to_owned()];
        profile.appx_remove = vec!["microsoft-windows-feedback-hub".to_owned()];
        profile.dns = Some("Cloudflare".to_owned());
        profile.update_mode = Some("security".to_owned());
        profile.services = vec![ServiceRule {
            name: "DiagTrack".to_owned(),
            startup: StartupType::Disabled,
            original: StartupType::Automatic,
            optional: true,
        }];

        let json = profile.to_json();
        let parsed = Profile::parse(&json).unwrap();
        assert_eq!(parsed, profile);

        let resolved = import_any(&json, &catalog).unwrap();
        assert_eq!(resolved.profile, profile);

        let minimal = Profile::new().to_json();
        assert_eq!(minimal, "{\n  \"version\": 1\n}");
    }

    #[test]
    fn imports_legacy_winutil_json_and_deduplicates_applications() {
        let catalog = catalog();
        let winutil = r#"{
            "WPFTweaks": ["WPFTweaksActivity", "WPFToggleDarkMode"],
            "WPFToggle": ["WPFToggleShowExt"],
            "WPFFeature": ["WPFFeaturewsl"],
            "WPFInstall": ["WPFInstall7zip", "bitwarden"],
            "Install": [
                {"winget": "7zip.7zip"},
                {"choco": "audacity"},
                {"winget": "NoSuch.Package"}
            ],
            "WPFAppx": ["WPFAppxMicrosoft_WindowsFeedbackHub"],
            "WPFAppxPackages": ["WPFAppxMSTeams"]
        }"#;

        let resolved = import_any(winutil, &catalog).unwrap().profile;
        assert_eq!(resolved.version, PROFILE_VERSION);
        assert_eq!(resolved.tweaks, ["activity"]);
        assert_eq!(resolved.toggles.get("dark-mode"), Some(&true));
        assert_eq!(resolved.toggles.get("show-ext"), Some(&true));
        assert_eq!(resolved.features, ["wsl"]);
        assert_eq!(resolved.apps, ["7zip", "bitwarden", "audacity"]);
        assert_eq!(resolved.appx_remove, ["microsoft-windows-feedback-hub", "msteams"]);
    }

    #[test]
    fn rejects_malformed_json_bad_shapes_unknown_keys_and_future_versions() {
        let catalog = catalog();
        assert!(matches!(Profile::parse("{not json"), Err(ProfileError::Malformed(_))));
        assert!(matches!(import_any("{not json", &catalog), Err(ProfileError::Malformed(_))));
        assert!(matches!(import_any("[]", &catalog), Err(ProfileError::Shape(_))));
        assert!(matches!(import_any("\"text\"", &catalog), Err(ProfileError::Shape(_))));
        assert!(matches!(Profile::parse(r#"{"version":1,"bogus":true}"#), Err(ProfileError::Shape(_))));
        assert!(matches!(Profile::parse(r#"{"version":"one"}"#), Err(ProfileError::Shape(_))));
        assert!(matches!(Profile::parse(r#"{"version":99}"#), Err(ProfileError::Version(99))));
        assert!(matches!(import_any(r#"{"version":2}"#, &catalog), Err(ProfileError::Version(2))));
        assert!(matches!(
            import_any(r#"{"WPFTweaks":[],"ExtraKey":[]}"#, &catalog),
            Err(ProfileError::UnknownKeys(keys)) if keys == "ExtraKey"
        ));
    }

    #[test]
    fn rejects_unknown_catalog_references_in_profiles_and_legacy_imports() {
        let catalog = catalog();
        let cases = [
            (r#"{"version":1,"tweaks":["no-such-tweak"]}"#, "tweak", "no-such-tweak"),
            (r#"{"version":1,"toggles":{"no-such-toggle":true}}"#, "tweak", "no-such-toggle"),
            (r#"{"version":1,"segmented":{"no-such-seg":"Enabled"}}"#, "tweak", "no-such-seg"),
            (r#"{"version":1,"segmented":{"multiplane-overlay":"Bogus"}}"#, "option", "Bogus"),
            (r#"{"version":1,"features":["no-such-feature"]}"#, "feature", "no-such-feature"),
            (r#"{"version":1,"apps":["no-such-app"]}"#, "application", "no-such-app"),
            (r#"{"version":1,"appx_remove":["no-such-appx"]}"#, "AppX package", "no-such-appx"),
            (r#"{"version":1,"update_mode":"turbo"}"#, "update mode", "turbo"),
            (r#"{"version":1,"dns":"BogusDns"}"#, "DNS provider", "BogusDns"),
            (r#"{"WPFTweaks":["WPFNoSuchTweak"]}"#, "tweak", "WPFNoSuchTweak"),
            (r#"{"WPFToggle":["WPFNoSuchToggle"]}"#, "toggle", "WPFNoSuchToggle"),
            (r#"{"WPFFeature":["WPFNoSuchFeature"]}"#, "feature", "WPFNoSuchFeature"),
            (r#"{"WPFInstall":["WPFInstallNoSuchApp"]}"#, "application", "NoSuchApp"),
            (r#"{"WPFAppx":["WPFAppxNoSuch"]}"#, "AppX package", "WPFAppxNoSuch"),
        ];
        for (source, expected_kind, expected_id) in cases {
            match import_any(source, &catalog) {
                Err(ProfileError::UnknownId { kind, id }) => {
                    assert_eq!((kind, id.as_str()), (expected_kind, expected_id), "for {source}");
                }
                other => panic!("expected UnknownId({expected_kind}, {expected_id}) for {source}, got {other:?}"),
            }
        }

        for builtin_dns in ["Default", "DHCP", "Fastest"] {
            let mut profile = Profile::new();
            profile.dns = Some(builtin_dns.to_owned());
            assert!(profile.resolve(&catalog).is_ok(), "{builtin_dns} must resolve");
        }
    }
}
