// Made by ycangignacy
//! Embedded catalogs, parsed once at startup.

use crate::schema::*;
use serde::de::DeserializeOwned;

const APPLICATIONS: &str = include_str!("../catalog/applications.json");
const TWEAKS: &str = include_str!("../catalog/tweaks.json");
const FEATURES: &str = include_str!("../catalog/feature.json");
const APPX: &str = include_str!("../catalog/appx.json");
const DNS: &str = include_str!("../catalog/dns.json");
const PRESETS: &str = include_str!("../catalog/preset.json");
const UPDATES: &str = include_str!("../catalog/updates.json");
/// WinUtil ids for importing configs exported by WinUtil.
const WINUTIL_IMPORT: &str = include_str!("../catalog/winutil-import.json");

#[derive(serde::Deserialize)]
struct WinUtilIds {
    tweaks: std::collections::BTreeMap<String, String>,
    features: std::collections::BTreeMap<String, String>,
    appx: std::collections::BTreeMap<String, String>,
}

/// Copies each WinUtil id onto the item it names. An id for an item that does
/// not exist is a broken table, so it fails the load.
fn attach_ids<'a>(
    table: &str,
    ids: &std::collections::BTreeMap<String, String>,
    items: impl Iterator<Item = (&'a str, &'a mut String)>,
) -> Result<(), String> {
    let mut found = 0;
    for (id, legacy) in items {
        if let Some(value) = ids.get(id) {
            legacy.clone_from(value);
            found += 1;
        }
    }
    if found == ids.len() {
        Ok(())
    } else {
        Err(format!("winutil-import.json: {} of the {table} ids name no catalog item", ids.len() - found))
    }
}

#[derive(Debug, Clone)]
pub struct Catalog {
    pub apps: Vec<App>,
    pub tweaks: Vec<Tweak>,
    pub features: Vec<Feature>,
    pub appx: Vec<AppxPackage>,
    pub dns: Vec<DnsProvider>,
    pub presets: Presets,
    pub updates: Vec<UpdateMode>,
}

fn parse<T: DeserializeOwned>(name: &str, source: &str) -> Result<T, String> {
    serde_json::from_str(source).map_err(|error| format!("{name}: {error}"))
}

impl Catalog {
    pub fn load() -> Result<Self, String> {
        let mut catalog = Self {
            apps: parse("applications.json", APPLICATIONS)?,
            tweaks: parse("tweaks.json", TWEAKS)?,
            features: parse("feature.json", FEATURES)?,
            appx: parse("appx.json", APPX)?,
            dns: parse("dns.json", DNS)?,
            presets: parse("preset.json", PRESETS)?,
            updates: parse("updates.json", UPDATES)?,
        };
        let ids: WinUtilIds = parse("winutil-import.json", WINUTIL_IMPORT)?;
        attach_ids("tweak", &ids.tweaks, catalog.tweaks.iter_mut().map(|t| (t.id.as_str(), &mut t.legacy_id)))?;
        attach_ids("feature", &ids.features, catalog.features.iter_mut().map(|f| (f.id.as_str(), &mut f.legacy_id)))?;
        attach_ids("appx", &ids.appx, catalog.appx.iter_mut().map(|a| (a.id.as_str(), &mut a.legacy_id)))?;
        Ok(catalog)
    }

    pub fn tweak(&self, id: &str) -> Option<&Tweak> {
        self.tweaks.iter().find(|tweak| tweak.id == id)
    }

    pub fn tweak_by_legacy(&self, legacy_id: &str) -> Option<&Tweak> {
        self.tweaks.iter().find(|tweak| tweak.legacy_id == legacy_id)
    }

    pub fn feature(&self, id: &str) -> Option<&Feature> {
        self.features.iter().find(|feature| feature.id == id)
    }

    pub fn app(&self, id: &str) -> Option<&App> {
        self.apps.iter().find(|app| app.id == id)
    }

    pub fn appx_package(&self, id: &str) -> Option<&AppxPackage> {
        self.appx.iter().find(|package| package.id == id)
    }

    pub fn dns_provider(&self, id: &str) -> Option<&DnsProvider> {
        self.dns.iter().find(|provider| provider.id == id)
    }

    pub fn update_mode(&self, id: &str) -> Option<&UpdateMode> {
        self.updates.iter().find(|mode| mode.id == id)
    }

    pub fn preset(&self, name: PresetName) -> &[String] {
        match name {
            PresetName::Minimal => &self.presets.minimal,
            PresetName::Standard => &self.presets.standard,
            PresetName::Advanced => &self.presets.advanced,
        }
    }

    /// Application categories in display order.
    pub fn app_categories(&self) -> Vec<&str> {
        let mut categories: Vec<&str> = self.apps.iter().map(|app| app.category.as_str()).collect();
        categories.sort_unstable();
        categories.dedup();
        categories
    }

    pub fn appx_categories(&self) -> Vec<&str> {
        let mut categories = Vec::new();
        for package in &self.appx {
            if !categories.contains(&package.category.as_str()) {
                categories.push(package.category.as_str());
            }
        }
        categories
    }
}
