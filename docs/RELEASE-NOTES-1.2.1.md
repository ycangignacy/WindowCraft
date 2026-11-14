<!-- Made by Nordan -->
# WindowCraft 1.2.1

This release fixes what a full test of 1.2.0 found. Nothing new was added, and the catalogs, the installers and the extensions work as in 1.2.0.

## What was tested in 1.2.0

The official 1.2.0 files from the Releases page ran in VirtualBox on Windows 11 Home 26H2, build 26300, Polish, at 1024x768. The run covered the checksums, `verify-install.ps1` for the MSI and `setup.exe` (57 of 57), an upgrade from 1.1.0 with history and settings kept, `WCLANGUAGE` in all eight languages, every page in every language in light and dark, the extensions with real downloads and real files, clicks through the Polish interface, the portable build and the live system tests. These passed. The findings below are fixed in 1.2.1.

## Fixes

| Finding in 1.2.0 | Fix in 1.2.1 |
| :--- | :--- |
| A list jumped back to the top when a notice appeared or was dismissed, on every page after every job | The drift banner and the notice keep a slot in the widget tree whether they show or not, so the page keeps its scroll position |
| The first window was 1280x800 centred on the whole monitor, so on 1024x768 and 1366x768 its bottom was under the taskbar | It shrinks to the work area when needed and is centred in it |
| Inspector tabs were 100 px wide, and "Benchmark de DNS" (Portuguese) and "Esecuzione automatica" (Italian) wrapped onto two lines | Tabs are as wide as the longest label; the Italian tab is "Avvio automatico" |
| YouTube files were named `<title> [<video id>].<ext>`, so the name carried the link | Files are named `<title>.<ext>`, with " (2)" when the name is taken |
| The installed stripper and converter said that uninstalling deletes downloaded tools | They say that they downloaded nothing |
| "Disable all updates" reported success while Windows had put wuauserv and BITS back to their old start type | The start types are read back; when Windows reverted one, the mode is rolled back and the message names the service |
| A DISM error without text showed only the code, "(0x800F0975)" | The message falls back to the Windows text, then to a pointer to `Logs\DISM\dism.log` |
| Writing `hosts` failed once with 0x80070020 while another process had it open | The replace is tried again for about six seconds |

The scroll problem came from iced: `Column::push` leaves out a child whose width or height is fixed at 0, and the empty banner place holders had a height of 0. A new headless test builds the real view, scrolls a page, shows and dismisses a notice and reads the scroll offset back. It fails on 1.2.0 and passes on 1.2.1.

## Tested

The unit and integration tests pass (511, clippy clean). New tests cover the scroll position around notices, the first window size and position on five screen sizes, the tab width in every language, the YouTube file names, the update mode read-back, the DISM fallback text and a replace while the target is held open.

The x64 packages of 1.2.1, built locally with `packaging/build.ps1`, and the same binaries ran in the VM:

| Test | Result |
| :--- | :--- |
| `verify-install.ps1 -Version 1.2.1` (MSI, MSI with `WCLANGUAGE=pl`, `setup.exe`) | No failed checks, nothing left after uninstall |
| `setup.exe` of 1.2.1 over the published 1.2.0 with a recorded session and German dark settings | One visible entry, the binary reports 1.2.1, history byte for byte the same, settings kept, the drift check reads the 1.2.0 history, uninstall leaves nothing |
| `WCLANGUAGE` unset and set to each of the eight languages, through the MSI and `setup.exe` | The value lands in HKLM, the app opens in that language, uninstall removes the key |
| Every page in every language, maximized, plus English and Ukrainian dark and German light | 168 checks pass; the screenshots were looked through |
| First window on 1024x768 with a 48 px taskbar | Opens at 8,8 to 1016,712 inside the 1024x720 work area; 1.2.0 opened at 156,156 to 1452,995 |
| Extensions page of the installed 1.2.1, scrolled, a notice dismissed and a new one shown by clicking | The page stays where it was both times |
| Inspector in Italian, Portuguese, Ukrainian and English | Every tab label on one line |
| A real YouTube video as MP3 and MP4 | Saved as "Me at the zoo.mp3" and "Me at the zoo.mp4", with no id in the name and no metadata inside |
| Engine unit tests in the VM | 241 pass |
| Update modes (t06) without network | Pass |
| Every reversible tweak applied and rolled back (t14) without network | 53 tweaks clean; the Adobe block list needs the network to fetch its list |

Not tested: the update mode read-back while Windows really reverts a service. With the network on, Windows Update was busy and wuauserv did not stop within 10 seconds, so the mode was rolled back before the read-back, as in 1.2.0. The DISM fallback text was not seen in the VM, because the error appears only while Windows is servicing. ARM64 is untested on ARM hardware, as before.

## Known bugs

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash, because their vendors publish them at fixed "latest" URLs. They must pass Authenticode. | None needed. |

## Known limitations

Names and descriptions of catalog entries stay in English in every language, so the Windows Update page is mostly English. The other limitations of [1.2.0](https://github.com/windowcraftdev/WindowCraft/releases/tag/v1.2.0) still apply.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.2.1-x64-setup.exe`, `WindowCraft-1.2.1-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.2.1-x64.msi`, `WindowCraft-1.2.1-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.2.1-x64-portable.exe`, `WindowCraft-1.2.1-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates and of LAME |
