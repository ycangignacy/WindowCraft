# WindowCraft 0.1.0

This is the first pre-release of WindowCraft, a Windows 11 administration tool written in Rust. It runs on x64 and ARM64.

It installs 239 packages through WinGet or Chocolatey, applies 68 tweaks, removes 34 AppX packages and offers 33 system tools, with three Windows Update policy modes. ISO Studio customizes a Windows 11 ISO and writes it to a file or a USB drive. Time Machine records every change before making it, so registry and service changes can be rolled back, and it reports settings that drifted later.

**Read this before running it.** The features that change the system have only run in automated tests, against a test registry key and simulated services. Tweaks, DISM, AppX removal, BitLocker, ISO builds and USB writes have not yet run on a real Windows install. Use a virtual machine or a machine you can restore.

The builds are unsigned, so SmartScreen shows a warning on first run. No ARM64 binary has been started yet. The x64 installers passed install and uninstall tests on Windows 11 build 26300.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-0.1.0-x64-setup.exe`, `WindowCraft-0.1.0-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-0.1.0-x64.msi`, `WindowCraft-0.1.0-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-0.1.0-x64-portable.exe`, `WindowCraft-0.1.0-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates |

## Known bugs

These are open in 0.1.0 and will be fixed in a later release.

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| KB-5 | Clearing "ISO Studio work files" in the cleaner while an ISO session is open makes the next build fail on missing files. | Open the ISO again. |
| KB-6 | Any unclassified snapshot database error counts as corruption, so the history file is moved aside. A future `redb` format upgrade would take this path. | The file stays as `timemachine.corrupt-<time>`. |
| KB-7 | Unknown fields on catalog operations without parameters are accepted. This affects catalog data only. | None needed. |
| KB-8 | Windows features enabled through DISM stay enabled after a rollback. The session shows "Partly rolled back". | Run `dism /online /disable-feature`. |
| SEC-1 | The optional CTT PowerShell profile comes from a moving `main` branch, is not signed, and runs in every new PowerShell session. | Read the profile before installing it. |
| SEC-2 | The Adobe block list comes from a moving third-party branch. Only sinkhole entries are kept. | Check the entries in `hosts`. |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash. They must pass Authenticode, so they are trusted by publisher signature alone. | None needed. |

## Known limitations

The interface is in English only.

An uninstall deletes Time Machine history for every user with no warning, so roll changes back first if you want them undone. It leaves the settings you changed: tweaks, removed apps, installed packages including Chocolatey, OpenSSH Server with its firewall rule, restore points and backups kept beside system files, such as `hosts.bak`.

Time Machine cannot restore removed packages, uninstalled apps or BitLocker decryption. A rollback of OpenSSH Server removes its firewall rule, and the server and its services stay installed.

The portable build has no uninstaller and no `windowcraft-cli.exe`. To remove its data, run `windowcraft.exe --purge-data` from an elevated terminal, then delete the file. Without elevation this removes only your own data folder. A shell does not wait for this build unless you use `Start-Process -Wait` or pipe it to `Out-Null`.

If you elevate with a different administrator account, per-user tweaks go to that account, and it has its own Time Machine history. `--run` and `--check-drift` fail while the app is open, because only one process can hold the snapshot database.

An ISO build installs `oscdimg` through WinGet when it is missing and removes it afterwards, so that build needs a network connection.
