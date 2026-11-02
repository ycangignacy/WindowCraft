# Contributing to WindowCraft

Send changes as GitHub pull requests. WindowCraft is written in Rust for Windows 11 on x64 and ARM64.

The project requires the Rust toolchain specified in `rust-toolchain.toml`, the MSVC build tools, and the Windows SDK.

## Build Commands

To build the release binary for the graphical interface:

```bash
cargo build --release -p windowcraft-app
```

The resulting binary is `target/release/windowcraft.exe`.

To build the installer packages into `dist/`, run PowerShell:

```powershell
./packaging/build.ps1
```

For ARM64 installers, provide the architecture parameter:

```powershell
./packaging/build.ps1 -Architecture arm64
```

Building for ARM64 also requires the target `aarch64-pc-windows-msvc`, the MSVC ARM64 build tools, and clang.

## Verification Commands

Run the following checks before submitting changes:

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Installer packages are verified by running PowerShell with elevated administrator privileges:

```powershell
./packaging/verify-install.ps1
```

## System Safety in Tests

Tests never change live system state. Code submitted in pull requests must preserve this separation.

Registry tests are confined to `HKCU\Software\WindowCraft_TestSandbox` and delete that tree on drop. Service operations run against an in-memory table. Native operations go to a recording runner. The integration probe in `crates/windowcraft-engine/tests/real_machine_probe.rs` reads the live system and writes nothing.
