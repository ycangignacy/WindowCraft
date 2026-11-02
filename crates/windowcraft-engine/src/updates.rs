// Made by ycangignacy
//! Windows Update policy modes.

use crate::error::{EngineError, Result};
use crate::executor::Engine;
use crate::registry::{RegPath, resolve_value};
use crate::snapshot::{Change, SessionState};
use crate::{cleaner, tasks};
use std::path::PathBuf;
use windowcraft_config::UpdateMode;

impl Engine {
    /// Applies an update mode inside one snapshot session.
    pub fn apply_update_mode(&self, mode: &UpdateMode) -> Result<u64> {
        let owner = format!("update-mode-{}", mode.id);
        let session = self.store.begin_session(&format!("Windows Updates: {}", mode.name))?;
        let outcome = self.update_mode_inner(session, &owner, mode);
        if outcome.is_err() {
            // A half-applied policy is worse than none, so a service that
            // refuses the change takes the registry writes back with it.
            let records = self.store.changes(session)?;
            let _ = self.revert_records(session, &records);
        }
        let state = if outcome.is_ok() { SessionState::Committed } else { SessionState::Failed };
        self.store.finish_session(session, state)?;
        outcome.map(|()| session)
    }

    fn update_mode_inner(&self, session: u64, owner: &str, mode: &UpdateMode) -> Result<()> {
        let targets: Vec<_> = mode
            .registry
            .iter()
            .map(|entry| resolve_value(entry, entry.value.as_deref(), self.ram_kb))
            .collect::<Result<_>>()?;

        let mut changes = Vec::with_capacity(targets.len() + mode.services.len());
        for target in &targets {
            changes.push(Change::Registry {
                path: target.path.display(),
                name: target.name.clone(),
                before: self.scope.read(&target.path, &target.name)?,
                after: target.value.clone(),
            });
        }
        for rule in &mode.services {
            let before = self.services.query(&rule.name)?.map(|info| info.startup);
            changes.push(Change::Service { name: rule.name.clone(), before, after: rule.startup });
        }
        self.store.record(session, owner, &changes)?;

        for target in &targets {
            self.scope.write_opt(&target.path, &target.name, target.value.as_ref())?;
        }
        for rule in &mode.services {
            if self.services.query(&rule.name)?.is_some() {
                self.services.set_startup(&rule.name, rule.startup)?;
                if rule.startup == windowcraft_config::StartupType::Disabled {
                    self.services.stop(&rule.name)?;
                }
            }
        }
        if self.scope == crate::registry::RegistryScope::System {
            for folder in &mode.task_folders {
                let changed = tasks::set_folder_enabled(&folder.path, folder.enabled)?;
                let records: Vec<Change> = changed
                    .into_iter()
                    .map(|(path, before)| Change::Task { path, before, after: folder.enabled })
                    .collect();
                self.store.record(session, owner, &records)?;
            }
            if mode.purge_software_distribution {
                let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_owned());
                cleaner::clean_dir(&PathBuf::from(root).join("SoftwareDistribution"));
            }
        }
        Ok(())
    }

    /// Identifies the active mode from policy values.
    pub fn current_update_mode(&self) -> Result<&'static str> {
        use crate::registry::RegValue;
        let au = RegPath::parse(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU")?;
        if self.scope.read(&au, "NoAutoUpdate")? == Some(RegValue::DWord(1)) {
            return Ok("disabled");
        }
        let policy = RegPath::parse(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate")?;
        let deferred = self.scope.read(&policy, "DeferFeatureUpdatesPeriodInDays")?;
        Ok(if deferred.is_some() { "security" } else { "default" })
    }
}

