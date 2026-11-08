// Made by ycangignacy
//! Extensions and the outside tools they need.
//!
//! WindowCraft ships none of these tools, and nothing is installed by default.
//! Each extension is installed on its own from the app. Installing fetches the
//! tools it needs (yt-dlp, ffmpeg, Deno) from their official GitHub releases,
//! checks each against the SHA-256 the release publishes, and keeps them in
//! WindowCraft's data folder, which uninstall removes. Removing an extension
//! deletes every tool no other installed extension still uses.

use crate::download::{block_on, download, download_accepting, fetch_text, fetch_text_accepting};
use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// yt-dlp follows YouTube's changes closely; an old copy soon stops working.
const YT_DLP_MAX_AGE: Duration = Duration::from_secs(14 * 24 * 3600);
const UNPACK_TIMEOUT: Duration = Duration::from_secs(600);
/// GitHub's release CDN sometimes answers 502 or 504 for a moment.
const ATTEMPTS: u32 = 4;
const FIRST_RETRY_WAIT: Duration = Duration::from_secs(2);
const MARKERS: &str = "extensions.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    YtDlp,
    Ffmpeg,
    Deno,
}

impl Tool {
    pub const ALL: [Self; 3] = [Self::YtDlp, Self::Ffmpeg, Self::Deno];

    pub fn name(self) -> &'static str {
        match self {
            Self::YtDlp => "yt-dlp",
            Self::Ffmpeg => "ffmpeg",
            Self::Deno => "Deno",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Extension {
    Stripper,
    Converter,
    /// ffmpeg for the formats WindowCraft cannot handle itself.
    Ffmpeg,
    YouTube,
}

impl Extension {
    pub const ALL: [Self; 4] = [Self::Stripper, Self::Converter, Self::Ffmpeg, Self::YouTube];

    pub fn id(self) -> &'static str {
        match self {
            Self::Stripper => "metadata-stripper",
            Self::Converter => "file-converter",
            Self::Ffmpeg => "more-formats",
            Self::YouTube => "youtube-downloader",
        }
    }

    fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|extension| extension.id() == id)
    }

    /// Tools fetched when the extension is installed. The stripper and the
    /// converter are built into WindowCraft; they use ffmpeg only for rare
    /// formats, and only when another extension brought it.
    pub fn tools(self) -> &'static [Tool] {
        match self {
            Self::Stripper | Self::Converter => &[],
            Self::Ffmpeg => &[Tool::Ffmpeg],
            Self::YouTube => &[Tool::YtDlp, Tool::Ffmpeg, Tool::Deno],
        }
    }
}

/// Where a tool download stands, for the progress line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stage {
    /// Fetching a tool: bytes received and the total, when known.
    Fetching(Tool, u64, Option<u64>),
    Unpacking(Tool),
    /// GitHub failed; another attempt starts after a short wait.
    Retrying(Tool, u32),
}

pub fn tools_dir() -> PathBuf {
    crate::win::local_app_dir().join("tools")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    pub yt_dlp: PathBuf,
    pub ffmpeg_dir: PathBuf,
    pub deno: PathBuf,
}

impl Tools {
    pub fn in_dir(dir: &Path) -> Self {
        Self { yt_dlp: dir.join("yt-dlp.exe"), ffmpeg_dir: dir.join("ffmpeg"), deno: dir.join("deno.exe") }
    }

    pub fn ffmpeg(&self) -> PathBuf {
        self.ffmpeg_dir.join("ffmpeg.exe")
    }

    pub fn ffprobe(&self) -> PathBuf {
        self.ffmpeg_dir.join("ffprobe.exe")
    }

    pub fn has(&self, tool: Tool) -> bool {
        match tool {
            Tool::YtDlp => self.yt_dlp.is_file(),
            Tool::Ffmpeg => self.ffmpeg().is_file() && self.ffprobe().is_file(),
            Tool::Deno => self.deno.is_file(),
        }
    }

