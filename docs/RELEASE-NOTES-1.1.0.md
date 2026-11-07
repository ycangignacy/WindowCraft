# WindowCraft 1.1.0

This release adds a Settings page with an optional scheduled check for reverted tweaks, replaces the text of every catalog entry, and fixes the installer and the Start menu entry. The registry values, services and operations of the tweaks are the same as in 1.0.3.

## Settings

The gear at the bottom of the sidebar opens Settings. The language list moved there from the sidebar, next to the theme (follow Windows, dark or light) and the new check for reverted tweaks. The theme choice is kept between runs; in 1.0.3 every start followed Windows again. `--open settings` opens the page directly.

## Checking for reverted tweaks

Windows updates and other programs sometimes set a value back after WindowCraft changed it. WindowCraft already showed this as configuration drift when it opened. Settings can now also check on a schedule: daily, every 7 days or every 30 days, and after the Windows build number changes, which happens with every cumulative update. Both options are off by default, and Check now runs the comparison straight away.

When one of them is on, WindowCraft registers a scheduled task named `WindowCraft drift check (<user name>)`. It runs as that user, without elevation, three minutes after sign-in and at noon, and a missed noon run starts once the computer is on again. Each run reads `settings.json`, compares only when the interval has passed or the build changed, and exits at once otherwise. It reads the system and changes nothing. While the WindowCraft window is open it does nothing, because the window shows the same banner.

If a tweak was reverted, a Windows notification says how many, and clicking it opens Time Machine, where Re-apply puts the values back. When Windows notifications are turned off, a message box asks the same question. Turning both options off deletes the task, and the uninstaller deletes it for every user.

## Catalog text

Every name and description in the catalogs is new: 239 applications, 68 tweaks, 33 System tools and 34 AppX packages. Names now say what happens, for example "Turn off activity history" for what was "Activity History - Disable", and application names lost their bracketed tags. Application and AppX categories have new names.

Profiles keep working, because the ids did not change. The ids that WinUtil uses in its exported configs now live in a separate table, `winutil-import.json`, and `--config` imports WinUtil files as before.

The presets are chosen anew:

| Preset | Tweaks |
| :--- | :--- |
| Minimal | 6: restore point, telemetry, activity history, location, promoted app installs, device metadata downloads |
| Standard | 13: Minimal plus Delivery Optimization, Store suggestions in search, folder type discovery, WPBT, services to manual, End task on the taskbar, temporary files |
| Advanced | 21: Standard plus Widgets, Windows AI components, Edge promotions, background apps, the classic right-click menu, Home and Gallery, Razer and Logitech auto-installs |

The default AppX selection has 14 packages instead of 18. Calculator, Clock, Sound Recorder and Sticky Notes are no longer in it. ISO Studio removes the same 14 packages plus Teams.

## PowerShell profile

The System page no longer downloads the CTT PowerShell profile. It installs a profile that is part of WindowCraft: history search with the arrow keys, list suggestions from history, a prompt with the folder, the git branch (read from `.git\HEAD`, so the prompt never starts git) and an admin marker, and the commands `which`, `ll`, `touch`, `mkcd`, `grep`, `df`, `uptime`, `flushdns`, `admin` and `reload`. Lines that look like they contain a password or token stay out of the history file. The profile downloads nothing and updates nothing.

Two bugs in the old install and remove steps are fixed. Installing a second time replaced the backup of your own profile with the first WindowCraft profile, so removing it later could not bring yours back. And removing it deleted whatever profile was there, even one you wrote after installing.

## Installer and Start menu

The first page of `setup.exe` has a checkbox, checked by default, that starts WindowCraft when setup finishes. The app starts as the signed-in user without elevation. With `/quiet` or `/passive` it never starts, and `LaunchWindowCraft=0` on the command line clears the checkbox.

`setup.exe` now picks its language from the Windows display language. Before, it went by the regional format, so a Polish Windows with US date and number formats got the English pages.

The Start menu shortcut is a plain shortcut now. In 1.0.3 it was an advertised shortcut, which Windows ties to the MSI, and `setup.exe` hides that MSI from Installed apps. The `setup.exe` entry also carries an `InstallLocation`, which it had none of before.

## Rollback fixes

"Turn off Windows telemetry" now writes `AllowTelemetry` as the Group Policy value, `HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection`, which is the documented "Allow Diagnostic Data" policy (KB-15). Up to 1.0.3 it wrote the copy under `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection`. That copy belongs to the DiagTrack service: in the VM, DiagTrack wrote back whatever value was last stored there within 10 seconds of a delete, and again after a restart. So a rollback deleted the value and Windows put 0 back, and diagnostic data stayed at its lowest level. The policy value is removed for good, also across a DiagTrack restart. A session recorded with 1.0.3 or earlier still points at the old copy, so rolling it back leaves that value; turning "Send optional diagnostic data" back on in Settings restores the level.

Rolling back "Turn off hibernation" failed on hardware without hibernation support, because `powercfg /hibernate on` exits with an error there (KB-16). WindowCraft now asks the firmware first and skips turning hibernation on when there is no S4 state.

Both came out of a new live test that applies and rolls back every tweak that can be undone, one at a time, and compares every registry value and service it touches 10 seconds later.

## Interface

In grouped lists, the hover and press highlight of the first and last row followed a square outline and covered the rounded corners of the group. It now follows the corners. The one-pixel highlight line along the top of each group is gone too; it was drawn straight across and showed as a bar past the rounded corners. The hover highlight fades in and out over 110 ms, or appears at once when Windows animations are off.

## Tested

The unit and integration tests pass (449, clippy clean at `-D warnings`). New tests cover the settings file, the check schedule, the scheduled task (registered and deleted for real on the development PC without elevation), the corner radii, the hover fade, the profile backup rules and the new presets.

In VirtualBox on Windows 11 Home 26H2, build 26300, Polish, from a clean snapshot:

| Check | Result |
| :--- | :--- |
| `setup.exe /quiet` of 1.0.3, then of 1.1.0 | Exit 0; one visible entry "WindowCraft 1.1.0" with `InstallLocation`; the shortcut points at `windowcraft.exe` (it pointed into `C:\Windows\Installer` with 1.0.3); history byte for byte the same; the quiet install did not start the app |
| `windowcraft.exe --scheduled-check` with a reverted tweak | Notifications did not show in the VM, so the message box appeared in Polish; Yes opened Time Machine with the drift banner; the run was recorded in `settings.json` and the language key kept; nothing on the system changed |
| A second run right after | Exits within a second |
| Start, right-click on WindowCraft | "Odinstaluj" is in the menu and opens Installed apps, where WindowCraft 1.1.0 is listed once |
| Uninstall through the `setup.exe` entry | Exit 0; a drift check task created beforehand is gone, with the program folder, data folder, shortcut and every uninstall entry |
| `setup.exe` with its interface over 1.0.3 | Pages in Polish, the launch checkbox checked; after the install WindowCraft opened without elevation ("Standard user") and the last page read "Instalacja zakończona pomyślnie" |

A second VirtualBox run from the clean snapshot covered the rest of the app:

| Phase | Result |
| :--- | :--- |
| `verify-install.ps1` on the MSI, the MSI with `WCLANGUAGE=pl` and `setup.exe` | 57 of 57 passed |
| MSI upgrade from 1.0.3 with a recorded session, uninstall while the app runs, portable `--purge-data` | All passed; history byte for byte the same |
| Command line: protected services, Disable ALL Updates, `--run`, `--check-drift`, `--fix`, `--export-report` | All passed |
| Live tests t01 to t07: tweaks, drift, services, AppX for all users, optional features, update modes, BitLocker | All passed |
| t10: the PowerShell profile installed twice over a profile of the user's own, loaded, removed | Passed; the user's profile came back |
| t11: the scheduled check as Settings creates it, then started by Task Scheduler | Passed; the task runs without elevation as the user, with 2 triggers, and its run was recorded |
| t12: the Standard preset applied and rolled back | Passed, with a restore point created |
| t13: every value of the telemetry tweak after a rollback, and again 20 s later | Passed; the Settings store copy is not touched |
| t14: each of the 54 tweaks that can be undone applied and rolled back on its own | Found KB-15 and KB-16, then passed |

ISO Studio and USB writing (t08, t09) did not run again; their code did not change since they passed for 1.0.1.

On the development PC the Settings page, the gear button and the rounded hover on the first and last row of a group were checked on screen with the GPU renderer and with `ICED_BACKEND=tiny-skia`. The notification there was dropped at once, because Windows notifications are turned off on that PC; that is the case the message box covers.

Not reproduced: in the VM, Uninstall in the Start menu opened Installed apps with 1.0.3 as well, so the failure seen with 1.0.3 did not happen there. The cause it came from, the advertised shortcut tied to a hidden MSI, is gone, but the fix is checked only on the VM. Not tested: a Windows notification actually on screen, and a click on it, because neither machine showed one.

## Known bugs

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash, because their vendors publish them at fixed "latest" URLs. They must pass Authenticode. | None needed. |

## Known limitations

The scheduled check uses the `windowcraft.exe` it was turned on from. For the portable build, move the file only after turning the check off, or turn it on again from the new place.

Mica shows behind the sidebar only on the CPU path. The builds are not code-signed yet, so SmartScreen warns on first run. No ARM64 binary has run on ARM hardware.

The other limitations of [1.0.1](https://github.com/ycangignacy/WindowCraft/releases/tag/v1.0.1) still apply: Time Machine cannot restore removed packages, uninstalled apps or BitLocker decryption; an uninstall deletes Time Machine history for every user; the portable build has no uninstaller; and an ISO build installs `oscdimg` through WinGet when it is missing.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.1.0-x64-setup.exe`, `WindowCraft-1.1.0-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.1.0-x64.msi`, `WindowCraft-1.1.0-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.1.0-x64-portable.exe`, `WindowCraft-1.1.0-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates |
