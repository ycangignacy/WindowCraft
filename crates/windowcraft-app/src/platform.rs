// Made by ycangignacy
//! Windows integration for the shell of the app: backdrop, accessibility
//! settings, taskbar progress, file dialogs and the console.

use std::path::PathBuf;
use windows::UI::ViewManagement::UISettings;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWMSBT_MAINWINDOW, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmExtendFrameIntoClientArea,
    DwmSetWindowAttribute,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, ITaskbarList3, SIGDN_FILESYSPATH, TBPF_INDETERMINATE,
    TBPF_NOPROGRESS, TBPF_NORMAL, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETHIGHCONTRAST, SPI_GETWHEELSCROLLLINES, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::core::{BOOL, HSTRING, w};

/// Restricts runtime DLL lookups to System32. This binary is often started
/// elevated from a Downloads folder, where a planted DLL would otherwise load.
pub fn harden_dll_search() {
    use windows::Win32::System::LibraryLoader::{LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories};
    // SAFETY: Takes a flag only. Failure leaves the default search order.
    unsafe {
        let _ = SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32);
    }
}

/// Claims the single GUI instance, or brings the running one forward.
/// Two windows would otherwise fight over one snapshot database.
pub fn claim_single_instance() -> Option<windowcraft_engine::win::HandleGuard> {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow};
    use windows::core::PCWSTR;
    // SAFETY: The name is a static string. Ownership of the handle moves into
    // the guard, and GetLastError is read straight after the creating call.
    let mutex = unsafe { CreateMutexW(None, false, w!("Local\\WindowCraft.SingleInstance")) }.ok()?;
    // SAFETY: Reads this thread's last error.
    let existing = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let guard = windowcraft_engine::win::HandleGuard(mutex);
    if !existing {
        return Some(guard);
    }
    // SAFETY: The title is a static string, and a failed lookup is harmless.
    unsafe {
        if let Ok(window) = FindWindowW(PCWSTR::null(), w!("WindowCraft")) {
            let _ = ShowWindow(window, SW_RESTORE);
            let _ = SetForegroundWindow(window);
        }
    }
    None
}

/// Connects a CLI invocation to the terminal that started it.
pub fn attach_parent_console() {
    // SAFETY: Takes no pointers. Failure just means there is no parent console.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accessibility {
    pub animations: bool,
    pub transparency: bool,
    pub high_contrast: bool,
    pub light_theme: bool,
    /// Lines per mouse-wheel notch from the Windows mouse settings.
    pub wheel_lines: u32,
}

pub fn accessibility() -> Accessibility {
    let settings = UISettings::new().ok();
    let flag = |read: fn(&UISettings) -> windows::core::Result<bool>| {
        settings.as_ref().is_none_or(|settings| read(settings).unwrap_or(true))
    };

    let mut contrast = HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    // SAFETY: `contrast` has cbSize set and is valid for this request.
    let high_contrast = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&raw mut contrast).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok()
        && contrast.dwFlags.contains(HCF_HIGHCONTRASTON);

    Accessibility {
        animations: flag(UISettings::AnimationsEnabled),
        transparency: flag(UISettings::AdvancedEffectsEnabled),
        high_contrast,
        light_theme: apps_use_light_theme(),
        wheel_lines: wheel_scroll_lines(),
    }
}

