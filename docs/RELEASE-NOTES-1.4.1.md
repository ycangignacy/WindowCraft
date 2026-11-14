<!-- Made by Nordan -->
# WindowCraft 1.4.1

This release fixes option lists in the seven translated interface languages and seven smaller bugs found in a code review of 1.4.0 and one layout bug in the X-Ray Inspector, and moves the credit to WindowCraft Dev Team. No pages or extensions were added.

## Option lists in translated languages

1.4.0 translated the catalog, including the labels of options. The option lists sent the shown label to the engine, which knows an option only by its English name. In every language except English this had three effects:

- Multiplane overlay (MPO) on the Tweaks page: no translated option matched a registry value, so both values were deleted, which is the "Enabled" state, and the run was reported as done.
- Reset Windows Update on the System page: "Agresywnie" and its other translations ran the standard reset.
- DNS server: "Default" and "Fastest" in translation failed with a "not found" error. Provider names such as Google or Cloudflare are the same in every language and worked.

A profile with English option names was also refused when the window ran in another language, and the lists showed no current choice.

`ChoiceOption` keeps the English name in a `value` field when the label is translated, and the registry maps, profile checks and the engine use `value()`. The pick lists show the label and send the value. A test loads the catalog in every language and checks that each option of every tweak and feature keeps its English value and that a profile with English option names loads.

## Other fixes

- Installing AppX packages: three packages in the catalog have no Microsoft Store id (Xbox Speech To Text Overlay, Xbox TCUI and Solitaire Collection). Install skipped them but counted them as installed and unticked them. They now stay ticked and the notice names them.
- Importing a profile from the dialog while a job runs is refused with the notice that another operation is running. Before, the end of the job removed its own ids from the newly imported toggles and options, so those choices were lost without running.
- A Time Machine session in the Failed state has the Roll back button. A feature set such as WSL can fail after some of its Windows components were enabled, and those stayed on with no way to undo them from the window.
- When a job ends, an open Time Machine session and the X-Ray Inspector load their values again. Before, they showed the values from before the job, and an undo button for a change that had just been undone.
- ISO Studio splits an `install.wim` larger than 3.8 GB into `install.swm`, `install2.swm` and so on before it writes a USB stick. The parts stayed in `work\media\sources` afterwards, so a later ISO export packed them next to `install.wim`. They are deleted after the copy, whatever its result, and before a new split.
- Startup folder items on the Autoruns tab of the System & Gaming Inspector always showed as on, because the scan did not read their `StartupApproved\StartupFolder` flag. Turning off an item of the common Startup folder under ProgramData wrote that flag to HKCU, where Windows ignores it for the common folder. The scan and the switch now use HKCU for the user's folder and HKLM for the common one, as Task Manager does.
- A session rollback ran the undo steps of a tweak once for each separate run of its records in the session. Each tweak's undo steps now run once, in session order. The present code records a tweak's changes in one piece, so this is a safeguard more than a fix someone could hit.
- The search field takes up to 460 px when the toolbar has room, and the placeholder is shorter in the seven translated languages. In a 1024 px window the full text is visible on every page in every language.
- In the X-Ray Inspector the overlay scroll bar sits on the right edge of the drawer content and covered the last letters of a description line ("input" in the telemetry tweak) and the right border of the cards. The content keeps 12 px clear for it, as the introduction does.

## Credit and contact

The credit reads "Made by WindowCraft Dev Team" in all eight languages. It left the sidebar and now ends every page, in small type on the right under the last group. Settings has a new About group with the support address, support@windowcraft.dev, which opens the mail program, and a button that opens <https://windowcraft.dev>. `LICENSE`, the installers, the exe version data and the Cargo manifests name WindowCraft Dev Team as the copyright holder and manufacturer. The README is rewritten: it is shorter, has new 1.4.1 screenshots drawn headless in the dark theme on the test sandbox, so they show no data of a real computer, and points to windowcraft.dev for the full description. The repository moved with the account to <https://github.com/windowcraft/WindowCraft>; the old address redirects, and every link in the repository, the installers and the site points to the new one.

## Not changed in this release

- Pinned versions for the downloaded tools (E14), with the plan from the 1.4.0 notes.
- The Spanish, Italian and French grammar points of L11, and a review of the catalog translations by native speakers.
- The per-item failure reasons inside the "Failed: N. First error: ..." notice still come from the engine in English.
- Importing a WinUtil file skips packages under `Install` that the catalog does not have, without saying so.

## Tested

`cargo clippy --workspace --all-targets -- -D warnings` has no warnings. `cargo test --workspace` passes 598 tests, 0 fail, and 27 are ignored: the live tests and the two screenshot renders (592 and 26 in 1.4.0). Each fix has a unit or sandbox test, except the reload after a job, which only queues two existing loads, and the rollback safeguard.

The interface was drawn headless with tiny-skia at 1024x720 in all eight languages, on the System and Inspector pages and on Settings, to check the search field, the credit and the About group.

A short pass ran in a VirtualBox VM with Windows 11 Home 26H2, build 26300, in Polish, from a clean snapshot. The upgrade from 1.4.0 with history and settings, `WCLANGUAGE` in all eight languages and the uninstall passed 64 of 64 checks. The fixes on the live system passed 12 of 12: with the window in Polish, a profile with "Fully Disabled" for multiplane overlay set `OverlayTestMode` to 5 and `DisableOverlays` to 1, and "Disabled (Compatibility)" kept 5 and removed `DisableOverlays`; the System page showed "Standardowo" in the reset list; the CLI help, the exe version data and the Installed apps entry name WindowCraft Dev Team. These packages were built before the X-Ray Inspector padding fix, which changes only the layout of the drawer.

Not checked on a live system: every page in every language and the introduction, which the full pass of 1.4.0 covered. The AppX install, the feature rollback, the Startup folder switch, a USB write with a split image and the mail and website buttons have not run on a real system.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.4.1-x64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.4.1-x64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.4.1-x64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates and of LAME |
