// Made by ycangignacy
//! Applies, detects and reverts catalog entries transactionally.

use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use crate::registry::{RegPath, RegTarget, RegValue, RegistryScope, resolve_value};
use crate::services::{ServiceBackend, guard_change};
use crate::snapshot::{AppliedState, Change, ChangeRecord, FeatureStep, SessionState, SnapshotStore, unix_now};

/// Description prefix of a feature step in the timeline.
const FEATURE_STEP: &str = "Enable Windows feature ";
use std::sync::Arc;
#[cfg(any(test, feature = "testing"))]
use std::sync::Mutex;
use windowcraft_config::{Catalog, Feature, NativeOp, RegistryEntry, ServiceRule, StartupType, Tweak, TweakKind};

/// Runs non-registry operations. The real implementation lives in `native`;
/// tests substitute a recorder that can inject failures.
pub trait NativeRunner: Send + Sync {
    fn run(&self, op: &NativeOp, cancel: &CancelToken) -> Result<()>;
    fn enable_feature(&self, name: &str, cancel: &CancelToken) -> Result<()>;
    fn disable_feature(&self, name: &str, cancel: &CancelToken) -> Result<()>;
    fn feature_enabled(&self, name: &str) -> Result<bool>;
}

/// Records operations without touching the system.
#[cfg(any(test, feature = "testing"))]
#[derive(Debug, Default)]
pub struct RecordingRunner {
    pub ran: Mutex<Vec<NativeOp>>,
    pub features: Mutex<Vec<String>>,
    pub disabled_features: Mutex<Vec<String>>,
    /// Features that report as already on.
    pub features_on: Mutex<Vec<String>>,
    pub fail_on: Mutex<Option<NativeOp>>,
    pub feature_query_fails: std::sync::atomic::AtomicBool,
}

#[cfg(any(test, feature = "testing"))]
impl NativeRunner for RecordingRunner {
    fn run(&self, op: &NativeOp, _cancel: &CancelToken) -> Result<()> {
        if self.fail_on.lock().is_ok_and(|fail| fail.as_ref() == Some(op)) {
            return Err(EngineError::Process { program: "injected".into(), code: 1, detail: "fault".into() });
        }
        if let Ok(mut ran) = self.ran.lock() {
            ran.push(op.clone());
        }
        Ok(())
    }

    fn enable_feature(&self, name: &str, _cancel: &CancelToken) -> Result<()> {
        if let Ok(mut features) = self.features.lock() {
            features.push(name.to_owned());
        }
        Ok(())
    }

    fn disable_feature(&self, name: &str, _cancel: &CancelToken) -> Result<()> {
        if let Ok(mut disabled) = self.disabled_features.lock() {
            disabled.push(name.to_owned());
        }
        Ok(())
    }

