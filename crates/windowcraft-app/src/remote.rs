// Made by WindowCraft Dev Team
//! Hands a second launch over to the running window.
//!
//! The single-instance mutex stops a second GUI. Before it exits, the second
//! launch sends its command line and working folder in a `WM_COPYDATA`
//! message, and the running window parses them with the same `Cli` as a
//! fresh start, so `--open`, `--inspect`, `--config` and `--preset` work the
//! same way in both.

use iced::futures::stream::{self, Stream};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    ChangeWindowMessageFilterEx, EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsIconic, MSGFLT_ALLOW,
    SMTO_ABORTIFHUNG, SW_RESTORE, SendMessageTimeoutW, SetForegroundWindow, ShowWindow, WM_COPYDATA,
};
use windows::core::{BOOL, PWSTR};

/// Marks a WindowCraft hand-over among other `WM_COPYDATA` traffic.
const MAGIC: usize = 0x5743_4F50;
/// A command line never comes near this. Larger payloads are ignored.
const MAX_BYTES: usize = 64 * 1024;
const SUBCLASS_ID: usize = 0x5743;

/// A second launch: its arguments without the program name, and the folder
/// relative paths in them start from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub cwd: PathBuf,
    pub args: Vec<String>,
}

impl Request {
    /// The arguments of this process.
    pub fn current() -> Self {
        Self {
            cwd: std::env::current_dir().unwrap_or_default(),
            args: std::env::args().skip(1).collect(),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        json!({ "cwd": self.cwd.to_string_lossy(), "args": self.args }).to_string().into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_BYTES {
            return None;
        }
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let cwd = PathBuf::from(value.get("cwd")?.as_str()?);
        let args = value.get("args")?.as_array()?.iter().map(|arg| arg.as_str().map(str::to_owned)).collect::<Option<_>>()?;
        Some(Self { cwd, args })
    }
}

/// `COPYDATASTRUCT` from winuser.h.
#[repr(C)]
struct CopyData {
    data: usize,
    size: u32,
    bytes: *const core::ffi::c_void,
}

fn image_name(process_id: u32) -> Option<String> {
    // SAFETY: The handle is closed below. The buffer outlives the call and
    // `size` holds its length in characters.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id).ok()?;
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        let named = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size);
        let _ = CloseHandle(process);
        named.ok()?;
        let path = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));
        Some(path.file_name()?.to_string_lossy().into_owned())
    }
}

/// True for the main window of a WindowCraft process. A File Explorer window
/// open on a folder named WindowCraft has the same title, so the process
/// decides.
fn is_windowcraft(window: HWND) -> bool {
    let mut title = [0u16; 32];
    // SAFETY: The buffer outlives the call and its length is passed.
    let length = unsafe { GetWindowTextW(window, &mut title) };
    if String::from_utf16_lossy(&title[..length.max(0) as usize]) != "WindowCraft" {
        return false;
    }
    let mut process_id = 0;
    // SAFETY: `process_id` is a valid out pointer.
    unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    process_id != std::process::id() && image_name(process_id).is_some_and(|name| name.eq_ignore_ascii_case("windowcraft.exe"))
}

unsafe extern "system" fn find_one(window: HWND, found: LPARAM) -> BOOL {
    if is_windowcraft(window) {
        // SAFETY: `found` points at the Option in `find_window`, alive for the enumeration.
        unsafe { *(found.0 as *mut Option<HWND>) = Some(window) };
        return BOOL::from(false);
    }
    BOOL::from(true)
}

fn find_window() -> Option<HWND> {
    let mut found: Option<HWND> = None;
    // SAFETY: The callback only writes through the pointer while `found` lives.
    // EnumWindows reports an error when the callback stops early, which is
    // the success case here.
    let _ = unsafe { EnumWindows(Some(find_one), LPARAM(&raw mut found as isize)) };
    found
}

/// Brings the running window forward and hands it `request`. A minimized
/// window is restored; a maximized one stays maximized. Returns false when
/// no window answered, for example an older version that ignores the message.
pub fn send(request: &Request) -> bool {
    let Some(window) = find_window() else { return false };
    let bytes = request.encode();
    let data = CopyData { data: MAGIC, size: bytes.len() as u32, bytes: bytes.as_ptr().cast() };
    let mut answer = 0usize;
    // SAFETY: `window` came from EnumWindows. `data` and `bytes` outlive the
    // synchronous send, which the receiver copies before returning.
    unsafe {
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        let _ = SetForegroundWindow(window);
        let _ = SendMessageTimeoutW(
            window,
            WM_COPYDATA,
            WPARAM(0),
            LPARAM(&raw const data as isize),
            SMTO_ABORTIFHUNG,
            2000,
            Some(&mut answer),
        );
    }
    answer == 1
}

/// The window procedure sends; the subscription takes the receiver once.
type Channel = (UnboundedSender<Request>, Mutex<Option<UnboundedReceiver<Request>>>);

fn channel() -> &'static Channel {
    static CHANNEL: OnceLock<Channel> = OnceLock::new();
    CHANNEL.get_or_init(|| {
        let (sender, receiver) = unbounded_channel();
        (sender, Mutex::new(Some(receiver)))
    })
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_COPYDATA && lparam.0 != 0 {
        // SAFETY: For WM_COPYDATA, `lparam` points at a COPYDATASTRUCT that
        // stays valid until this call returns.
        let data = unsafe { &*(lparam.0 as *const CopyData) };
        if data.data == MAGIC && (data.size as usize) <= MAX_BYTES && !data.bytes.is_null() {
            // SAFETY: The sender owns `size` bytes at `bytes` for the call.
            let bytes = unsafe { std::slice::from_raw_parts(data.bytes.cast::<u8>(), data.size as usize) };
            if let Some(request) = Request::decode(bytes) {
                let _ = channel().0.send(request);
                return LRESULT(1);
            }
            return LRESULT(0);
        }
    }
    // SAFETY: Passes every other message on unchanged.
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

/// Lets `window` receive hand-overs, also from a launch without elevation
/// while this one runs elevated.
pub fn listen(window: u64) {
    let hwnd = HWND(window as *mut core::ffi::c_void);
    // SAFETY: `hwnd` is this process's live main window. Subclassing twice
    // with the same id only replaces the data, so a repeat is harmless.
    unsafe {
        let _ = ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW, None);
        let _ = SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0);
    }
}

/// Hand-overs as they arrive. Only the first subscription gets them.
pub fn requests() -> impl Stream<Item = Request> {
    let receiver = channel().1.lock().ok().and_then(|mut slot| slot.take());
    stream::unfold(receiver, |receiver| async move {
        let mut receiver = receiver?;
        let request = receiver.recv().await?;
        Some((request, Some(receiver)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_survives_the_trip() {
        let request = Request { cwd: PathBuf::from(r"C:\Users\Test"), args: vec!["--open".into(), "iso".into(), "żółć \"x\"".into()] };
        assert_eq!(Request::decode(&request.encode()), Some(request));
    }

    #[test]
    fn broken_or_oversized_payloads_are_ignored() {
        assert_eq!(Request::decode(b"not json"), None);
        assert_eq!(Request::decode(br#"{"cwd":"C:\\","args":[1]}"#), None);
        assert_eq!(Request::decode(&vec![b' '; MAX_BYTES + 1]), None);
    }
}
