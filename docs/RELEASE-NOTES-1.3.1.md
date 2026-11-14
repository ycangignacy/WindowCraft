<!-- Made by Nordan -->
# WindowCraft 1.3.1

This release fixes what a full audit of 1.3.0 found across the engine, ISO Studio, the interface, profile handling and the eight interface languages. No new pages or extensions were added.

## Engine and ISO Studio fixes

On a clean Windows 11 install, eight toggles whose Windows default state has no registry value (centered taskbar icons, Task View, Game Mode, Bing search, Start menu recommendations, the Settings home page, sign-in blur and Modern Standby network connectivity) read as off because the value was absent. Detection and drift checks now read the catalog's `default_on` flag when a value is missing, so those toggles show as on and are not reported as reverted. The three IPv6 tweaks (`ipv46`, `teredo` and `disable-ipv6`) now enforce their `exclusive` group in the interface and during profile validation.

Rolling back a tweak skips a catalog registry tree deletion when the rollback has just restored a value inside that tree, so rolling back `disable-explorer-auto-discovery` keeps an existing `Bags\AllFolders\Shell\FolderType` value. Registry tree deletions also run through the same registry scope as every other write, which covers `right-click-menu` and keeps sandboxed tests inside the sandbox.

In "Reset Windows Update components", every service that was stopped before an error or cancellation is started again before the job returns. In WMI method calls used by BitLocker decryption and IPv6 binding changes, `invoke_all` reads the `ReturnValue` property from the output object and returns a Win32 error when it is non-zero. The leftover-folder cleaner checks `symlink_metadata` so directory junctions and symbolic links in `%APPDATA%` and `%LOCALAPPDATA%` are skipped during scanning and never entered if passed to `remove_orphan`.

In ISO Studio, `copy_media` clears read-only attributes without following symlinks and deletes the existing `work\media` tree before copying a new ISO, so an older `sources\install.wim` cannot stay next to a newer `install.esd`. When driver injection is on and the staged media has `sources\install.esd` without `install.wim`, ISO Studio converts the selected edition to a one-image `install.wim` before injecting the install drivers, and matches `edition_id` on a later build so it finds the edition at index 1.

In Extensions, the MP3 metadata stripper validates that an APEv2 footer has a size of at least 32 bytes and fits within the remaining slice before subtracting it. A zero-sized or oversized tag looped forever before. The MP4 and Matroska remuxers use checked arithmetic for 64-bit box sizes and block header lengths and return an error on damaged input instead of panicking, which lets the converter fall back to `ffmpeg` when installed.

## Interface, CLI and profile fixes

When a tweak batch is cancelled, the notice says "Cancelled after N of M." and only the tweaks that finished before cancellation are unticked. Across all queues, finishing a job removes only the items that were sent and completed, so items the user ticked while the job was running stay selected. A failed "Check now" on the Time Machine page shows a warning notice and leaves the current drift list and timestamp in place.

Importing a profile in the window stores `toggles`, `segmented`, `update_mode` and `services`. The toggles and segmented values count toward the Tweaks queue and run with "Run Tweaks", and exporting the selection writes all four fields back out. When an imported file contains `update_mode` or `services`, the notice explains that they are kept in exports and applied by `windowcraft.exe --config <file> --run`. JSON parsing strips a leading UTF-8 BOM for `settings.json`, profiles and WinUtil files. Every write to `settings.json`, including `save_settings_language`, uses an atomic temporary-file write and backs up an unreadable non-empty file to `settings.json.bad`. A WindowCraft profile without `"version"` reports the missing version field instead of a WinUtil error.

When a second instance starts while the window is open, it sends its working directory and arguments to the running window over `WM_COPYDATA` (`crates/windowcraft-app/src/remote.rs`) and restores the window only if it was minimized. The running window ignores `--run`, `--check-drift`, `--export-report`, `--purge-data` and `--close-running` from a hand-over, and `--config` only loads the selection to review. `--inspect` with an unknown id shows a notice instead of opening an empty drawer, and `--export-report` given a folder path writes `windowcraft-report.json` inside that folder.

