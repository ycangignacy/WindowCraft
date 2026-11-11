// Made by ycangignacy
//! Structural validation of catalogs. Only tests run this over the embedded data.

use crate::catalog::Catalog;
use crate::schema::*;
use std::collections::HashSet;

const HIVES: [&str; 4] = ["HKLM\\", "HKCU\\", "HKU\\", "HKCR\\"];

/// Checks one registry entry. Returns a description of the first problem.
pub fn check_registry_entry(owner: &str, entry: &RegistryEntry) -> Result<(), String> {
    if entry.path.contains(':') {
        return Err(format!("{owner}: PowerShell-style path '{}'", entry.path));
    }
    if !HIVES.iter().any(|hive| entry.path.starts_with(hive)) {
        return Err(format!("{owner}: bad hive in '{}'", entry.path));
    }
    if entry.path.ends_with('\\') || entry.path.contains("\\\\") {
        return Err(format!("{owner}: malformed path '{}'", entry.path));
    }

    let mut values: Vec<&str> = entry.value.iter().chain(entry.original.iter()).map(String::as_str).collect();
    if let Some(per_option) = &entry.values {
        values.extend(per_option.values().map(String::as_str));
    }
    if values.is_empty() {
        return Err(format!("{owner}: '{}' has no value", entry.name));
    }
    for value in values {
        check_value(owner, entry, value)?;
    }
    Ok(())
}

fn check_value(owner: &str, entry: &RegistryEntry, value: &str) -> Result<(), String> {
    if value == REMOVE_ENTRY || value == RAM_KB_TOKEN {
        return Ok(());
    }
    let valid = match entry.kind {
        RegKind::DWord => value.parse::<u32>().is_ok(),
        RegKind::QWord => value.parse::<u64>().is_ok(),
        RegKind::Binary => {
            value.len().is_multiple_of(2) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }
        RegKind::String => true,
    };
    if valid {
        Ok(())
    } else {
        Err(format!("{owner}: '{value}' is not a valid {:?} for {}", entry.kind, entry.name))
    }
}

/// A downloaded list must name one commit and carry its hash, so the
/// content cannot change after review.
fn check_pinned(url: &str, sha256: &str) -> Result<(), String> {
    let is_hex = |text: &str, len: usize| text.len() == len && text.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !url.starts_with("https://") || !url.split('/').any(|segment| is_hex(segment, 40)) {
        return Err(format!("'{url}' is not pinned to a commit"));
    }
    if !is_hex(sha256, 64) {
        return Err(format!("bad sha256 '{sha256}'"));
    }
    Ok(())
}

fn check_ids<'a>(label: &str, ids: impl Iterator<Item = &'a str>, problems: &mut Vec<String>) {
    let mut seen = HashSet::new();
    for id in ids {
        if id.is_empty() {
            problems.push(format!("{label}: empty id"));
        } else if !seen.insert(id) {
            problems.push(format!("{label}: duplicate id '{id}'"));
        }
    }
}

fn check_refs(label: &str, ids: &[String], known: &HashSet<&str>, problems: &mut Vec<String>) {
    for id in ids {
        if !known.contains(id.as_str()) {
            problems.push(format!("preset {label}: unknown reference '{id}'"));
        }
    }
}

impl Catalog {
    /// Returns every problem found; an empty list means the catalog is sound.
    ///
    /// `Catalog::load` does not call this. The catalog is compiled into the
    /// binary, and `embedded_catalog_passes_every_validation_rule` and
    /// `tests/catalog_integrity.rs` run it on every build, so a broken entry
    /// fails the tests before it can ship.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();

        check_ids("applications", self.apps.iter().map(|app| app.id.as_str()), &mut problems);
        check_ids("tweaks", self.tweaks.iter().map(|tweak| tweak.id.as_str()), &mut problems);
        check_ids("tweaks.legacy", self.tweaks.iter().map(|tweak| tweak.legacy_id.as_str()), &mut problems);
        check_ids("features", self.features.iter().map(|feature| feature.id.as_str()), &mut problems);
        check_ids("appx", self.appx.iter().map(|package| package.id.as_str()), &mut problems);
        check_ids("dns", self.dns.iter().map(|provider| provider.id.as_str()), &mut problems);

        for app in &self.apps {
            if app.winget.is_none() && app.choco.is_none() {
                problems.push(format!("application {}: no package source", app.id));
            }
            if !app.link.starts_with("http") {
                problems.push(format!("application {}: bad link '{}'", app.id, app.link));
            }
        }

