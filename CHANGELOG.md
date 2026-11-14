<!-- Made by Nordan -->
# Changelog

## Unreleased

- The credit is "Made by Nordan" in the app, the CLI help, the installers, the exe version data, `LICENSE`, `Cargo.toml`, the README and the website. Email addresses and links stay the same.

## 1.4.1 - 2026-11-13

Fixes for option lists in the translated interface, and the credit moved to WindowCraft Dev Team.

- In every language but English, choosing an option from a list sent its translated label to the engine. "Multiplane overlay" in Polish matched no registry value, so both MPO values were deleted and the change was reported as done. "Agresywnie" in the Windows Update reset ran the standard reset, and "Domyślny" or "Najszybszy" in the DNS list failed with "not found". The option lists now show the translated label and send the English option name, and a profile with English option names loads in every language. The lists also show the current choice again: in 1.4.0 they were empty in the other seven languages.
- Installing AppX packages counted packages that have no Microsoft Store id as installed and unticked them: Xbox Speech To Text Overlay, Xbox TCUI and Solitaire Collection. They now stay ticked, and the notice names them.
- A profile chosen in the import dialog while a job runs is refused with the "another operation is running" notice. Before, the end of the job unticked the same ids in the new profile.
- A Time Machine session that failed, for example a feature set where one Windows component did not install, can be rolled back. Before, only finished and interrupted sessions had the button.
- When a job ends, an open Time Machine session and the X-Ray Inspector read their values again. They kept the values from before the job, with an undo button for a change already undone.
- ISO Studio left `install.swm`, `install2.swm` and the other parts of a split image in `work\media\sources` after writing a large image to USB. A later ISO export packed them next to `install.wim`. The parts are deleted after the copy, and before a new split.
- Startup folder items in the System & Gaming Inspector always showed as on, even when Task Manager had turned them off. Turning off an item of the common Startup folder under ProgramData wrote its flag to HKCU, where Windows ignores it. Both now use the `StartupApproved\StartupFolder` key of the folder's own hive, HKCU or HKLM.
- A session rollback runs the undo steps of each tweak once, also when the records of that tweak are not next to each other in the session.
- The search field grows up to 460 px when the window has room, and the placeholder is shorter in Polish, German, French, Spanish, Italian, Brazilian Portuguese and Ukrainian, so it is no longer cut off in a 1024 px window.
- In the X-Ray Inspector the scroll bar covered the last letters of a description line and the right border of the cards. The drawer content keeps 12 px clear for it.
- The credit is "Made by WindowCraft Dev Team" in all eight languages. It moved from the sidebar to the end of every page, in small type on the right under the last group. Settings has a new About group with the support address, support@windowcraft.dev, and a button that opens windowcraft.dev.
- The copyright holder in `LICENSE`, the installers, the exe version data and `Cargo.toml` is WindowCraft Dev Team, and every source file starts with that credit.
- The README is rewritten: shorter, with new screenshots of 1.4.1 drawn in the dark theme, and with windowcraft.dev at the top for the full description. The repository moved with the account to <https://github.com/windowcraftdev/WindowCraft>, and every link points there.

## 1.4.0 - 2026-11-11

The catalog in every interface language, and the rest of the 1.3.0 audit.

- Tweak, application, fix and update mode names and descriptions, tweak notices, AppX warnings, option labels and the application and AppX categories are translated into Polish, German, French, Spanish, Italian, Brazilian Portuguese and Ukrainian. The texts live in `crates/windowcraft-config/catalog/i18n/<tag>.json`, keyed by catalog id, and an item that has no entry keeps its English text. Changing the language in Settings switches the catalog at once, and the application categories are sorted alphabetically in that language, with accents and the Ukrainian letters І, Ї, Є and Ґ in their alphabet place. Search finds an item by its translated words and by its English ones. A test fails the build when a language leaves an item in English, names an id that does not exist or drops a path such as `%SystemRoot%\Temp` from a description.
- Search results for tweaks and fixes show the tab or page name in the chosen language instead of "Essential" or "System Config".
- The tools folder of the extensions (`%LOCALAPPDATA%\WindowCraft\tools`) is locked to Administrators and SYSTEM, with read and run rights for users. Before yt-dlp, ffmpeg or Deno runs, WindowCraft checks that the folder and every file the tool loads are owned by Administrators or SYSTEM and that no one else can write to them; a file that fails the check is deleted and downloaded again. Tools that 1.3.1 or older downloaded sit in a folder users could write to, so the first use after the update downloads them again.
- Before ISO Studio erases a USB drive, it compares the serial number and the partition table id of the disk with the ones read when the disk was chosen. A stick that reports neither is refused with a message, so another stick of the same model that got the same disk number is never erased.
- The live VM test of every reversible tweak skips the tweaks that download a list when the network is off and lists them as skipped.
- Removed code nothing used: the `Close` and `Continue` messages in all eight tables, the English `title()` methods of pages and tabs, `project_momentum`, `relative_velocity` and `Spring2D` in the physics crate, `usb::volume_label` and `DiskFacts::removable`, the unused fields of `DriverInfo`, `MountedImage::path`, the English `label()` methods of session states, autorun sources, package managers, service run states, interrupt priorities and search kinds, and the `ExpandString` and `MultiString` catalog value types, which no catalog entry used. The registry sandbox and the recording runner for tests are built only for tests (feature `testing`). `copy_tree`, the text decoder for console output and INF files, the mounted-image check and the EBML Void writer each have one copy now. `MiscState` is now `ExtensionsState`.

