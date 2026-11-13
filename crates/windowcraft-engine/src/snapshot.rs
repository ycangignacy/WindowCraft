// Made by WindowCraft Dev Team
//! Transactional snapshot store backed by redb.
//!
//! The pre-change state of every target is committed before the first
//! mutation, so a crash or power loss mid-apply still leaves a usable undo.

use crate::error::Result;
use crate::registry::RegValue;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use windowcraft_config::StartupType;

const SESSIONS: TableDefinition<u64, &str> = TableDefinition::new("sessions");
const CHANGES: TableDefinition<(u64, u32), &str> = TableDefinition::new("changes");
const APPLIED: TableDefinition<&str, &str> = TableDefinition::new("applied");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    Open,
    Committed,
    RolledBack,
    Failed,
    /// Rolled back, but some native steps have no undo.
    PartlyRolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: u64,
    pub created_unix: u64,
    pub label: String,
    pub state: SessionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Change {
    Registry { path: String, name: String, before: Option<RegValue>, after: Option<RegValue> },
    Service { name: String, before: Option<StartupType>, after: StartupType },
    Task { path: String, before: bool, after: bool },
    Native {
        description: String,
        /// Set when this step enables a Windows feature. Absent for every
        /// other native step and for feature steps written by 0.1.0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        feature: Option<FeatureStep>,
    },
}

/// State of one Windows feature before WindowCraft enabled it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureStep {
    pub name: String,
    /// A rollback turns the feature off only when it was off before.
    pub was_on: bool,
}

impl Change {
    /// A native step that a rollback cannot undo by itself.
    pub fn native(description: impl Into<String>) -> Self {
        Self::Native { description: description.into(), feature: None }
    }

