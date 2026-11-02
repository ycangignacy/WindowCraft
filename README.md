<img src="crates/windowcraft-app/assets/logo-256.png" alt="WindowCraft logo" width="128">

# WindowCraft

WindowCraft is a Windows 11 administration, debloating, optimization and ISO mastering tool written in Rust. It provides a transactional snapshot engine, configuration drift detection and a system inspector.

Catalogs for applications, tweaks, features, AppX packages, DNS providers and presets come from Chris Titus Tech's WinUtil (<https://github.com/ChrisTitusTech/winutil>). WindowCraft is not affiliated with Chris Titus Tech or WinUtil.

## Download

Downloads are on the [Releases page](https://github.com/ycangignacy/WindowCraft/releases). Supported systems are Windows 11 on x64 and ARM64. Release notes for version 0.1.0 are in [docs/RELEASE-NOTES-0.1.0.md](docs/RELEASE-NOTES-0.1.0.md).

| File | Use it when |
| :--- | :--- |
| `WindowCraft-<version>-<arch>-setup.exe` | You want a standard installation with a Start menu entry. |
| `WindowCraft-<version>-<arch>.msi` | You deploy with Intune, GPO or `msiexec /qn`. |
| `WindowCraft-<version>-<arch>-portable.exe` | You run the tool without installing files to disk. |

Checksums are listed in `SHA256SUMS.txt`. Builds are unsigned, so Windows SmartScreen displays a warning on first run.

WindowCraft modifies system settings. Create a restore point before applying changes and inspect items in the X-Ray Inspector. Time Machine rolls back registry and service changes. Removed packages, uninstalled apps, enabled Windows features and BitLocker decryption have no automatic undo.

## Build

Building WindowCraft requires the Rust toolchain specified in `rust-toolchain.toml`, the MSVC build tools, and the Windows SDK.

```bash
cargo build --release -p windowcraft-app
```

The resulting binary is `target/release/windowcraft.exe`. It links the C runtime statically. It starts unelevated, and the sidebar badge restarts it with administrator privileges.

To build the installers into `dist/`, run this from PowerShell. It downloads WiX into `target/tools` and installs nothing on the machine.

```powershell
./packaging/build.ps1
```

`./packaging/make-icons.ps1` regenerates icons and installer images from `crates/windowcraft-app/assets/logo-source.jpg`.

ARM64 builds use `./packaging/build.ps1 -Architecture arm64`. This target requires `rustup target add aarch64-pc-windows-msvc`, the MSVC ARM64 tools, and clang on PATH or unpacked into `target/tools/llvm`.

To test the installers on a machine you can afford to change, run `./packaging/verify-install.ps1` from an elevated PowerShell. First runs of the system-changing features belong in a virtual machine. See [docs/VM-TEST-PLAN.md](docs/VM-TEST-PLAN.md).

## Verify

```bash
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tests do not modify live system settings. Registry tests are confined to `HKCU\Software\WindowCraft_TestSandbox` and delete the key on drop. Service operations execute against an in-memory table. Native operations go to a recording runner. `tests/real_machine_probe.rs` reads the live system and writes nothing.

## Command line

```bash
windowcraft-cli --preset standard --run
windowcraft-cli --config "C:\path\to\config.json" --run
windowcraft-cli --check-drift --fix
windowcraft-cli --export-report "C:\report.json"
windowcraft-cli --open tweaks --inspect telemetry
```

The installers place `windowcraft-cli.exe` next to `windowcraft.exe`. A shell waits for it, and it returns the exit code, which `windowcraft.exe` cannot do because it is a GUI program.

The portable build has only `windowcraft.exe`. To make PowerShell wait for it, use `Start-Process -Wait` or pipe it to `Out-Null`.

`--config` accepts WindowCraft profiles and WinUtil JSON files. Without `--run`, a preset or config opens in the interface for review. Commands that modify system settings require an elevated terminal.

## Workspace

| Crate | Purpose |
| :--- | :--- |
| `windowcraft-physics` | Spring solver, velocity tracking, momentum, gestures, demand-driven timeline |
| `windowcraft-config` | Typed schemas, embedded catalogs, profiles, global search |
| `windowcraft-engine` | Native Win32, COM, WinRT and DISM adapters, snapshots, drift, inspection |
| `windowcraft-iso` | ISO Studio: WIM bindings, answer files, drivers, FAT32, USB guard |
| `windowcraft-app` | Three-column graphical interface |
| `windowcraft-cli` | Console program for command line execution and exit codes |

## Rendering

The graphical renderer is software-based through `tiny-skia`. It meets memory and startup budgets (26 MB working set, 62 to 89 ms to a titled window). Its GDI surface preserves an alpha channel that DWM composites over Mica. `wgpu` is left out of the build: it used about 120 MB more memory and added 54 crates.

## Footprint

WindowCraft runs no background service, notification tray icon, scheduled task, update check or telemetry background process. It creates no custom registry key. It initiates network connections only when a user starts an action that requests one, such as downloading a package.

Closing the window during an active job prompts for confirmation. Quitting anyway cancels the job.

Application data is stored in `%LOCALAPPDATA%\WindowCraft`. This directory holds Time Machine snapshots, the append-only job log, exports, downloaded tools and ISO Studio work files.

Uninstalling removes `%LOCALAPPDATA%\WindowCraft` for all users, the `*.wcbak` folders left by Windows Update resets and the `AutoRegBackup` task. Time Machine history goes with it, so roll changes back first if you want them undone. An upgrade keeps your data.

The portable build has no uninstaller. `windowcraft.exe --purge-data` from an elevated terminal does the same cleanup, then delete the file. Without elevation it removes only your own data folder.

Settings you chose to change stay as they are after uninstalling. That covers tweaks, removed apps, installed packages including Chocolatey, OpenSSH Server with its firewall rule, restore points and the backups kept beside system files, such as `hosts.bak`.

## Safety

- Every change records the prior value in redb before the initial modification.
- A failing tweak reverts itself without halting the remaining queue.
- A rollback lists what it could not undo, and the session shows "Partly rolled back". Rolling back the `reg-backup` feature deletes its task, and rolling back OpenSSH Server removes its firewall rule. Enabled DISM features stay enabled.
- Seven critical services cannot be disabled: `RpcSs`, `DcomLaunch`, `LSM`, `BFE`, `mpssvc`, `WinDefend`, and `ProfSvc`.
- Temporary ACL adjustments use a `Drop` guard. The release profile keeps unwinding enabled because `panic = "abort"` would bypass the guard.
- System files are replaced atomically with a `.bak` backup copy.
- USB drive erasure requires the USB bus, absence of system volumes, valid disk geometry, and a matching model string re-checked before the initial write.
- Generated answer files never configure disks, so Setup always asks where to install.
- Downloaded hosts lists keep sinkhole entries only. Downloaded executables must pass Authenticode before launch.

See [NOTICE.md](NOTICE.md) for catalog attribution and [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for Rust crate notices.

Made by ycangignacy.
