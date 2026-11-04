// Made by ycangignacy
//! Integrity checks over every embedded catalog.

use std::collections::BTreeMap;
use windowcraft_config::*;

fn catalog() -> Catalog {
    Catalog::load().expect("embedded catalogs must parse")
}

#[test]
fn catalog_has_no_structural_problems() {
    let problems = catalog().validate();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn application_counts_match_specification() {
    let catalog = catalog();
    assert_eq!(catalog.apps.len(), 239);
    let mut counts = BTreeMap::new();
    for app in &catalog.apps {
        *counts.entry(app.category.as_str()).or_insert(0) += 1;
    }
    let expected = [
        ("Browsers", 15),
        ("Communications", 18),
        ("Development", 43),
        ("Document", 17),
        ("Games", 17),
        ("Microsoft Tools", 19),
        ("Multimedia Tools", 21),
        ("Pro Tools", 24),
        ("Selfhosted Tools", 12),
        ("Utilities", 53),
    ];
    assert_eq!(counts, expected.into_iter().collect());
}

#[test]
fn tweak_counts_match_specification() {
    let catalog = catalog();
    assert_eq!(catalog.tweaks.len(), 68);
    let count = |group| catalog.tweaks.iter().filter(|tweak| tweak.group == group).count();
    assert_eq!(count(TweakGroup::Essential), 18);
    assert_eq!(count(TweakGroup::Advanced), 23);
    assert_eq!(count(TweakGroup::Preferences), 25);
    assert_eq!(count(TweakGroup::Performance), 2);
}

#[test]
fn tool_and_appx_counts_match_specification() {
    let catalog = catalog();
    assert_eq!(catalog.features.len(), 33);
    assert_eq!(catalog.appx.len(), 34);
    let panels = catalog.features.iter().filter(|f| f.kind == FeatureKind::Panel).count();
    let fixes = catalog.features.iter().filter(|f| f.kind == FeatureKind::Fix).count();
    assert_eq!(panels, 14);
    assert_eq!(fixes, 6);
}

#[test]
fn preset_sizes_match_specification() {
    let catalog = catalog();
    assert_eq!(catalog.preset(PresetName::Minimal).len(), 4);
    assert_eq!(catalog.preset(PresetName::Standard).len(), 12);
    assert_eq!(catalog.preset(PresetName::Advanced).len(), 17);
    assert_eq!(catalog.presets.appx_default.len(), 18);
}

#[test]
fn telemetry_tweak_keeps_all_upstream_values() {
    let catalog = catalog();
    let tweak = catalog.tweak("telemetry").unwrap();
    // 13 upstream values plus the Defender policy and environment variable
    // that replace its PowerShell block.
    assert_eq!(tweak.registry.len(), 15);
    assert!(tweak.registry.iter().any(|e| e.name == "AllowTelemetry" && e.value.as_deref() == Some("0")));
    assert!(tweak.registry.iter().any(|e| e.name == "SubmitSamplesConsent" && e.value.as_deref() == Some("2")));
    assert_eq!(tweak.services.len(), 2);
}

#[test]
fn mpo_segmented_control_has_three_states() {
    let catalog = catalog();
    let tweak = catalog.tweak("multiplane-overlay").unwrap();
    assert_eq!(tweak.kind, TweakKind::Segmented);
    assert_eq!(tweak.options.len(), 3);
    let overlay = tweak.registry.iter().find(|e| e.name == "DisableOverlays").unwrap();
    assert_eq!(overlay.values.as_ref().unwrap()["Fully Disabled"], "1");
}

#[test]
fn catalogs_contain_no_powershell() {
    for source in [
        include_str!("../catalog/tweaks.json"),
        include_str!("../catalog/feature.json"),
        include_str!("../catalog/updates.json"),
    ] {
        for needle in ["InvokeScript", "UndoScript", "Invoke-WinUtil", "Set-ItemProperty", "Get-AppxPackage"] {
            assert!(!source.contains(needle), "found {needle}");
        }
    }
}

#[test]
fn every_tweak_can_describe_its_native_operations() {
    for tweak in &catalog().tweaks {
        for op in tweak.prepare.iter().chain(&tweak.apply).chain(&tweak.undo) {
            assert!(!op.describe().is_empty(), "{}", tweak.id);
        }
    }
}

#[test]
fn copy_contains_no_marketing_terms_or_emoji() {
    let forbidden = ["supercharge", "unleash", "next-gen", "ultimate performance!", "blazing"];
    let catalog = catalog();
    let texts = catalog
        .tweaks
        .iter()
        .flat_map(|t| [t.name.as_str(), t.description.as_str()])
        .chain(catalog.features.iter().flat_map(|f| [f.name.as_str(), f.description.as_str()]));
    for text in texts {
        let lower = text.to_lowercase();
        for word in forbidden {
            assert!(!lower.contains(word), "'{word}' in '{text}'");
        }
        assert!(text.chars().all(|c| (c as u32) < 0x2190), "non-text glyph in '{text}'");
    }
}

#[test]
fn search_spans_every_kind() {
    let catalog = catalog();
    assert!(search(&catalog, "firefox", 10).iter().any(|hit| hit.kind == SearchKind::App));
    assert!(search(&catalog, "telemetry", 10).iter().any(|hit| hit.kind == SearchKind::Tweak));
    assert!(search(&catalog, "AllowTelemetry", 10).iter().any(|hit| hit.kind == SearchKind::RegistryKey));
    assert!(search(&catalog, "DiagTrack", 10).iter().any(|hit| hit.kind == SearchKind::Service));
    assert!(search(&catalog, "corruption", 10).iter().any(|hit| hit.kind == SearchKind::Fix));
    assert!(search(&catalog, "FeedbackHub", 10).iter().any(|hit| hit.kind == SearchKind::Appx));
    assert!(search(&catalog, "   ", 10).is_empty());
}

#[test]
fn search_requires_all_terms_and_respects_limit() {
    let catalog = catalog();
    assert!(search(&catalog, "telemetry zzzznotaword", 10).is_empty());
    assert_eq!(search(&catalog, "e", 7).len(), 7);
}

#[test]
fn profile_roundtrips() {
    let catalog = catalog();
    let mut profile = Profile::new();
    profile.tweaks.push("telemetry".into());
    profile.toggles.insert("dark-mode".into(), true);
    profile.segmented.insert("multiplane-overlay".into(), "Fully Disabled".into());
    profile.apps.push("7zip".into());
    profile.dns = Some("Cloudflare".into());
    let json = profile.to_json();
    let resolved = import_any(&json, &catalog).unwrap();
    assert_eq!(resolved.profile, profile);
}

#[test]
fn profile_rejects_unknown_keys() {
    let catalog = catalog();
    let error = import_any(r#"{"version":1,"tweeks":["telemetry"]}"#, &catalog).unwrap_err();
    assert!(matches!(error, ProfileError::Shape(_)), "{error}");
}

#[test]
fn profile_rejects_unknown_references() {
    let catalog = catalog();
    let cases = [
        r#"{"version":1,"tweaks":["no-such-tweak"]}"#,
        r#"{"version":1,"apps":["no-such-app"]}"#,
        r#"{"version":1,"features":["no-such-feature"]}"#,
        r#"{"version":1,"appx_remove":["no-such-package"]}"#,
        r#"{"version":1,"dns":"NotADns"}"#,
        r#"{"version":1,"update_mode":"sometimes"}"#,
        r#"{"version":1,"segmented":{"multiplane-overlay":"Maybe"}}"#,
    ];
    for case in cases {
        assert!(
            matches!(import_any(case, &catalog), Err(ProfileError::UnknownId { .. })),
            "accepted {case}"
        );
    }
}

#[test]
fn profile_rejects_corrupted_json_gracefully() {
    let catalog = catalog();
    for case in ["", "{", "[1,2,3]", "null", r#"{"version":"one"}"#, "\u{0}\u{1}"] {
        assert!(import_any(case, &catalog).is_err(), "accepted {case:?}");
    }
    assert!(matches!(import_any(r#"{"version":99}"#, &catalog), Err(ProfileError::Version(99))));
}

#[test]
fn winutil_config_imports() {
    let catalog = catalog();
    let source = r#"{
        "WPFTweaks": ["WPFTweaksTelemetry", "WPFToggleDarkMode", "WPFTweaksActivity"],
        "WPFFeature": ["WPFFeaturewsl"],
        "WPFInstall": ["WPFInstall7zip"],
        "Install": [{"winget": "Mozilla.Firefox", "choco": "firefox"}]
    }"#;
    let profile = import_any(source, &catalog).unwrap().profile;
    assert_eq!(profile.tweaks, ["telemetry", "activity"]);
    assert_eq!(profile.toggles.get("dark-mode"), Some(&true));
    assert_eq!(profile.features, ["wsl"]);
    assert!(profile.apps.contains(&"7zip".to_owned()));
    assert_eq!(profile.apps.len(), 2);
}

#[test]
fn winutil_config_rejects_unknown_keys_and_ids() {
    let catalog = catalog();
    let unknown = import_any(r#"{"WPFTweaks":[],"Surprise":1}"#, &catalog).unwrap_err();
    assert!(matches!(unknown, ProfileError::UnknownKeys(_)));
    let bad_id = import_any(r#"{"WPFTweaks":["WPFTweaksNope"]}"#, &catalog).unwrap_err();
    assert!(matches!(bad_id, ProfileError::UnknownId { .. }));
}

#[test]
fn registry_validation_rejects_bad_entries() {
    use windowcraft_config::validate::check_registry_entry;
    let good = RegistryEntry {
        path: "HKLM\\SOFTWARE\\X".into(),
        name: "A".into(),
        kind: RegKind::DWord,
        value: Some("1".into()),
        original: Some(REMOVE_ENTRY.into()),
        default_on: None,
        values: None,
    };
    assert!(check_registry_entry("t", &good).is_ok());
    for path in ["HKXX\\A", "HKLM:\\A", "HKLM\\A\\", "HKLM\\A\\\\B", "A"] {
        let entry = RegistryEntry { path: path.into(), ..good.clone() };
        assert!(check_registry_entry("t", &entry).is_err(), "{path}");
    }
    let bad_number = RegistryEntry { value: Some("not-a-number".into()), ..good.clone() };
    assert!(check_registry_entry("t", &bad_number).is_err());
    let bad_binary = RegistryEntry { kind: RegKind::Binary, value: Some("ABC".into()), ..good };
    assert!(check_registry_entry("t", &bad_binary).is_err());
}

/// Collects every object with an `op` key, wherever it sits in the catalog.
fn native_ops<'a>(value: &'a serde_json::Value, found: &mut Vec<&'a serde_json::Map<String, serde_json::Value>>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("op").is_some_and(serde_json::Value::is_string) {
                found.push(map);
            }
            map.values().for_each(|child| native_ops(child, found));
        }
        serde_json::Value::Array(items) => items.iter().for_each(|child| native_ops(child, found)),
        _ => {}
    }
}

/// `deny_unknown_fields` does not reach variants without fields in an
/// internally tagged enum, so serde accepts `{"op":"fix_network","x":1}`.
/// Round-tripping each operation finds such leftover keys in the data.
#[test]
fn native_ops_carry_no_unknown_fields() {
    let mut problems = Vec::new();
    let mut checked = 0;
    for (name, text) in [
        ("tweaks.json", include_str!("../catalog/tweaks.json")),
        ("feature.json", include_str!("../catalog/feature.json")),
        ("updates.json", include_str!("../catalog/updates.json")),
    ] {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        let mut found = Vec::new();
        native_ops(&value, &mut found);
        checked += found.len();
        for raw in found {
            let op: NativeOp = serde_json::from_value(serde_json::Value::Object(raw.clone())).unwrap();
            let serde_json::Value::Object(round) = serde_json::to_value(&op).unwrap() else { unreachable!() };
            for key in raw.keys().filter(|key| !round.contains_key(*key)) {
                problems.push(format!("{name}: {} has unknown field '{key}'", raw["op"]));
            }
        }
    }
    assert!(checked > 20, "only {checked} operations found");
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn round_trip_check_sees_fields_serde_lets_through() {
    let raw = serde_json::json!({"op": "fix_network", "stray": true});
    let op: NativeOp = serde_json::from_value(raw).unwrap();
    let serde_json::Value::Object(round) = serde_json::to_value(&op).unwrap() else { unreachable!() };
    assert!(!round.contains_key("stray"), "serde started rejecting this itself; the check can go");
}
