// Made by WindowCraft Dev Team
//! YouTube downloads through yt-dlp, as MP4 video or MP3 audio.
//!
//! The tools (yt-dlp, ffmpeg, and Deno because yt-dlp needs a JavaScript
//! runtime for YouTube) arrive when the user installs the extension, see
//! `tools`.
//!
//! Privacy: yt-dlp ignores every config file and runs without a cache, a
//! cookie jar or a browser profile. It writes no info JSON, description,
//! thumbnail, subtitles or comments, and embeds nothing. The finished file
//! then goes through `metadata::strip_in_place`, so it carries no title,
//! source URL or encoder tag either. Only YouTube itself and GitHub, for the
//! tools, are contacted.

use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use crate::metadata;
use crate::tools::{Tools, io_error};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp4,
    Mp3,
}

/// Where a job stands, for the progress line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    /// yt-dlp is being refreshed, or a missing tool fetched again.
    Tools(crate::tools::Stage),
    /// The video downloads: share of the current part, and which part it is.
    /// MP4 usually comes as a video part followed by an audio part.
    Downloading(f32, u32),
    /// ffmpeg merges the parts or converts the audio.
    Converting,
    /// Metadata is being removed from the result.
    Cleaning,
}

/// Accepts youtube.com, its mobile, music and no-cookie hosts, and youtu.be.
pub fn is_youtube_url(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return false;
    }
    let Some(rest) = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")) else {
        return false;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..end].to_ascii_lowercase();
    let host = host.split(':').next().unwrap_or_default();
    let path = &rest[end..];
    let base = ["www.", "m.", "music."].iter().fold(host, |host, prefix| host.strip_prefix(prefix).unwrap_or(host));
    matches!(base, "youtube.com" | "youtu.be" | "youtube-nocookie.com") && path.len() > 1
}

fn format_args(format: Format) -> &'static [&'static str] {
    match format {
        // H.264 and AAC first, so the file plays everywhere, including the
        // Windows Photos app and phones.
        // Bitexact output keeps the ffmpeg version out of the file. The LAME
        // tag of an MP3 would carry it otherwise, for example "Lavc63.26".
        Format::Mp4 => &[
            "-f",
            "bv*+ba/b",
            "-S",
            "vcodec:h264,acodec:aac",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
            "--postprocessor-args",
            "Merger+ffmpeg_o:-fflags +bitexact -flags:v +bitexact -flags:a +bitexact",
        ],
        Format::Mp3 => &[
            "-f",
            "ba/b",
            "--extract-audio",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "0",
            "--postprocessor-args",
            "ExtractAudio+ffmpeg_o:-fflags +bitexact -flags:a +bitexact",
        ],
    }
}

/// Parses `WCP <downloaded> <total> <estimate>` from the progress template.
fn parse_progress(line: &str) -> Option<(f64, Option<f64>)> {
    let mut parts = line.strip_prefix("WCP ")?.split_whitespace();
    let downloaded = parts.next()?.parse::<f64>().ok()?;
    let total = parts.next().and_then(|value| value.parse::<f64>().ok());
    let estimate = parts.next().and_then(|value| value.parse::<f64>().ok());
    Some((downloaded, total.or(estimate).filter(|total| *total > 0.0)))
}

