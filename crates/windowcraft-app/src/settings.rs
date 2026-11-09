// Made by ycangignacy
//! What the user chose on the Settings page, kept in
//! `%LOCALAPPDATA%\WindowCraft\settings.json` next to the language.
//!
//! The file is read and written as loose JSON, so keys this version does not
//! know survive a save.

use crate::app::Appearance;
use serde_json::{Value, json};
use std::path::PathBuf;
use windowcraft_config::Onboarding;

const SECONDS_PER_DAY: u64 = 86_400;

/// How often the scheduled task compares applied tweaks with the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CheckInterval {
    Off,
    Daily,
    Weekly,
    Monthly,
}

impl CheckInterval {
    pub const ALL: [Self; 4] = [Self::Off, Self::Daily, Self::Weekly, Self::Monthly];

    pub fn days(self) -> u64 {
        match self {
            Self::Off => 0,
            Self::Daily => 1,
            Self::Weekly => 7,
            Self::Monthly => 30,
        }
    }

    fn from_days(days: u64) -> Self {
        match days {
            0 => Self::Off,
            1 => Self::Daily,
            2..=7 => Self::Weekly,
            _ => Self::Monthly,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub appearance: Appearance,
    pub interval: CheckInterval,
    pub after_updates: bool,
    /// Unix time of the last finished check, 0 before the first.
    pub last_check: u64,
    /// Windows build and UBR seen by the last check, for example `26300.7171`.
    pub last_build: String,
    /// First-start questions and page tips, under the `onboarding` key.
    pub onboarding: Onboarding,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            interval: CheckInterval::Off,
            after_updates: false,
            last_check: 0,
            last_build: String::new(),
            onboarding: Onboarding::new(),
        }
    }
}

fn appearance_tag(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::System => "system",
        Appearance::Dark => "dark",
        Appearance::Light => "light",
    }
}

fn appearance_from(tag: &str) -> Appearance {
    match tag {
        "dark" => Appearance::Dark,
        "light" => Appearance::Light,
        _ => Appearance::System,
    }
}

impl Settings {
    /// True when the scheduled task should exist.
    pub fn wants_schedule(&self) -> bool {
        self.interval != CheckInterval::Off || self.after_updates
    }

    /// True when a scheduled run at `now` on Windows build `build` should check.
    pub fn check_due(&self, now: u64, build: &str) -> bool {
        let interval = self.interval.days() * SECONDS_PER_DAY;
        let elapsed = interval > 0 && now.saturating_sub(self.last_check) >= interval;
        let updated = self.after_updates && build != self.last_build;
        elapsed || updated
    }

    pub fn parse(content: &str) -> Self {
        let Ok(value) = serde_json::from_str::<Value>(content) else { return Self::default() };
        let check = &value["drift_check"];
        let state = &value["drift_check_state"];
        Self {
            appearance: appearance_from(value["appearance"].as_str().unwrap_or_default()),
            interval: CheckInterval::from_days(check["interval_days"].as_u64().unwrap_or(0)),
            after_updates: check["after_updates"].as_bool().unwrap_or(false),
            last_check: state["last_run"].as_u64().unwrap_or(0),
            last_build: state["build"].as_str().unwrap_or_default().to_owned(),
            onboarding: onboarding_from(&value["onboarding"]),
        }
    }

    /// Writes these values into `existing`, leaving other keys alone.
    fn merge_into(&self, existing: &str) -> String {
        let mut value = serde_json::from_str::<Value>(existing)
            .ok()
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        value["appearance"] = json!(appearance_tag(self.appearance));
        value["drift_check"] = json!({ "interval_days": self.interval.days(), "after_updates": self.after_updates });
        value["drift_check_state"] = json!({ "last_run": self.last_check, "build": self.last_build });
        value["onboarding"] = self.onboarding.to_value();
        serde_json::to_string_pretty(&value).unwrap_or_default()
    }
}

/// A missing key means the questions were never shown. A state this build
/// cannot read turns both the questions and the tips off.
fn onboarding_from(value: &Value) -> Onboarding {
    if value.is_null() {
        return Onboarding::new();
    }
    Onboarding::from_value(value.clone()).unwrap_or_else(|_| Onboarding::quiet())
}

#[cfg(test)]
thread_local! {
    /// A settings file for the current test thread only, see `use_test_file`.
    static TEST_FILE: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

/// Points `path` at `file` for the calling test thread.
#[cfg(test)]
pub fn use_test_file(file: Option<PathBuf>) {
    TEST_FILE.with(|slot| *slot.borrow_mut() = file);
}

/// Unit tests write to a file of their own, never to the user's settings.
pub fn path() -> PathBuf {
    #[cfg(test)]
    if let Some(file) = TEST_FILE.with(|slot| slot.borrow().clone()) {
        return file;
    }
    if cfg!(test) {
        return std::env::temp_dir().join(format!("windowcraft-test-settings-{}.json", std::process::id()));
    }
    windowcraft_engine::win::local_app_dir().join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(path()).map(|content| Settings::parse(&content)).unwrap_or_default()
}

pub fn save(settings: &Settings) -> Result<(), String> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    // Write and rename, so a crash never leaves half a file.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, settings.merge_into(&existing)).map_err(|e| e.to_string())?;
    std::fs::rename(&temporary, &path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        e.to_string()
    })
}