## 1.3.1 - 2026-11-10

Fixes from a full audit of 1.3.0.

- On a clean Windows 11 install, tweaks whose Windows default has no registry value (centered taskbar icons, Task View, Game Mode, Bing search, Start menu recommendations, the Settings home page, sign-in blur and Modern Standby network) show as on instead of off, and the drift check does not report them as reverted when the value is absent.
- The three IPv6 tweaks (`ipv46`, `teredo` and `disable-ipv6`) are mutually exclusive: ticking one unticks the others, and a profile that lists more than one is rejected on import.
- Rolling back a tweak skips a catalog registry tree deletion when the rollback has just restored a value inside that tree, so rolling back "Disable Explorer folder type auto-discovery" keeps an existing `FolderType` value. Registry tree deletions also go through the same registry scope as every other write, which covers `right-click-menu`.
- "Reset Windows Update components" starts every service it stopped again when stopping a service, renaming `SoftwareDistribution` or cancelling ends the reset early. BitLocker decryption and IPv6 binding changes read the `ReturnValue` of the WMI method call and report an error when it is non-zero.
- The leftover-folder cleaner skips directory junctions and symbolic links in `%APPDATA%` and `%LOCALAPPDATA%` during a scan, and if a junction or symbolic link is passed to removal, it deletes only the link without entering the target folder.
- In ISO Studio, opening a new ISO deletes the previous `work\media` folder after clearing read-only flags, so files from an earlier ISO do not stay behind. When driver injection is enabled for an ISO that has `sources\install.esd`, ISO Studio converts the chosen edition to `install.wim` before injecting the install drivers and finds that edition again in the one-image WIM on a second build.
- The MP3 metadata stripper checks the size in an APEv2 footer and stops when it is under 32 bytes or larger than the remaining file, instead of looping forever on a broken tag. The MP4 and Matroska remuxers return an error on oversized 64-bit box sizes or truncated block headers instead of panicking, and fall back to ffmpeg when it is installed.
- Cancelling a tweak run shows "Cancelled after N of M." instead of "Items applied: N". When any queue job finishes or is cancelled, only the items that actually finished are unticked, leaving unapplied items and items ticked during the run in the selection.
- When "Check now" on the Time Machine page fails to read the system, it shows a warning notice and keeps the existing drift list and last-check timestamp instead of reporting that no tweaks were reverted.
- Importing a profile in the window keeps `toggles`, `segmented`, `update_mode` and `services`. The toggles and segmented choices run with "Run Tweaks", all four fields are written back when the selection is exported, and a notice explains that `update_mode` and `services` are applied by `windowcraft.exe --config <file> --run`.
- `settings.json` and imported profile or WinUtil files accept a UTF-8 BOM. Every write to `settings.json`, including a language change, goes through a temporary file, and an unreadable non-empty `settings.json` is backed up to `settings.json.bad` before a new file is written. A WindowCraft profile that is missing `"version": 1` now reports the missing version field instead of a WinUtil key error.
- Starting `windowcraft.exe` while a window is already open brings the running window to the front without unmaximizing it and sends `--open`, `--inspect`, `--config` and `--preset` to it over `WM_COPYDATA`. The running window ignores `--run`, `--check-drift`, `--export-report`, `--purge-data` and `--close-running` from a second start, and `--config` only loads the selection to review. `--inspect` with an unknown tweak id shows a notice instead of opening an empty inspector drawer, and `--export-report` given a folder path writes `windowcraft-report.json` inside that folder.
- Changing the language while an ISO is opening still loads the edition list when the job finishes. Opening items quickly in the X-Ray Inspector drops late results from an earlier item. Toggling the automatic check in Settings queues one scheduled-task update at a time and runs the latest choice afterwards. "Show the introduction again" and "Show the tips again" also work when the window was opened with `--open`, `--inspect`, `--config` or `--preset`.
- When a job fails because it needs administrator rights, the notice banner shows the "Restart as Administrator" button in every language. On the Applications and Tweaks pages, the action buttons and category chips wrap onto a second line on narrow windows with the inspector open, the Tweaks tabs widen to fit longer translations, and the X-Ray Inspector has a wider label column. The Extensions page notes state that the unpacked disk size of ffmpeg and the YouTube downloader tools is larger than the download size.
- The Cleaner scan rows fill in the file count and unit instead of showing `{}`. ISO Studio step messages, engine errors, cleaner target names, the three display toggles and the kernel timer labels on the Inspector's Display tab are translated in all eight languages. Polish counts such as 21, 31 and 101 take the plural instead of the singular; the CLI says "1 item" instead of "1 items"; the Ukrainian ISO configuration heading, the Polish onboarding button and depth label, the French "Paquet" label and the search bar placeholder in the seven non-English languages are fixed. `setup.exe` translates its Windows 11 requirement message in all eight installer languages.

