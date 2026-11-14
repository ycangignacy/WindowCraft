// Made by Nordan
//! Console front end for `windowcraft.exe`.
//!
//! The main binary uses the GUI subsystem so a double click opens no console,
//! and shells do not wait for such programs. This one is a console program:
//! it runs the sibling exe, shares its console and returns its exit code.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn sibling() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("windowcraft.exe"))
}

fn main() -> ExitCode {
    let Some(program) = sibling() else {
        eprintln!("error: cannot locate this program's folder");
        return ExitCode::FAILURE;
    };
    match Command::new(&program).args(std::env::args_os().skip(1)).status() {
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(error) => {
            eprintln!("error: {}: {error}", program.display());
            ExitCode::FAILURE
        }
    }
}