Changing the language while an ISO is opening no longer drops the edition list when the job ends, because the job is tagged by kind instead of its translated label. Opening items in the X-Ray Inspector tags the loaded rows with the requested item and ignores late replies from an earlier click. Changes to the automatic drift check update the scheduled task one at a time, and the last choice wins. "Show the introduction again" and "Show the tips again" re-enable guided mode when the window was started with a deep link. Job failures carry a `needs_elevation` flag set before the error is formatted, so "Restart as Administrator" appears on the notice banner in every language. Action buttons and category chips on Applications and Tweaks wrap on narrow windows with the inspector open, and the Extensions notes state that the unpacked disk size of ffmpeg and the YouTube downloader tools is larger than the download size.

## Translations and layout

The Cleaner scan rows format the file count and unit into `Msg::FilesCountOne/Few/Many` instead of printing `{}`. ISO Studio step messages, `EngineError` messages, cleaner target and group names, the three display toggles and the kernel timer labels on the Display tab of the System & Gaming Inspector are translated in all eight languages. Polish plural selection gives 1 the singular, numbers ending in 2 to 4 (except 12 to 14) the paucal form, and 21, 31 and 101 the plural form, while Ukrainian keeps its rule for numbers ending in 1. The CLI uses singular and plural forms for item counts. `Msg::NoticeConverted`, `Msg::DriftResetFormat` and `Msg::NotifyDriftBody` (L6) already used colon formats and needed no change.

The Ukrainian ISO configuration heading no longer contains a Latin `f`, the Polish Settings button says "Pokaż wprowadzenie ponownie" and the standard depth option says "Standardowo", the French inspector label says "Paquet", and the search placeholder in the seven non-English languages lists tweaks and registry keys like the English text. Tweaks tabs size themselves to the longest translated label, and the X-Ray Inspector `compare` label column is 110 px wide instead of 72 px. `setup.exe` translates its Windows 11 check message through `#(loc.ConditionWindows11)` in all eight installer language files; the MSI's own condition message stays in English because `setup.exe` runs the MSI without its internal UI.

## Not changed in this release

- Translation of the catalog: tweak, application and feature names and descriptions stay English in every language (L2).
- Re-checking downloaded tools before each start (E10).
- A stronger USB disk identity before formatting in ISO Studio (E11).
- Pinned tool versions for external downloads (E14).
- Dead code removal across the workspace (D1 to D12).
- Making the `t14` tweak round-trip test skip network tweaks when offline.
- The Spanish, Italian and French grammar points of L11, which wait for native speakers.
- A refused start of a private job while another job is running marks the running job as private.
- The per-item failure reasons inside the "Failed: N. First error: ..." notice still come from the engine in English.
- "Sticky Keys shortcut" and "Search button on the taskbar" still show as off on a clean Windows 11. There the values exist, with other data than the catalog's "on" value (`Flags` is not 506, `SearchboxTaskbarMode` is not 1), so `default_on` does not apply.
- The longer search placeholder is cut off at the end of the search field in a 1024 px window in some languages.

## Tested

`cargo clippy --workspace --all-targets -- -D warnings` has no warnings. `cargo test --workspace --locked` passes 583 tests, 0 fail, and 26 are ignored live tests (538 passed before the fixes). Each fix has its own unit or sandbox test.

Not checked on a live system: E3 and E7 (no Windows ISO was available) and the new translations by native speakers.

The x64 packages ran in a VirtualBox VM with Windows 11 Home 26H2, build 26300, in Polish, on a 1024x768 screen, each pass from a clean snapshot. verify-install found 0 failed checks. The upgrade from 1.3.0 with history and settings, `WCLANGUAGE` in all eight languages and the uninstall passed 64 of 64 checks. Every page in every language, in light, dark and system mode, passed 168 of 168. The introduction and the tips in all eight languages passed 153 of 153. The extensions with real ffmpeg, yt-dlp and a real video passed 10 of 10, the portable build 11 of 11, and the edge cases (settings files with a BOM or broken JSON, wrong flags, a second start) 15 of 15.

Seen in the VM: a second start with `--open iso` switched the running window to ISO Studio; a `settings.json` with a BOM and `"language":"de"` opened in German; a profile with a BOM loaded for review; `--inspect` with an unknown id showed a notice; the Cleaner rows read "System  1 plik"; the eight toggles listed above showed as on; the Ukrainian ISO heading is all Cyrillic.

The live tests that change the system ran offline: tweaks, drift, services, AppX removal, DISM features, update modes, BitLocker decryption, the PowerShell profile, the scheduled check, the Standard preset and telemetry passed. The round trip of every reversible tweak (t14) passed for 53 tweaks and failed only on `block-adobe-net`, which downloads its host list and cannot run offline, the same as in 1.2.0, 1.2.1 and 1.3.0.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.3.1-x64-setup.exe`, `WindowCraft-1.3.1-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.3.1-x64.msi`, `WindowCraft-1.3.1-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.3.1-x64-portable.exe`, `WindowCraft-1.3.1-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates and of LAME |

