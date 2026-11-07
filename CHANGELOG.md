# Changelog

## 1.1.0 - 2026-11-07

A Settings page, an optional scheduled check for reverted tweaks, catalog text written for WindowCraft, and installer fixes.

- Settings opens from the new gear at the bottom of the sidebar. It holds the language (moved there from the sidebar), the theme and the automatic check. The theme choice is kept between runs; before, every start followed Windows again.
- The automatic check compares applied tweaks with the system daily, every 7 days or every 30 days, and optionally after the Windows build changes. It is off by default. When it is on, WindowCraft registers one scheduled task for the user, `WindowCraft drift check (<user name>)`, which runs without elevation at sign-in and at noon and exits at once unless a check is due. When a tweak was reverted it shows a Windows notification that opens Time Machine, or a message box when Windows notifications are turned off. Check now runs the same comparison straight away. `--open settings` opens the page.
- Every name and description in the catalogs is new: 239 applications, 68 tweaks, 33 System tools and 34 AppX packages. Names say what happens, for example "Turn off activity history" instead of "Activity History - Disable", and application names lost their bracketed tags, so "VLC (Video Player)" is "VLC media player". The application and AppX categories have new names.
- The presets are chosen anew. Minimal has 6 tweaks, Standard 13 and Advanced 21; each contains the one below and starts with a restore point. The default AppX selection has 14 packages and no longer removes Calculator, Clock, Sound Recorder or Sticky Notes, and ISO Studio removes the same 14 plus Teams.
- WinUtil's item ids moved out of the catalogs into `winutil-import.json`. `--config` reads WinUtil exports as before.
- The CTT PowerShell profile is gone. In its place the System page installs a profile that is part of WindowCraft and downloads nothing: history search, a prompt with the folder, git branch and an admin marker, and short commands such as `ll`, `which`, `touch`, `mkcd`, `df`, `uptime`, `flushdns` and `admin`. Installing it a second time no longer replaces the backup of your own profile with the first WindowCraft one, and removing it leaves alone a profile you wrote after installing it.
- In grouped lists the hover and press highlight of the first and last row now follows the rounded corners of the group instead of drawing square corners over them. The light line along the top edge of each group is gone: it was drawn straight across and stuck out past the corners. The hover highlight fades in and out over 110 ms, or appears at once with Windows animations off.
- The Start menu shortcut is no longer advertised. An advertised shortcut belongs to the MSI, and `setup.exe` hides the MSI from Installed apps, so Start had no entry to uninstall. The `setup.exe` entry also gets an `InstallLocation` now.
- The first page of `setup.exe` has a checkbox, checked by default, that starts WindowCraft when setup finishes. The app starts as the signed-in user without elevation. With `/quiet` or `/passive` it does not start.
- The uninstaller and `--purge-data` remove the drift check tasks.
- "Turn off Windows telemetry" writes `AllowTelemetry` as the Group Policy value under `HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection` (KB-15). Up to 1.0.3 it wrote the copy under `CurrentVersion\Policies\DataCollection`, which the DiagTrack service keeps: after a rollback deleted it, DiagTrack wrote it back within seconds, so diagnostic data stayed at its lowest level. A rollback of a session recorded with 1.0.3 or earlier still meets that copy.
- Rolling back "Turn off hibernation" failed on hardware without hibernation support, where `powercfg /hibernate on` returns an error (KB-16). Turning hibernation back on is skipped when the firmware has no S4 state.
- `setup.exe` picks its language from the Windows display language. It went by the regional format before, so a Polish Windows with US formats got English pages.

## 1.0.3 - 2026-11-07

Bug fixes. Nothing that changes the system is different from 1.0.2.

- The ten Control Panel buttons on the System page that open a `.cpl` applet (Mouse, Network Connections, Power, Programs and Features, Region, Security and Maintenance, Sound, System Properties, Time and Date, Windows Defender Firewall) failed with "ShellExecuteW failed for main.cpl" and error 0x0000001F. Windows has no `open` verb for `.cpl` files, so the request failed, and code 31 was then shown with the unrelated system message for ERROR_GEN_FAILURE. Applets now open through `control.exe`, every other target opens with its default verb, and an error shows the real Windows message. Declining the UAC prompt for Windows Restore no longer shows an error.
- Uninstalling while WindowCraft is open closes it (KB-9). Before it removes files, the uninstaller ends every WindowCraft process started from the program folder and waits for it to exit. An upgrade from 1.0.3 to a later version does the same when it removes 1.0.3. A job that is still running stops as if the process had crashed.
- `windowcraft.exe --close-running` is the command the uninstaller uses for this.
- If 1.0.2-rc1 and 1.0.2 were both installed with `setup.exe`, Installed apps listed WindowCraft twice (KB-10). `setup.exe` of 1.0.3 replaces both entries with one (checked in the VM).
- The configuration drift banner and the drift details on the Time Machine page were in English in every language. They are translated now.

## 1.0.2 - 2026-11-07

Smooth scrolling and a GPU renderer. Nothing that changes the system is different from 1.0.1.

- The interface draws on the GPU through wgpu (Direct3D 12 only). Without a Direct3D 12 GPU it draws on the CPU through tiny-skia as before. `ICED_BACKEND` overrides the choice. The GPU path uses more memory (113 MB working set against 32 MB on one PC) and shows no Mica.
- A wheel notch glides over about 150 ms instead of jumping 60 pixels in one frame. A critically damped spring carries the list, so a fast spin of the wheel moves at an even speed. The distance per notch follows the Windows "lines to scroll" setting. With Windows animations off, the wheel jumps as before.
- Animation frames follow the display refresh: the GPU waits for it, and on the CPU path a frame clock wakes just after each vertical blank that DWM reports.
- Springs stop once they move less than a quarter of a pixel. Opening the inspector takes 34 frames instead of 63, a page change 26 instead of 43.
- A new page or tab rises 8 pixels into place. The WinGet and Chocolatey switch slides like the other segmented controls.
- Iced's layout, text shaping and glyph rendering are built at `opt-level = 3`. List edge fades are solid bands instead of a gradient.
- `WINDOWCRAFT_SCROLL_TRACE=<file>` logs every eased scroll frame, and `crates/windowcraft-app/tests/frame_bench.rs` measures the parts of a frame without a window.
- The MSI installs over a build of the same version instead of beside it. The release workflow accepts pre-release tags such as `v1.0.2-rc1`.

## 1.0.2-rc1 - 2026-11-06 (Pre-release)

The same scrolling work on the CPU renderer alone. At 2560 by 1440 a scrolled frame took about 36 ms, so the eased scrolling stuttered; 1.0.2 moved drawing to the GPU.

## 1.0.1 - 2026-11-04

First stable release. Every system-changing feature ran on Windows 11 Home 26H2 in VirtualBox before it, and these fixes came out of that run:

- "Services - Set to Manual", and with it the Minimal and Standard presets, failed on Windows Home, which has no CscService (Offline Files). That service is optional now.
- Memory read as 0 in virtual machines, where GetPhysicallyInstalledSystemMemory fails. The report showed `"ram_gb": 0`, and the svchost split tweak would have written a threshold of 0. GlobalMemoryStatusEx is the fallback.
- "Dark mode by default" in ISO Studio did nothing on Windows 11 26H2, which ignores the theme values in the answer file. `SetupComplete.cmd` now writes them into the Default user hive, next to the privacy values that already arrive that way.
- ISOs from ISO Studio carry the volume label WIN11, like USB drives, instead of oscdimg's default DVD_ROM.
- `crates/windowcraft-app/tests/live_vm.rs` holds the live tests used for the VM run. They refuse to run outside an elevated VirtualBox guest with `WINDOWCRAFT_LIVE_VM=1`.

## 1.0.0 - 2026-11-04 (Pre-release)

- Interface in English, Polish, German, French, Spanish, Italian, Brazilian Portuguese and Ukrainian, chosen from the Windows display language or from the list in the sidebar.
- `setup.exe` in the same eight languages. `WCLANGUAGE` on the MSI or `setup.exe` command line presets the app language for every user.
- Rolling back a Windows feature session turns off the features WindowCraft turned on (KB-8).
- The CTT PowerShell profile and the Adobe block list download from fixed commits and are checked by SHA-256 (SEC-1, SEC-2).
- Only real damage moves the Time Machine history aside (KB-6).
- The cleaner keeps ISO Studio work files while an ISO is open or an image is mounted (KB-5).
- A catalog test reports unknown fields on operations without parameters (KB-7).
- `NOTICE.md` credits WiX, whose bootstrapper is part of `setup.exe`.

## 0.1.0 - 2026-11-02 (Pre-release)

First release, for x64 and ARM64.

- Applications: 239 packages through WinGet or Chocolatey.
- Tweaks & Privacy: 68 tweaks and 34 AppX packages, with presets.
- System Config & Fixes: 33 tools, including features, repairs and Control Panel applets.
- Windows Updates: three policy modes.
- ISO Studio: answer files, driver injection, ISO and USB output.
- Time Machine: snapshot before every change, rollback and drift detection.
- System & Gaming Inspector: MSI mode, autoruns, cache cleaner, DNS benchmark.
- Command line, WinUtil config import, and a privacy-safe environment report.
- Uninstall leaves nothing of its own behind, and `--purge-data` does the same for the portable build.
