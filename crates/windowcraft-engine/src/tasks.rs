// Made by ycangignacy
//! Task Scheduler through its COM interfaces.

use crate::error::{EngineError, Result};
use crate::win::ComApartment;
use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::TaskScheduler::*;
use windows::Win32::System::Variant::VARIANT;
use windows::core::{BSTR, Interface};

const TASK_ENUM_HIDDEN: i32 = 1;

/// The daily task behind the `reg-backup` feature.
pub const REGISTRY_BACKUP_TASK: &str = "AutoRegBackup";

fn connect() -> Result<ITaskService> {
    let fail = |error: &windows::core::Error| EngineError::windows("ITaskService", "Task Scheduler", error);
    // SAFETY: The CLSID is valid and the apartment was entered by the caller.
    let service: ITaskService = unsafe { CoCreateInstance(&TaskScheduler, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| fail(&error))?;
    let empty = VARIANT::default();
    // SAFETY: Empty variants select the local machine and current user.
    unsafe { service.Connect(&empty, &empty, &empty, &empty) }.map_err(|error| fail(&error))?;
    Ok(service)
}

fn split(path: &str) -> (&str, &str) {
    match path.rsplit_once('\\') {
        Some(("", name)) => ("\\", name),
        Some((folder, name)) => (folder, name),
        None => ("\\", path),
    }
}

fn open_folder(service: &ITaskService, path: &str) -> Result<Option<ITaskFolder>> {
    // SAFETY: The service is connected and the BSTR outlives the call.
    match unsafe { service.GetFolder(&BSTR::from(path)) } {
        Ok(folder) => Ok(Some(folder)),
        Err(error) if matches!(error.code().0.cast_unsigned() & 0xFFFF, 2 | 3) => Ok(None),
        Err(error) => Err(EngineError::windows("ITaskService::GetFolder", path, &error)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInfo {
    pub path: String,
    pub name: String,
    pub enabled: bool,
    /// Program of the first exec action. COM handler tasks have none.
    pub executable: Option<String>,
}

/// # Safety
/// `task` must be a live object from the current apartment.
unsafe fn first_executable(task: &IRegisteredTask) -> Option<String> {
    // SAFETY: Guaranteed by the caller. Action collections are 1-based.
    unsafe {
        let action = task.Definition().ok()?.Actions().ok()?.get_Item(1).ok()?;
        let exec: IExecAction = action.cast().ok()?;
        let mut path = BSTR::new();
        exec.Path(&mut path).ok()?;
        Some(path.to_string()).filter(|text| !text.is_empty())
    }
}

/// Lists tasks in a folder, including hidden ones. Missing folders are empty.
pub fn list_folder(path: &str) -> Result<Vec<TaskInfo>> {
    let _apartment = ComApartment::enter()?;
    let service = connect()?;
    let Some(folder) = open_folder(&service, path)? else {
        return Ok(Vec::new());
    };
    let fail = |error: &windows::core::Error| EngineError::windows("ITaskFolder::GetTasks", path, error);
    // SAFETY: The folder is live.
    let tasks = unsafe { folder.GetTasks(TASK_ENUM_HIDDEN) }.map_err(|error| fail(&error))?;
    // SAFETY: The collection is live.
    let count = unsafe { tasks.Count() }.map_err(|error| fail(&error))?;
    let mut items = Vec::with_capacity(count.max(0).cast_unsigned() as usize);
    for index in 1..=count {
        // SAFETY: Collection indexes are 1-based, and `index` is within range.
        let task = unsafe { tasks.get_Item(&VARIANT::from(index)) }.map_err(|error| fail(&error))?;
        // SAFETY: The task object is live, and these are plain property reads.
        let (name, task_path, enabled) = unsafe { (task.Name(), task.Path(), task.Enabled()) };
        let name = name.map_err(|error| fail(&error))?;
        let task_path = task_path.map_err(|error| fail(&error))?;
        let enabled = enabled.map_err(|error| fail(&error))?;
        // SAFETY: The task object is live.
        let executable = unsafe { first_executable(&task) };
        items.push(TaskInfo { path: task_path.to_string(), name: name.to_string(), enabled: enabled.as_bool(), executable });
    }
    Ok(items)
}

pub fn set_enabled(path: &str, enabled: bool) -> Result<()> {
    let _apartment = ComApartment::enter()?;
    let service = connect()?;
    let (folder_path, name) = split(path);
    let folder = open_folder(&service, folder_path)?
        .ok_or_else(|| EngineError::NotFound(format!("task folder {folder_path}")))?;
    // SAFETY: The folder is live and the BSTR outlives the call.
    let task = unsafe { folder.GetTask(&BSTR::from(name)) }
        .map_err(|error| EngineError::windows("ITaskFolder::GetTask", path, &error))?;
    let flag = if enabled { VARIANT_TRUE } else { VARIANT_FALSE };
    // SAFETY: The task object is live.
    unsafe { task.SetEnabled(flag) }.map_err(|error| EngineError::windows("IRegisteredTask::put_Enabled", path, &error))
}

/// Enables or disables every task in a folder, skipping locked system tasks.
/// Returns the paths that changed along with their previous state.
pub fn set_folder_enabled(path: &str, enabled: bool) -> Result<Vec<(String, bool)>> {
    let mut changed = Vec::new();
    for task in list_folder(path)? {
        if task.enabled != enabled && set_enabled(&task.path, enabled).is_ok() {
            changed.push((task.path, task.enabled));
        }
    }
    Ok(changed)
}

/// Deletes a task. Returns false when it does not exist.
pub fn delete(path: &str) -> Result<bool> {
    let _apartment = ComApartment::enter()?;
    let service = connect()?;
    let (folder_path, name) = split(path);
    let Some(folder) = open_folder(&service, folder_path)? else { return Ok(false) };
    // SAFETY: The folder is live and the BSTR outlives the call.
    match unsafe { folder.DeleteTask(&BSTR::from(name), 0) } {
        Ok(()) => Ok(true),
        Err(error) => {
            let error = EngineError::windows("ITaskFolder::DeleteTask", path, &error);
            if error.is_not_found() { Ok(false) } else { Err(error) }
        }
    }
}

/// Registers a daily task running as SYSTEM.
pub fn register_daily(name: &str, description: &str, program: &str, arguments: &str, start: &str) -> Result<()> {
    let _apartment = ComApartment::enter()?;
    let service = connect()?;
    let fail = |error: windows::core::Error| EngineError::windows("RegisterTaskDefinition", name, &error);

    // SAFETY: Every call below operates on live COM objects returned by the
    // previous call, and BSTR arguments outlive their calls.
    unsafe {
        let root = service.GetFolder(&BSTR::from("\\")).map_err(fail)?;
        let definition = service.NewTask(0).map_err(fail)?;
        definition.RegistrationInfo().map_err(fail)?.SetDescription(&BSTR::from(description)).map_err(fail)?;

        let trigger = definition.Triggers().map_err(fail)?.Create(TASK_TRIGGER_DAILY).map_err(fail)?;
        trigger.SetStartBoundary(&BSTR::from(start)).map_err(fail)?;
        trigger.cast::<IDailyTrigger>().map_err(fail)?.SetDaysInterval(1).map_err(fail)?;

        let action = definition.Actions().map_err(fail)?.Create(TASK_ACTION_EXEC).map_err(fail)?;
        let action: IExecAction = action.cast().map_err(fail)?;
        action.SetPath(&BSTR::from(program)).map_err(fail)?;
        action.SetArguments(&BSTR::from(arguments)).map_err(fail)?;

        let empty = VARIANT::default();
        root.RegisterTaskDefinition(
            &BSTR::from(name),
            &definition,
            TASK_CREATE_OR_UPDATE.0,
            &VARIANT::from(BSTR::from("SYSTEM")),
            &empty,
            TASK_LOGON_SERVICE_ACCOUNT,
            &empty,
        )
        .map_err(fail)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_paths() {
        assert_eq!(split(r"\Microsoft\Windows\Task"), (r"\Microsoft\Windows", "Task"));
        assert_eq!(
            split(r"\Microsoft\Windows\UpdateOrchestrator\Schedule Scan"),
            (r"\Microsoft\Windows\UpdateOrchestrator", "Schedule Scan")
        );
        assert_eq!(split(r"CustomFolder\SubTask"), ("CustomFolder", "SubTask"));
        assert_eq!(split(r"\Task"), ("\\", "Task"));
        assert_eq!(split("Task"), ("\\", "Task"));
        assert_eq!(split(r"\"), ("\\", ""));
        assert_eq!(split(""), ("\\", ""));
    }

    #[test]
    fn missing_folder_is_empty() {
        assert!(list_folder(r"\WindowCraft\NoSuchFolder").unwrap().is_empty());
    }

    #[test]
    fn listing_a_real_folder_does_not_leak_or_fail() {
        for _ in 0..5 {
            let tasks = list_folder(r"\Microsoft\Windows\Registry").unwrap();
            for task in &tasks {
                assert!(!task.name.is_empty());
                assert!(task.path.ends_with(&task.name), "{}", task.path);
            }
        }
    }

    #[test]
    fn open_folder_resolves_root_and_returns_none_for_absent_folder() {
        let _apartment = ComApartment::enter().unwrap();
        let service = connect().unwrap();
        assert!(open_folder(&service, r"\").unwrap().is_some());
        assert!(open_folder(&service, r"\WindowCraft\NoSuchSubfolder").unwrap().is_none());
    }

    #[test]
    fn set_enabled_reports_not_found_for_missing_folder_and_missing_task() {
        let missing_folder = set_enabled(r"\WindowCraft_NoSuchFolder\NoSuchTask", false).unwrap_err();
        assert!(missing_folder.is_not_found());
        assert!(matches!(
            &missing_folder,
            EngineError::NotFound(detail) if detail == r"task folder \WindowCraft_NoSuchFolder"
        ));

        let missing_task = set_enabled(r"\WindowCraft_NoSuchTask_98765", true).unwrap_err();
        assert!(missing_task.is_not_found());
        assert!(matches!(
            &missing_task,
            EngineError::Win32 { operation: "ITaskFolder::GetTask", target, .. }
                if target == r"\WindowCraft_NoSuchTask_98765"
        ));
    }

    #[test]
    fn deleting_a_missing_task_or_folder_reports_nothing_removed() {
        assert!(!delete(r"\WindowCraft_NoSuchTask_98765").unwrap());
        assert!(!delete(r"\WindowCraft_NoSuchFolder\NoSuchTask").unwrap());
    }

    #[test]
    fn set_folder_enabled_on_missing_folder_returns_no_changes() {
        assert!(set_folder_enabled(r"\WindowCraft\NoSuchFolder", false).unwrap().is_empty());
    }
}