/// Lines per wheel notch. "One screen at a time" reads as u32::MAX and is
/// capped by the caller; a failed query keeps the Windows default of 3.
fn wheel_scroll_lines() -> u32 {
    let mut lines: u32 = 3;
    // SAFETY: `lines` is a valid u32 for this request.
    let read = unsafe {
        SystemParametersInfoW(SPI_GETWHEELSCROLLLINES, 0, Some((&raw mut lines).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
    };
    if read.is_ok() { lines } else { 3 }
}

fn apps_use_light_theme() -> bool {
    use windowcraft_engine::{RegPath, RegValue, RegistryScope};
    let Ok(path) = RegPath::parse(r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize") else {
        return false;
    };
    matches!(RegistryScope::System.read(&path, "AppsUseLightTheme"), Ok(Some(RegValue::DWord(1))))
}

/// Whether a GPU that runs Direct3D 12 is present.
///
/// Without one, wgpu falls back to WARP, the software rasterizer of Direct3D,
/// which paints this interface more slowly than tiny-skia does, so the caller
/// picks tiny-skia instead. Virtual machines without 3D acceleration and the
/// Microsoft Basic Display Adapter end up there.
pub fn hardware_gpu() -> bool {
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
    use windows::Win32::Graphics::Direct3D12::{D3D12CreateDevice, ID3D12Device};
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
    /// PCI vendor id Microsoft uses for WARP and the Basic Render Driver.
    const MICROSOFT_VENDOR: u32 = 0x1414;

    // SAFETY: Creates a COM factory; the interface is released on drop.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return false;
    };
    for index in 0.. {
        // SAFETY: Enumeration ends at the first index without an adapter.
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
            break;
        };
        // SAFETY: `adapter` is a live interface.
        let Ok(description) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        let software = description.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
        if software || description.VendorId == MICROSOFT_VENDOR {
            continue;
        }
        // SAFETY: A null output pointer only asks whether a device could be
        // created on this adapter; nothing is created.
        let supported = unsafe {
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, std::ptr::null_mut::<Option<ID3D12Device>>())
        };
        if supported.is_ok() {
            return true;
        }
    }
    false
}

/// Requests the Mica backdrop and dark title bar. Unsupported builds ignore it.
pub fn apply_backdrop(window: u64, dark: bool, mica: bool) {
    let hwnd = HWND(window as *mut core::ffi::c_void);
    let dark_flag = BOOL::from(dark);
    // SAFETY: `hwnd` is the live main window. Sizes match each attribute.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark_flag).cast(),
            size_of::<BOOL>() as u32,
        );
        if mica {
            // Extending the frame over the client area makes DWM honor the
            // alpha channel of whatever the renderer presents.
            let margins = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
            let backdrop = DWMSBT_MAINWINDOW;
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_SYSTEMBACKDROP_TYPE,
                (&raw const backdrop).cast(),
                size_of_val(&backdrop) as u32,
            );
        }
    }
}

/// Mirrors job progress on the taskbar button.
pub fn taskbar_progress(window: u64, fraction: Option<f32>) {
    std::thread::spawn(move || {
        // SAFETY: The apartment is balanced below. `hwnd` is the main window;
        // the calls fail harmlessly if it has gone.
        unsafe {
            if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
                return;
            }
            let hwnd = HWND(window as *mut core::ffi::c_void);
            if let Ok(taskbar) = CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER) {
                let _ = taskbar.HrInit();
                match fraction {
                    Some(value) if value > 0.0 => {
                        let _ = taskbar.SetProgressState(hwnd, TBPF_NORMAL);
                        let _ = taskbar.SetProgressValue(hwnd, (value * 1000.0) as u64, 1000);
                    }
                    Some(_) => {
                        let _ = taskbar.SetProgressState(hwnd, TBPF_INDETERMINATE);
                    }
                    None => {
                        let _ = taskbar.SetProgressState(hwnd, TBPF_NOPROGRESS);
                    }
                }
            }
            CoUninitialize();
        }
    });
}

fn dialog_path(item: windows::Win32::UI::Shell::IShellItem) -> Option<PathBuf> {
    // SAFETY: `item` is live. The string is CoTaskMem-allocated and freed below.
    unsafe {
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = name.to_string().ok();
        CoTaskMemFree(Some(name.0.cast()));
        text.map(PathBuf::from)
    }
}

/// Shows the system open dialog on a dedicated STA thread.
pub async fn pick_open_file(title: &'static str, label: &'static str, pattern: &'static str) -> Option<PathBuf> {
    let task = tokio::task::spawn_blocking(move || {
        // SAFETY: The apartment is balanced on this thread. Every string
        // outlives the calls that take it, and the dialog is used on this
        // thread only.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().ok()?;
            let result = (|| {
                let dialog: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
                let (name, spec) = (HSTRING::from(label), HSTRING::from(pattern));
                let filter = [COMDLG_FILTERSPEC {
                    pszName: windows::core::PCWSTR(name.as_ptr()),
                    pszSpec: windows::core::PCWSTR(spec.as_ptr()),
                }];
                dialog.SetFileTypes(&filter).ok()?;
                dialog.SetTitle(&HSTRING::from(title)).ok()?;
                dialog.Show(None).ok()?;
                dialog_path(dialog.GetResult().ok()?)
            })();
            CoUninitialize();
            result
        }
    });
    task.await.ok().flatten()
}