/// Downloads `url` into `folder` and returns the finished file.
pub fn download_video(
    tools: &Tools,
    url: &str,
    format: Format,
    folder: &Path,
    cancel: &CancelToken,
    mut progress: impl FnMut(Stage),
) -> Result<PathBuf> {
    let url = url.trim();
    if !is_youtube_url(url) {
        return Err(EngineError::Invalid(url.to_owned()));
    }
    std::fs::create_dir_all(folder).map_err(io_error(folder))?;
    // Parts and fragments go to a private folder that is emptied afterwards,
    // so a cancelled download leaves nothing next to the user's files.
    // The finished file is also cleaned there and only then moved into
    // `folder`, so the user never sees an uncleaned copy, and a file that
    // already has the same name gets " (2)" instead of being reused.
    let staging = staging_dir();
    let _ = std::fs::remove_dir_all(&staging);
    let (finished, parts) = (staging.join("finished"), staging.join("parts"));
    for dir in [&finished, &parts] {
        std::fs::create_dir_all(dir).map_err(io_error(dir))?;
    }
    let result = run_yt_dlp(tools, url, format, &finished, &parts, cancel, &mut progress).and_then(|path| {
        progress(Stage::Cleaning);
        metadata::strip_in_place(&path, Some(&tools.ffmpeg()), cancel)?;
        move_into(&path, folder)
    });
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// Moves `file` into `folder` under a name nothing there uses yet.
fn move_into(file: &Path, folder: &Path) -> Result<PathBuf> {
    let name = file.file_name().ok_or_else(|| EngineError::NotFound("the downloaded file".to_owned()))?;
    let target = free_path(folder, Path::new(name));
    if std::fs::rename(file, &target).is_err() {
        // Downloads can sit on another drive, where a rename cannot work.
        std::fs::copy(file, &target).map_err(io_error(&target))?;
        let _ = std::fs::remove_file(file);
    }
    Ok(target)
}

/// `name.ext` in `folder`, or `name (2).ext` and up when that is taken.
fn free_path(folder: &Path, name: &Path) -> PathBuf {
    let stem = name.file_stem().map_or_else(|| "video".into(), |stem| stem.to_string_lossy().into_owned());
    let extension = name.extension().map(|extension| format!(".{}", extension.to_string_lossy())).unwrap_or_default();
    let mut candidate = folder.join(format!("{stem}{extension}"));
    let mut number = 2;
    while candidate.exists() {
        candidate = folder.join(format!("{stem} ({number}){extension}"));
        number += 1;
    }
    candidate
}

fn staging_dir() -> PathBuf {
    crate::win::local_app_dir().join("youtube-staging")
}

fn run_yt_dlp(
    tools: &Tools,
    url: &str,
    format: Format,
    folder: &Path,
    staging: &Path,
    cancel: &CancelToken,
    progress: &mut impl FnMut(Stage),
) -> Result<PathBuf> {
    let ffmpeg_dir = tools.ffmpeg_dir.display().to_string();
    let js_runtime = format!("deno:{}", tools.deno.display());
    let home = format!("home:{}", folder.display());
    let temp = format!("temp:{}", staging.display());
    let mut args: Vec<&str> = vec![
        "--ignore-config",
        "--no-cache-dir",
        "--no-cookies",
        "--no-cookies-from-browser",
        "--no-playlist",
        "--no-mtime",
        "--no-write-info-json",
        "--no-write-description",
        "--no-write-thumbnail",
        "--no-write-subs",
        "--no-write-auto-subs",
        "--no-write-comments",
        "--no-embed-metadata",
        "--no-embed-thumbnail",
        "--no-embed-chapters",
        "--no-embed-subs",
        "--no-embed-info-json",
        "--windows-filenames",
        "--encoding",
        "utf-8",
        "--newline",
        "--progress",
        "--no-simulate",
        "--progress-template",
        "download:WCP %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s",
        "--print",
        "after_move:WCF %(filepath)s",
        "--ffmpeg-location",
        ffmpeg_dir.as_str(),
        "--js-runtimes",
        js_runtime.as_str(),
        "-P",
        home.as_str(),
        "-P",
        temp.as_str(),
        "-o",
        // The video id would put the link back into the file name.
        "%(title).150B.%(ext)s",
    ];
    args.extend_from_slice(format_args(format));
    // The URL can never be read as an option.
    args.extend_from_slice(&["--", url]);

    let env = [
        ("PYTHONIOENCODING", "utf-8"),
        ("PYTHONUTF8", "1"),
        ("DENO_NO_UPDATE_CHECK", "1"),
        ("DENO_NO_PROMPT", "1"),
        ("DENO_NO_PACKAGE_JSON", "1"),
    ];
    let mut finished: Option<PathBuf> = None;
    let mut part = 1u32;
    let mut last = 0.0f64;
    let mut converting = false;
    let output = crate::process::run_with_env(&tools.yt_dlp, &args, &env, cancel, DOWNLOAD_TIMEOUT, |line| {
        if let Some((downloaded, total)) = parse_progress(line) {
            if downloaded + 1.0 < last {
                part += 1;
            }
            last = downloaded;
            if let Some(total) = total {
                progress(Stage::Downloading((downloaded / total).clamp(0.0, 1.0) as f32, part));
            }
        } else if let Some(path) = line.strip_prefix("WCF ") {
            finished = Some(PathBuf::from(path.trim()));
        } else if !converting && (line.starts_with("[Merger]") || line.starts_with("[ExtractAudio]")) {
            converting = true;
            progress(Stage::Converting);
        }
    })?;
    if output.code != 0 {
        let detail = output
            .text
            .lines()
            .rev()
            .find(|line| line.starts_with("ERROR:"))
            .or_else(|| output.text.lines().last())
            .unwrap_or_default()
            .to_owned();
        return Err(EngineError::Process { program: "yt-dlp".to_owned(), code: output.code, detail });
    }
    finished.filter(|path| path.is_file()).ok_or_else(|| EngineError::NotFound("the downloaded file".to_owned()))
}

/// The user's Downloads folder, or the profile when it cannot be resolved.
pub fn downloads_dir() -> PathBuf {
    use windows::Win32::UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    // SAFETY: The GUID is a static and no token is passed. The returned string
    // is CoTaskMem-allocated and freed below.
    let text = match unsafe { SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None) } {
        Ok(path) => {
            // SAFETY: On success `path` is a valid null-terminated string.
            let text = unsafe { path.to_string() }.unwrap_or_default();
            // SAFETY: Frees the allocation made by SHGetKnownFolderPath, exactly once.
            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(path.0.cast())) };
            text
        }
        Err(_) => String::new(),
    };
    if text.is_empty() {
        std::env::var_os("USERPROFILE").map_or_else(std::env::temp_dir, |profile| PathBuf::from(profile).join("Downloads"))
    } else {
        PathBuf::from(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_youtube_links_only() {
        for good in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtu.be/dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=abc",
            "https://music.youtube.com/watch?v=abc",
            "https://www.youtube.com/shorts/abc",
            "http://youtube.com/watch?v=abc",
            "  https://www.youtube-nocookie.com/embed/abc  ",
        ] {
            assert!(is_youtube_url(good), "{good}");
        }
        for bad in [
            "",
            "youtube.com/watch?v=abc",
            "https://www.youtube.com",
            "https://www.youtube.com/",
            "https://evil.example/youtube.com/watch",
            "https://youtube.com.evil.example/watch?v=a",
            "https://notyoutube.com/watch?v=a",
            "--exec calc https://youtu.be/a",
            "file:///C:/Windows/win.ini",
        ] {
            assert!(!is_youtube_url(bad), "{bad}");
        }
    }

    #[test]
    fn a_taken_name_gets_a_number_and_the_move_keeps_the_file() {
        let staging = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("Me at the zoo.mp3"), b"older").unwrap();
        std::fs::write(folder.path().join("Me at the zoo (2).mp3"), b"older").unwrap();
        let fresh = staging.path().join("Me at the zoo.mp3");
        std::fs::write(&fresh, b"new").unwrap();

        let moved = move_into(&fresh, folder.path()).unwrap();
        assert_eq!(moved, folder.path().join("Me at the zoo (3).mp3"));
        assert_eq!(std::fs::read(&moved).unwrap(), b"new");
        assert!(!fresh.exists());
        assert_eq!(std::fs::read(folder.path().join("Me at the zoo.mp3")).unwrap(), b"older");
        assert_eq!(free_path(folder.path(), Path::new("clip.mp4")), folder.path().join("clip.mp4"));
    }

    #[test]
    fn reads_progress_lines() {
        assert_eq!(parse_progress("WCP 512 1024 NA"), Some((512.0, Some(1024.0))));
        assert_eq!(parse_progress("WCP 512 NA 2048.5"), Some((512.0, Some(2048.5))));
        assert_eq!(parse_progress("WCP 512 NA NA"), Some((512.0, None)));
        assert_eq!(parse_progress("[download] 5%"), None);
    }

    /// Fetches the tools and one video for real. Run with
    /// `WINDOWCRAFT_LIVE_YOUTUBE=<link> cargo test -p windowcraft-engine live_youtube -- --ignored --nocapture`.
    #[test]
    #[ignore = "downloads about 160 MB of tools and a video"]
    fn live_youtube_download_has_no_metadata() {
        let url = std::env::var("WINDOWCRAFT_LIVE_YOUTUBE").expect("WINDOWCRAFT_LIVE_YOUTUBE");
        let cancel = CancelToken::new();
        let tools = crate::tools::install(crate::tools::Extension::YouTube, &cancel, |stage| {
            if let crate::tools::Stage::Unpacking(tool) | crate::tools::Stage::Retrying(tool, _) = stage {
                println!("{stage:?} {}", tool.name());
            }
        })
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        for format in [Format::Mp3, Format::Mp4] {
            let path = download_video(&tools, &url, format, dir.path(), &cancel, |_| {}).unwrap();
            assert_eq!(path.parent(), Some(dir.path()));
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let id = url.rsplit(['=', '/']).next().unwrap_or_default();
            assert!(id.is_empty() || !name.contains(id), "the video id is in the file name {name}");
            let probe = crate::process::run_ok(
                &tools.ffprobe(),
                &["-v", "error", "-show_entries", "format_tags:stream_tags", "-of", "default", &path.display().to_string()],
                &cancel,
                Duration::from_secs(60),
            )
            .unwrap();
            println!("{} -> {probe:?}", path.display());
            // The Xing/LAME header of a VBR MP3 names its encoder; bitexact
            // reduces that to "Lavf lame" without a version. Dropping the
            // header would break seeking, so only that one value may remain.
            // ffprobe also lists the fields every MP4 needs as tags: brands,
            // "und" for no language and the default handler names.
            const STRUCTURE: [&str; 6] = [
                "TAG:major_brand=",
                "TAG:minor_version=",
                "TAG:compatible_brands=",
                "TAG:language=und",
                "TAG:handler_name=VideoHandler",
                "TAG:handler_name=SoundHandler",
            ];
            let tags: Vec<&str> = probe
                .lines()
                .filter(|line| line.starts_with("TAG:"))
                .filter(|line| !(line.starts_with("TAG:encoder=") && !line.bytes().any(|byte| byte.is_ascii_digit())))
                .filter(|line| !STRUCTURE.iter().any(|field| line.starts_with(field)))
                .collect();
            assert!(tags.is_empty(), "tags left in {}: {tags:?}", path.display());
        }
        assert!(!staging_dir().exists());
    }

    #[test]
    fn rejects_other_sites_before_starting_anything() {
        let tools = Tools::in_dir(Path::new(r"C:\nonexistent"));
        let result =
            download_video(&tools, "https://example.com/video", Format::Mp4, Path::new(r"C:\nonexistent"), &CancelToken::new(), |_| {});
        assert!(matches!(result, Err(EngineError::Invalid(_))));
    }
}
