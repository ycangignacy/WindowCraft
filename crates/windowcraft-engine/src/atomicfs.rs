// Made by WindowCraft Dev Team
//! Crash-safe replacement of system files, and the fenced hosts file editor.

use crate::error::{EngineError, Result};
use crate::win::{pcwstr, wide};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use windows::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW, REPLACEFILE_WRITE_THROUGH, ReplaceFileW,
};
use windows::core::PCWSTR;

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        // Already renamed away on success; nothing to report on failure.
        let _ = fs::remove_file(&self.0);
    }
}

/// Replaces `path` with `content` so a crash leaves either the old or the new
/// file, never a partial one. `.bak` holds the previous version.
///
/// `ReplaceFileW` keeps the target's ACL and attributes, which matters for
/// `hosts` and the OpenSSH files.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let label = || path.display().to_string();
    let parent = path.parent().ok_or_else(|| EngineError::Invalid(label()))?;
    let name = path.file_name().ok_or_else(|| EngineError::Invalid(label()))?.to_string_lossy();
    let temporary = parent.join(format!(".{name}.wc-{}.tmp", std::process::id()));
    let cleanup = RemoveOnDrop(temporary.clone());

    {
        let mut file = File::create(&temporary).map_err(|e| EngineError::io(temporary.display().to_string(), e))?;
        file.write_all(content).map_err(|e| EngineError::io(temporary.display().to_string(), e))?;
        file.sync_all().map_err(|e| EngineError::io(temporary.display().to_string(), e))?;
    }

    let target = wide(path);
    let source = wide(&temporary);
    if path.exists() {
        let backup = path.with_file_name(format!("{name}.bak"));
        fs::copy(path, &backup).map_err(|e| EngineError::io(backup.display().to_string(), e))?;
        // Defender and the DNS Client open `hosts` for a moment after every
        // change, so a sharing violation is retried before giving up. With
        // that error nothing has moved yet, so trying again is safe.
        let mut attempt = 0u32;
        loop {
            // SAFETY: All strings are null-terminated and outlive the call.
            let replaced = unsafe {
                ReplaceFileW(pcwstr(&target), pcwstr(&source), PCWSTR::null(), REPLACEFILE_WRITE_THROUGH, None, None)
            };
            match replaced {
                Ok(()) => break,
                Err(error) if attempt < REPLACE_RETRIES && is_sharing_error(&error) => {
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(100 << attempt));
                }
                Err(error) => return Err(EngineError::windows("ReplaceFileW", label(), &error)),
            }
        }
    } else {
        // SAFETY: Both strings are null-terminated and outlive the call.
        unsafe { MoveFileExW(pcwstr(&source), pcwstr(&target), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH) }
            .map_err(|error| EngineError::windows("MoveFileExW", label(), &error))?;
    }
    drop(cleanup);
    Ok(())
}

/// Retries after 200, 400, 800, 1600 and 3200 ms, about six seconds in all.
const REPLACE_RETRIES: u32 = 5;

/// ERROR_SHARING_VIOLATION or ERROR_LOCK_VIOLATION: another process has the
/// file open right now.
fn is_sharing_error(error: &windows::core::Error) -> bool {
    let code = error.code();
    code == windows::core::HRESULT::from_win32(32) || code == windows::core::HRESULT::from_win32(33)
}

fn begin_marker(section: &str) -> String {
    format!("# --- BEGIN WINDOWCRAFT {section} BLOCK ---")
}

fn end_marker(section: &str) -> String {
    format!("# --- END WINDOWCRAFT {section} BLOCK ---")
}

/// Removes a fenced section, leaving everything else byte-for-byte intact.
pub fn hosts_without(text: &str, section: &str) -> String {
    let (begin, end) = (begin_marker(section), end_marker(section));
    let Some(start) = text.find(&begin) else {
        return text.to_owned();
    };
    let Some(stop) = text[start..].find(&end).map(|offset| start + offset + end.len()) else {
        // An unterminated fence is not safe to guess at, so it stays.
        return text.to_owned();
    };
    let tail = text[stop..].strip_prefix("\r\n").or_else(|| text[stop..].strip_prefix('\n')).unwrap_or(&text[stop..]);
    format!("{}{tail}", &text[..start])
}