/// Shows the open dialog with multiple selection. The picked files are kept
/// out of the Recent list of Windows.
pub async fn pick_open_files(title: &'static str, label: &'static str, pattern: &'static str) -> Vec<PathBuf> {
    use windows::Win32::UI::Shell::{FOS_ALLOWMULTISELECT, FOS_DONTADDTORECENT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM};
    let task = tokio::task::spawn_blocking(move || {
        // SAFETY: As in `pick_open_file`. Items are read on this thread only.
        unsafe {
            if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
                return Vec::new();
            }
            let result = (|| {
                let dialog: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
                let (name, spec) = (HSTRING::from(label), HSTRING::from(pattern));
                let filter = [COMDLG_FILTERSPEC {
                    pszName: windows::core::PCWSTR(name.as_ptr()),
                    pszSpec: windows::core::PCWSTR(spec.as_ptr()),
                }];
                dialog.SetFileTypes(&filter).ok()?;
                let options = dialog.GetOptions().ok()?;
                dialog
                    .SetOptions(options | FOS_ALLOWMULTISELECT | FOS_DONTADDTORECENT | FOS_FILEMUSTEXIST | FOS_FORCEFILESYSTEM)
                    .ok()?;
                dialog.SetTitle(&HSTRING::from(title)).ok()?;
                dialog.Show(None).ok()?;
                let items = dialog.GetResults().ok()?;
                let count = items.GetCount().ok()?;
                Some((0..count).filter_map(|index| items.GetItemAt(index).ok().and_then(dialog_path)).collect::<Vec<_>>())
            })();
            CoUninitialize();
            result.unwrap_or_default()
        }
    });
    task.await.unwrap_or_default()
}

/// Shows the open dialog in folder mode.
pub async fn pick_folder(title: &'static str) -> Option<PathBuf> {
    use windows::Win32::UI::Shell::{FOS_DONTADDTORECENT, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS};
    let task = tokio::task::spawn_blocking(move || {
        // SAFETY: As in `pick_open_file`.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().ok()?;
            let result = (|| {
                let dialog: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
                let options = dialog.GetOptions().ok()?;
                dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_DONTADDTORECENT).ok()?;
                dialog.SetTitle(&HSTRING::from(title)).ok()?;
                dialog.Show(None).ok()?;
                dialog_path(dialog.GetResult().ok()?)
            })();
            CoUninitialize();
            result
        }
    });
    task.await.ok().flatten()
}

/// Shows the system save dialog on a dedicated STA thread.
pub async fn pick_save_file(title: &'static str, default_name: &'static str) -> Option<PathBuf> {
    let task = tokio::task::spawn_blocking(move || {
        // SAFETY: As in `pick_open_file`.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().ok()?;
            let result = (|| {
                let dialog: IFileSaveDialog =
                    CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
                dialog.SetFileName(&HSTRING::from(default_name)).ok()?;
                dialog.SetTitle(&HSTRING::from(title)).ok()?;
                dialog.SetDefaultExtension(w!("iso")).ok()?;
                dialog.Show(None).ok()?;
                dialog_path(dialog.GetResult().ok()?)
            })();
            CoUninitialize();
            result
        }
    });
    task.await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessibility_settings_are_readable() {
        let _ = accessibility();
    }

    #[test]
    fn single_instance_mutex_blocks_second_caller_until_dropped() {
        // A running WindowCraft already owns the mutex.
        let Some(first) = claim_single_instance() else { return };
        assert!(claim_single_instance().is_none());
        drop(first);
        assert!(claim_single_instance().is_some());
    }

    #[test]
    fn backdrop_and_taskbar_progress_tolerate_null_window_handle() {
        apply_backdrop(0, true, true);
        apply_backdrop(0, false, false);
        taskbar_progress(0, Some(0.5));
        taskbar_progress(0, Some(0.0));
        taskbar_progress(0, None);
    }
}
