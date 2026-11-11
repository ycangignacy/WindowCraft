// Made by ycangignacy
//! Child processes, used only where Windows exposes no public API.
//!
//! Binaries are addressed by absolute path, windows stay hidden, and a guard
//! kills the child if the caller is cancelled or unwinds.

use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const OUTPUT_LINES: usize = 256;

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // Reaping a child that already exited is harmless, so both results
        // can be ignored.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub code: i32,
    pub text: String,
}

/// Decodes console output, which may be UTF-16 (`sfc`) or the OEM code page,
/// and text files such as driver INFs, which may be UTF-16 with a byte order
/// mark.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(body) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        return String::from_utf16_lossy(&units);
    }
    let looks_utf16 = bytes.len() >= 2 && bytes.iter().skip(1).step_by(2).take(32).filter(|&&b| b == 0).count() > 8;
    if looks_utf16 {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn pump(stream: impl Read + Send + 'static, lines: SyncSender<String>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        let mut buffer = Vec::new();
        loop {
            buffer.clear();
            match reader.read_until(b'\n', &mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = decode(&buffer).trim().to_owned();
                    if line.is_empty() {
                        continue;
                    }
                    match lines.try_send(line) {
                        Ok(()) | Err(TrySendError::Full(_)) => {}
                        Err(TrySendError::Disconnected(_)) => break,
                    }
                }
            }
        }
    });
}

/// Runs a program to completion, streaming output lines to `on_line`.
pub fn run(
    program: &Path,
    args: &[&str],
    cancel: &CancelToken,
    timeout: Duration,
    on_line: impl FnMut(&str),
) -> Result<Output> {
    run_with_env(program, args, &[], cancel, timeout, on_line)
}

/// Like `run`, with extra environment variables for the child.
pub fn run_with_env(
    program: &Path,
    args: &[&str],
    env: &[(&str, &str)],
    cancel: &CancelToken,
    timeout: Duration,
    mut on_line: impl FnMut(&str),
) -> Result<Output> {
    let name = program.file_name().map_or_else(|| program.display().to_string(), |n| n.to_string_lossy().into_owned());
    let child = Command::new(program)
        .args(args)
        .envs(env.iter().copied())
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| EngineError::io(name.clone(), error))?;
    let mut child = KillOnDrop(child);

    let (sender, receiver) = sync_channel(OUTPUT_LINES);
    if let Some(stdout) = child.0.stdout.take() {
        pump(stdout, sender.clone());
    }
    if let Some(stderr) = child.0.stderr.take() {
        pump(stderr, sender);
    }

    let started = Instant::now();
    let mut tail = Vec::new();
    loop {
        while let Ok(line) = receiver.try_recv() {
            on_line(&line);
            tail.push(line);
            if tail.len() > OUTPUT_LINES {
                tail.remove(0);
            }
        }
        if let Some(status) = child.0.try_wait().map_err(|error| EngineError::io(name.clone(), error))? {
            // Pipes drain briefly after exit.
            while let Ok(line) = receiver.recv_timeout(Duration::from_millis(40)) {
                on_line(&line);
                tail.push(line);
            }
            return Ok(Output { code: status.code().unwrap_or(-1), text: tail.join("\n") });
        }
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        if started.elapsed() > timeout {
            return Err(EngineError::Process {
                program: name,
                code: -1,
                detail: format!("timed out after {} seconds", timeout.as_secs()),
            });
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Runs a program and treats any non-zero exit as failure.
pub fn run_ok(program: &Path, args: &[&str], cancel: &CancelToken, timeout: Duration) -> Result<String> {
    let output = run(program, args, cancel, timeout, |_| {})?;
    if output.code != 0 {
        return Err(EngineError::Process {
            program: program.display().to_string(),
            code: output.code,
            detail: output.text.lines().last().unwrap_or_default().to_owned(),
        });
    }
    Ok(output.text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win::system32;

    #[test]
    fn captures_output_and_exit_code() {
        let cmd = system32("cmd.exe");
        let cancel = CancelToken::new();
        let output = run(&cmd, &["/c", "echo hello & exit 3"], &cancel, Duration::from_secs(10), |_| {}).unwrap();
        assert_eq!(output.code, 3);
        assert!(output.text.contains("hello"));
    }

    #[test]
    fn nonzero_exit_is_an_error_for_run_ok() {
        let cmd = system32("cmd.exe");
        let result = run_ok(&cmd, &["/c", "exit 1"], &CancelToken::new(), Duration::from_secs(10));
        assert!(matches!(result, Err(EngineError::Process { code: 1, .. })));
    }

    #[test]
    fn timeout_kills_the_child() {
        let ping = system32("ping.exe");
        let started = Instant::now();
        let result = run(&ping, &["-n", "30", "127.0.0.1"], &CancelToken::new(), Duration::from_millis(300), |_| {});
        assert!(matches!(result, Err(EngineError::Process { .. })));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn cancellation_kills_the_child_promptly() {
        let ping = system32("ping.exe");
        let cancel = CancelToken::new();
        let trigger = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            trigger.cancel();
        });
        let started = Instant::now();
        let result = run(&ping, &["-n", "30", "127.0.0.1"], &cancel, Duration::from_secs(60), |_| {});
        assert!(matches!(result, Err(EngineError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn missing_program_is_reported() {
        let result = run(Path::new(r"C:\no\such.exe"), &[], &CancelToken::new(), Duration::from_secs(1), |_| {});
        assert!(result.unwrap_err().is_not_found());
    }

    #[test]
    fn utf16_output_is_decoded() {
        let bytes: Vec<u8> = "Verification complete".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(decode(&bytes), "Verification complete");
        assert_eq!(decode(b"plain"), "plain");
    }
}
