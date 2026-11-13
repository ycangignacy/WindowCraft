// Made by WindowCraft Dev Team
//! Translated catalog text.
//!
//! The catalog itself is English. `catalog/i18n/<tag>.json` holds the names,
//! descriptions, notices and warnings of one language, keyed by catalog id.
//! An entry or a field that is missing keeps the English text. The English
//! text of every translated item stays in `Catalog::english`, so search finds
//! an item by either language.

use crate::catalog::Catalog;
use crate::schema::ChoiceOption;
use serde::Deserialize;
use std::collections::BTreeMap;

/// Every language the app offers besides English, with its catalog text.
pub const LANGUAGES: [(&str, &str); 7] = [
    ("pl", include_str!("../catalog/i18n/pl.json")),
    ("de", include_str!("../catalog/i18n/de.json")),
    ("fr", include_str!("../catalog/i18n/fr.json")),
    ("es", include_str!("../catalog/i18n/es.json")),
    ("it", include_str!("../catalog/i18n/it.json")),
    ("pt-BR", include_str!("../catalog/i18n/pt-BR.json")),
    ("uk", include_str!("../catalog/i18n/uk.json")),
];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogText {
    #[serde(default)]
    pub tweaks: BTreeMap<String, ItemText>,
    #[serde(default)]
    pub features: BTreeMap<String, ItemText>,
    #[serde(default)]
    pub appx: BTreeMap<String, ItemText>,
    #[serde(default)]
    pub updates: BTreeMap<String, ItemText>,
    /// Application id to description. Application names are product names.
    #[serde(default)]
    pub apps: BTreeMap<String, String>,
    /// English category name to the translated one.
    #[serde(default)]
    pub categories: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemText {
    pub name: Option<String>,
    pub description: Option<String>,
    pub notice: Option<String>,
    pub warning: Option<String>,
    /// Same length and order as the options in the catalog.
    #[serde(default)]
    pub options: Vec<OptionText>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionText {
    pub label: String,
    pub description: String,
}

/// Parsed text for `tag`, or None for English and unknown tags.
pub fn text_for(tag: &str) -> Option<Result<CatalogText, String>> {
    let (tag, source) = LANGUAGES.iter().find(|(known, _)| known.eq_ignore_ascii_case(tag))?;
    Some(serde_json::from_str(source).map_err(|error| format!("catalog/i18n/{tag}.json: {error}")))
}

fn set(target: &mut String, value: Option<&String>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        target.clone_from(value);
    }
}

fn set_options(options: &mut [ChoiceOption], text: &[OptionText]) {
    if text.len() != options.len() {
        return;
    }
    for (option, text) in options.iter_mut().zip(text) {
        if option.value.is_empty() {
            option.value.clone_from(&option.label);
        }
        set(&mut option.label, Some(&text.label));
        set(&mut option.description, Some(&text.description));
    }
}

impl Catalog {
    /// The catalog with names and descriptions in `tag`. English, and any
    /// language without a text file, gives the plain catalog.
    pub fn load_in(tag: &str) -> Result<Self, String> {
        let mut catalog = Self::load()?;
        if let Some(text) = text_for(tag) {
            catalog.apply(&text?);
        }
        Ok(catalog)
    }

    /// English name and description of an item this catalog shows in another
    /// language, for search. `kind` is `tweak`, `feature`, `appx` or `app`.
    pub fn english_text(&self, kind: &str, id: &str) -> Option<&str> {
        self.english.get(&format!("{kind}:{id}")).map(String::as_str)
    }

    fn remember(&mut self, kind: &str, id: &str, name: &str, description: &str) {
        self.english.insert(format!("{kind}:{id}"), format!("{name} {description}"));
    }

    pub fn apply(&mut self, text: &CatalogText) {
        let mut english = Vec::new();
        for tweak in &mut self.tweaks {
            if let Some(item) = text.tweaks.get(&tweak.id) {
                english.push(("tweak", tweak.id.clone(), tweak.name.clone(), tweak.description.clone()));
                set(&mut tweak.name, item.name.as_ref());
                set(&mut tweak.description, item.description.as_ref());
                if let (Some(notice), Some(translated)) = (tweak.notice.as_mut(), item.notice.as_ref()) {
                    set(notice, Some(translated));
                }
                set_options(&mut tweak.options, &item.options);
            }
        }
        for feature in &mut self.features {
            if let Some(item) = text.features.get(&feature.id) {
                english.push(("feature", feature.id.clone(), feature.name.clone(), feature.description.clone()));
                set(&mut feature.name, item.name.as_ref());
                set(&mut feature.description, item.description.as_ref());
                set_options(&mut feature.options, &item.options);
            }
        }
        for package in &mut self.appx {
            if let Some(item) = text.appx.get(&package.id) {
                english.push(("appx", package.id.clone(), package.name.clone(), package.description.clone()));
                set(&mut package.name, item.name.as_ref());
                set(&mut package.description, item.description.as_ref());
                if let (Some(warning), Some(translated)) = (package.warning.as_mut(), item.warning.as_ref()) {
                    set(warning, Some(translated));
                }
            }
            if let Some(category) = text.categories.get(&package.category) {
                set(&mut package.category, Some(category));
            }
        }
        for mode in &mut self.updates {
            if let Some(item) = text.updates.get(&mode.id) {
                set(&mut mode.name, item.name.as_ref());
                set(&mut mode.description, item.description.as_ref());
            }
        }
        for app in &mut self.apps {
            if let Some(description) = text.apps.get(&app.id) {
                english.push(("app", app.id.clone(), app.name.clone(), app.description.clone()));
                set(&mut app.description, Some(description));
            }
            if let Some(category) = text.categories.get(&app.category) {
                set(&mut app.category, Some(category));
            }
        }
        for (kind, id, name, description) in english {
            self.remember(kind, &id, &name, &description);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english() -> Catalog {
        Catalog::load().unwrap()
    }

    /// Ids in a text file that name nothing in the catalog.
    fn unknown_ids(catalog: &Catalog, text: &CatalogText) -> Vec<String> {
        let mut unknown = Vec::new();
        let mut check = |section: &str, ids: Vec<&String>, known: &dyn Fn(&str) -> bool| {
            unknown.extend(ids.into_iter().filter(|id| !known(id)).map(|id| format!("{section}.{id}")));
        };
        check("tweaks", text.tweaks.keys().collect(), &|id| catalog.tweak(id).is_some());
        check("features", text.features.keys().collect(), &|id| catalog.feature(id).is_some());
        check("appx", text.appx.keys().collect(), &|id| catalog.appx_package(id).is_some());
        check("updates", text.updates.keys().collect(), &|id| catalog.update_mode(id).is_some());
        check("apps", text.apps.keys().collect(), &|id| catalog.app(id).is_some());
        let categories: Vec<&str> = catalog
            .apps
            .iter()
            .map(|app| app.category.as_str())
            .chain(catalog.appx.iter().map(|p| p.category.as_str()))
            .collect();
        check("categories", text.categories.keys().collect(), &|name| categories.contains(&name));
        unknown
    }

    /// Catalog items a text file leaves in English.
    fn missing(catalog: &Catalog, text: &CatalogText) -> Vec<String> {
        let mut missing = Vec::new();
        let full = |item: Option<&ItemText>, notice: bool, warning: bool, options: usize| {
            item.is_some_and(|item| {
                item.name.is_some()
                    && item.description.is_some()
                    && (!notice || item.notice.is_some())
                    && (!warning || item.warning.is_some())
                    && (options == 0 || item.options.len() == options)
            })
        };
        for tweak in &catalog.tweaks {
            if !full(text.tweaks.get(&tweak.id), tweak.notice.is_some(), false, tweak.options.len()) {
                missing.push(format!("tweaks.{}", tweak.id));
            }
        }
        for feature in &catalog.features {
            if !full(text.features.get(&feature.id), false, false, feature.options.len()) {
                missing.push(format!("features.{}", feature.id));
            }
        }
        for package in &catalog.appx {
            if !full(text.appx.get(&package.id), false, package.warning.is_some(), 0) {
                missing.push(format!("appx.{}", package.id));
            }
        }
        for mode in &catalog.updates {
            if !full(text.updates.get(&mode.id), false, false, 0) {
                missing.push(format!("updates.{}", mode.id));
            }
        }
        for app in &catalog.apps {
            if !text.apps.contains_key(&app.id) {
                missing.push(format!("apps.{}", app.id));
            }
        }
        for category in catalog.app_categories().into_iter().chain(catalog.appx_categories()) {
            if !text.categories.contains_key(category) {
                missing.push(format!("categories.{category}"));
            }
        }
        missing
    }

    #[test]
    fn every_language_translates_the_whole_catalog_and_nothing_else() {
        let catalog = english();
        for (tag, _) in LANGUAGES {
            let text = text_for(tag).unwrap().unwrap();
            assert_eq!(unknown_ids(&catalog, &text), Vec::<String>::new(), "{tag}: ids that name nothing");
            assert_eq!(missing(&catalog, &text), Vec::<String>::new(), "{tag}: left in English");
        }
    }

    /// 1.4.0 sent the translated label as the option value, so in Polish
    /// "Całkowicie wyłączone" matched no registry value and the MPO values
    /// were deleted, and "Agresywnie" ran the standard Windows Update reset.
    #[test]
    fn translated_options_keep_their_english_values() {
        let catalog = english();
        let profile = r#"{"version":1,"segmented":{"multiplane-overlay":"Fully Disabled"},"dns":"Cloudflare"}"#;
        for (tag, _) in LANGUAGES {
            let localized = Catalog::load_in(tag).unwrap();
            let pairs = catalog
                .tweaks
                .iter()
                .zip(&localized.tweaks)
                .map(|(before, after)| (&before.id, &before.options, &after.options))
                .chain(catalog.features.iter().zip(&localized.features).map(|(before, after)| (&before.id, &before.options, &after.options)));
            for (id, before, after) in pairs {
                assert_eq!(before.len(), after.len(), "{tag}: {id}");
                for (english, shown) in before.iter().zip(after) {
                    assert_eq!(shown.value(), english.label, "{tag}: {id} changed the value of '{}'", english.label);
                }
            }
            let update = localized.feature("update").unwrap();
            assert!(update.options.iter().any(|option| option.value() == "Aggressive"), "{tag}");
            crate::import_any(profile, &localized).unwrap_or_else(|error| panic!("{tag}: {error}"));
        }
    }

    #[test]
    fn translated_text_keeps_placeholders_and_has_no_empty_fields() {
        let catalog = english();
        for (tag, _) in LANGUAGES {
            let localized = Catalog::load_in(tag).unwrap();
            for (before, after) in catalog.tweaks.iter().zip(&localized.tweaks) {
                assert!(!after.name.trim().is_empty() && !after.description.trim().is_empty(), "{tag}: {}", after.id);
                // Paths and registry names in the description stay as they are.
                for token in
                    before.description.split_whitespace().filter(|word| word.contains('%') || word.contains('\\'))
                {
                    let token = token.trim_end_matches(['.', ',']);
                    assert!(after.description.contains(token), "{tag}: {} lost '{token}'", after.id);
                }
            }
        }
    }

    #[test]
    fn localizing_changes_only_text() {
        let catalog = english();
        let localized = Catalog::load_in("pl").unwrap();
        for (before, after) in catalog.tweaks.iter().zip(&localized.tweaks) {
            assert_eq!(before.id, after.id);
            assert_eq!(before.registry, after.registry);
            assert_eq!(before.services, after.services);
            assert_eq!(before.apply, after.apply);
            assert_eq!(before.undo, after.undo);
            assert_eq!(before.irreversible, after.irreversible);
            assert_eq!(before.exclusive, after.exclusive);
        }
        assert_eq!(catalog.presets, localized.presets);
        let disable = localized.update_mode("disabled").unwrap();
        assert_ne!(disable.description, catalog.update_mode("disabled").unwrap().description);
        assert!(localized.english_text("tweak", "telemetry").unwrap().contains("Turn off Windows telemetry"));
        assert!(catalog.english_text("tweak", "telemetry").is_none(), "English needs no second copy");
    }

    #[test]
    fn english_and_unknown_tags_load_the_plain_catalog() {
        let catalog = english();
        for tag in ["en", "xx"] {
            let loaded = Catalog::load_in(tag).unwrap();
            assert_eq!(loaded.tweaks, catalog.tweaks);
        }
    }

    #[test]
    fn translated_categories_are_in_alphabetical_order() {
        let fr = Catalog::load_in("fr").unwrap();
        let fr = fr.app_categories();
        let position = |name: &str| fr.iter().position(|c| *c == name).unwrap();
        assert!(position("Développement") < position("Documents"), "{fr:?}");

        let uk = Catalog::load_in("uk").unwrap();
        let uk = uk.app_categories();
        let position = |name: &str| uk.iter().position(|c| *c == name).unwrap();
        assert!(position("Браузери") < position("Ігри"), "{uk:?}");
        assert!(position("Документи") < position("Ігри"), "{uk:?}");
        assert!(position("Ігри") < position("Медіа"), "{uk:?}");
    }
}