/// Arguments the scheduled task passes to `windowcraft.exe`.
pub const SCHEDULED_CHECK_ARGUMENT: &str = "--scheduled-check";

/// Creates or removes this user's scheduled check to match `settings`. The
/// task starts the executable that is running now.
pub fn apply_schedule(settings: &Settings, description: &str) -> Result<(), String> {
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    apply_schedule_for(settings, description, &program)
}

/// `apply_schedule` with the program the task starts given explicitly.
pub fn apply_schedule_for(settings: &Settings, description: &str, program: &std::path::Path) -> Result<(), String> {
    use windowcraft_engine::tasks;
    let name = tasks::drift_check_task();
    if !settings.wants_schedule() {
        return tasks::delete(&format!("\\{name}")).map(drop).map_err(|e| e.to_string());
    }
    tasks::register_user_check(&name, description, &program.to_string_lossy(), SCHEDULED_CHECK_ARGUMENT)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_files_give_the_defaults() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(Settings::parse("{not json"), Settings::default());
        assert_eq!(Settings::parse(r#"{"language":"pl"}"#), Settings::default());
    }

    #[test]
    fn a_save_keeps_the_language_and_unknown_keys() {
        let settings = Settings {
            appearance: Appearance::Dark,
            interval: CheckInterval::Weekly,
            after_updates: true,
            last_check: 1_700_000_000,
            last_build: "26300.7171".into(),
            onboarding: Onboarding::quiet(),
        };
        let text = settings.merge_into(r#"{"language":"pl","future":{"x":1}}"#);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["language"], "pl");
        assert_eq!(value["future"]["x"], 1);
        assert_eq!(Settings::parse(&text), settings);
        assert_eq!(crate::i18n::Language::from_tag(value["language"].as_str().unwrap()), Some(crate::i18n::Language::Pl));
    }

    #[test]
    fn onboarding_state_round_trips_and_unreadable_states_stay_quiet() {
        use windowcraft_config::{Answers, Area, Goal};
        assert_eq!(Settings::parse(r#"{"language":"pl"}"#).onboarding, Onboarding::new(), "1.2 files ask the questions");

        let mut settings = Settings::default();
        settings.onboarding.finish(Answers { goal: Some(Goal::Gaming), ..Answers::default() });
        settings.onboarding.tip_seen(Area::Iso);
        let text = settings.merge_into(r#"{"language":"de"}"#);
        assert_eq!(Settings::parse(&text), settings);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["onboarding"]["status"], "finished");
        assert_eq!(value["language"], "de");

        for broken in [r#"{"onboarding":{"version":9}}"#, r#"{"onboarding":{"version":1,"new_field":true}}"#, r#"{"onboarding":"yes"}"#] {
            assert_eq!(Settings::parse(broken).onboarding, Onboarding::quiet(), "{broken}");
        }
    }

    #[test]
    fn save_and_load_use_the_test_file_and_report_a_failed_write() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        use_test_file(Some(file.clone()));
        let mut settings = Settings::default();
        settings.onboarding.skip(windowcraft_config::Answers::default());
        save(&settings).unwrap();
        assert_eq!(load(), settings);
        assert!(!file.with_extension("json.tmp").exists(), "the temporary file is renamed");

        // A file where the folder should be: the folder cannot be created.
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, "").unwrap();
        use_test_file(Some(blocked.join("settings.json")));
        assert!(save(&settings).is_err());
        assert_eq!(load(), Settings::default(), "nothing could be read either");
        use_test_file(None);
    }

    #[test]
    fn a_save_over_a_broken_file_starts_fresh() {
        let text = Settings::default().merge_into("[1, 2]");
        assert_eq!(Settings::parse(&text), Settings::default());
    }

    #[test]
    fn interval_days_round_trip_and_odd_values_snap() {
        for interval in CheckInterval::ALL {
            assert_eq!(CheckInterval::from_days(interval.days()), interval);
        }
        assert_eq!(CheckInterval::from_days(3), CheckInterval::Weekly);
        assert_eq!(CheckInterval::from_days(90), CheckInterval::Monthly);
    }

    #[test]
    fn a_check_is_due_after_the_interval_or_a_new_build() {
        let day = SECONDS_PER_DAY;
        let mut settings = Settings {
            interval: CheckInterval::Weekly,
            last_check: 10 * day,
            last_build: "26300.1".into(),
            ..Settings::default()
        };
        assert!(!settings.check_due(16 * day, "26300.1"));
        assert!(settings.check_due(17 * day, "26300.1"));
        assert!(!settings.check_due(11 * day, "26300.2"), "a new build counts only with after_updates");

        settings.after_updates = true;
        assert!(settings.check_due(11 * day, "26300.2"));
        settings.interval = CheckInterval::Off;
        assert!(!settings.check_due(400 * day, "26300.1"));
        assert!(settings.check_due(400 * day, "26300.2"));
    }

    #[test]
    fn the_first_run_checks_when_anything_is_on() {
        let weekly = Settings { interval: CheckInterval::Weekly, ..Settings::default() };
        assert!(weekly.check_due(1_700_000_000, "26300.1"));
        let updates = Settings { after_updates: true, ..Settings::default() };
        assert!(updates.check_due(1_700_000_000, "26300.1"));
        assert!(!Settings::default().check_due(1_700_000_000, "26300.1"));
        assert!(!Settings::default().wants_schedule());
        assert!(weekly.wants_schedule() && updates.wants_schedule());
    }
}
