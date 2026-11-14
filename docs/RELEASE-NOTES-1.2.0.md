<!-- Made by Nordan -->
# WindowCraft 1.2.0

This release adds the Extensions page: a metadata stripper, a file converter, additional formats through ffmpeg and a YouTube downloader. None of them is installed until you choose it. Tweaks, applications, ISO Studio, Time Machine and the installers work as in 1.1.0.

## Extensions

The page is the last one in the sidebar, under System & Gaming Inspector, and `--open extensions` opens it. Each extension has its own Install button. The two whose code is part of WindowCraft carry a blue "Made by WindowCraft" badge.

| Extension | Downloads | Notes |
| :--- | :--- | :--- |
| Metadata stripper | Nothing | Part of WindowCraft, works offline |
| File converter | Nothing | Part of WindowCraft, works offline |
| Additional formats (ffmpeg) | ffmpeg, about 90 MB | Used only for what WindowCraft cannot do itself |
| YouTube downloader | yt-dlp, ffmpeg and Deno, about 150 MB | Shares ffmpeg with Additional formats |

Downloaded tools go to `%LOCALAPPDATA%\WindowCraft\tools`. Uninstall on the page deletes the tools that no other installed extension still uses; uninstalling WindowCraft deletes the whole folder.

Tools come from their own GitHub releases through the GitHub API. The SHA-256 is taken from the release asset, or from the checksum file of the release when the asset has none, and a file that does not match is not used. The plain release download links are the fallback. On 2026-11-08 those links answered "504 Gateway Timeout" for hours while the API worked, which is what stopped the YouTube downloader in the test builds. A failed download is now tried 4 times with longer waits, and the job shows each attempt. When GitHub still does not answer, the message says so and asks to try again later.

## Metadata stripper

The stripper writes a copy with `-clean` in its name and leaves the original as it was. It removes GPS location, camera data, dates, author, title, comments, cover art, chapters, thumbnails and editing history. In video and audio it also clears creation dates, encoder names and track names, and a phone's separate GPS track in an MP4 goes with its data.

WindowCraft reads and writes these formats itself: JPEG, PNG, WebP, GIF, MP3, AAC, FLAC, MP4, M4A, M4V, MOV, 3GP, MKV, MKA, WebM, AVI, WAV, AIFF, Ogg, Opus, WMA, WMV and ASF. In video and audio files the picture and sound data stay byte for byte the same; only the metadata around them is blanked or removed. A JPEG keeps its color profile and, when the photo is rotated, only the orientation tag. MKV and WebM keep their font attachments, which subtitles need. FLV, OGV and TS need Additional formats.

## File converter

The converter has 21 targets. The new file is saved next to the original, with a number added when the name is taken.

| Kind | Targets | Done by WindowCraft | Needs Additional formats |
| :--- | :--- | :--- | :--- |
| Picture | PNG, JPG, WebP, AVIF, BMP, TIFF, GIF, ICO | PNG, JPG, WebP (lossless), BMP, TIFF, GIF, ICO up to 256 px | AVIF, HEIC input, animated GIF input |
| Video | MP4, MKV, WebM, MOV, AVI | MP4, MOV, MKV and WebM when the streams fit the target, without re-encoding | AVI, re-encoding, for example H.264 to WebM or a clip to GIF |
| Sound | MP3, M4A, FLAC, WAV, OGG, Opus, AIFF, WMA | MP3 (LAME, VBR V0), FLAC, WAV, AIFF; AAC into M4A and Opus into Opus without re-encoding | OGG, WMA, AAC from other codecs |

Pictures are turned upright from their EXIF orientation, and a transparent picture saved as JPG gets a white background. Sound is decoded by Symphonia from MP3, AAC, ALAC, FLAC, Vorbis, PCM and ADPCM. The video remuxer handles H.264, HEVC, VP9 and AV1 with AAC, Opus, MP3, AC-3 or FLAC, plus SRT and MP4 text subtitles; WebM gets no subtitles. Fragmented MP4 goes to ffmpeg. With ffmpeg installed, a JPEG converted to WebP is saved lossy; without it, lossless.

Without Additional formats, the converter names each file it could not convert and says that it needs them.

## YouTube downloader

The downloader saves one video as MP4 (video with sound, H.264 when YouTube has it) or MP3 (sound only), from youtube.com, youtu.be, music.youtube.com and youtube-nocookie.com links. yt-dlp runs without an account, cookies, cache or config files and writes no info JSON, description, thumbnail, subtitles or comments. Partial files stay in a private folder that is emptied after every download, also a cancelled one. yt-dlp is fetched again when the copy is older than 14 days, and the older copy is used when GitHub cannot be reached.

Every finished download goes through the metadata stripper. YouTube writes "ISO Media file produced by Google Inc." with a date into the track names of its MP4 files; the stripper blanks those too, so ffprobe shows only the language tag and the file type of the saved file.

None of the extensions writes file names or links to `windowcraft.log`, and their file dialogs keep their choices out of the Windows Recent list.

## Size and licenses

`windowcraft.exe` is 18.2 MB, 3.3 MB more than 1.1.0, for the `image` crate, Symphonia, flacenc and LAME. LAME is compiled into the program and is under the LGPL; Symphonia is under the MPL-2.0. `NOTICE.md` says where their source code is, and `THIRD-PARTY-NOTICES.md` carries their license texts.

## Tested

The unit and integration tests pass (504, clippy clean at `-D warnings`). New tests cover the downloads with retries and cancel, the GitHub API asset lookup, the install markers and which tools an uninstall keeps, every native stripper container, Ogg page checksums and continuation flags, and the MP4 and Matroska readers and writers.

These tests ran against real files and real downloads on the development PC:

| Test | Result |
| :--- | :--- |
| Stripper on 15 formats tagged with a title, artist, comment, creation date, encoder and GPS position, only FLV through ffmpeg | None of the values and no XMP is left in any clean copy, and every copy decodes without errors |
| Converter, every one of the 21 targets with ffmpeg installed, WindowCraft first | Every output decodes |
| Pictures and sound through every native format, without ffmpeg | Same size and sample count after the round trip; what needs ffmpeg reports so |
| Remux of H.264, HEVC, VP9 and AV1 with AAC, Opus, MP3, AC-3 and FLAC between MP4, MOV, MKV, WebM, M4A and Opus | Same packet count as the source, decodes without errors, subtitle times match |
| A real YouTube video as MP4 and MP3, tools fetched through the GitHub API | Both saved; the MP4 shows only the language tag and file type |
| A JPEG with EXIF rotation converted to PNG | Saved upright |

The page was checked on screenshots of a development build of 1.2.0 with no extension, two and all four installed. The buttons were not clicked by hand.

Not tested: HEIC input, a video over 4 GB, the ARM64 build on ARM hardware (it compiles; LAME is compiled with the ARM64 toolchain) and the installers in the VM. The installers and the uninstaller did not change since 1.1.0, apart from the notice files they carry.

## Known bugs

| ID | Bug | Workaround |
| :--- | :--- | :--- |
| SEC-3 | `OOSU10.exe` and `Autologon.exe` have no pinned hash, because their vendors publish them at fixed "latest" URLs. They must pass Authenticode. | None needed. |

## Known limitations

ffmpeg and yt-dlp are fetched from their "latest" GitHub releases and checked against the SHA-256 that the same release publishes, so the check catches a broken download, not a compromised release.

The builds are not code-signed yet, so SmartScreen warns on first run. No ARM64 binary has run on ARM hardware. The other limitations of [1.1.0](https://github.com/windowcraftdev/WindowCraft/releases/tag/v1.1.0) still apply.

## Downloads

| File | Use |
| :--- | :--- |
| `WindowCraft-1.2.0-x64-setup.exe`, `WindowCraft-1.2.0-arm64-setup.exe` | Installer with a Start menu entry |
| `WindowCraft-1.2.0-x64.msi`, `WindowCraft-1.2.0-arm64.msi` | For Intune, GPO or `msiexec /qn` |
| `WindowCraft-1.2.0-x64-portable.exe`, `WindowCraft-1.2.0-arm64-portable.exe` | Runs without installing |
| `SHA256SUMS.txt` | Checksums |
| `THIRD-PARTY-NOTICES.md` | Licenses of the bundled Rust crates and of LAME |