    fn remove(&self, tool: Tool) -> Result<()> {
        let result = match tool {
            Tool::YtDlp => std::fs::remove_file(&self.yt_dlp),
            Tool::Ffmpeg => std::fs::remove_dir_all(&self.ffmpeg_dir),
            Tool::Deno => std::fs::remove_file(&self.deno),
        };
        match result {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(EngineError::io(tool.name(), error))
            }
            _ => Ok(()),
        }
    }
}

/// The ffmpeg WindowCraft fetched earlier, if any. The metadata stripper uses
/// it for video and for audio other than MP3 and FLAC.
pub fn installed_ffmpeg() -> Option<PathBuf> {
    Some(Tools::in_dir(&tools_dir()).ffmpeg()).filter(|path| path.is_file())
}

fn read_markers(dir: &Path) -> Vec<Extension> {
    let text = std::fs::read_to_string(dir.join(MARKERS)).unwrap_or_default();
    let mut found: Vec<Extension> = text.lines().filter_map(|line| Extension::from_id(line.trim())).collect();
    found.sort();
    found.dedup();
    found
}

fn write_markers(dir: &Path, extensions: &[Extension]) -> Result<()> {
    let path = dir.join(MARKERS);
    if extensions.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(EngineError::io(path.display().to_string(), error)),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(dir).map_err(io_error(dir))?;
    let text: String = extensions.iter().map(|extension| format!("{}\n", extension.id())).collect();
    std::fs::write(&path, text).map_err(io_error(&path))
}

/// Extensions the user installed whose tools are all still in place.
pub fn installed() -> Vec<Extension> {
    installed_in(&tools_dir())
}

fn installed_in(dir: &Path) -> Vec<Extension> {
    let tools = Tools::in_dir(dir);
    read_markers(dir).into_iter().filter(|extension| extension.tools().iter().all(|tool| tools.has(*tool))).collect()
}

/// Fetches what `extension` needs and marks it installed.
pub fn install(extension: Extension, cancel: &CancelToken, progress: impl FnMut(Stage)) -> Result<Tools> {
    let dir = tools_dir();
    let tools = ensure(&dir, extension.tools(), cancel, progress)?;
    let mut markers = read_markers(&dir);
    if !markers.contains(&extension) {
        markers.push(extension);
        markers.sort();
    }
    write_markers(&dir, &markers)?;
    Ok(tools)
}

/// Unmarks `extension` and deletes the tools nothing else needs. The folder
/// goes too once no extension is left.
pub fn uninstall(extension: Extension) -> Result<()> {
    uninstall_in(&tools_dir(), extension)
}

