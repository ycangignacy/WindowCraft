# WindowCraft 1.0.2-rc1

This is a pre-release of 1.0.2. It changes how the interface scrolls and animates and nothing that touches the system. If it holds up, this release becomes the stable 1.0.2 as it is, with the same files.

## Scrolling and animation

A mouse wheel notch used to move a list by 60 pixels in one frame. The distance now arrives over about 150 ms, eased out, and a notch in the other direction cancels what is left. The distance per notch follows the "lines to scroll" setting in Windows, at 20 pixels a line, so the default is still 60. Touchpads that report pixels scroll as before. With Windows animations turned off, the wheel jumps the old way.

Animations now wait for the display. The software renderer presents through GDI, which never waits for the screen, so an animation used to draw as fast as one core allowed and its frames landed at uneven points between refreshes. Each animation frame now starts just after a vertical blank, timed from the refresh period that DWM reports, so every frame advances by the same step and an idle window still uses no CPU.

Springs stop once they move less than a quarter of a pixel. Before, they ran until the error was under 0.0005 units, which for the inspector drawer meant 63 frames, the last 33 of them invisible and each a full repaint. The drawer now takes 34 frames, and a page change 26 instead of 43.

A new page or tab rises 8 pixels into place over 220 ms. The transition runs inside its widget and only asks for redraws, so it never rebuilds the view. The WinGet and Chocolatey switch on Applications now slides like the other segmented controls.

## Speed

Layout, text shaping and glyph rendering in Iced were built for size (`opt-level = "z"`) and run on every animated frame. They are now built for speed, like the tiny-skia rasterizer already was. In a headless benchmark, building every crate for speed painted frames 10 to 15 percent faster; this release does that only for the crates on the frame path, and the x64 executable grew from 11.11 MB to 11.59 MB.

The fades at the top and bottom of every list are drawn as solid bands of falling opacity instead of a gradient, which tiny-skia paints through its slower high-precision path.

The benchmark that measured all of this is `crates/windowcraft-app/tests/frame_bench.rs`. It is ignored by default; run it with `cargo test --release -p windowcraft-app --test frame_bench -- --ignored --nocapture`. It showed that painting pixels, not building the view, takes over 90 percent of a frame, and most of that is text. Pages with long registry paths, such as Tweaks, cost the most.

## Installers

The MSI accepts a build of the same version over the installed one instead of installing beside it. `setup.exe` does not: its WiX 3 bootstrapper keeps the entry of an older bundle with the same version, so a second build of 1.0.2 over the first left two entries in Installed apps. That is why this release turns stable without a rebuild. The release workflow accepts tags with a suffix such as `v1.0.2-rc1` and publishes them as pre-releases; before, the version check rejected them.

## Tested

In VirtualBox on Windows 11 Home 26H2, build 26300, Polish, 2 virtual CPUs and no 3D acceleration, from the clean snapshot used for 1.0.1:

| Check | Result |
| :--- | :--- |
| `verify-install.ps1` on the MSI, the MSI with `WCLANGUAGE=pl` and `setup.exe` | 52 of 52 passed |
| MSI upgrade from 1.0.1 with a recorded session | Exit 0, history byte for byte the same, one uninstall entry, 1.0.2 reads the 1.0.1 history |
| Uninstall while the app is open | Exit 0, data and program folder removed; the window kept running (KB-9, unchanged) |
| Portable `--purge-data` | Data folder removed |
| Every page opened from `--open` | The window opened and kept responding |
| Mouse wheel through `SendInput`, 20 notches | Applications, Tweaks and System scrolled; only these three long pages were checked for movement |
| CPU with the window idle, before and after scrolling | 0% on every page |
| CPU while scrolling 10 notches a second | 93% of one core on Applications, 65% on System, 47% on Tweaks |
| Same version over itself | The MSI replaced the first install; `setup.exe` left two entries |

The VM has no GPU, so every frame is painted on a virtual CPU. In it, the frame benchmark took 21 ms for a frame of Applications at 100% scaling and 38 ms at 150%; about 10 and 20 ms of that is a copy into a screenshot buffer that the real window does not make. Faster hardware paints more frames in the same 150 ms of easing, and a slow one paints fewer, but the list lands in the same place in the same time.

## Known bugs and limitations

Nothing changed here since 1.0.1; see the [1.0.1 release notes](https://github.com/ycangignacy/WindowCraft/releases/tag/v1.0.1). SEC-3 and KB-9 are still open, and the builds are still not code-signed.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.0.2-x64-setup.exe`, `WindowCraft-1.0.2-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.0.2-x64.msi`, `WindowCraft-1.0.2-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.0.2-x64-portable.exe`, `WindowCraft-1.0.2-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates |
