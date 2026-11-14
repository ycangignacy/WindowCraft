<!-- Made by Nordan -->
# WindowCraft 1.3.0

This release adds an introduction on the first start and a short tip on each page. The catalogs, the installers and the extensions work as in 1.2.1.

## Introduction

On the first start without command line options, four questions open over the window. Each has two to four answers and changes nothing on its own.

| Question | Answers | What the answer changes |
| :--- | :--- | :--- |
| What do you want to do with WindowCraft? | Fresh install, slimmed down / Privacy / Games and performance / General tidy-up | A fresh install also marks the 14 default AppX packages for removal. A tidy-up opens the Cleaner tab instead of Tweaks & Privacy. |
| Do you want to keep your files? | Yes / No, the system is new | What the cleaner ticks after its first scan: temporary files, logs and minidumps, or those plus GPU shader and developer caches |
| Will you build your own Windows ISO? | Yes / No | Adds ISO Studio to the plan |
| How much should change? | Carefully / Standard / Everything | The Minimal (6), Standard (13) or Advanced (21) preset; Everything also marks the 14 AppX packages |

The last screen lists the plan. Start puts it into the queues and opens the first page; nothing runs until you press Run Tweaks, Remove or Clean Selected. Skip, on every screen, and Escape close the questions for good. The keys 1 to 4 pick an answer and Enter starts.

After that, each page shows a tip once. Next opens the next page with an unread tip, in sidebar order, and the last tip has Done. Don't show again turns them all off, and a click outside a tip closes it without saving. Settings can show the introduction and the tips again.

A start with `--open`, `--inspect`, `--config` or `--preset` shows neither the questions nor the tips.

## Tested

The unit and integration tests pass (538, clippy clean). New tests cover the saved state and what happens to a newer or broken one, Skip on each of the five screens and after a restart, Escape, the number keys and Enter, what Start ticks, each tip once, Next through every page, a click outside a tip, the Settings buttons, a settings file that cannot be written, and every new text in every language. A layout test puts each question screen and tip into windows of 878x425, 980x600 and 1280x800 in all eight languages and checks that Skip, Back, Start, Next and Don't show again lie fully inside.

The x64 packages, built locally with `packaging/build.ps1`, ran in VirtualBox on Windows 11 Home 26H2, build 26300, Polish, at 1024x768:

| Test | Result |
| :--- | :--- |
| `verify-install.ps1 -Version 1.3.0` (MSI, MSI with `WCLANGUAGE=pl`, `setup.exe`) | No failed checks, nothing left after uninstall |
| `setup.exe` of 1.3.0 over 1.2.1 with a recorded session and German dark settings | One visible entry, the binary reports 1.3.0, history byte for byte the same, settings kept, uninstall leaves nothing |
| `WCLANGUAGE` unset and set to each of the eight languages | The value lands in HKLM, the app opens in that language, uninstall removes the key |
| Every page in every language, plus dark and light | 168 checks pass; no question or tip covers a page opened with `--open` |
| First start in each of the eight languages: answers by key, Start, eight real mouse clicks on Next and Done, then a second start | 153 checks pass; each click saved one more tip in sidebar order, and the second start showed nothing and changed nothing |
| Escape on the first screen in each language, then a second start | Saved as skipped; the questions did not show again |
| A click on a sidebar row while a tip is open | The tip closed without saving and the page opened |

Not tested: the answers picked with the mouse in the VM (they were picked with the number keys; unit tests cover the mouse path), the Settings buttons in the VM, ARM64 hardware and display scaling above 100 %. The texts in German, French, Spanish, Italian, Portuguese and Ukrainian were not read by native speakers.

## Known bugs

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash, because their vendors publish them at fixed "latest" URLs. They must pass Authenticode. | None needed. |

## Known limitations

- An upgrade from 1.2.x shows the questions once, because the older settings file has no record of them.
- If `settings.json` cannot be written, the questions come back on the next start.
- A tip covers the lower right corner of the page until it is closed.
- The builds are not code-signed, so SmartScreen warns on the first run.
- The other limitations of [1.2.0](https://github.com/windowcraftdev/WindowCraft/releases/tag/v1.2.0) still apply.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.3.0-x64-setup.exe`, `WindowCraft-1.3.0-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.3.0-x64.msi`, `WindowCraft-1.3.0-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.3.0-x64-portable.exe`, `WindowCraft-1.3.0-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates and of LAME |
