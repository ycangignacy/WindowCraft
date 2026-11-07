# WindowCraft 1.0.2

This release makes scrolling and animation smooth. It moves drawing to the GPU and eases the mouse wheel. Nothing that changes the system is different from 1.0.1.

## Smooth scrolling

A mouse wheel notch used to move a list by 60 pixels in one frame. The list now glides there in about 150 ms. A critically damped spring carries it: a new notch moves the target and keeps the current speed, so spinning the wheel fast gives one even motion instead of a jolt per notch, and a notch in the other direction turns the list on the next frame. A click stops the motion where it is. The distance per notch follows the "lines to scroll" setting in Windows, at 20 pixels a line, so the default is still 60. With Windows animations turned off, the wheel jumps as before.

Windows reports touchpad scrolling as small wheel notches, so touchpads are eased too.

Inside a scrolling list, Ctrl and the wheel over a drop-down now scroll the list. In 1.0.1 they changed the selected option, and on settings that apply at once that changed the system.

## Drawing on the GPU

WindowCraft used to paint every frame on the CPU through tiny-skia. On a 2560 by 1440 window a scrolled frame took about 36 ms, so eased scrolling ran at roughly 25 frames per second and looked worse than the old jump. The interface now draws through wgpu on Direct3D 12. On the test PC, a GeForce RTX 2060 with a 164 Hz display, 152 frames of a fast wheel spin came 6.07 ms apart on average, with the slowest at 6.85 ms, so every refresh got a frame.

A computer without a Direct3D 12 GPU, for example a virtual machine without 3D acceleration, draws through tiny-skia as before. WindowCraft checks for such a GPU at startup instead of letting wgpu fall back to WARP, the software rasterizer of Direct3D. `ICED_BACKEND=tiny-skia` or `ICED_BACKEND=wgpu` picks the renderer by hand.

Animation frames follow the display refresh. The GPU waits for it on its own; on the CPU path a frame clock wakes just after each vertical blank that DWM reports, instead of drawing as fast as one core allows. Springs stop once they move less than a quarter of a pixel, so opening the inspector takes 34 frames instead of 63 and a page change 26 instead of 43. A new page or tab rises 8 pixels into place, and the WinGet and Chocolatey switch slides like the other segmented controls.

The price is memory and size. With the Applications page open, the GPU path had a 113 MB working set where the CPU path had 32 MB, and the x64 executable grew from 10.6 MB to 14.0 MB. wgpu is built with Direct3D 12 only; with Vulkan and GL as well it used 191 MB.

## For developers

`WINDOWCRAFT_SCROLL_TRACE=<file>` writes one line per eased scroll frame: time, time since the previous frame, pixels moved, pixels left and speed. `crates/windowcraft-app/tests/frame_bench.rs` measures view, layout and painting of each page on the CPU renderer without a window; run it with `cargo test --release -p windowcraft-app --test frame_bench -- --ignored --nocapture`.

The MSI now installs over a build of the same version instead of beside it, and the release workflow publishes tags with a suffix, such as `v1.0.2-rc1`, as pre-releases.

## Tested

On the PC above, the GPU path ran the wheel test and the memory figures. In VirtualBox on Windows 11 Home 26H2, build 26300, Polish, with 2 virtual CPUs and no 3D acceleration, from a clean snapshot:

| Check | Result |
| :--- | :--- |
| Renderer without a GPU | tiny-skia on every page, 32 to 46 MB working set |
| `verify-install.ps1` on the MSI, the MSI with `WCLANGUAGE=pl` and `setup.exe` | 53 of 53 passed |
| MSI upgrade from 1.0.1 with a recorded session | Exit 0, history byte for byte the same, one uninstall entry, 1.0.2 reads the 1.0.1 history |
| Uninstall while the app is open | Exit 0, data and program folder removed; the window kept running (KB-9) |
| Portable `--purge-data` | Data folder removed |
| Every page opened from `--open` | The window opened and kept responding |
| Mouse wheel through `SendInput`, 20 notches | Applications, Tweaks and System scrolled; only these three long pages were checked for movement |
| CPU with the window idle, before and after scrolling | 0% on every page |
| `setup.exe` of 1.0.2 over `setup.exe` of 1.0.2-rc1 | Exit 0 and one MSI product, but two entries in Installed apps (KB-10); removing the older one left 1.0.2 installed |

## Known bugs

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| KB-10 | If 1.0.2-rc1 was installed with `setup.exe`, installing 1.0.2 with `setup.exe` leaves two "WindowCraft 1.0.2" entries in Installed apps. Both have the same version, and the WiX 3 bootstrapper keeps the older one. | Uninstall 1.0.2-rc1 before you install 1.0.2; this also deletes its Time Machine history. If both entries are already there, WindowCraft 1.0.2 works and the extra entry can stay. Removing the entry installed first left 1.0.2 in place in the test; removing the other one was not tested. |
| KB-9 | Uninstalling while WindowCraft is open removes the files and data, and the open window keeps running until it is closed. | Close WindowCraft before you uninstall. |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash, because their vendors publish them at fixed "latest" URLs. They must pass Authenticode. | None needed. |

## Known limitations

Mica shows behind the sidebar only on the CPU path. wgpu presents through a Direct3D 12 swap chain without an alpha channel that DWM could blend, so on the GPU the sidebar is solid.

The builds are not code-signed yet, so SmartScreen warns on first run.

ARM64: the files build and have the right platform, but no ARM64 binary has run on ARM hardware, and the GPU path is untested there.

Everything else in the limitations of [1.0.1](https://github.com/ycangignacy/WindowCraft/releases/tag/v1.0.1) still applies: Time Machine cannot restore removed packages, uninstalled apps or BitLocker decryption; an uninstall deletes Time Machine history for every user; the portable build has no uninstaller; and an ISO build installs `oscdimg` through WinGet when it is missing.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.0.2-x64-setup.exe`, `WindowCraft-1.0.2-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.0.2-x64.msi`, `WindowCraft-1.0.2-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.0.2-x64-portable.exe`, `WindowCraft-1.0.2-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates |
