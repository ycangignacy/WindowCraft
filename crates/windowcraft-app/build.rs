// Made by WindowCraft Dev Team
//! Embeds the manifest, icon and version resource.
//!
//! The binary stays `asInvoker` and elevates at runtime, so read-only views
//! and tests never trigger a UAC prompt.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=windowcraft.manifest");
    println!("cargo:rerun-if-changed=assets/windowcraft.ico");
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env != "msvc" {
        return;
    }
    let directory = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let manifest = Path::new(&directory).join("windowcraft.manifest");
    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
    // Static imports resolve from System32 only, which blocks DLL planting
    // next to the executable before any Rust code runs.
    println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");

    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/windowcraft.ico")
        .set("ProductName", "WindowCraft")
        .set("FileDescription", "WindowCraft")
        .set("CompanyName", "WindowCraft Dev Team")
        .set("LegalCopyright", "Copyright (c) 2026 WindowCraft Dev Team. MIT License.")
        .set("OriginalFilename", "windowcraft.exe");
    if let Err(error) = resource.compile() {
        println!("cargo:warning=version resource not embedded: {error}");
    }
}
