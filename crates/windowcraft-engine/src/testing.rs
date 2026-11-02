// Made by ycangignacy
//! Sandboxed helpers shared by unit and integration tests.

use crate::registry::{Hive, RegPath, RegistryScope};
use std::sync::Mutex;

/// Root for every sandboxed registry test.
pub const SANDBOX_ROOT: &str = r"Software\WindowCraft_TestSandbox";

/// Sandboxes alive in this process. Deleting the shared root while a sibling
/// creates its subtree makes that sibling's write fail with ERROR_KEY_DELETED,
/// so the root only goes with the last sandbox, under this lock.
static LIVE: Mutex<usize> = Mutex::new(0);

/// A private registry subtree under HKCU, deleted on drop even if a test fails.
pub struct Sandbox {
    pub scope: RegistryScope,
    root: String,
}

impl Sandbox {
    pub fn new(name: &str) -> Self {
        *LIVE.lock().unwrap() += 1;
        let root = format!("{SANDBOX_ROOT}\\{name}-{}", std::process::id());
        let sandbox = Self { scope: RegistryScope::Sandbox(root.clone()), root };
        sandbox.clean();
        sandbox
    }

    fn clean(&self) {
        let path = RegPath { hive: Hive::CurrentUser, subkey: self.root.clone() };
        // A failed cleanup in a test teardown must not mask the real failure.
        let _ = RegistryScope::System.delete_tree(&path);
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        self.clean();
        let mut live = LIVE.lock().unwrap();
        *live -= 1;
        if *live == 0 {
            let parent = RegPath { hive: Hive::CurrentUser, subkey: SANDBOX_ROOT.to_owned() };
            // Fails if another process still has keys there, which is the wanted outcome.
            let _ = crate::registry::delete_empty_key(&parent);
        }
    }
}