        for tweak in &self.tweaks {
            for entry in &tweak.registry {
                if let Err(problem) = check_registry_entry(&tweak.id, entry) {
                    problems.push(problem);
                }
                if let Some(values) = &entry.values {
                    for option in &tweak.options {
                        if !values.contains_key(&option.label) {
                            problems.push(format!("{}: no value for option '{}'", tweak.id, option.label));
                        }
                    }
                }
            }
            if matches!(tweak.kind, TweakKind::Segmented | TweakKind::Dns) && tweak.options.is_empty() {
                problems.push(format!("{}: choice control without options", tweak.id));
            }
            if tweak.kind == TweakKind::Check
                && tweak.registry.is_empty()
                && tweak.services.is_empty()
                && tweak.apply.is_empty()
            {
                problems.push(format!("{}: does nothing", tweak.id));
            }
            for op in &tweak.apply {
                if let NativeOp::HostsBlock { url, sha256, .. } = op
                    && let Err(problem) = check_pinned(url, sha256)
                {
                    problems.push(format!("{}: {problem}", tweak.id));
                }
            }
        }

        for feature in &self.features {
            for entry in &feature.registry {
                if let Err(problem) = check_registry_entry(&feature.id, entry) {
                    problems.push(problem);
                }
            }
        }

        for mode in &self.updates {
            for entry in &mode.registry {
                if let Err(problem) = check_registry_entry(&mode.id, entry) {
                    problems.push(problem);
                }
            }
        }

        for provider in &self.dns {
            for address in [&provider.primary, &provider.secondary, &provider.primary6, &provider.secondary6] {
                if address.parse::<std::net::IpAddr>().is_err() {
                    problems.push(format!("dns {}: bad address '{address}'", provider.id));
                }
            }
            if !provider.doh_template.starts_with("https://") {
                problems.push(format!("dns {}: bad DoH template", provider.id));
            }
        }

        let tweak_ids: HashSet<&str> = self.tweaks.iter().map(|tweak| tweak.id.as_str()).collect();
        let appx_ids: HashSet<&str> = self.appx.iter().map(|package| package.id.as_str()).collect();
        check_refs("minimal", &self.presets.minimal, &tweak_ids, &mut problems);
        check_refs("standard", &self.presets.standard, &tweak_ids, &mut problems);
        check_refs("advanced", &self.presets.advanced, &tweak_ids, &mut problems);
        check_refs("appx_default", &self.presets.appx_default, &appx_ids, &mut problems);

        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn valid_entry() -> RegistryEntry {
        RegistryEntry {
            path: r"HKCU\Software\WindowCraft".to_owned(),
            name: "Flag".to_owned(),
            kind: RegKind::DWord,
            value: Some("1".to_owned()),
            original: Some("0".to_owned()),
            default_on: None,
            values: None,
        }
    }

