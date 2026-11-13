// Made by WindowCraft Dev Team
//! What the first-start questions recorded and which page tips were seen.
//!
//! Parsed and written the same way as [`crate::Profile`]: a version number,
//! no unknown fields, and a typed error for anything else. The application
//! keeps this object under the `onboarding` key of its settings file.

use crate::schema::PresetName;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use thiserror::Error;

pub const ONBOARDING_VERSION: u32 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OnboardingError {
    #[error("the onboarding state is not valid JSON: {0}")]
    Malformed(String),
    #[error("the onboarding state does not match the expected shape: {0}")]
    Shape(String),
    #[error("onboarding state version {0} is newer than this build supports")]
    Version(u32),
}

/// Where the questions stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Not answered yet, so the questions open at the next start.
    #[default]
    Pending,
    /// Answered to the end and started with "Start".
    Finished,
    /// Closed with "Skip" on one of the screens.
    Skipped,
}

/// First question: what the user wants to do with this Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Goal {
    CleanInstall,
    Privacy,
    Gaming,
    Tidy,
}

impl Goal {
    pub const ALL: [Self; 4] = [Self::CleanInstall, Self::Privacy, Self::Gaming, Self::Tidy];
}

/// Last question: how much may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Depth {
    Careful,
    Standard,
    Everything,
}

impl Depth {
    pub const ALL: [Self; 3] = [Self::Careful, Self::Standard, Self::Everything];

    pub fn preset(self) -> PresetName {
        match self {
            Self::Careful => PresetName::Minimal,
            Self::Standard => PresetName::Standard,
            Self::Everything => PresetName::Advanced,
        }
    }
}

/// A page that has its own tip. The names match the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Area {
    Applications,
    Tweaks,
    System,
    Updates,
    Iso,
    TimeMachine,
    Inspector,
    Extensions,
}

impl Area {
    pub const ALL: [Self; 8] = [
        Self::Applications,
        Self::Tweaks,
        Self::System,
        Self::Updates,
        Self::Iso,
        Self::TimeMachine,
        Self::Inspector,
        Self::Extensions,
    ];
}

/// The answers. A question left unanswered stays `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Answers {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<Goal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_files: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_iso: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<Depth>,
}

impl Answers {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// What "Start" selects. Nothing in it changes the system: it fills the
    /// same queues a click on a preset fills.
    pub fn plan(&self) -> StartPlan {
        let depth = self.depth.unwrap_or(Depth::Standard);
        let goal = self.goal;
        StartPlan {
            preset: depth.preset(),
            appx_defaults: goal == Some(Goal::CleanInstall) || depth == Depth::Everything,
            interrupts: goal == Some(Goal::Gaming),
            // Without an answer the cleaner stays on the careful side.
            clean: if self.keep_files == Some(false) { CleanScope::Caches } else { CleanScope::TempOnly },
            iso: self.build_iso == Some(true),
            first: if goal == Some(Goal::Tidy) { Area::Inspector } else { Area::Tweaks },
        }
    }
}

/// What the cleaner ticks after its first scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanScope {
    /// Temporary files, logs and crash dumps.
    TempOnly,
    /// Those, plus GPU shader and developer package caches.
    Caches,
}

/// The selections behind the summary screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartPlan {
    pub preset: PresetName,
    /// Also mark the preset list of preinstalled AppX packages for removal.
    pub appx_defaults: bool,
    /// Point to interrupt priority and MSI mode in the Inspector.
    pub interrupts: bool,
    pub clean: CleanScope,
    pub iso: bool,
    /// The page "Start" opens.
    pub first: Area,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Onboarding {
    pub version: u32,
    #[serde(default)]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Answers::is_empty")]
    pub answers: Answers,
    /// Pages whose tip was closed with "Next".
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub tips_seen: BTreeSet<Area>,
    /// Set by "Don't show again" on any tip.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tips_off: bool,
}