/// Installs a fenced section, replacing any earlier copy.
pub fn hosts_with(text: &str, section: &str, lines: &[String]) -> String {
    let mut output = hosts_without(text, section);
    if !output.is_empty() && !output.ends_with('\n') {
        output.push_str("\r\n");
    }
    output.push_str(&begin_marker(section));
    output.push_str("\r\n");
    for line in lines {
        output.push_str(line);
        output.push_str("\r\n");
    }
    output.push_str(&end_marker(section));
    output.push_str("\r\n");
    output
}

/// Keeps only sinkhole entries from a downloaded block list. A remote list
/// must never be able to point a real hostname at an arbitrary address.
pub fn sanitize_blocklist(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let entry = raw.split('#').next().unwrap_or_default().trim();
        let mut fields = entry.split_whitespace();
        let (Some(address), Some(host), None) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        let sinkhole = matches!(address, "0.0.0.0" | "127.0.0.1" | "::" | "::1");
        let valid_host = host.len() <= 253
            && host.contains('.')
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if sinkhole && valid_host {
            lines.push(format!("0.0.0.0 {host}"));
        }
    }
    lines.sort_unstable();
    lines.dedup();
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_creates_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        atomic_write(&path, b"one").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
        assert!(!dir.path().join("hosts.bak").exists());

        atomic_write(&path, b"two").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(fs::read(dir.path().join("hosts.bak")).unwrap(), b"one");
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sshd_config");
        atomic_write(&path, b"a").unwrap();
        atomic_write(&path, b"b").unwrap();
        let names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[test]
    fn failed_write_leaves_the_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("target");
        atomic_write(&path, b"original").unwrap();
        let blocked = dir.path().join("no-such-dir").join("target");
        assert!(atomic_write(&blocked, b"new").is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
    }

    #[test]
    fn a_briefly_held_target_is_replaced_once_it_is_released() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        atomic_write(&path, b"original").unwrap();
        // Readers may share it, as a scanner does, but nobody may replace it.
        let held = fs::OpenOptions::new().read(true).share_mode(1).open(&path).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            drop(held);
        });
        atomic_write(&path, b"new").unwrap();
        release.join().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
    }

    #[test]
    fn hosts_block_roundtrip_is_lossless() {
        let original = "# Copyright\r\n127.0.0.1 localhost\r\n10.0.0.5 intranet\r\n";
        let lines = vec!["0.0.0.0 a.example".to_owned(), "0.0.0.0 b.example".to_owned()];
        let installed = hosts_with(original, "ADOBE", &lines);
        assert!(installed.contains("# --- BEGIN WINDOWCRAFT ADOBE BLOCK ---"));
        assert!(installed.starts_with(original));
        assert_eq!(hosts_without(&installed, "ADOBE"), original);
    }

    #[test]
    fn reinstalling_replaces_instead_of_stacking() {
        let once = hosts_with("", "ADOBE", &["0.0.0.0 a.example".to_owned()]);
        let twice = hosts_with(&once, "ADOBE", &["0.0.0.0 b.example".to_owned()]);
        assert_eq!(twice.matches("BEGIN WINDOWCRAFT").count(), 1);
        assert!(!twice.contains("a.example"));
    }

    #[test]
    fn other_sections_and_trailing_entries_survive() {
        let first = hosts_with("10.0.0.1 a\r\n", "ONE", &["0.0.0.0 x.example".to_owned()]);
        let second = hosts_with(&first, "TWO", &["0.0.0.0 y.example".to_owned()]);
        let extended = format!("{second}10.0.0.2 b\r\n");
        let removed = hosts_without(&extended, "ONE");
        assert!(removed.contains("y.example"));
        assert!(removed.contains("10.0.0.1 a"));
        assert!(removed.contains("10.0.0.2 b"));
    }

    #[test]
    fn unterminated_fence_is_left_alone() {
        let text = "# --- BEGIN WINDOWCRAFT ADOBE BLOCK ---\r\n10.0.0.1 keep\r\n";
        assert_eq!(hosts_without(text, "ADOBE"), text);
    }

    #[test]
    fn blocklist_rejects_anything_but_sinkholes() {
        let list = "\
# header
0.0.0.0 ads.example.com
127.0.0.1 tracker.example.com # note
6.6.6.6 login.bank.example
0.0.0.0 bad host extra
0.0.0.0 inv@lid.example
0.0.0.0 localhost

0.0.0.0 ads.example.com";
        assert_eq!(sanitize_blocklist(list), ["0.0.0.0 ads.example.com", "0.0.0.0 tracker.example.com"]);
    }
}
