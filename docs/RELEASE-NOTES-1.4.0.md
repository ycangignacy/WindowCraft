# WindowCraft 1.4.0

The catalog now follows the interface language. This release also closes the rest of the 1.3.0 audit: a lock on the downloaded tools, a stronger disk check before ISO Studio erases a USB drive, and the removal of code nothing used.

## The catalog in every language

Tweak, application, fix and update mode names and descriptions, tweak notices, AppX warnings, option labels and the application and AppX categories are translated into Polish, German, French, Spanish, Italian, Brazilian Portuguese and Ukrainian. The texts live in `crates/windowcraft-config/catalog/i18n/<tag>.json`, one file per language, keyed by catalog id. Only text is replaced; ids, registry paths, commands and presets stay the same in every language, and a test checks that. An item with no entry keeps its English text.

Changing the language in Settings reloads the catalog at once and clears the category filters, since their names change with the language. Application categories are sorted alphabetically in the chosen language; accents are ignored for the order, and the Ukrainian letters І, Ї, Є and Ґ come right after И, Е and Г, as in the Ukrainian alphabet. Search matches the translated words and the English ones, so "telemetry" still finds "Wyłącz telemetrię Windows" in Polish. Search results for tweaks and fixes show the tab or page name in the chosen language. On the Time Machine page, the name of a reverted tweak is shown in the chosen language.

The build fails when a language leaves a catalog item in English, names an id that does not exist, or drops a token such as `%SystemRoot%\Temp` or a registry path from a description. Windows error details and the CLI output stay in English.

## Tools folder lock (E10)

The extensions download yt-dlp, ffmpeg and Deno to `%LOCALAPPDATA%\WindowCraft\tools`. Up to 1.3.1 that folder inherited the user profile's permissions, so any program running as the user could replace a tool that WindowCraft later started as an administrator.

When WindowCraft runs elevated, it now sets the folder's owner to Administrators and gives it a protected DACL: full control for SYSTEM and Administrators, and read and run rights for users. The DACL includes the OWNER RIGHTS entry, so the owner has no hidden right to change the DACL. A link in place of the folder is removed before the folder is created. Before a tool runs, WindowCraft checks that the folder is still locked and that every file the tool loads (for ffmpeg, every file in its folder) is owned by Administrators or SYSTEM and cannot be written by anyone else. A tool that fails the check counts as missing; it is deleted and downloaded again.

Tools that 1.3.1 or older downloaded sit in a folder users could write to, so the first use after the update deletes them and downloads them again. Without elevation the check is skipped, because the tools then run with the user's own rights.

## USB disk identity (E11)

ISO Studio used to recognize the chosen USB disk by its number and model only. Before it erases the drive, it now also compares the serial number and the partition table id (the MBR signature or the GPT disk id) with the ones read when the disk was chosen. If another stick of the same model got the same disk number in the meantime, the write stops with a message. A disk that reports neither a serial number nor a partition table id is refused, since there is nothing to compare.

## Removed code (D1 to D12)

- The `Close` and `Continue` messages, from the enum and from all eight tables (D1). `DriftResetFormat` and `DriftResetByUpdateFormat` stay, because `i18n::drift_summary` uses them; the audit listed them as unused by mistake.
- The English `title()` methods of pages, Tweaks tabs and Inspector tabs (D2).
- `project_momentum`, `relative_velocity` and `Spring2D` in the physics crate (D3).
- `usb::volume_label` and `DiskFacts::removable` (D4).
- The English `label()` methods of session states, autorun sources, package managers, service run states, interrupt priorities and search kinds (D6). `DeviceClass::label` sorts the device list, `TweakGroup::label` is part of the search text and `Engine::drift_summary` writes the English CLI output, so those three stay.
- The `DriverInfo` fields nothing read and `MountedImage::path` (D7).
- The registry sandbox and the recording runner are built only for tests, behind the `testing` feature (D8).
- `MiscState` is now `ExtensionsState`; the `misc` alias for old shortcuts stays (D9).
- The `ExpandString` and `MultiString` catalog value types, which no catalog entry used (D10).
- One copy each of `copy_tree`, the console and INF text decoder (it also reads UTF-16 with a BOM), the mounted-image check and the EBML Void writer (D11).