## 1.3.0 - 2026-11-09

An introduction on the first start and a short tip on each page.

- On the first plain start, four questions open over the window: what to do with WindowCraft, whether to keep your files, whether you build your own ISO, and how much should change. Each screen has a progress bar, Skip and, from the second screen on, Back. The answers carry numbers, and the keys 1 to 4 pick them. Escape is Skip.
- The last screen lists the plan, and Start puts it into the queues without running anything. The answer about change picks the Minimal, Standard or Advanced preset. A fresh install, or Everything, also marks the 14 default AppX packages for removal. The answer about files decides what the cleaner ticks after its first scan: temporary files, logs and minidumps, or those plus the GPU shader and developer caches. ISO Studio work files are never ticked. Start opens Tweaks & Privacy, or the Cleaner tab for a tidy-up.
- After the questions each page shows a tip of two or three sentences once. Next marks it as read and opens the next page in the sidebar whose tip is unread; on the last one the button reads Done. Don't show again turns all tips off. A click outside the tip closes it without saving and still reaches what was clicked.
- Settings has a new group with "Show the introduction again" and "Show the tips again".
- The state is kept under the `onboarding` key of `settings.json`, versioned and checked like a profile. A state that this version cannot read turns the questions and the tips off. A start with `--open`, `--inspect`, `--config` or `--preset` shows neither, so screenshots and scripts see the page they asked for.
- Settings are now written to a temporary file and renamed over `settings.json`. When the file cannot be written, a notice says so and the choice holds until the app closes.
- All new text is in the eight interface languages.

## 1.2.1 - 2026-11-08

Fixes for what a full test of 1.2.0 in a VirtualBox VM found, in all eight languages, on a 1024x768 screen.

- A list no longer jumps back to the top when a notice appears or is dismissed. The empty place holders for the drift banner and the notice had a fixed height of 0, and iced leaves such children out of a column, so the page body moved to another position in the widget tree and lost its scroll state. Both banners now keep a slot that stays in the tree.
- The first window fits the screen. It opened at 1280x800 and was centred on the whole monitor, so on 1024x768 and 1366x768 it ran under the taskbar. It now shrinks to the work area when needed and is centred in it. The smallest size shrinks with it on small or scaled screens.
- The Inspector tabs are as wide as their longest label, so "Benchmark de DNS" in Portuguese no longer wraps onto two lines. The Italian Autoruns tab is "Avvio automatico" now; "Esecuzione automatica" would not fit five tabs into a 1024 px window.
- YouTube downloads are saved as `<title>.mp4` or `<title>.mp3`. Up to 1.2.0 the name ended in the video id, for example `[jNQXAC9IVRw]`, which is the link. The file is cleaned before it is moved to the chosen folder, and a name that is taken gets " (2)". Before, yt-dlp would have taken an existing file of the same name for the finished download. The privacy note on the page now says that the file name comes from the title.
- The installed metadata stripper and file converter said that uninstalling deletes downloaded tools. They download nothing, and the line now says so.
- After switching the Windows Update mode, WindowCraft reads the start type of each service back. When Windows puts wuauserv or BITS back at once, which Windows Update Medic does while updates are running, the mode is rolled back and the message names the service. Before, the mode reported success while the services kept their old start type.
- When DISM gives no text for an error, the message falls back to the Windows text for the code, and when that is empty too, it points to `Logs\DISM\dism.log`. Changing reserved storage while Windows was servicing showed only "(0x800F0975)".
- Replacing `hosts` and other system files is tried again for about six seconds when another process has the file open. A rollback of the Adobe block list once failed with "used by another process" (0x80070020).

## 1.2.0 - 2026-11-08

An Extensions page with a metadata stripper, a file converter and a YouTube downloader. Nothing on it is installed until you choose it.

- The new Extensions page is the last one in the sidebar, under System & Gaming Inspector. `--open extensions` opens it.
- The page has four extensions: Metadata stripper, File converter, Additional formats (ffmpeg) and YouTube downloader. Each one has its own Install button, and none is installed by default. The stripper and the converter are part of WindowCraft and download nothing. A blue "Made by WindowCraft" badge next to their names marks them. Additional formats downloads ffmpeg, about 90 MB. The YouTube downloader downloads yt-dlp, ffmpeg and Deno, about 150 MB. Downloaded tools go to `%LOCALAPPDATA%\WindowCraft\tools`. Uninstall on the page deletes the tools that no other installed extension still uses, and uninstalling WindowCraft deletes the folder.
- Tools are fetched through the GitHub API, with the SHA-256 taken from the release asset or from the checksum file of the release. The plain release download links are the fallback. A failed download is tried 4 times, with longer waits each time, and the job shows each new attempt. On 2026-11-08 those links answered "504 Gateway Timeout" for hours while the API worked.
- The metadata stripper writes a copy of each chosen file with `-clean` in its name and leaves the original as it was. It handles JPEG, PNG, WebP, GIF, MP3, AAC, FLAC, MP4, M4A, M4V, MOV, 3GP, MKV, MKA, WebM, AVI, WAV, AIFF, Ogg, Opus, WMA, WMV and ASF itself, without ffmpeg. FLV, OGV and TS need Additional formats.
- The stripper removes GPS location, camera data, dates, author, title, comments, cover art, chapters, thumbnails and editing history, and in video and audio also the creation dates, encoder names and track names. A phone's separate GPS track in an MP4 goes with its data. A JPEG keeps its color profile and, when the photo is rotated, only the orientation tag, so it still shows upright. MKV and WebM keep their font attachments, which subtitles need, and lose cover images. In video and audio files the picture and sound data are not touched, so quality stays the same.
- The file converter has 21 targets: PNG, JPG, WebP, AVIF, BMP, TIFF, GIF and ICO for pictures, MP4, MKV, WebM, MOV and AVI for video, and MP3, M4A, FLAC, WAV, OGG, Opus, AIFF and WMA for sound. The new file is saved next to the original, with a number added when the name is taken.
- WindowCraft converts most of this itself. Pictures go between PNG, JPG, WebP, BMP, TIFF, GIF and ICO; the EXIF rotation is applied, a transparent picture saved as JPG gets a white background, WebP is saved lossless and ICO is limited to 256 pixels. Sound in MP3, AAC, ALAC, FLAC, Vorbis, PCM or ADPCM is decoded by Symphonia and encoded to MP3 with LAME (VBR V0), to FLAC, WAV or AIFF. AAC goes into M4A and Opus into Opus without re-encoding. Video moves between MP4, MOV, MKV and WebM without re-encoding: H.264, HEVC, VP9 and AV1 with AAC, Opus, MP3, AC-3 or FLAC, and SRT or MP4 text subtitles. WebM gets no subtitles.
- Everything else needs Additional formats: AVIF, AVI, OGG, WMA, HEIC pictures, video re-encoding (for example H.264 to WebM, or a clip to GIF), animated GIF input and fragmented MP4. With ffmpeg installed, a JPEG converted to WebP is saved lossy. Without it, the converter says which file needs it.
- The YouTube downloader saves one video as MP4 (video with sound, H.264 when YouTube has it) or MP3 (sound only). It accepts youtube.com, youtu.be, music.youtube.com and youtube-nocookie.com links. It runs yt-dlp without an account, cookies, cache or config files, and yt-dlp writes no info JSON, description, thumbnail, subtitles or comments and embeds nothing. Partial files stay in a private folder that is emptied after every download, also a cancelled one. yt-dlp is fetched again when the copy is older than 14 days, and an older copy is used when GitHub cannot be reached.
- Every finished download goes through the metadata stripper. It also blanks the track name "ISO Media file produced by Google Inc." with a date that YouTube writes into its MP4 files, so the saved file keeps only the language tag and the file type.
- A job that failed shows its message with a warning icon.
- The stripper, the converter and the downloader do not write the names of files or videos to `windowcraft.log`, and their file dialogs keep their choices out of the Windows Recent list.
- `windowcraft.exe` grew by 3.3 MB, to 18.2 MB, for the picture and sound libraries and LAME. `NOTICE.md` and `THIRD-PARTY-NOTICES.md` carry the LAME (LGPL), Symphonia (MPL-2.0) and flacenc (Apache-2.0) licenses.

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