    fn feature_enabled(&self, name: &str) -> Result<bool> {
        if self.feature_query_fails.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(EngineError::Invalid("injected".into()));
        }
        Ok(self.features_on.lock().is_ok_and(|on| on.iter().any(|feature| feature == name)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TweakAction {
    /// Queue-style apply for check tweaks and actions.
    Apply,
    /// Instant toggle.
    Set(bool),
    /// Segmented control option.
    Select(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detection {
    Applied,
    Partial { matched: usize, total: usize },
    NotApplied,
    /// Nothing about this entry can be compared with the live system.
    Unknown,
}

/// One row of the X-Ray Inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectRow {
    pub target: String,
    pub kind: String,
    pub current: String,
    pub desired: String,
    pub undo: String,
    pub matches: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub tweak_id: String,
    pub tweak_name: String,
    /// Target, expected value, live value.
    pub entries: Vec<(String, String, String)>,
    /// The OS build changed after this tweak was applied.
    pub after_update: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RunReport {
    pub session: u64,
    pub applied: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub cancelled: bool,
}

pub struct Engine {
    pub scope: RegistryScope,
    pub services: Arc<dyn ServiceBackend>,
    pub native: Arc<dyn NativeRunner>,
    pub store: Arc<SnapshotStore>,
    pub catalog: Arc<Catalog>,
    pub ram_kb: u64,
    pub os_build: String,
}

fn value_text(value: Option<&RegValue>) -> String {
    value.map_or_else(|| "(not set)".to_owned(), RegValue::display)
}

fn startup_text(value: Option<StartupType>) -> String {
    value.map_or_else(|| "(absent)".to_owned(), |startup| startup.label().to_owned())
}

/// Picks the text for an entry under a given action.
fn desired_text<'a>(entry: &'a RegistryEntry, action: &TweakAction) -> Option<&'a str> {
    match action {
        TweakAction::Apply | TweakAction::Set(true) => entry.value.as_deref(),
        TweakAction::Set(false) => entry.original.as_deref(),
        TweakAction::Select(option) => {
            entry.values.as_ref().and_then(|values| values.get(option)).map(String::as_str)
        }
    }
}

/// True when a live value, or its absence, equals the wanted value. A missing
/// value stands for `absent`, the state `default_on` names, when there is one.
fn same_value(live: Option<&RegValue>, want: Option<&RegValue>, absent: Option<Option<&RegValue>>) -> bool {
    match (live, want) {
        (Some(live), Some(want)) => live.matches(want),
        (None, None) => true,
        (None, Some(want)) => absent.flatten().is_some_and(|value| value.matches(want)),
        (Some(_), None) => false,
    }
}

impl Engine {
    /// The value a missing entry stands for: the "on" value when `default_on`
    /// is true, the "off" value when it is false, nothing when it is unset.
    fn absent_value(&self, entry: &RegistryEntry) -> Result<Option<Option<RegValue>>> {
        let Some(on) = entry.default_on else { return Ok(None) };
        let text = if on { entry.value.as_deref() } else { entry.original.as_deref() };
        Ok(Some(resolve_value(entry, text, self.ram_kb)?.value))
    }

    fn registry_targets(&self, entries: &[RegistryEntry], action: &TweakAction) -> Result<Vec<RegTarget>> {
        entries.iter().map(|entry| resolve_value(entry, desired_text(entry, action), self.ram_kb)).collect()
    }

    fn service_target(rule: &ServiceRule, action: &TweakAction) -> StartupType {
        if matches!(action, TweakAction::Set(false)) { rule.original } else { rule.startup }
    }

    /// Compares a tweak with the live system.
    pub fn detect(&self, tweak: &Tweak) -> Result<Detection> {
        if !tweak.is_detectable() {
            return Ok(Detection::Unknown);
        }
        if tweak.kind == TweakKind::Segmented {
            return Ok(if self.current_option(tweak)?.is_some() { Detection::Applied } else { Detection::NotApplied });
        }

        let (mut matched, mut total) = (0, 0);
        for (entry, target) in tweak.registry.iter().zip(self.registry_targets(&tweak.registry, &TweakAction::Apply)?) {
            total += 1;
            let live = self.scope.read(&target.path, &target.name)?;
            let absent = self.absent_value(entry)?;
            let same = same_value(live.as_ref(), target.value.as_ref(), absent.as_ref().map(Option::as_ref));
            matched += usize::from(same);
        }
        for rule in &tweak.services {
            if let Some(info) = self.services.query(&rule.name)? {
                total += 1;
                matched += usize::from(info.startup == rule.startup);
            }
        }

        Ok(match matched {
            0 => Detection::NotApplied,
            n if n == total => Detection::Applied,
            n => Detection::Partial { matched: n, total },
        })
    }

    /// Finds which segmented option the live registry corresponds to.
    pub fn current_option(&self, tweak: &Tweak) -> Result<Option<String>> {
        for option in &tweak.options {
            let action = TweakAction::Select(option.label.clone());
            let mut all = true;
            for target in self.registry_targets(&tweak.registry, &action)? {
                let live = self.scope.read(&target.path, &target.name)?;
                all &= match (&live, &target.value) {
                    (Some(live), Some(want)) => live.matches(want),
                    (None, None) => true,
                    _ => false,
                };
            }
            if all {
                return Ok(Some(option.label.clone()));
            }
        }
        Ok(None)
    }

    /// Current, target and undo values for every target of a tweak.
    pub fn inspect(&self, tweak: &Tweak, action: &TweakAction) -> Result<Vec<InspectRow>> {
        let snapshot = self.undo_values(&tweak.id)?;
        let mut rows = Vec::new();
        for (entry, target) in tweak.registry.iter().zip(self.registry_targets(&tweak.registry, action)?) {
            let live = self.scope.read(&target.path, &target.name)?;
            let key = format!("{}\\{}", entry.path, entry.name);
            let undo = snapshot.as_ref().and_then(|values| values.iter().find(|(k, _)| *k == key)).map_or_else(
                || match entry.original.as_deref() {
                    None | Some(windowcraft_config::REMOVE_ENTRY) => "(remove value)".to_owned(),
                    Some(text) => text.to_owned(),
                },
                |(_, value)| value.clone(),
            );
            let matches = match (&live, &target.value) {
                (Some(live), Some(want)) => live.matches(want),
                (None, None) => true,
                _ => false,
            };
            rows.push(InspectRow {
                target: key,
                kind: format!("{:?}", entry.kind),
                current: value_text(live.as_ref()),
                desired: target.value.as_ref().map_or_else(|| "(remove value)".to_owned(), RegValue::display),
                undo,
                matches,
            });
        }
        for rule in &tweak.services {
            let live = self.services.query(&rule.name)?.map(|info| info.startup);
            let desired = Self::service_target(rule, action);
            rows.push(InspectRow {
                target: format!("Service {}", rule.name),
                kind: "StartupType".to_owned(),
                current: startup_text(live),
                desired: desired.label().to_owned(),
                undo: rule.original.label().to_owned(),
                matches: live == Some(desired),
            });
        }
        Ok(rows)
    }

    /// True pre-tweak values, from the first apply in an unbroken chain.
    fn undo_values(&self, tweak_id: &str) -> Result<Option<Vec<(String, String)>>> {
        let Some(state) = self.store.applied_state(tweak_id)? else {
            return Ok(None);
        };
        let records = self.store.changes(state.undo_session())?;
        let mut values = Vec::new();
        for record in records.iter().filter(|record| record.owner == tweak_id) {
            if let Change::Registry { path, name, before, .. } = &record.change {
                values.push((format!("{path}\\{name}"), value_text(before.as_ref())));
            }
        }
        Ok(Some(values))
    }

    /// Applies one tweak inside an open session. On failure every change it
    /// made is reverted before the error returns.
    pub fn apply_tweak(&self, session: u64, tweak: &Tweak, action: &TweakAction, cancel: &CancelToken) -> Result<()> {
        // Refuse protected services before anything is written.
        for rule in &tweak.services {
            guard_change(&rule.name, Self::service_target(rule, action))?;
        }

        let targets = self.registry_targets(&tweak.registry, action)?;
        let mut changes = Vec::with_capacity(targets.len() + tweak.services.len());
        for target in &targets {
            changes.push(Change::Registry {
                path: target.path.display(),
                name: target.name.clone(),
                before: self.scope.read(&target.path, &target.name)?,
                after: target.value.clone(),
            });
        }
        let mut service_targets = Vec::new();
        for rule in &tweak.services {
            match self.services.query(&rule.name)? {
                Some(info) => {
                    let after = Self::service_target(rule, action);
                    changes.push(Change::Service { name: rule.name.clone(), before: Some(info.startup), after });
                    service_targets.push((rule.name.clone(), after));
                }
                None if rule.optional => {}
                None => return Err(EngineError::NotFound(format!("service {}", rule.name))),
            }
        }
        let native_ops = self.native_ops(tweak, action);
        for op in &native_ops {
            changes.push(Change::native(op.describe()));
        }

        // Write-ahead: the undo record is durable before the first mutation.
        let indexes = self.store.record(session, &tweak.id, &changes)?;

        let outcome = self.mutate(tweak, action, &targets, &service_targets, &native_ops, cancel);
        if let Err(error) = outcome {
            let records: Vec<ChangeRecord> = self
                .store
                .changes(session)?
                .into_iter()
                .filter(|record| indexes.contains(&record.index))
                .collect();
            self.revert_records(session, &records)?;
            return Err(error);
        }

        if self.tracks_drift(tweak, action) {
            // Applying twice must not forget what was there before the first
            // time, or undo would restore an already-tweaked value.
            let baseline_session = self.store.applied_state(&tweak.id)?.map_or(session, |state| state.undo_session());
            let mut registry = Vec::with_capacity(targets.len());
            for target in &targets {
                registry.push((target.path.display(), target.name.clone(), target.value.clone()));
            }
            self.store.set_applied(&AppliedState {
                tweak_id: tweak.id.clone(),
                session,
                baseline_session,
                applied_unix: unix_now(),
                os_build: self.os_build.clone(),
                registry,
                services: service_targets,
            })?;
        } else {
            self.store.clear_applied(&tweak.id)?;
        }
        Ok(())
    }

    fn tracks_drift(&self, tweak: &Tweak, action: &TweakAction) -> bool {
        tweak.is_detectable() && !matches!(action, TweakAction::Set(false))
    }

    fn prepare_count(tweak: &Tweak, action: &TweakAction) -> usize {
        if matches!(action, TweakAction::Set(false)) { 0 } else { tweak.prepare.len() }
    }

    fn native_ops<'a>(&self, tweak: &'a Tweak, action: &TweakAction) -> Vec<&'a NativeOp> {
        let main = if matches!(action, TweakAction::Set(false)) { &tweak.undo } else { &tweak.apply };
        tweak.prepare.iter().take(Self::prepare_count(tweak, action)).chain(main).collect()
    }

    fn mutate(
        &self,
        tweak: &Tweak,
        action: &TweakAction,
        targets: &[RegTarget],
        services: &[(String, StartupType)],
        native_ops: &[&NativeOp],
        cancel: &CancelToken,
    ) -> Result<()> {
        // `prepare` steps clear keys that registry writes then repopulate.
        let prepare_count = Self::prepare_count(tweak, action);
        for op in &native_ops[..prepare_count] {
            cancel.check()?;
            self.run_op(op, cancel)?;
        }
        for target in targets {
            cancel.check()?;
            self.scope.write_opt(&target.path, &target.name, target.value.as_ref())?;
        }
        for (name, startup) in services {
            cancel.check()?;
            self.services.set_startup(name, *startup)?;
        }
        for op in &native_ops[prepare_count..] {
            cancel.check()?;
            self.run_op(op, cancel)?;
        }
        Ok(())
    }

    /// Enables a feature set inside its own snapshot session.
    pub fn apply_feature(&self, feature: &Feature, option: Option<&str>, cancel: &CancelToken) -> Result<u64> {
        let session = self.store.begin_session(&feature.name)?;
        let outcome = self.feature_inner(session, feature, option, cancel);
        let state = if outcome.is_ok() { SessionState::Committed } else { SessionState::Failed };
        self.store.finish_session(session, state)?;
        outcome.map(|()| session)
    }

    fn feature_inner(
        &self,
        session: u64,
        feature: &Feature,
        option: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<()> {
        let targets = self.registry_targets(&feature.registry, &TweakAction::Apply)?;
        let mut changes = Vec::with_capacity(targets.len());
        for target in &targets {
            changes.push(Change::Registry {
                path: target.path.display(),
                name: target.name.clone(),
                before: self.scope.read(&target.path, &target.name)?,
                after: target.value.clone(),
            });
        }
        for name in &feature.features {
            // A feature that was already on stays on after a rollback. When
            // the state cannot be read, it counts as on: turning off something
            // the user had before is worse than leaving one feature enabled.
            let was_on = self.native.feature_enabled(name).unwrap_or(true);
            changes.push(Change::Native {
                description: format!("{FEATURE_STEP}{name}"),
                feature: Some(FeatureStep { name: name.clone(), was_on }),
            });
        }
        for op in &feature.apply {
            changes.push(Change::native(op.describe()));
        }
        self.store.record(session, &feature.id, &changes)?;

        // One missing component must not cost the rest of the set, so the
        // first error is reported only after every feature was attempted.
        let mut first_error = None;
        for name in &feature.features {
            cancel.check()?;
            if let Err(error) = self.native.enable_feature(name, cancel) {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        for target in &targets {
            self.scope.write_opt(&target.path, &target.name, target.value.as_ref())?;
        }
        for op in &feature.apply {
            cancel.check()?;
            // The aggressive reset is an option on the same catalog entry.
            let selected = match op {
                NativeOp::FixWindowsUpdate { .. } => {
                    NativeOp::FixWindowsUpdate { aggressive: option == Some("Aggressive") }
                }
                other => other.clone(),
            };
            self.native.run(&selected, cancel)?;
        }
        Ok(())
    }

    /// Runs one recorded change in its own session, so one-off controls such
    /// as MSI mode still appear in the timeline and can be rolled back.
    pub fn tracked(&self, label: &str, owner: &str, change: Change, work: impl FnOnce() -> Result<()>) -> Result<u64> {
        let session = self.store.begin_session(label)?;
        self.store.record(session, owner, &[change])?;
        let outcome = work();
        let state = if outcome.is_ok() { SessionState::Committed } else { SessionState::Failed };
        self.store.finish_session(session, state)?;
        outcome.map(|()| session)
    }

    /// Writes one registry value with a snapshot of the previous one.
    pub fn write_tracked(&self, label: &str, owner: &str, path: &RegPath, name: &str, value: RegValue) -> Result<u64> {
        let change = Change::Registry {
            path: path.display(),
            name: name.to_owned(),
            before: self.scope.read(path, name)?,
            after: Some(value.clone()),
        };
        self.tracked(label, owner, change, || self.scope.write(path, name, &value))
    }

    /// Runs a batch under a new session. One failure does not stop the rest.
    pub fn run(
        &self,
        label: &str,
        items: &[(&Tweak, TweakAction)],
        cancel: &CancelToken,
        mut progress: impl FnMut(usize, &str),
    ) -> Result<RunReport> {
        let session = self.store.begin_session(label)?;
        let mut report = RunReport { session, ..RunReport::default() };
        for (index, (tweak, action)) in items.iter().enumerate() {
            if cancel.is_cancelled() {
                report.cancelled = true;
                break;
            }
            progress(index, &tweak.name);
            match self.apply_tweak(session, tweak, action, cancel) {
                Ok(()) => report.applied.push(tweak.id.clone()),
                Err(EngineError::Cancelled) => {
                    report.cancelled = true;
                    break;
                }
                Err(error) => report.failed.push((tweak.id.clone(), error.to_string())),
            }
        }
        let state = if report.applied.is_empty() && !report.failed.is_empty() {
            SessionState::Failed
        } else {
            SessionState::Committed
        };
        self.store.finish_session(session, state)?;
        Ok(report)
    }

    /// Restores recorded values in reverse order.
    pub(crate) fn revert_records(&self, session: u64, records: &[ChangeRecord]) -> Result<()> {
        let mut first_error = None;
        let mut reverted = Vec::with_capacity(records.len());
        for record in records.iter().rev().filter(|record| !record.rolled_back) {
            let outcome = match &record.change {
                Change::Registry { path, name, before, .. } => crate::registry::RegPath::parse(path)
                    .and_then(|path| self.scope.write_opt(&path, name, before.as_ref())),
                Change::Service { name, before: Some(before), .. } => self.services.set_startup(name, *before),
                Change::Native { feature: Some(FeatureStep { name, was_on: false }), .. } => {
                    self.native.disable_feature(name, &CancelToken::new())
                }
                Change::Service { .. } | Change::Native { .. } => Ok(()),
                Change::Task { path, before, .. } => crate::tasks::set_enabled(path, *before),
            };
            match outcome {
                Ok(()) => reverted.push(record.index),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        self.store.mark_rolled_back(session, &reverted)?;
        first_error.map_or(Ok(()), Err)
    }

    /// Runs one catalog operation. Registry trees go through `scope` like
    /// every other registry write, so a sandbox keeps them inside it.
    fn run_op(&self, op: &NativeOp, cancel: &CancelToken) -> Result<()> {
        match op {
            NativeOp::RegDeleteTree { path } => self.scope.delete_tree(&RegPath::parse(path)?),
            other => self.native.run(other, cancel),
        }
    }

    /// Runs the catalog `undo` steps after `records` were reverted. A tree
    /// deletion is skipped when it holds a value those records just put back,
    /// because the value existed before the tweak and has to stay.
    fn run_undo_ops(&self, owner: &str, records: &[ChangeRecord]) -> Result<()> {
        let tweak = self.catalog.tweak(owner).map(|tweak| &tweak.undo);
        let undo = tweak.or_else(|| self.catalog.feature(owner).map(|feature| &feature.undo));
        let restored: Vec<String> = records
            .iter()
            .filter(|record| record.owner == owner)
            .filter_map(|record| match &record.change {
                Change::Registry { path, before: Some(_), .. } => Some(format!("{}\\", path.to_lowercase())),
                _ => None,
            })
            .collect();
        for op in undo.into_iter().flatten() {
            if let NativeOp::RegDeleteTree { path } = op {
                let tree = format!("{}\\", RegPath::parse(path)?.display().to_lowercase());
                if restored.iter().any(|key| key.starts_with(&tree)) {
                    continue;
                }
            }
            self.run_op(op, &CancelToken::new())?;
        }
        Ok(())
    }

    /// Reverts a whole session, newest change first.
    pub fn rollback_session(&self, session: u64) -> Result<()> {
        let records = self.store.changes(session)?;
        let partial = !self.irreversible_in_session(session)?.is_empty();
        self.revert_records(session, &records)?;
        let mut owners: Vec<&str> = records.iter().map(|record| record.owner.as_str()).collect();
        owners.dedup();
        for owner in owners {
            self.run_undo_ops(owner, &records)?;
            if let Some(mut state) = self.store.applied_state(owner)? {
                if state.undo_session() == session {
                    self.store.clear_applied(owner)?;
                } else if state.session == session {
                    // Only a re-apply was reverted, so the tweak is still on.
                    state.session = state.undo_session();
                    self.store.set_applied(&state)?;
                }
            }
        }
        let state = if partial { SessionState::PartlyRolledBack } else { SessionState::RolledBack };
        self.store.finish_session(session, state)
    }

    /// Names of tweaks in a session that a rollback cannot fully restore.
    pub fn irreversible_in_session(&self, session: u64) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for record in self.store.changes(session)? {
            let native = matches!(record.change, Change::Native { .. });
            let kept = if let Some(tweak) = self.catalog.tweak(&record.owner) {
                let no_undo = native && tweak.undo.is_empty() && !tweak.apply.is_empty();
                (tweak.irreversible || no_undo).then_some(&tweak.name)
            } else if let Some(feature) = self.catalog.feature(&record.owner) {
                // 0.1.0 recorded feature steps without their earlier state,
                // so a rollback of those sessions cannot turn the feature off.
                let no_undo = match &record.change {
                    Change::Native { feature: Some(_), .. } => false,
                    Change::Native { description, .. } if description.starts_with(FEATURE_STEP) => true,
                    Change::Native { .. } => feature.undo.is_empty(),
                    _ => false,
                };
                (feature.irreversible || no_undo).then_some(&feature.name)
            } else {
                None
            };
            if let Some(name) = kept
                && !names.contains(name)
            {
                names.push(name.clone());
            }
        }
        Ok(names)
    }

    /// Reverts a single key or service from a session.
    pub fn rollback_change(&self, session: u64, index: u32) -> Result<()> {
        let records = self.store.changes(session)?;
        let record: Vec<ChangeRecord> = records.into_iter().filter(|record| record.index == index).collect();
        if record.is_empty() {
            return Err(EngineError::NotFound(format!("change {index} in session {session}")));
        }
        self.revert_records(session, &record)
    }

    /// Reverts a tweak using its snapshot, or the catalog when none exists.
    pub fn undo_tweak(&self, tweak: &Tweak) -> Result<()> {
        if tweak.irreversible {
            return Err(EngineError::Unsupported(format!("{} cannot be undone automatically", tweak.name)));
        }
        if let Some(state) = self.store.applied_state(&tweak.id)? {
            let session = state.undo_session();
            let records: Vec<ChangeRecord> =
                self.store.changes(session)?.into_iter().filter(|record| record.owner == tweak.id).collect();
            self.revert_records(session, &records)?;
            self.run_undo_ops(&tweak.id, &records)?;
            return self.store.clear_applied(&tweak.id);
        }

        let session = self.store.begin_session(&format!("Undo {}", tweak.name))?;
        let outcome = self.apply_tweak(session, tweak, &TweakAction::Set(false), &CancelToken::new());
        let state = if outcome.is_ok() { SessionState::Committed } else { SessionState::Failed };
        self.store.finish_session(session, state)?;
        outcome
    }

    /// Finds applied tweaks whose values no longer hold.
    pub fn detect_drift(&self) -> Result<Vec<Drift>> {
        let mut drifted = Vec::new();
        for state in self.store.applied()? {
            let mut entries = Vec::new();
            let tweak = self.catalog.tweak(&state.tweak_id);
            for (path, name, expected) in &state.registry {
                let parsed = crate::registry::RegPath::parse(path)?;
                let live = self.scope.read(&parsed, name)?;
                let entry = tweak.and_then(|tweak| {
                    tweak.registry.iter().find(|entry| {
                        entry.name.eq_ignore_ascii_case(name)
                            && RegPath::parse(&entry.path).is_ok_and(|p| p.display().eq_ignore_ascii_case(path))
                    })
                });
                let absent = match entry {
                    Some(entry) => self.absent_value(entry)?,
                    None => None,
                };
                let same = same_value(live.as_ref(), expected.as_ref(), absent.as_ref().map(Option::as_ref));
                if !same {
                    entries.push((format!("{path}\\{name}"), value_text(expected.as_ref()), value_text(live.as_ref())));
                }
            }
            for (name, expected) in &state.services {
                if let Some(info) = self.services.query(name)?
                    && info.startup != *expected
                {
                    entries.push((format!("Service {name}"), expected.label().to_owned(), info.startup.label().to_owned()));
                }
            }
            if !entries.is_empty() {
                let tweak_name =
                    self.catalog.tweak(&state.tweak_id).map_or_else(|| state.tweak_id.clone(), |tweak| tweak.name.clone());
                drifted.push(Drift {
                    tweak_id: state.tweak_id.clone(),
                    tweak_name,
                    entries,
                    after_update: state.os_build != self.os_build,
                });
            }
        }
        Ok(drifted)
    }

    /// Re-applies drifted values. The original session stays the undo source,
    /// so a later rollback still reaches the true pre-tweak state.
    pub fn repair_drift(&self) -> Result<usize> {
        let drifted = self.detect_drift()?;
        if drifted.is_empty() {
            return Ok(0);
        }
        let session = self.store.begin_session(&format!("Re-apply {} drifted tweaks", drifted.len()))?;
        let mut repaired = 0;
        for drift in &drifted {
            let Some(state) = self.store.applied_state(&drift.tweak_id)? else { continue };
            let audit = Change::native(format!("Re-applied {}", drift.tweak_name));
            self.store.record(session, &drift.tweak_id, &[audit])?;
            for (path, name, expected) in &state.registry {
                let parsed = crate::registry::RegPath::parse(path)?;
                self.scope.write_opt(&parsed, name, expected.as_ref())?;
            }
            for (name, expected) in &state.services {
                guard_change(name, *expected)?;
                if self.services.query(name)?.is_some() {
                    self.services.set_startup(name, *expected)?;
                }
            }
            repaired += 1;
        }
        self.store.finish_session(session, SessionState::Committed)?;
        Ok(repaired)
    }

    /// Banner text for the drift guard.
    pub fn drift_summary(drifted: &[Drift]) -> Option<String> {
        if drifted.is_empty() {
            return None;
        }
        let count = drifted.len();
        let noun = if count == 1 { "tweak was" } else { "tweaks were" };
        let cause = if drifted.iter().any(|drift| drift.after_update) {
            "by a recent Windows Update"
        } else {
            "since it was applied"
        };
        Some(format!("{count} applied {noun} reset {cause}"))
    }
}