## Audit items

| ID | Fix in 1.3.1 |
| :--- | :--- |
| E1 | Check APEv2 footer size in MP3 metadata stripper before subtracting from slice end |
| E2 | Skip junctions and symlinks in orphan scan and remove only the link if passed to removal |
| E3 | Clear read-only attributes and remove previous `work\media` directory before copying a new ISO |
| E4 | Skip catalog `RegDeleteTree` on rollback when a value inside the tree was restored; run tree deletions through registry scope |
| E5 | Restart every service stopped by `reset_windows_update` when stopping, renaming or cancellation ends early |
| E6 | Check WMI `ReturnValue` in `wmi::invoke_all` for BitLocker decryption and IPv6 binding changes |
| E7 | Convert `install.esd` to `install.wim` before injecting install drivers and match `edition_id` in the one-image WIM |
| E8 | Use checked 64-bit box bounds in the MP4 remuxer and return an error on overflow |
| E9 | Check Matroska block header size before subtracting and slicing |
| E12 | Use `default_on` when a toggle's registry value is absent during detection and drift checks |
| E13 | Untick peer tweaks in the same `exclusive` group in the UI and reject conflicting profiles |
| A1 | Show "Cancelled after N of M." when a tweak batch is cancelled and keep unapplied tweaks ticked |
| A2 | Untick only the items sent and finished by a job, keeping items ticked while the job ran |
| A3 | Show a warning notice when "Check now" fails instead of reporting no reverted tweaks |
| A4 | Strip a leading UTF-8 BOM when reading `settings.json`, profiles and WinUtil configs |
| A5 | Keep `toggles`, `segmented`, `update_mode` and `services` on GUI profile import and re-export |
| A6 | Tag the open-ISO job with `JobKind::OpenIso` instead of matching the translated job label |
| A7 | Hand over second-launch arguments via `WM_COPYDATA`, ignore headless flags in the running window and load `--config` for review only |
| A8 | Tag loaded X-Ray Inspector rows with the requested item and drop stale replies |
| A9 | Serialize scheduled-task updates in Settings and apply the latest state when a run finishes |
| A10 | Re-enable guided mode on "Show the introduction again" and "Show the tips again" |
| A11 | Show a notice when `--inspect` receives an unknown tweak id |
| A12 | Wrap the action buttons and category chips on the Applications and Tweaks pages |
| A13 | Wrap the Applications category row on a 1024 px window with the inspector open |
| A14 | Carry `needs_elevation` on `JobFailure` so "Restart as Administrator" appears in every language |
| A15 | Write `settings.json` atomically on language changes too |
| A17 | Report missing `"version": 1` on a WindowCraft profile instead of a WinUtil unknown-key error |
| A18 | Write `windowcraft-report.json` inside the directory when `--export-report` gets a folder path |
| A19 | State in the Extensions notes that unpacked ffmpeg and YouTube downloader tools use more disk space |
| A20 | Back up an unreadable non-empty `settings.json` to `settings.json.bad` before overwriting |
| L1 | Format file count and unit into the Cleaner scan row template instead of showing `{}` |
| L3 | Translate ISO Studio steps, engine errors and cleaner target names in all eight languages |
| L4 | Translate the three display toggles and the kernel timer labels on the Inspector's Display tab |
| L5 | Use the Polish plural rule (`count == 1` only for singular) separately from Ukrainian |
| L6 | Checked: `NoticeConverted`, `DriftResetFormat` and `NotifyDriftBody` needed no change |
| L7 | Use singular and plural item forms in CLI output |
| L8 | Replace the Latin `f` in Ukrainian `3. Конфігурація` |
| L9 | Use "Pokaż wprowadzenie ponownie" and "Standardowo" in Polish onboarding strings |
| L10 | Use "Paquet" instead of "Package" in the French inspector |
| L12 | Size Tweaks tabs to the longest label and widen the X-Ray Inspector `compare` label column to 110 px |
| L13 | Translate the Windows 11 requirement message of `setup.exe` in all eight `thm.wxl` files |
| L14 | Include tweaks and registry keys in the search placeholder across all seven non-English languages |
