// Made by Nordan
//! Global search across apps, tweaks, registry keys, services and fixes.

use crate::catalog::Catalog;
use crate::schema::FeatureKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchKind {
    App,
    Tweak,
    RegistryKey,
    Service,
    Fix,
    Appx,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub kind: SearchKind,
    /// Catalog id of the owning entry.
    pub id: String,
    pub title: String,
    pub detail: String,
    score: u32,
}

/// Case-insensitive, all-terms match. Multiple words narrow the result.
fn score(haystack: &str, terms: &[String]) -> Option<u32> {
    let lower = haystack.to_lowercase();
    let mut total = 0;
    for term in terms {
        let position = lower.find(term.as_str())?;
        total += match position {
            0 => 3,
            _ if lower.as_bytes()[position - 1] == b' ' => 2,
            _ => 1,
        };
    }
    Some(total)
}

/// Returns up to `limit` hits, best first.
pub fn search(catalog: &Catalog, query: &str, limit: usize) -> Vec<SearchHit> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let mut push = |kind, id: &str, title: &str, detail: &str, base: u32, text: &str| {
        if let Some(found) = score(text, &terms) {
            hits.push(SearchHit {
                kind,
                id: id.to_owned(),
                title: title.to_owned(),
                detail: detail.to_owned(),
                score: base + found,
            });
        }
    };

    for app in &catalog.apps {
        let source = app.winget.as_deref().or(app.choco.as_deref()).unwrap_or_default();
        let english = catalog.english_text("app", &app.id).unwrap_or_default();
        let text = format!("{} {} {} {english}", app.name, app.description, source);
        push(SearchKind::App, &app.id, &app.name, source, 10, &text);
    }
    for tweak in &catalog.tweaks {
        let english = catalog.english_text("tweak", &tweak.id).unwrap_or_default();
        let text = format!("{} {} {english}", tweak.name, tweak.description);
        push(SearchKind::Tweak, &tweak.id, &tweak.name, tweak.group.label(), 20, &text);
        for entry in &tweak.registry {
            let key = format!("{}\\{}", entry.path, entry.name);
            push(SearchKind::RegistryKey, &tweak.id, &key, &tweak.name, 5, &key);
        }
        for rule in &tweak.services {
            push(SearchKind::Service, &tweak.id, &rule.name, &tweak.name, 8, &rule.name);
        }
    }
    for feature in &catalog.features {
        if feature.kind != FeatureKind::Panel {
            let english = catalog.english_text("feature", &feature.id).unwrap_or_default();
            let text = format!("{} {} {english}", feature.name, feature.description);
            push(SearchKind::Fix, &feature.id, &feature.name, "System Config", 15, &text);
        }
    }
    for package in &catalog.appx {
        let english = catalog.english_text("appx", &package.id).unwrap_or_default();
        let text = format!("{} {} {english}", package.name, package.package_id);
        push(SearchKind::Appx, &package.id, &package.name, &package.package_id, 6, &text);
    }

    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.title.cmp(&b.title)));
    hits.truncate(limit);
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_translated_catalog_is_found_by_either_language() {
        let catalog = Catalog::load_in("pl").unwrap();
        let by_english = search(&catalog, "telemetry", 10);
        let hit = by_english.iter().find(|hit| hit.id == "telemetry" && hit.kind == SearchKind::Tweak).unwrap();
        assert_eq!(hit.title, "Wyłącz telemetrię Windows", "the hit shows the translated name");
        assert!(search(&catalog, "telemetrię", 10).iter().any(|hit| hit.id == "telemetry"));
        assert!(search(&catalog, "przeglądarka", 60).iter().any(|hit| hit.id == "firefox"));
        assert!(search(&catalog, "browser mozilla", 10).iter().any(|hit| hit.id == "firefox"));
    }

    fn catalog() -> Catalog {
        Catalog::load().unwrap()
    }

    #[test]
    fn empty_and_whitespace_only_queries_return_no_hits() {
        let catalog = catalog();
        assert!(search(&catalog, "", 20).is_empty());
        assert!(search(&catalog, "   \t\n  ", 20).is_empty());
    }

    #[test]
    fn unmatched_queries_and_unmatched_second_terms_return_no_hits() {
        let catalog = catalog();
        assert!(search(&catalog, "xyzzynonexistenttoken99", 20).is_empty());
        assert!(!search(&catalog, "telemetry", 20).is_empty());
        assert!(search(&catalog, "telemetry xyzzynonexistenttoken99", 20).is_empty());
    }

    #[test]
    fn matches_case_insensitively_across_kinds_and_excludes_panels() {
        let catalog = catalog();
        let lower = search(&catalog, "allowtelemetry", 50);
        let upper = search(&catalog, "ALLOWTELEMETRY", 50);
        let mixed = search(&catalog, "AlLoWtElEmEtRy", 50);
        assert!(!lower.is_empty());
        assert_eq!(lower, upper);
        assert_eq!(lower, mixed);
        assert!(lower.iter().any(|hit| hit.kind == SearchKind::RegistryKey && hit.id == "telemetry"));

        assert!(search(&catalog, "7zip.7zip", 10).iter().any(|hit| hit.kind == SearchKind::App && hit.id == "7zip"));
        assert!(search(&catalog, "DiagTrack", 10).iter().any(|hit| hit.kind == SearchKind::Service));
        assert!(search(&catalog, "WSL", 10).iter().any(|hit| hit.kind == SearchKind::Fix && hit.id == "wsl"));
        assert!(
            search(&catalog, "WindowsFeedbackHub", 10)
                .iter()
                .any(|hit| hit.kind == SearchKind::Appx && hit.id == "microsoft-windows-feedback-hub")
        );
        assert!(
            search(&catalog, "compmgmt.msc", 50).iter().all(|hit| hit.id != "panel-computer"),
            "panel entries must not appear in search hits"
        );
    }

    #[test]
    fn ranks_tweaks_above_lower_base_kinds_and_rewards_prefix_and_word_boundaries() {
        let catalog = catalog();
        let hits = search(&catalog, "telemetry", 50);
        let first_tweak = hits.iter().position(|hit| hit.kind == SearchKind::Tweak && hit.id == "telemetry").unwrap();
        let first_reg = hits.iter().position(|hit| hit.kind == SearchKind::RegistryKey).unwrap();
        assert_eq!(first_tweak, 0, "tweak prefix match must outrank registry keys and description matches");
        assert!(first_tweak < first_reg);

        assert_eq!(score("telemetry disable", &["telemetry".to_owned()]), Some(3));
        assert_eq!(score("disable telemetry", &["telemetry".to_owned()]), Some(2));
        assert_eq!(score("allowtelemetry", &["telemetry".to_owned()]), Some(1));
    }

    #[test]
    fn respects_limit_and_breaks_score_ties_alphabetically_by_title() {
        let catalog = catalog();
        let full = search(&catalog, "microsoft", 100);
        assert!(full.len() > 5);
        let limited = search(&catalog, "microsoft", 3);
        assert_eq!(limited.len(), 3);
        assert_eq!(&full[..3], &limited[..]);
        assert!(search(&catalog, "microsoft", 0).is_empty());

        for pair in full.windows(2) {
            assert!(
                pair[0].score > pair[1].score
                    || (pair[0].score == pair[1].score && pair[0].title <= pair[1].title),
                "out of order: {:?} before {:?}",
                pair[0],
                pair[1]
            );
        }
    }
}
