// Made by Nordan
//! The scheduled drift check and its notification.
//!
//! The task runs `windowcraft.exe --scheduled-check` as the signed-in user.
//! It checks only when the interval chosen in Settings has passed or the
//! Windows build changed, and stays silent when nothing was reverted. When
//! something was, a notification offers to open Time Machine. When Windows
//! shows no notifications at all, a message box asks instead.

use crate::i18n::{self, Msg, format_1, t};
use crate::settings;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::sync::atomic::{AtomicBool, Ordering};
use windowcraft_config::Catalog;
use windowcraft_engine::Engine;
use windowcraft_engine::snapshot::unix_now;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_LARGE_ICON, NIIF_USER, NIM_ADD, NIM_DELETE, NIM_SETVERSION,
    NIN_BALLOONTIMEOUT, NIN_BALLOONUSERCLICK, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HICON, IDI_APPLICATION, KillTimer,
    LoadIconW, MSG, PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage, WINDOW_EX_STYLE, WM_APP,
    WM_LBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

const CALLBACK: u32 = WM_APP + 1;
const NIN_BALLOONHIDE: u32 = 0x0403;
/// The notification gives up after this long without a click.
const WAIT_MS: u32 = 60_000;
/// A notification that is gone sooner than this was never on screen. Windows
/// drops it at once when notifications are turned off.
const SHOWN_AT_LEAST: Duration = Duration::from_millis(1500);

static CLICKED: AtomicBool = AtomicBool::new(false);

/// True while a WindowCraft window is open. Its own drift banner covers that case.
fn app_running() -> bool {
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};
    // SAFETY: The name is a static string; a handle that opens is closed by the guard.
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, w!("Local\\WindowCraft.SingleInstance")) } {
        Ok(handle) => {
            drop(windowcraft_engine::win::HandleGuard(handle));
            true
        }
        Err(_) => false,
    }
}

pub fn run() -> Result<(), String> {
    let mut chosen = settings::load();
    if !chosen.wants_schedule() || app_running() {
        return Ok(());
    }
    let build = windowcraft_engine::sysinfo::query().map_err(|e| e.to_string())?.full_build();
    let now = unix_now();
    if !chosen.check_due(now, &build) {
        return Ok(());
    }
    let catalog = Arc::new(Catalog::load()?);
    let engine = Engine::system(catalog).map_err(|e| e.to_string())?;
    let drifted = engine.detect_drift().map_err(|e| e.to_string())?;
    drop(engine);
    chosen.last_check = now;
    chosen.last_build = build;
    settings::save(&chosen)?;
    if drifted.is_empty() {
        return Ok(());
    }
    let lang = i18n::detect_language();
    let body = format_1(t(lang, Msg::NotifyDriftBody), &drifted.len().to_string());
    let open = match notify(t(lang, Msg::NotifyDriftTitle), &body) {
        Outcome::Clicked => true,
        Outcome::Dismissed => false,
        Outcome::NotShown => ask(t(lang, Msg::NotifyDriftTitle), &format!("{body}\n\n{}", t(lang, Msg::NotifyOpenQuestion))),
    };
    if open {
        open_time_machine();
    }
    Ok(())
}

/// Message box used when notifications are off. True for Yes.
fn ask(title: &str, body: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{
        IDYES, MB_ICONINFORMATION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MessageBoxW,
    };
    use windows::core::HSTRING;
    // SAFETY: Both strings are null-terminated and outlive the call.
    let answer = unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(body),
            &HSTRING::from(title),
            MB_YESNO | MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST,
        )
    };
    answer == IDYES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Clicked,
    Dismissed,
    /// Windows did not show it, for example because notifications are off.
    NotShown,
}

fn open_time_machine() {
    if let Ok(program) = std::env::current_exe() {
        let _ = std::process::Command::new(program).args(["--open", "timemachine"]).spawn();
    }
}