impl Default for Onboarding {
    fn default() -> Self {
        Self::new()
    }
}

impl Onboarding {
    pub fn new() -> Self {
        Self {
            version: ONBOARDING_VERSION,
            status: Status::Pending,
            answers: Answers::default(),
            tips_seen: BTreeSet::new(),
            tips_off: false,
        }
    }

    /// For a state this build cannot read: no questions and no tips, so a
    /// newer or damaged file never brings them back by surprise. Settings
    /// can still turn both on again.
    pub fn quiet() -> Self {
        Self { status: Status::Skipped, tips_off: true, ..Self::new() }
    }

    pub fn parse(source: &str) -> Result<Self, OnboardingError> {
        let value: Value = serde_json::from_str(source).map_err(|e| OnboardingError::Malformed(e.to_string()))?;
        Self::from_value(value)
    }

    pub fn from_value(value: Value) -> Result<Self, OnboardingError> {
        let state: Self = serde_json::from_value(value).map_err(|e| OnboardingError::Shape(e.to_string()))?;
        if state.version > ONBOARDING_VERSION {
            return Err(OnboardingError::Version(state.version));
        }
        Ok(state)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// True while the questions have not been finished or skipped.
    pub fn wants_questions(&self) -> bool {
        self.status == Status::Pending
    }

    pub fn finish(&mut self, answers: Answers) {
        self.answers = answers;
        self.status = Status::Finished;
    }

    /// Keeps what was answered so far, so a later run starts from it.
    pub fn skip(&mut self, answers: Answers) {
        self.answers = answers;
        self.status = Status::Skipped;
    }

    /// "Show the introduction again" in Settings.
    pub fn ask_again(&mut self) {
        self.status = Status::Pending;
    }

    /// Tips start once the questions are out of the way, and each shows once.
    pub fn tip_due(&self, area: Area) -> bool {
        !self.wants_questions() && !self.tips_off && !self.tips_seen.contains(&area)
    }

    pub fn tip_seen(&mut self, area: Area) {
        self.tips_seen.insert(area);
    }

    pub fn tips_never(&mut self) {
        self.tips_off = true;
    }

    /// "Show the tips again" in Settings.
    pub fn reset_tips(&mut self) {
        self.tips_seen.clear();
        self.tips_off = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_state_is_one_short_object_and_round_trips() {
        let state = Onboarding::new();
        assert_eq!(state.to_json(), "{\n  \"version\": 1,\n  \"status\": \"pending\"\n}");
        assert_eq!(Onboarding::parse(&state.to_json()).unwrap(), state);
        assert!(state.wants_questions());
        assert!(Area::ALL.iter().all(|area| !state.tip_due(*area)), "no tips while the questions wait");
    }

    #[test]
    fn a_finished_state_with_tips_round_trips() {
        let mut state = Onboarding::new();
        state.finish(Answers {
            goal: Some(Goal::Gaming),
            keep_files: Some(false),
            build_iso: Some(true),
            depth: Some(Depth::Everything),
        });
        state.tip_seen(Area::TimeMachine);
        state.tip_seen(Area::Applications);
        let json = state.to_json();
        assert!(json.contains("\"goal\": \"gaming\""), "{json}");
        assert!(json.contains("\"time-machine\""), "{json}");
        let parsed = Onboarding::parse(&json).unwrap();
        assert_eq!(parsed, state);
        assert!(!parsed.wants_questions());
        assert!(!parsed.tip_due(Area::TimeMachine));
        assert!(parsed.tip_due(Area::Inspector));
    }

    #[test]
    fn old_and_hand_written_states_load_with_defaults() {
        let state = Onboarding::parse(r#"{"version":1}"#).unwrap();
        assert_eq!(state, Onboarding::new());
        let skipped = Onboarding::parse(r#"{"version":1,"status":"skipped","answers":{"goal":"tidy"}}"#).unwrap();
        assert_eq!(skipped.status, Status::Skipped);
        assert_eq!(skipped.answers.goal, Some(Goal::Tidy));
        assert_eq!(skipped.answers.depth, None);
    }

    #[test]
    fn rejects_broken_json_unknown_fields_and_newer_versions() {
        assert!(matches!(Onboarding::parse("{not json"), Err(OnboardingError::Malformed(_))));
        assert!(matches!(Onboarding::parse("[]"), Err(OnboardingError::Shape(_))));
        assert!(matches!(Onboarding::parse(r#"{"version":1,"extra":1}"#), Err(OnboardingError::Shape(_))));
        assert!(matches!(Onboarding::parse(r#"{"version":1,"status":"done"}"#), Err(OnboardingError::Shape(_))));
        assert!(matches!(Onboarding::parse(r#"{"version":1,"tips_seen":["moon"]}"#), Err(OnboardingError::Shape(_))));
        assert_eq!(Onboarding::parse(r#"{"version":2}"#), Err(OnboardingError::Version(2)));
    }

    #[test]
    fn skipping_keeps_the_answers_and_asking_again_reopens() {
        let mut state = Onboarding::new();
        let partial = Answers { goal: Some(Goal::Privacy), ..Answers::default() };
        state.skip(partial);
        assert_eq!(state.status, Status::Skipped);
        assert_eq!(state.answers, partial);
        assert!(!state.wants_questions());
        assert!(state.tip_due(Area::Tweaks), "tips follow a skip too");

        state.ask_again();
        assert!(state.wants_questions());
        assert_eq!(state.answers, partial, "the earlier answers stay for the next run");
    }

    #[test]
    fn each_tip_shows_once_and_never_or_reset_apply_to_all() {
        let mut state = Onboarding::new();
        state.finish(Answers::default());
        for area in Area::ALL {
            assert!(state.tip_due(area));
            state.tip_seen(area);
            assert!(!state.tip_due(area));
        }
        state.reset_tips();
        assert!(Area::ALL.iter().all(|area| state.tip_due(*area)));

        state.tip_seen(Area::Iso);
        state.tips_never();
        assert!(Area::ALL.iter().all(|area| !state.tip_due(*area)));
        let parsed = Onboarding::parse(&state.to_json()).unwrap();
        assert!(parsed.tips_off);

        state.reset_tips();
        assert!(!state.tips_off && state.tips_seen.is_empty());
    }

    #[test]
    fn the_quiet_state_asks_nothing_and_shows_no_tips() {
        let state = Onboarding::quiet();
        assert!(!state.wants_questions());
        assert!(Area::ALL.iter().all(|area| !state.tip_due(*area)));
    }

    #[test]
    fn the_plan_follows_the_answers() {
        let none = Answers::default().plan();
        assert_eq!(none.preset, PresetName::Standard);
        assert!(!none.appx_defaults && !none.interrupts && !none.iso);
        assert_eq!(none.clean, CleanScope::TempOnly);
        assert_eq!(none.first, Area::Tweaks);

        let clean = Answers { goal: Some(Goal::CleanInstall), depth: Some(Depth::Careful), ..Answers::default() }.plan();
        assert_eq!(clean.preset, PresetName::Minimal);
        assert!(clean.appx_defaults);

        let gaming = Answers {
            goal: Some(Goal::Gaming),
            keep_files: Some(false),
            build_iso: Some(true),
            depth: Some(Depth::Everything),
        }
        .plan();
        assert_eq!(gaming.preset, PresetName::Advanced);
        assert!(gaming.appx_defaults && gaming.interrupts && gaming.iso);
        assert_eq!(gaming.clean, CleanScope::Caches);

        let tidy = Answers { goal: Some(Goal::Tidy), keep_files: Some(true), ..Answers::default() }.plan();
        assert_eq!(tidy.first, Area::Inspector);
        assert_eq!(tidy.clean, CleanScope::TempOnly);
        assert!(!tidy.appx_defaults);
    }
}