`Catalog::load` still does not call `validate`; the catalog is built into the binary and the tests run the validation, which a comment now says (D5). D12 was done in 1.3.1.

## Live tests

The live VM test that applies and rolls back every reversible tweak (t14) skips the tweaks that download a list when the network is off and prints them as skipped. In 1.3.0 and 1.3.1 `block-adobe-net` failed offline for that reason.

## Not changed in this release

- Pinned versions for the downloaded tools (E14). The SHA-256 still comes from the same GitHub release API as the file. The plan is to pin ffmpeg and Deno with their hashes in the code, since they change rarely, and to keep yt-dlp on its latest release: YouTube changes often enough that a pinned yt-dlp would need a new WindowCraft release each time.
- The Spanish, Italian and French grammar points of L11, and a review of the catalog translations by native speakers.
- The per-item failure reasons inside the "Failed: N. First error: ..." notice still come from the engine in English.
- The longer search placeholder is cut off at the end of the search field in a 1024 px window in some languages.

## Tested

`cargo clippy --workspace --all-targets -- -D warnings` has no warnings. `cargo test --workspace` passes 592 tests, 0 fail, and 26 are ignored live tests (583 in 1.3.1). The tests that need elevation (the tools lock and the DACL read-back) skip on a normal shell and ran elevated in the VM.

Not checked on a live system: E11, because no Windows ISO was available for the ISO build and USB write tests (t08 and t09), and the translations by native speakers.

The x64 packages ran in a VirtualBox VM with Windows 11 Home 26H2, build 26300, in Polish, on a 1024x768 screen, each pass from a clean snapshot. The upgrade from 1.3.1 with history and settings, `WCLANGUAGE` in all eight languages and the uninstall passed 64 of 64 checks. Every page in every language, in light, dark and system mode, passed 168 of 168. The introduction and the tips in all eight languages passed 153 of 153. The edge cases (settings files with a BOM or broken JSON, wrong flags, a second start) passed 15 of 15 and the portable build 11 of 11, the same as 1.3.1.

The engine unit tests passed elevated in the VM, 257 of 257, including the tools lock tests. The extensions with real ffmpeg, yt-dlp and a real video passed 10 of 10: the tools were downloaded into the locked folder, which read back as owned by Administrators with a protected DACL, and yt-dlp and ffmpeg ran from it. The first YouTube download got "HTTP Error 403: Forbidden" from YouTube and passed when run again a few minutes later.

The live tests that change the system ran offline from a clean snapshot: tweaks, drift, services, AppX removal, DISM features, update modes, BitLocker decryption, the PowerShell profile, the scheduled check, the Standard preset and telemetry passed. The round trip of every reversible tweak (t14) passed for 53 tweaks and skipped `block-adobe-net`, which needs the network; in 1.3.1 that tweak made t14 fail offline. The click run showed the catalog in Polish, Spanish, French, Italian, German and Ukrainian, with the Ukrainian categories in alphabetical order.

The first pass ran on packages built before the category sort was fixed, so the rebuilt packages ran once more from a clean snapshot. Their SHA-256 sums matched, the upgrade from 1.3.1 and the uninstall passed 64 of 64, every page in every language 168 of 168 (with "Développement" now before "Documents" in French) and the portable build 11 of 11. The first try of that pass stopped on a Windows stop error (IRQL_NOT_LESS_OR_EQUAL in `ntoskrnl.exe`) while the harness read the edition, the BitLocker state and the AppX list, before any WindowCraft file ran; the run from a fresh snapshot did not show it again.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.4.0-x64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.4.0-x64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.4.0-x64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates and of LAME |

## Audit items

| ID | Change in 1.4.0 |
| :--- | :--- |
| L2 | Translate catalog names, descriptions, notices, warnings, options and categories into the seven other languages |
| E10 | Lock the tools folder to Administrators and SYSTEM and check owner and DACL before each tool runs |
| E11 | Compare serial number and partition table id before erasing a USB disk |
| E14 | Not changed; plan above |
| D1 to D11 | Remove the unused code listed above; D5 gets a comment |
| t14 | Skip network tweaks when the VM is offline |
