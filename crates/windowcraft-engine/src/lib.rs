// Made by Nordan
//! Native Windows engine behind WindowCraft.

pub mod appx;
pub mod atomicfs;
pub mod authenticode;
pub mod autoruns;
pub mod convert;
pub mod convert_native;
pub mod cleaner;
pub mod containers;
pub mod dism;
pub mod dns;
pub mod dns_bench;
pub mod download;
pub mod dynlib;
pub mod error;
pub mod executor;
pub mod fixes;
pub mod gaming;
pub mod jobs;
pub mod metadata;
pub mod native;
pub mod packages;
pub mod power;
pub mod process;
pub mod purge;
pub mod registry;
pub mod remux;
pub mod report;
pub mod security;
pub mod services;
pub mod shell;
pub mod snapshot;
pub mod ssh;
pub mod sysinfo;
pub mod tasks;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod tools;
pub mod updates;
pub mod win;
pub mod wmi;
pub mod youtube;

pub use error::{EngineError, Result};
#[cfg(any(test, feature = "testing"))]
pub use executor::RecordingRunner;
pub use executor::{Detection, Drift, Engine, InspectRow, NativeRunner, RunReport, TweakAction};
pub use jobs::{CancelToken, JobContext, JobEvent, JobFailure, JobHandle, LogRing};
pub use native::SystemRunner;
pub use registry::{RegPath, RegValue, RegistryScope};
pub use snapshot::{SessionState, SnapshotStore};

use std::path::PathBuf;
use std::sync::Arc;
use windowcraft_config::Catalog;

/// Default location of the snapshot database.
pub fn default_store_path() -> PathBuf {
    win::local_app_dir().join("timemachine.redb")
}

/// Relaunching elevated starts the new process while the old one still holds
/// the database, so a short wait beats a fatal error.
fn open_store_with_retry() -> Result<SnapshotStore> {
    let path = default_store_path();
    let mut attempts = 0;
    loop {
        match SnapshotStore::open(&path) {
            Err(EngineError::StoreBusy) if attempts < 30 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

impl Engine {
    /// Wires the engine to the live system.
    pub fn system(catalog: Arc<Catalog>) -> Result<Self> {
        let info = sysinfo::query()?;
        Ok(Self {
            scope: RegistryScope::System,
            services: Arc::new(services::ScmBackend),
            native: Arc::new(SystemRunner),
            store: Arc::new(open_store_with_retry()?),
            catalog,
            ram_kb: info.ram_kb,
            os_build: info.full_build(),
        })
    }
}