fn uninstall_in(dir: &Path, extension: Extension) -> Result<()> {
    let remaining: Vec<Extension> = read_markers(dir).into_iter().filter(|other| *other != extension).collect();
    write_markers(dir, &remaining)?;
    let tools = Tools::in_dir(dir);
    for tool in Tool::ALL {
        if !remaining.iter().any(|other| other.tools().contains(&tool)) {
            tools.remove(tool)?;
        }
    }
    if remaining.is_empty() {
        match std::fs::remove_dir_all(dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(EngineError::io(dir.display().to_string(), error));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Makes sure the tools of an installed extension are in place before use,
/// and refreshes a yt-dlp older than two weeks.
pub fn ready(extension: Extension, cancel: &CancelToken, progress: impl FnMut(Stage)) -> Result<Tools> {
    ensure(&tools_dir(), extension.tools(), cancel, progress)
}

struct Source {
    tool: Tool,
    /// `owner/name` on GitHub.
    repo: &'static str,
    /// A fixed tag, or the latest release when empty.
    tag: &'static str,
    url: &'static str,
    sums: &'static str,
    asset: &'static str,
    sums_asset: &'static str,
}

fn source(tool: Tool) -> Source {
    let arm = cfg!(target_arch = "aarch64");
    match tool {
        Tool::YtDlp => Source {
            tool,
            repo: "yt-dlp/yt-dlp",
            tag: "",
            sums_asset: "SHA2-256SUMS",
            url: if arm {
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_arm64.exe"
            } else {
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe"
            },
            sums: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS",
            asset: if arm { "yt-dlp_arm64.exe" } else { "yt-dlp.exe" },
        },
        Tool::Ffmpeg => Source {
            tool,
            repo: "yt-dlp/FFmpeg-Builds",
            tag: "latest",
            sums_asset: "checksums.sha256",
            url: if arm {
                "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-winarm64-gpl-shared.zip"
            } else {
                "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip"
            },
            sums: "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest/checksums.sha256",
            asset: if arm { "ffmpeg-master-latest-winarm64-gpl-shared.zip" } else { "ffmpeg-master-latest-win64-gpl-shared.zip" },
        },
        Tool::Deno => Source {
            tool,
            repo: "denoland/deno",
            tag: "",
            sums_asset: if arm { "deno-aarch64-pc-windows-msvc.zip.sha256sum" } else { "deno-x86_64-pc-windows-msvc.zip.sha256sum" },
            url: if arm {
                "https://github.com/denoland/deno/releases/latest/download/deno-aarch64-pc-windows-msvc.zip"
            } else {
                "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip"
            },
            sums: if arm {
                "https://github.com/denoland/deno/releases/latest/download/deno-aarch64-pc-windows-msvc.zip.sha256sum"
            } else {
                "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip.sha256sum"
            },
            asset: if arm { "deno-aarch64-pc-windows-msvc.zip" } else { "deno-x86_64-pc-windows-msvc.zip" },
        },
    }
}

fn is_hash(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Finds the SHA-256 of `asset` in a checksum file. yt-dlp and ffmpeg list
/// `<hash>  <name>` per line; Deno's file holds a single hash in
/// `Get-FileHash` layout.
fn find_hash(sums: &str, asset: &str) -> Option<String> {
    let named = sums.lines().find_map(|line| {
        let mut tokens = line.split_whitespace();
        let hash = tokens.next().filter(|token| is_hash(token))?;
        tokens.any(|name| name.trim_start_matches('*') == asset).then(|| hash.to_ascii_lowercase())
    });
    named.or_else(|| {
        let hashes: Vec<&str> =
            sums.split(|c: char| !c.is_ascii_hexdigit()).filter(|token| is_hash(token)).collect();
        (hashes.len() == 1).then(|| hashes[0].to_ascii_lowercase())
    })
}

fn stale(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > YT_DLP_MAX_AGE)
}

pub(crate) fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> EngineError + '_ {
    move |error| EngineError::io(path.display().to_string(), error)
}

fn ensure(dir: &Path, needed: &[Tool], cancel: &CancelToken, mut progress: impl FnMut(Stage)) -> Result<Tools> {
    let tools = Tools::in_dir(dir);
    if needed.is_empty() {
        return Ok(tools);
    }
    std::fs::create_dir_all(dir).map_err(io_error(dir))?;
    for tool in needed {
        let refresh = *tool == Tool::YtDlp && tools.has(Tool::YtDlp) && stale(&tools.yt_dlp);
        if tools.has(*tool) && !refresh {
            continue;
        }
        let fetched = fetch_tool(&source(*tool), dir, &tools, cancel, &mut progress);
        match fetched {
            // A stale yt-dlp still beats none when GitHub cannot be reached.
            Err(EngineError::Network(_)) if refresh => {}
            other => other?,
        }
    }
    Ok(tools)
}

/// Server errors and dropped connections are worth another try; a 404 or a
/// checksum mismatch is not.
fn transient(error: &EngineError) -> bool {
    matches!(error, EngineError::Network(detail) if !detail.contains("client error"))
}

/// Waits in short steps so Cancel still answers at once.
fn pause(wait: Duration, cancel: &CancelToken) -> Result<()> {
    let until = std::time::Instant::now() + wait;
    while std::time::Instant::now() < until {
        cancel.check()?;
        std::thread::sleep(Duration::from_millis(100));
    }
    cancel.check()
}

fn with_retries<T>(
    tool: Tool,
    cancel: &CancelToken,
    progress: &mut impl FnMut(Stage),
    wait: Duration,
    mut attempt: impl FnMut(&mut dyn FnMut(Stage)) -> Result<T>,
) -> Result<T> {
    let mut wait = wait;
    let mut number = 1;
    loop {
        match attempt(progress) {
            Err(error) if number < ATTEMPTS && transient(&error) => {
                number += 1;
                progress(Stage::Retrying(tool, number));
                pause(wait, cancel)?;
                wait *= 3;
            }
            other => return other,
        }
    }
}

fn fetch_tool(
    source: &Source,
    dir: &Path,
    tools: &Tools,
    cancel: &CancelToken,
    progress: &mut impl FnMut(Stage),
) -> Result<()> {
    let tool = source.tool;
    let file = dir.join(source.asset).with_extension("download");
    with_retries(tool, cancel, progress, FIRST_RETRY_WAIT, |progress| {
        // The release download links on github.com can fail for hours while
        // the API still serves the same files, so the API goes first.
        match fetch_through_api(source, &file, cancel, progress) {
            Ok(()) => Ok(()),
            Err(EngineError::Cancelled) => Err(EngineError::Cancelled),
            Err(EngineError::Checksum { file, expected, actual }) => Err(EngineError::Checksum { file, expected, actual }),
            Err(_) => fetch_directly(source, &file, cancel, progress),
        }
    })?;
    cancel.check()?;
    let result = match tool {
        Tool::YtDlp => std::fs::rename(&file, &tools.yt_dlp).map_err(io_error(&tools.yt_dlp)),
        Tool::Deno => {
            progress(Stage::Unpacking(tool));
            unpack(&file, dir, &["deno.exe"], cancel)
        }
        Tool::Ffmpeg => {
            progress(Stage::Unpacking(tool));
            unpack_ffmpeg(&file, dir, &tools.ffmpeg_dir, cancel)
        }
    };
    let _ = std::fs::remove_file(&file);
    result
}

const OCTET: Option<&str> = Some("application/octet-stream");

/// An asset of a GitHub release, as the REST API lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Asset {
    api_url: String,
    /// The SHA-256 GitHub computed when the file was uploaded.
    sha256: Option<String>,
}

fn find_asset(release_json: &str, name: &str) -> Option<Asset> {
    let release: serde_json::Value = serde_json::from_str(release_json).ok()?;
    let asset = release.get("assets")?.as_array()?.iter().find(|asset| asset.get("name").and_then(serde_json::Value::as_str) == Some(name))?;
    Some(Asset {
        api_url: asset.get("url")?.as_str()?.to_owned(),
        sha256: asset
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .filter(|hash| is_hash(hash))
            .map(str::to_ascii_lowercase),
    })
}

fn fetch_through_api(source: &Source, file: &Path, cancel: &CancelToken, progress: &mut dyn FnMut(Stage)) -> Result<()> {
    let tool = source.tool;
    let release_url = if source.tag.is_empty() {
        format!("https://api.github.com/repos/{}/releases/latest", source.repo)
    } else {
        format!("https://api.github.com/repos/{}/releases/tags/{}", source.repo, source.tag)
    };
    let release = block_on(fetch_text_accepting(&release_url, Some("application/vnd.github+json")))??;
    let asset = find_asset(&release, source.asset).ok_or_else(|| EngineError::NotFound(source.asset.to_owned()))?;
    let hash = match asset.sha256 {
        Some(hash) => hash,
        None => {
            let sums = find_asset(&release, source.sums_asset).ok_or_else(|| EngineError::NotFound(source.sums_asset.to_owned()))?;
            let text = block_on(fetch_text_accepting(&sums.api_url, OCTET))??;
            find_hash(&text, source.asset).ok_or_else(|| EngineError::NotFound(format!("SHA-256 of {}", source.asset)))?
        }
    };
    block_on(download_accepting(&asset.api_url, OCTET, file, Some(&hash), cancel, |received, total| {
        progress(Stage::Fetching(tool, received, total));
    }))?
}

fn fetch_directly(source: &Source, file: &Path, cancel: &CancelToken, progress: &mut dyn FnMut(Stage)) -> Result<()> {
    let tool = source.tool;
    let sums = block_on(fetch_text(source.sums))??;
    let hash = find_hash(&sums, source.asset)
        .ok_or_else(|| EngineError::NotFound(format!("SHA-256 of {} in {}", source.asset, source.sums)))?;
    block_on(download(source.url, file, Some(&hash), cancel, |received, total| {
        progress(Stage::Fetching(tool, received, total));
    }))?
}

/// Unpacks with the `tar.exe` that ships with Windows, so WindowCraft needs no
/// zip code of its own.
fn unpack(archive: &Path, into: &Path, members: &[&str], cancel: &CancelToken) -> Result<()> {
    let tar = crate::win::system32("tar.exe");
    let archive_text = archive.display().to_string();
    let into_text = into.display().to_string();
    let mut args = vec!["-xf", archive_text.as_str(), "-C", into_text.as_str()];
    args.extend_from_slice(members);
    crate::process::run_ok(&tar, &args, cancel, UNPACK_TIMEOUT).map(|_| ())
}

/// Keeps `ffmpeg.exe`, `ffprobe.exe` and their DLLs. `ffplay.exe`, the
/// headers, the docs and the import libraries are dropped.
fn unpack_ffmpeg(archive: &Path, dir: &Path, target: &Path, cancel: &CancelToken) -> Result<()> {
    let staging = dir.join("ffmpeg-unpack");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(io_error(&staging))?;
    let result = (|| {
        unpack(archive, &staging, &[], cancel)?;
        let bin = std::fs::read_dir(&staging)
            .map_err(io_error(&staging))?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path().join("bin"))
            .find(|bin| bin.join("ffmpeg.exe").is_file())
            .ok_or_else(|| EngineError::NotFound("ffmpeg.exe in the archive".to_owned()))?;
        let _ = std::fs::remove_dir_all(target);
        std::fs::create_dir_all(target).map_err(io_error(target))?;
        for entry in std::fs::read_dir(&bin).map_err(io_error(&bin))?.filter_map(std::result::Result::ok) {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if name == "ffmpeg.exe" || name == "ffprobe.exe" || name.ends_with(".dll") {
                let destination = target.join(entry.file_name());
                std::fs::rename(entry.path(), &destination).map_err(io_error(&destination))?;
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn finds_hashes_in_every_checksum_layout() {
        let ytdlp = "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d  yt-dlp.exe\n\
                     0066dbac3c124028e2dcae2a197fccee33cd91f456313d24f7af89fd54590e98  yt-dlp_arm64.exe\n";
        assert_eq!(
            find_hash(ytdlp, "yt-dlp_arm64.exe").as_deref(),
            Some("0066dbac3c124028e2dcae2a197fccee33cd91f456313d24f7af89fd54590e98")
        );
        assert!(find_hash(ytdlp, "yt-dlp_x86.exe").is_none());
        let deno = "Algorithm : SHA256\r\nHash      : CBA84B26E0DF5AE7F74FC66B86A95B866872A86292EBD84FA1E959927EDD93BF\r\nPath      : C:\\a\\deno.zip\r\n";
        assert_eq!(
            find_hash(deno, "deno-x86_64-pc-windows-msvc.zip").as_deref(),
            Some("cba84b26e0df5ae7f74fc66b86a95b866872a86292ebd84fa1e959927edd93bf")
        );
    }

    #[test]
    fn reads_assets_and_digests_from_the_release_api() {
        let release = r#"{"tag_name":"2026.10.26","assets":[
            {"name":"SHA2-256SUMS","url":"https://api.github.com/repos/yt-dlp/yt-dlp/releases/assets/679669608","digest":null},
            {"name":"yt-dlp.exe","url":"https://api.github.com/repos/yt-dlp/yt-dlp/releases/assets/679669633","digest":"sha256:F5389D3698ECDE4EF06F4E6B1CE0294FE9AC1396228C8990B8D3416D6D177A7D"}]}"#;
        let exe = find_asset(release, "yt-dlp.exe").unwrap();
        assert_eq!(exe.api_url, "https://api.github.com/repos/yt-dlp/yt-dlp/releases/assets/679669633");
        assert_eq!(exe.sha256.as_deref(), Some("f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d"));
        assert_eq!(find_asset(release, "SHA2-256SUMS").unwrap().sha256, None);
        assert!(find_asset(release, "yt-dlp_x86.exe").is_none());
        assert!(find_asset("<html>504</html>", "yt-dlp.exe").is_none());
    }

    #[test]
    fn server_errors_are_retried_and_client_errors_are_not() {
        let gateway = "HTTP status server error (504 Gateway Timeout) for url (https://github.com/x)";
        let calls = Cell::new(0);
        let mut stages = Vec::new();
        let result = with_retries(Tool::YtDlp, &CancelToken::new(), &mut |stage| stages.push(stage), Duration::ZERO, |_| {
            calls.set(calls.get() + 1);
            if calls.get() < 3 { Err(EngineError::Network(gateway.to_owned())) } else { Ok(7) }
        });
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.get(), 3);
        assert_eq!(stages, vec![Stage::Retrying(Tool::YtDlp, 2), Stage::Retrying(Tool::YtDlp, 3)]);

        calls.set(0);
        let result: Result<()> = with_retries(Tool::Deno, &CancelToken::new(), &mut |_| {}, Duration::ZERO, |_| {
            calls.set(calls.get() + 1);
            Err(EngineError::Network(gateway.to_owned()))
        });
        assert!(matches!(result, Err(EngineError::Network(_))));
        assert_eq!(calls.get(), ATTEMPTS);

        calls.set(0);
        let result: Result<()> = with_retries(Tool::Deno, &CancelToken::new(), &mut |_| {}, Duration::ZERO, |_| {
            calls.set(calls.get() + 1);
            Err(EngineError::Network("HTTP status client error (404 Not Found) for url (x)".to_owned()))
        });
        assert!(result.is_err());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn cancel_stops_the_wait_between_attempts() {
        let cancel = CancelToken::new();
        cancel.cancel();
        let result: Result<()> = with_retries(Tool::Ffmpeg, &cancel, &mut |_| {}, Duration::from_secs(30), |_| {
            Err(EngineError::Network("HTTP status server error (502 Bad Gateway)".to_owned()))
        });
        assert!(matches!(result, Err(EngineError::Cancelled)));
    }

    fn fake_tools(dir: &Path) {
        let tools = Tools::in_dir(dir);
        std::fs::create_dir_all(&tools.ffmpeg_dir).unwrap();
        for file in [tools.yt_dlp.clone(), tools.deno.clone(), tools.ffmpeg(), tools.ffprobe(), tools.ffmpeg_dir.join("avcodec.dll")] {
            std::fs::write(file, b"x").unwrap();
        }
    }

    #[test]
    fn nothing_is_installed_until_marked() {
        let dir = tempfile::tempdir().unwrap();
        fake_tools(dir.path());
        assert!(installed_in(dir.path()).is_empty());
        write_markers(dir.path(), &[Extension::YouTube, Extension::Stripper]).unwrap();
        assert_eq!(installed_in(dir.path()), vec![Extension::Stripper, Extension::YouTube]);
        // A marked extension whose tool went missing counts as not installed.
        std::fs::remove_file(Tools::in_dir(dir.path()).deno).unwrap();
        assert_eq!(installed_in(dir.path()), vec![Extension::Stripper]);
    }

    #[test]
    fn uninstalling_keeps_tools_another_extension_needs() {
        let dir = tempfile::tempdir().unwrap();
        let tools = Tools::in_dir(dir.path());
        fake_tools(dir.path());
        write_markers(dir.path(), &[Extension::Ffmpeg, Extension::YouTube]).unwrap();

        uninstall_in(dir.path(), Extension::YouTube).unwrap();
        assert!(!tools.yt_dlp.exists() && !tools.deno.exists());
        assert!(tools.has(Tool::Ffmpeg), "the extra formats still need ffmpeg");
        assert_eq!(installed_in(dir.path()), vec![Extension::Ffmpeg]);

        uninstall_in(dir.path(), Extension::Ffmpeg).unwrap();
        assert!(!dir.path().exists(), "the folder goes with the last extension");
    }

    #[test]
    fn the_stripper_and_converter_need_no_download() {
        let dir = tempfile::tempdir().unwrap();
        for extension in [Extension::Stripper, Extension::Converter] {
            let tools = ensure(dir.path(), extension.tools(), &CancelToken::new(), |_| panic!("nothing to fetch")).unwrap();
            assert!(!tools.has(Tool::Ffmpeg));
        }
    }
}
