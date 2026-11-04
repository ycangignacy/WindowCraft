# WindowCraft 1.0.1

This is the first stable release of WindowCraft, a Windows 11 tool for apps, tweaks, Windows Update policy and custom installation media. It runs on x64 and ARM64, in English, Polish, German, French, Spanish, Italian, Brazilian Portuguese and Ukrainian.

Before this release every feature that changes the system ran on a real Windows install for the first time: Windows 11 Home 26H2, build 26300, in VirtualBox. The run found the bugs fixed below.

## Tested on a live system

Each change was applied, checked with `reg`, `Get-Service`, `Get-WindowsOptionalFeature` and similar commands instead of WindowCraft's own code, and rolled back.

| Area | Result |
| :--- | :--- |
| Tweaks | Applied and rolled back to the exact earlier values, by single change and by session |
| Drift | A value reset outside WindowCraft was found, repaired, and a rollback still reached the original baseline |
| Services | Start types set and restored; WinDefend refused from a config file |
| AppX | Feedback Hub removed for all users and deprovisioned |
| Windows features | WSL enabled; the rollback turned it off and left VirtualMachinePlatform, which was on before |
| Windows Update | Security and Disable ALL modes; rolling back Disable ALL returned to Security |
| BitLocker | Decryption started on a volume Windows had begun to encrypt |
| ISO Studio | A Windows 11 ISO built with bypasses, local account and the bundled profile; it mounts |
| USB | Only the USB disk was offered; written as GPT, FAT32, WIN11, with `install.wim` split |
| Installing from that media | Setup installed Pro on a VM with no TPM, offered the local account path, ran `SetupComplete.cmd` and removed WindowCraft after the first logon; 18 of 19 default AppX packages were gone |
| Command line | Exit codes, the elevation check, drift check and repair, and the environment report |
| Installers | `verify-install.ps1` passed; MSI upgrades from 0.1.0 and from 1.0.0, and a `setup.exe` upgrade from 0.1.0, kept the Time Machine history byte for byte |

## Fixed

| Was | Now |
| :--- | :--- |
| "Services - Set to Manual", and with it the Minimal and Standard presets, failed on Windows Home, which has no CscService. | CscService is optional. |
| Memory read as 0 in virtual machines. The report said `"ram_gb": 0`, and the svchost split tweak would have written a threshold of 0. | GlobalMemoryStatusEx is the fallback. |
| "Dark mode by default" in ISO Studio had no effect on 26H2, which ignores the answer file's theme values. | `SetupComplete.cmd` writes dark mode into the Default user hive. This path delivered the privacy values in the live install; the dark mode values themselves were not run through a second install. |
| Built ISOs had the volume label DVD_ROM. | They are labeled WIN11, like USB drives. |

## Not tested yet

ARM64: the files build and have the right platform, but no ARM64 binary has run on ARM hardware.

Applying a bundled profile at first logon: in the test the bundle held the test runner instead of `windowcraft.exe`, so the cleanup after first logon ran for real and the profile step did not.

By hand: the setup screens in every language, an English Windows, scaling above 100%, high contrast, keyboard-only use and Narrator, uninstall with two user profiles, and Windows 11 build 22000.

## Known bugs

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash, because their vendors publish them at fixed "latest" URLs. They must pass Authenticode. | None needed. |
| KB-9 | Uninstalling while WindowCraft is open removes the files and data, and the open window keeps running until it is closed. | Close WindowCraft before you uninstall. |

## Known limitations

The builds are not code-signed yet, so SmartScreen warns on first run. For 1.0.0, VirusTotal reported machine learning verdicts on the portable exe (Microsoft `Wacatac.C!ml`, SentinelOne "Static AI") and at most one engine on the installers.

On Windows 11 26H2, `Microsoft.StartExperiencesApp` is part of the system and is not provisioned, so the default AppX removal leaves it.

Sessions recorded by 0.1.0 do not know whether a feature was on before, so rolling them back still leaves Windows features enabled. Parent features that DISM enables along with a requested one stay on after a rollback.

The CTT PowerShell profile updates itself from its author's `main` branch every 7 days in interactive PowerShell sessions. That is the profile's own behavior; WindowCraft only pins the version it installs.

`setup.exe` has no language picker of its own. To start the app in a language other than the Windows display language, use `WCLANGUAGE` or the list in the app.

An uninstall deletes Time Machine history for every user with no warning, so roll changes back first if you want them undone. It leaves the settings you changed: tweaks, removed apps, installed packages including Chocolatey, OpenSSH Server with its firewall rule, restore points and backups kept beside system files, for example `hosts.bak`.

Time Machine cannot restore removed packages, uninstalled apps or BitLocker decryption. A rollback of OpenSSH Server removes its firewall rule, and the server and its services stay installed.

The portable build has no uninstaller and no `windowcraft-cli.exe`. To remove its data, run `windowcraft.exe --purge-data` from an elevated terminal, then delete the file. Without elevation this removes only your own data folder. A shell does not wait for this build unless you use `Start-Process -Wait` or pipe it to `Out-Null`.

If you elevate with a different administrator account, per-user tweaks go to that account, and it has its own Time Machine history. `--run` and `--check-drift` fail while the app is open, because only one process can hold the snapshot database.

An ISO build installs `oscdimg` through WinGet when it is missing and removes it afterwards, so that build needs a network connection. ISO Studio cannot open an ISO that sits on a CD or DVD; copy it to a disk first.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.0.1-x64-setup.exe`, `WindowCraft-1.0.1-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.0.1-x64.msi`, `WindowCraft-1.0.1-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.0.1-x64-portable.exe`, `WindowCraft-1.0.1-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates |
