# Changelog

## 1.0.2-rc1 - 2026-11-06 (Pre-release)

Scrolling and animation. Nothing that changes the system is different from 1.0.1.

- A wheel notch eases over about 150 ms instead of jumping 60 pixels in one frame. The distance per notch follows the Windows "lines to scroll" setting. With Windows animations off, the wheel jumps as before.
- Animation frames are timed to the display refresh that DWM reports, instead of running as fast as the CPU allows.
- Springs stop once they move less than a quarter of a pixel. Opening the inspector takes 34 frames instead of 63, a page change 26 instead of 43.
- A new page or tab rises 8 pixels into place. The WinGet and Chocolatey switch slides like the other segmented controls.
- Iced's layout, text shaping and glyph rendering are built at `opt-level = 3`. The x64 executable grew by 480 KB.
- List edge fades are solid bands instead of a gradient, which tiny-skia paints more slowly.
- `crates/windowcraft-app/tests/frame_bench.rs` measures the cost of each part of a frame without a window.
- The MSI installs a build of the same version over the installed one, so 1.0.2 replaces this pre-release.
- The release workflow accepts pre-release tags such as `v1.0.2-rc1`.

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