/// Spelled-out guard for the destructive mode, used by the CLI and the dialog.
pub fn require_confirmation(mode: &UpdateMode, confirmed: bool) -> Result<()> {
    if mode.confirm && !confirmed {
        return Err(EngineError::Invalid(format!("{} needs explicit confirmation", mode.name)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::RecordingRunner;
    use crate::registry::RegValue;
    use crate::services::MemoryServices;
    use crate::snapshot::SnapshotStore;
    use crate::testing::Sandbox;
    use std::sync::Arc;
    use windowcraft_config::{Catalog, ServiceRule, StartupType};

    struct Rig {
        engine: Engine,
        services: Arc<MemoryServices>,
        _sandbox: Sandbox,
        _dir: tempfile::TempDir,
    }

    fn rig(name: &str) -> Rig {
        let sandbox = Sandbox::new(name);
        let dir = tempfile::tempdir().unwrap();
        let catalog = Arc::new(Catalog::load().unwrap());
        let services = Arc::new(MemoryServices::with(&[
            ("BITS", StartupType::Manual),
            ("wuauserv", StartupType::Manual),
            ("UsoSvc", StartupType::Automatic),
            ("RpcSs", StartupType::Automatic),
        ]));
        let engine = Engine {
            scope: sandbox.scope.clone(),
            services: services.clone(),
            native: Arc::new(RecordingRunner::default()),
            store: Arc::new(SnapshotStore::open(&dir.path().join("u.redb")).unwrap()),
            catalog,
            ram_kb: 16 * 1024 * 1024,
            os_build: "26100.1".to_owned(),
        };
        Rig { engine, services, _sandbox: sandbox, _dir: dir }
    }

    #[test]
    fn destructive_update_mode_requires_explicit_confirmation() {
        let catalog = Catalog::load().unwrap();
        let default_mode = catalog.update_mode("default").unwrap();
        let security_mode = catalog.update_mode("security").unwrap();
        let disabled_mode = catalog.update_mode("disabled").unwrap();

        assert!(require_confirmation(default_mode, false).is_ok());
        assert!(require_confirmation(security_mode, false).is_ok());
        assert!(require_confirmation(disabled_mode, false).is_err());
        assert!(require_confirmation(disabled_mode, true).is_ok());
    }

    #[test]
    fn applying_and_rolling_back_update_modes_switches_active_policy_and_services() {
        let rig = rig("updatemodes");
        assert_eq!(rig.engine.current_update_mode().unwrap(), "default");

        let security = rig.engine.catalog.update_mode("security").unwrap().clone();
        let sec_session = rig.engine.apply_update_mode(&security).unwrap();
        assert_eq!(rig.engine.current_update_mode().unwrap(), "security");

        let disabled = rig.engine.catalog.update_mode("disabled").unwrap().clone();
        let dis_session = rig.engine.apply_update_mode(&disabled).unwrap();
        assert_eq!(rig.engine.current_update_mode().unwrap(), "disabled");
        assert_eq!(rig.services.startup("wuauserv"), Some(StartupType::Disabled));
        assert_eq!(rig.services.startup("UsoSvc"), Some(StartupType::Disabled));

        rig.engine.rollback_session(dis_session).unwrap();
        assert_eq!(rig.engine.current_update_mode().unwrap(), "security");
        assert_eq!(rig.services.startup("wuauserv"), Some(StartupType::Manual));
        assert_eq!(rig.services.startup("UsoSvc"), Some(StartupType::Automatic));

        rig.engine.rollback_session(sec_session).unwrap();
        assert_eq!(rig.engine.current_update_mode().unwrap(), "default");
    }

    #[test]
    fn a_failing_service_change_reverts_earlier_registry_writes_and_marks_session_failed() {
        let rig = rig("updatefail");
        let mut mode = rig.engine.catalog.update_mode("disabled").unwrap().clone();
        mode.services.push(ServiceRule {
            name: "RpcSs".to_owned(),
            startup: StartupType::Disabled,
            original: StartupType::Automatic,
            optional: false,
        });

        assert!(matches!(rig.engine.apply_update_mode(&mode), Err(EngineError::ProtectedService(_))));
        let au = RegPath::parse(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU").unwrap();
        assert_eq!(
            rig.engine.scope.read(&au, "NoAutoUpdate").unwrap(),
            None,
            "registry write must be reverted when a later service rule fails"
        );
        assert_eq!(rig.engine.current_update_mode().unwrap(), "default");
        let sessions = rig.engine.store.sessions().unwrap();
        assert_eq!(sessions[0].state, SessionState::Failed);
    }

    #[test]
    fn default_mode_clears_existing_policy_values_and_restores_service_startups() {
        let rig = rig("updatedefault");
        let au = RegPath::parse(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU").unwrap();
        rig.engine.scope.write(&au, "NoAutoUpdate", &RegValue::DWord(1)).unwrap();
        rig.services.force("wuauserv", StartupType::Disabled);
        assert_eq!(rig.engine.current_update_mode().unwrap(), "disabled");

        let default_mode = rig.engine.catalog.update_mode("default").unwrap().clone();
        rig.engine.apply_update_mode(&default_mode).unwrap();
        assert_eq!(rig.engine.current_update_mode().unwrap(), "default");
        assert_eq!(rig.engine.scope.read(&au, "NoAutoUpdate").unwrap(), None);
        assert_eq!(rig.services.startup("wuauserv"), Some(StartupType::Manual));
    }
}
