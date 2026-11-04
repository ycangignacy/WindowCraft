// Made by ycangignacy
//! Privacy-safe environment report.
//!
//! Only hardware class, OS build, tool versions and boolean tweak states go in.
//! Names, paths, serial numbers and addresses are excluded by construction,
//! because the struct has no field that could hold them.

use crate::error::Result;
use crate::executor::{Detection, Engine};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Serialize)]
pub struct EnvironmentReport {
    pub generator: String,
    pub os_product: String,
    pub os_display_version: String,
    pub os_build: String,
    pub architecture: String,
    pub ram_gb: u64,
    pub logical_processors: u32,
    pub winget_present: bool,
    pub chocolatey_present: bool,
    pub tweaks: BTreeMap<String, bool>,
}

pub fn build(engine: &Engine) -> Result<EnvironmentReport> {
    let info = crate::sysinfo::query()?;
    let mut tweaks = BTreeMap::new();
    for tweak in &engine.catalog.tweaks {
        if let Ok(detection) = engine.detect(tweak)
            && detection != Detection::Unknown
        {
            tweaks.insert(tweak.id.clone(), detection == Detection::Applied);
        }
    }
    let os_build = info.full_build();
    Ok(EnvironmentReport {
        generator: concat!("WindowCraft ", env!("CARGO_PKG_VERSION")).to_owned(),
        os_product: info.product,
        os_display_version: info.display_version,
        os_build,
        architecture: info.architecture.to_owned(),
        // Rounded, because usable memory in a VM comes in just under the GB.
        ram_gb: (info.ram_kb + 512 * 1024) / (1024 * 1024),
        logical_processors: info.logical_processors,
        winget_present: crate::packages::winget_path().is_some(),
        chocolatey_present: crate::packages::choco_path().is_some(),
        tweaks,
    })
}

pub fn to_json(report: &EnvironmentReport) -> Result<String> {
    Ok(serde_json::to_string_pretty(report)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{RecordingRunner, TweakAction};
    use crate::jobs::CancelToken;
    use crate::services::MemoryServices;
    use crate::snapshot::SnapshotStore;
    use crate::testing::Sandbox;
    use std::sync::Arc;
    use windowcraft_config::Catalog;

    #[test]
    fn report_includes_only_whitelisted_fields_and_excludes_user_or_machine_identifiers() {
        let sandbox = Sandbox::new("reportprivacy");
        let dir = tempfile::tempdir().unwrap();
        let catalog = Arc::new(Catalog::load().unwrap());
        let engine = Engine {
            scope: sandbox.scope.clone(),
            services: Arc::new(MemoryServices::default()),
            native: Arc::new(RecordingRunner::default()),
            store: Arc::new(SnapshotStore::open(&dir.path().join("r.redb")).unwrap()),
            catalog: catalog.clone(),
            ram_kb: 16 * 1024 * 1024,
            os_build: "26100.1".to_owned(),
        };

        let activity = catalog.tweak("activity").unwrap();
        let session = engine.store.begin_session("test").unwrap();
        engine.apply_tweak(session, activity, &TweakAction::Apply, &CancelToken::new()).unwrap();

        let built = build(&engine).unwrap();
        assert_eq!(built.tweaks.get("activity"), Some(&true));
        assert_eq!(built.tweaks.get("wpbt"), Some(&false));
        assert!(
            !built.tweaks.contains_key("disk-cleanup"),
            "undetectable action-only tweaks must be omitted from the report"
        );

        let json = to_json(&built).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let object = parsed.as_object().unwrap();

        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "architecture",
                "chocolatey_present",
                "generator",
                "logical_processors",
                "os_build",
                "os_display_version",
                "os_product",
                "ram_gb",
                "tweaks",
                "winget_present",
            ]
        );
        for forbidden in ["username", "user", "computername", "hostname", "machine", "serial", "path", "ip"] {
            assert!(!object.contains_key(forbidden), "leaked field {forbidden}");
        }
        if let Ok(profile_dir) = std::env::var("USERPROFILE")
            && !profile_dir.is_empty()
        {
            assert!(!json.contains(&profile_dir), "report must not contain the user profile path");
        }
    }
}
