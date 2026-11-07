# Changelog

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