fn copy_into(target: &mut [u16], text: &str) {
    let limit = target.len() - 1;
    let mut length = 0;
    for (slot, unit) in target.iter_mut().zip(text.encode_utf16().take(limit)) {
        *slot = unit;
        length += 1;
    }
    target[length] = 0;
}

extern "system" fn window_proc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        CALLBACK => {
            // With NOTIFYICON_VERSION_4 the event is in the low word of lParam.
            let event = (lparam.0 & 0xFFFF) as u32;
            if event == NIN_BALLOONUSERCLICK || event == NIN_SELECT || event == WM_LBUTTONUP {
                CLICKED.store(true, Ordering::SeqCst);
                // SAFETY: Called on the thread that owns the message loop.
                unsafe { PostQuitMessage(0) };
            } else if event == NIN_BALLOONTIMEOUT || event == NIN_BALLOONHIDE {
                // SAFETY: As above.
                unsafe { PostQuitMessage(0) };
            }
            LRESULT(0)
        }
        WM_TIMER => {
            // SAFETY: As above.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: Unhandled messages go to the default procedure with their own arguments.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// Shows a Windows notification and waits until it is clicked or gone.
fn notify(title: &str, body: &str) -> Outcome {
    CLICKED.store(false, Ordering::SeqCst);
    let started = Instant::now();
    // SAFETY: Plain Win32 calls on this thread. The window, icon and timer are
    // released before returning, and every string outlives the call using it.
    unsafe {
        let Ok(module) = GetModuleHandleW(None) else { return Outcome::NotShown };
        let instance = module.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("WindowCraft.DriftNotice"),
            ..WNDCLASSW::default()
        };
        RegisterClassW(&class);
        let Ok(window) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class.lpszClassName,
            w!("WindowCraft drift notice"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        ) else {
            return Outcome::NotShown;
        };
        // The executable's icon is resource 1, passed as MAKEINTRESOURCE(1).
        let icon: HICON = LoadIconW(Some(instance), PCWSTR(std::ptr::without_provenance::<u16>(1)))
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default();

        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: 1,
            uFlags: NIF_ICON | NIF_TIP | NIF_MESSAGE | NIF_INFO,
            uCallbackMessage: CALLBACK,
            hIcon: icon,
            dwInfoFlags: NIIF_USER | NIIF_LARGE_ICON,
            hBalloonIcon: icon,
            ..NOTIFYICONDATAW::default()
        };
        copy_into(&mut data.szTip, "WindowCraft");
        copy_into(&mut data.szInfoTitle, title);
        copy_into(&mut data.szInfo, body);
        if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            let _ = DestroyWindow(window);
            return Outcome::NotShown;
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);

        let timer = SetTimer(Some(window), 1, WAIT_MS, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        if timer != 0 {
            let _ = KillTimer(Some(window), timer);
        }
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        let _ = DestroyWindow(window);
    }
    if CLICKED.load(Ordering::SeqCst) {
        Outcome::Clicked
    } else if started.elapsed() < SHOWN_AT_LEAST {
        Outcome::NotShown
    } else {
        Outcome::Dismissed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shows a real notification for up to a minute. Run it by hand.
    #[test]
    #[ignore = "shows a notification on the desktop"]
    fn shows_a_notification() {
        let outcome = notify("WindowCraft test", "Tweaks reverted: 3. Click to see them in WindowCraft Time Machine.");
        println!("outcome: {outcome:?}");
    }

    #[test]
    fn strings_are_cut_to_the_buffer_and_terminated() {
        let mut buffer = [0xFFFFu16; 6];
        copy_into(&mut buffer, "WindowCraft");
        assert_eq!(String::from_utf16(&buffer[..5]).unwrap(), "Windo");
        assert_eq!(buffer[5], 0);

        let mut buffer = [0xFFFFu16; 8];
        copy_into(&mut buffer, "ok");
        assert_eq!(&buffer[..3], &[u16::from(b'o'), u16::from(b'k'), 0]);
    }
}