    #[test]
    fn embedded_catalog_passes_every_validation_rule() {
        let catalog = Catalog::load().unwrap();
        let problems = catalog.validate();
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn registry_entry_rejects_unknown_hive() {
        let entry = RegistryEntry { path: r"HKEY_CURRENT_USER\Software\Test".to_owned(), ..valid_entry() };
        let error = check_registry_entry("tweak-a", &entry).unwrap_err();
        assert!(error.contains("bad hive"), "{error}");
    }

    #[test]
    fn registry_entry_rejects_trailing_or_double_backslash() {
        let trailing = RegistryEntry { path: r"HKCU\Software\Test\".to_owned(), ..valid_entry() };
        assert!(check_registry_entry("tweak-a", &trailing).unwrap_err().contains("malformed path"));

        let doubled = RegistryEntry { path: r"HKCU\Software\\Test".to_owned(), ..valid_entry() };
        assert!(check_registry_entry("tweak-a", &doubled).unwrap_err().contains("malformed path"));
    }

    #[test]
    fn registry_entry_rejects_colon_in_subkey_path() {
        let entry = RegistryEntry { path: r"HKCU\Software:Test".to_owned(), ..valid_entry() };
        let error = check_registry_entry("tweak-a", &entry).unwrap_err();
        assert!(error.contains("PowerShell-style path"), "{error}");
    }

    #[test]
    fn registry_entry_rejects_powershell_colon_path() {
        let entry = RegistryEntry { path: r"HKCU:\Software\Test".to_owned(), ..valid_entry() };
        let error = check_registry_entry("tweak-a", &entry).unwrap_err();
        assert!(error.contains("PowerShell-style path"), "{error}");
    }

    #[test]
    fn registry_entry_rejects_entry_with_no_values() {
        let entry = RegistryEntry { value: None, original: None, values: None, ..valid_entry() };
        let error = check_registry_entry("tweak-a", &entry).unwrap_err();
        assert!(error.contains("has no value"), "{error}");
    }

    #[test]
    fn registry_entry_rejects_invalid_dword_and_accepts_sentinels() {
        let bad = RegistryEntry { value: Some("not-a-number".to_owned()), ..valid_entry() };
        assert!(check_registry_entry("tweak-a", &bad).unwrap_err().contains("not a valid DWord"));

        let overflow = RegistryEntry { value: Some("4294967296".to_owned()), ..valid_entry() };
        assert!(check_registry_entry("tweak-a", &overflow).is_err());

        let sentinels = RegistryEntry {
            value: Some(RAM_KB_TOKEN.to_owned()),
            original: Some(REMOVE_ENTRY.to_owned()),
            ..valid_entry()
        };
        assert!(check_registry_entry("tweak-a", &sentinels).is_ok());
    }

    #[test]
    fn registry_entry_rejects_invalid_qword() {
        let bad = RegistryEntry {
            kind: RegKind::QWord,
            value: Some("-1".to_owned()),
            original: Some("18446744073709551615".to_owned()),
            ..valid_entry()
        };
        let error = check_registry_entry("tweak-a", &bad).unwrap_err();
        assert!(error.contains("not a valid QWord"), "{error}");
    }

    #[test]
    fn registry_entry_rejects_odd_length_or_non_hex_binary() {
        let odd = RegistryEntry {
            kind: RegKind::Binary,
            value: Some("ABC".to_owned()),
            original: None,
            ..valid_entry()
        };
        assert!(check_registry_entry("tweak-a", &odd).unwrap_err().contains("not a valid Binary"));

        let non_hex = RegistryEntry {
            kind: RegKind::Binary,
            value: Some("00GG".to_owned()),
            original: None,
            ..valid_entry()
        };
        assert!(check_registry_entry("tweak-a", &non_hex).unwrap_err().contains("not a valid Binary"));
    }

    #[test]
    fn catalog_rejects_empty_identifier() {
        let mut catalog = Catalog::load().unwrap();
        catalog.apps[0].id.clear();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p == "applications: empty id"), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_duplicate_identifier() {
        let mut catalog = Catalog::load().unwrap();
        let dup = catalog.tweaks[0].id.clone();
        catalog.tweaks[1].id.clone_from(&dup);
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p == &format!("tweaks: duplicate id '{dup}'")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_application_without_package_source() {
        let mut catalog = Catalog::load().unwrap();
        catalog.apps[0].winget = None;
        catalog.apps[0].choco = None;
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("no package source")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_application_with_non_http_link() {
        let mut catalog = Catalog::load().unwrap();
        catalog.apps[0].link = "ftp://example.com".to_owned();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("bad link")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_segmented_tweak_missing_value_for_option() {
        let mut catalog = Catalog::load().unwrap();
        let tweak = catalog.tweaks.iter_mut().find(|t| t.kind == TweakKind::Segmented).unwrap();
        tweak.registry[0].values = Some(BTreeMap::new());
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("no value for option")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_choice_control_without_options() {
        let mut catalog = Catalog::load().unwrap();
        let tweak = catalog.tweaks.iter_mut().find(|t| t.kind == TweakKind::Dns).unwrap();
        tweak.options.clear();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("choice control without options")), "{problems:?}");
    }

    fn block_list(catalog: &mut Catalog) -> (&mut String, &mut String) {
        let op = catalog.tweaks.iter_mut().flat_map(|tweak| tweak.apply.iter_mut()).find_map(|op| match op {
            NativeOp::HostsBlock { url, sha256, .. } => Some((url, sha256)),
            _ => None,
        });
        op.unwrap()
    }

    #[test]
    fn catalog_rejects_unpinned_block_lists() {
        let mut catalog = Catalog::load().unwrap();
        *block_list(&mut catalog).0 = "https://github.com/example/list/raw/refs/heads/master/hosts".to_owned();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("not pinned")), "{problems:?}");

        let mut catalog = Catalog::load().unwrap();
        *block_list(&mut catalog).1 = "abc".to_owned();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("bad sha256")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_check_tweak_that_does_nothing() {
        let mut catalog = Catalog::load().unwrap();
        let tweak = catalog.tweaks.iter_mut().find(|t| t.kind == TweakKind::Check).unwrap();
        tweak.registry.clear();
        tweak.services.clear();
        tweak.apply.clear();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("does nothing")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_bad_registry_entries_in_features_and_updates() {
        let mut catalog = Catalog::load().unwrap();
        let feature = catalog.features.iter_mut().find(|f| !f.registry.is_empty()).unwrap();
        feature.registry[0].path = "BAD\\Path".to_owned();
        catalog.updates[0].registry[0].path = "HKLM\\Trailing\\".to_owned();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("bad hive")), "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("malformed path")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_dns_provider_with_invalid_ip_address() {
        let mut catalog = Catalog::load().unwrap();
        catalog.dns[0].primary6 = "not-an-ipv6".to_owned();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("bad address 'not-an-ipv6'")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_dns_provider_with_non_https_doh_template() {
        let mut catalog = Catalog::load().unwrap();
        catalog.dns[0].doh_template = "http://dns.google/dns-query".to_owned();
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p.contains("bad DoH template")), "{problems:?}");
    }

    #[test]
    fn catalog_rejects_unknown_preset_references() {
        let mut catalog = Catalog::load().unwrap();
        catalog.presets.minimal.push("ghost-tweak".to_owned());
        catalog.presets.appx_default.push("ghost-appx".to_owned());
        let problems = catalog.validate();
        assert!(problems.iter().any(|p| p == "preset minimal: unknown reference 'ghost-tweak'"), "{problems:?}");
        assert!(problems.iter().any(|p| p == "preset appx_default: unknown reference 'ghost-appx'"), "{problems:?}");
    }
}
