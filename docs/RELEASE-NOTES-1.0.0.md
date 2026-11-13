<!-- Made by WindowCraft Dev Team -->
# WindowCraft 1.0.0

This pre-release adds seven interface languages and fixes the bugs listed as open in 0.1.0. It runs on Windows 11, x64 and ARM64.

The features that change the system have run in automated tests and on an elevated Windows Server 2025 CI runner, against a test registry key and simulated services. Tweaks, DISM, AppX removal, BitLocker, ISO builds and USB writes have not yet run on a real Windows install. Use a virtual machine or a machine you can restore. The builds are unsigned, so SmartScreen shows a warning on first run. No ARM64 binary has been started yet.

## Languages

The interface is in English, Polish, German, French, Spanish, Italian, Brazilian Portuguese and Ukrainian. WindowCraft starts in the Windows display language when it is one of these, and the list at the bottom of the sidebar changes it without a restart. `setup.exe` follows the Windows display language too. `WCLANGUAGE=pl` (or `en`, `de`, `fr`, `es`, `it`, `pt-BR`, `uk`) on the MSI or `setup.exe` command line presets the language for every user.

Catalog names and descriptions stay in English, and so do error details passed on from Windows and its tools, the job log and command line output.

## Fixed

| ID | Was | Now |
| :--- | :--- | :--- |
| KB-5 | Clearing "ISO Studio work files" while an ISO session was open broke the next build. | The cleaner skips them while an ISO is open or an image is mounted. |
| KB-6 | Any unclassified snapshot database error counted as corruption, and the history file was moved aside. | Only a corrupted page or an unreadable header moves it. A format upgrade or another error leaves the file in place and reports it. |
| KB-7 | Unknown fields on operations without parameters were accepted. | A catalog test finds them. |
| KB-8 | Windows features enabled through DISM stayed enabled after a rollback. | A rollback turns off the features WindowCraft turned on and leaves those that were on before. |
| SEC-1 | The CTT PowerShell profile came from a moving `main` branch. | It comes from a fixed commit and must match a SHA-256 hash. |
| SEC-2 | The Adobe block list came from a moving branch. | It comes from a fixed commit and must match a SHA-256 hash. Catalog validation rejects unpinned lists. |

## Known bugs

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash, because their vendors publish them at fixed "latest" URLs. They must pass Authenticode, so they are trusted by publisher signature. | None needed. |

## Known limitations

Sessions recorded by 0.1.0 do not know whether a feature was on before, so rolling them back still leaves Windows features enabled. Parent features that DISM enables along with a requested one stay on after a rollback.

The CTT PowerShell profile updates itself from its author's `main` branch every 7 days in interactive PowerShell sessions. That is the profile's own behavior; WindowCraft only pins the version it installs.

`setup.exe` has no language picker of its own. To start the app in a language other than the Windows display language, use `WCLANGUAGE` or the list in the app.

An uninstall deletes Time Machine history for every user with no warning, so roll changes back first if you want them undone. It leaves the settings you changed: tweaks, removed apps, installed packages including Chocolatey, OpenSSH Server with its firewall rule, restore points and backups kept beside system files, for example `hosts.bak`.

Time Machine cannot restore removed packages, uninstalled apps or BitLocker decryption. A rollback of OpenSSH Server removes its firewall rule, and the server and its services stay installed.

The portable build has no uninstaller and no `windowcraft-cli.exe`. To remove its data, run `windowcraft.exe --purge-data` from an elevated terminal, then delete the file. Without elevation this removes only your own data folder. A shell does not wait for this build unless you use `Start-Process -Wait` or pipe it to `Out-Null`.

If you elevate with a different administrator account, per-user tweaks go to that account, and it has its own Time Machine history. `--run` and `--check-drift` fail while the app is open, because only one process can hold the snapshot database.

An ISO build installs `oscdimg` through WinGet when it is missing and removes it afterwards, so that build needs a network connection.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.0.0-x64-setup.exe`, `WindowCraft-1.0.0-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.0.0-x64.msi`, `WindowCraft-1.0.0-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.0.0-x64-portable.exe`, `WindowCraft-1.0.0-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates |

An upgrade from 0.1.0 with `setup.exe` kept the Time Machine history byte for byte in one test on the development machine. An upgrade with the MSI alone has not been tested yet.