    pub fn target(&self) -> String {
        match self {
            Self::Registry { path, name, .. } => format!("{path}\\{name}"),
            Self::Service { name, .. } => format!("Service {name}"),
            Self::Task { path, .. } => format!("Task {path}"),
            Self::Native { description, .. } => description.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub session: u64,
    pub index: u32,
    /// Catalog id of the tweak that made this change.
    pub owner: String,
    pub change: Change,
    pub rolled_back: bool,
}

/// What a tweak left on the system, used to detect drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedState {
    pub tweak_id: String,
    pub session: u64,
    /// Session of the first apply. Re-applying records "tweaked to tweaked",
    /// so the true pre-tweak values only exist here.
    #[serde(default)]
    pub baseline_session: u64,
    pub applied_unix: u64,
    pub os_build: String,
    pub registry: Vec<(String, String, Option<RegValue>)>,
    pub services: Vec<(String, StartupType)>,
}

impl AppliedState {
    /// The session whose `before` values are the real originals.
    pub fn undo_session(&self) -> u64 {
        if self.baseline_session == 0 { self.session } else { self.baseline_session }
    }
}

pub struct SnapshotStore {
    database: Database,
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

/// Only real damage moves the history aside. A format upgrade, an aborted
/// repair or a plain I/O failure leaves the file in place and reports it.
fn is_damage(error: &redb::DatabaseError) -> bool {
    match error {
        redb::DatabaseError::Storage(redb::StorageError::Corrupted(_)) => true,
        // redb reports a bad file header as InvalidData.
        redb::DatabaseError::Storage(redb::StorageError::Io(error)) => error.kind() == std::io::ErrorKind::InvalidData,
        _ => false,
    }
}

impl SnapshotStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| crate::error::EngineError::io(parent.display().to_string(), e))?;
        }
        let database = match Database::create(path) {
            Ok(database) => database,
            Err(redb::DatabaseError::DatabaseAlreadyOpen) => return Err(crate::error::EngineError::StoreBusy),
            Err(redb::DatabaseError::UpgradeRequired(_)) => {
                return Err(crate::error::EngineError::Store(format!(
                    "{} was written by a different WindowCraft version and was left in place",
                    path.display()
                )));
            }
            Err(error) if is_damage(&error) => {
                // A damaged history must not stop the app from starting. The
                // file is kept beside the new one for manual recovery.
                let aside = path.with_extension(format!("corrupt-{}", unix_now()));
                std::fs::rename(path, &aside)
                    .map_err(|e| crate::error::EngineError::io(path.display().to_string(), e))?;
                Database::create(path)?
            }
            Err(redb::DatabaseError::Storage(redb::StorageError::Io(error))) => {
                return Err(crate::error::EngineError::io(path.display().to_string(), error));
            }
            Err(error) => return Err(error.into()),
        };
        let transaction = database.begin_write()?;
        transaction.open_table(SESSIONS)?;
        transaction.open_table(CHANGES)?;
        transaction.open_table(APPLIED)?;
        transaction.commit()?;
        Ok(Self { database })
    }

    pub fn begin_session(&self, label: &str) -> Result<u64> {
        let transaction = self.database.begin_write()?;
        let id;
        {
            let mut table = transaction.open_table(SESSIONS)?;
            id = table.last()?.map_or(1, |(key, _)| key.value() + 1);
            let session = Session { id, created_unix: unix_now(), label: label.to_owned(), state: SessionState::Open };
            table.insert(id, serde_json::to_string(&session)?.as_str())?;
        }
        transaction.commit()?;
        Ok(id)
    }

    pub fn finish_session(&self, id: u64, state: SessionState) -> Result<()> {
        let transaction = self.database.begin_write()?;
        {
            let mut table = transaction.open_table(SESSIONS)?;
            let current = table.get(id)?.map(|value| value.value().to_owned());
            if let Some(text) = current {
                let mut session: Session = serde_json::from_str(&text)?;
                session.state = state;
                table.insert(id, serde_json::to_string(&session)?.as_str())?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Appends a batch of changes atomically. Call before mutating anything.
    pub fn record(&self, session: u64, owner: &str, changes: &[Change]) -> Result<Vec<u32>> {
        let transaction = self.database.begin_write()?;
        let mut indexes = Vec::with_capacity(changes.len());
        {
            let mut table = transaction.open_table(CHANGES)?;
            let first = table
                .range((session, 0)..=(session, u32::MAX))?
                .next_back()
                .transpose()?
                .map_or(0, |(key, _)| key.value().1 + 1);
            for (index, change) in (first..).zip(changes) {
                let record = ChangeRecord {
                    session,
                    index,
                    owner: owner.to_owned(),
                    change: change.clone(),
                    rolled_back: false,
                };
                table.insert((session, index), serde_json::to_string(&record)?.as_str())?;
                indexes.push(index);
            }
        }
        transaction.commit()?;
        Ok(indexes)
    }

    pub fn mark_rolled_back(&self, session: u64, indexes: &[u32]) -> Result<()> {
        let transaction = self.database.begin_write()?;
        {
            let mut table = transaction.open_table(CHANGES)?;
            for &index in indexes {
                let current = table.get((session, index))?.map(|value| value.value().to_owned());
                if let Some(text) = current {
                    let mut record: ChangeRecord = serde_json::from_str(&text)?;
                    record.rolled_back = true;
                    table.insert((session, index), serde_json::to_string(&record)?.as_str())?;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn sessions(&self) -> Result<Vec<Session>> {
        let transaction = self.database.begin_read()?;
        let table = transaction.open_table(SESSIONS)?;
        let mut sessions = Vec::new();
        for row in table.iter()? {
            sessions.push(serde_json::from_str(row?.1.value())?);
        }
        sessions.reverse();
        Ok(sessions)
    }

    pub fn session(&self, id: u64) -> Result<Option<Session>> {
        let transaction = self.database.begin_read()?;
        let table = transaction.open_table(SESSIONS)?;
        let row = table.get(id)?;
        Ok(match row {
            Some(value) => Some(serde_json::from_str(value.value())?),
            None => None,
        })
    }

    pub fn changes(&self, session: u64) -> Result<Vec<ChangeRecord>> {
        let transaction = self.database.begin_read()?;
        let table = transaction.open_table(CHANGES)?;
        let mut records = Vec::new();
        for row in table.range((session, 0)..=(session, u32::MAX))? {
            records.push(serde_json::from_str(row?.1.value())?);
        }
        Ok(records)
    }

    pub fn set_applied(&self, state: &AppliedState) -> Result<()> {
        let transaction = self.database.begin_write()?;
        {
            let mut table = transaction.open_table(APPLIED)?;
            table.insert(state.tweak_id.as_str(), serde_json::to_string(state)?.as_str())?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn clear_applied(&self, tweak_id: &str) -> Result<()> {
        let transaction = self.database.begin_write()?;
        {
            let mut table = transaction.open_table(APPLIED)?;
            table.remove(tweak_id)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn applied(&self) -> Result<Vec<AppliedState>> {
        let transaction = self.database.begin_read()?;
        let table = transaction.open_table(APPLIED)?;
        let mut states = Vec::new();
        for row in table.iter()? {
            states.push(serde_json::from_str(row?.1.value())?);
        }
        Ok(states)
    }

    pub fn applied_state(&self, tweak_id: &str) -> Result<Option<AppliedState>> {
        let transaction = self.database.begin_read()?;
        let table = transaction.open_table(APPLIED)?;
        let row = table.get(tweak_id)?;
        Ok(match row {
            Some(value) => Some(serde_json::from_str(value.value())?),
            None => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn store() -> (tempfile::TempDir, SnapshotStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SnapshotStore::open(&dir.path().join("snapshots.redb")).unwrap();
        (dir, store)
    }

    fn change(value: u32) -> Change {
        Change::Registry {
            path: r"HKCU\Test".into(),
            name: "v".into(),
            before: None,
            after: Some(RegValue::DWord(value)),
        }
    }

    #[test]
    fn sessions_are_numbered_and_listed_newest_first() {
        let (_dir, store) = store();
        let first = store.begin_session("one").unwrap();
        let second = store.begin_session("two").unwrap();
        assert_eq!(second, first + 1);
        store.finish_session(first, SessionState::Committed).unwrap();
        let sessions = store.sessions().unwrap();
        assert_eq!(sessions[0].id, second);
        assert_eq!(sessions[0].state, SessionState::Open);
        assert_eq!(sessions[1].state, SessionState::Committed);
    }

    #[test]
    fn changes_append_in_order_within_a_session() {
        let (_dir, store) = store();
        let session = store.begin_session("s").unwrap();
        assert_eq!(store.record(session, "a", &[change(1), change(2)]).unwrap(), [0, 1]);
        assert_eq!(store.record(session, "b", &[change(3)]).unwrap(), [2]);
        let records = store.changes(session).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(records[2].owner, "b");

        store.mark_rolled_back(session, &[1]).unwrap();
        assert!(store.changes(session).unwrap()[1].rolled_back);
    }

    #[test]
    fn data_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.redb");
        {
            let store = SnapshotStore::open(&path).unwrap();
            let session = store.begin_session("persisted").unwrap();
            store.record(session, "t", &[change(7)]).unwrap();
        }
        let store = SnapshotStore::open(&path).unwrap();
        let sessions = store.sessions().unwrap();
        assert_eq!(sessions[0].label, "persisted");
        assert_eq!(sessions[0].state, SessionState::Open, "an unfinished session reads as interrupted");
        assert_eq!(store.changes(sessions[0].id).unwrap().len(), 1);
    }

    #[test]
    fn a_second_open_of_the_same_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.redb");
        let _first = SnapshotStore::open(&path).unwrap();
        assert!(SnapshotStore::open(&path).is_err());
    }

    #[test]
    fn a_second_open_reports_busy_instead_of_failing_obscurely() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.redb");
        let first = SnapshotStore::open(&path).unwrap();
        assert!(matches!(SnapshotStore::open(&path), Err(crate::error::EngineError::StoreBusy)));
        drop(first);
        SnapshotStore::open(&path).unwrap();
    }

    #[test]
    fn a_corrupt_database_is_set_aside_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.redb");
        std::fs::write(&path, vec![0x5A; 8192]).unwrap();
        let store = SnapshotStore::open(&path).unwrap();
        assert!(store.sessions().unwrap().is_empty());
        let names: Vec<String> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|name| name.contains("corrupt-")), "{names:?}");
    }

    #[test]
    fn a_garbage_file_is_set_aside_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("garbage.redb");
        std::fs::write(&path, b"not a valid redb database file header or contents").unwrap();
        let store = SnapshotStore::open(&path).unwrap();
        assert!(store.sessions().unwrap().is_empty());
        let names: Vec<String> =
            std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().any(|name| name.contains("corrupt-")), "{names:?}");
    }

    #[test]
    fn is_damage_classifies_only_corruption_and_invalid_data() {
        let corrupt = redb::DatabaseError::Storage(redb::StorageError::Corrupted("corrupt page".into()));
        assert!(is_damage(&corrupt));

        let invalid_data = redb::DatabaseError::Storage(redb::StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad magic header",
        )));
        assert!(is_damage(&invalid_data));

        let io_other = redb::DatabaseError::Storage(redb::StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "access denied",
        )));
        assert!(!is_damage(&io_other));

        assert!(!is_damage(&redb::DatabaseError::DatabaseAlreadyOpen));
        assert!(!is_damage(&redb::DatabaseError::UpgradeRequired(1)));
        assert!(!is_damage(&redb::DatabaseError::RepairAborted));
        assert!(!is_damage(&redb::DatabaseError::TransactionInProgress));
        assert!(!is_damage(&redb::DatabaseError::Storage(redb::StorageError::ValueTooLarge(100))));
        assert!(!is_damage(&redb::DatabaseError::Storage(redb::StorageError::Unsupported)));
        assert!(!is_damage(&redb::DatabaseError::Storage(redb::StorageError::PreviousIo)));
        assert!(!is_damage(&redb::DatabaseError::Storage(redb::StorageError::DatabaseClosed)));
    }

    #[test]
    fn concurrent_readers_and_writers_do_not_deadlock_or_lose_data() {
        let (_dir, store) = store();
        let store = Arc::new(store);
        let session = store.begin_session("stress").unwrap();
        let mut workers = Vec::new();
        for worker in 0..8 {
            let store = Arc::clone(&store);
            workers.push(std::thread::spawn(move || {
                for i in 0..25 {
                    store.record(session, &format!("w{worker}"), &[change(i)]).unwrap();
                    assert!(store.changes(session).unwrap().len() as u32 > i);
                    store.sessions().unwrap();
                }
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let records = store.changes(session).unwrap();
        assert_eq!(records.len(), 200);
        let mut indexes: Vec<u32> = records.iter().map(|record| record.index).collect();
        indexes.dedup();
        assert_eq!(indexes.len(), 200, "indexes must be unique");
    }

    #[test]
    fn native_change_with_feature_roundtrips_through_json() {
        let original = Change::Native {
            description: "Enable Windows feature NetFx3".into(),
            feature: Some(FeatureStep {
                name: "NetFx3".into(),
                was_on: false,
            }),
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Change = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn v0_1_0_native_change_json_deserializes_with_none_feature() {
        let json = r#"{"kind":"Native","description":"Enable Windows feature NetFx3"}"#;
        let deserialized: Change = serde_json::from_str(json).unwrap();
        assert_eq!(
            deserialized,
            Change::Native {
                description: "Enable Windows feature NetFx3".into(),
                feature: None,
            }
        );
    }

    #[test]
    fn change_native_serializes_without_feature_key() {
        let change = Change::native("x");
        let json = serde_json::to_string(&change).unwrap();
        assert!(!json.contains("feature"));
    }
}
