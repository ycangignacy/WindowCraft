// Made by WindowCraft Dev Team
//! Converts pictures, video and audio between formats with ffmpeg.
//!
//! Everything happens on this machine: ffmpeg reads the source and writes the
//! result next to it, and nothing is uploaded. The source is never changed.
//! When the target container can hold the source streams as they are, they
//! are copied without re-encoding, which is fast and loses nothing; otherwise
//! they are encoded with settings that play almost anywhere.

use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use crate::tools::Tools;
use std::path::{Path, PathBuf};
use std::time::Duration;

const CONVERT_TIMEOUT: Duration = Duration::from_secs(12 * 3600);
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// Every extension the file dialog offers. ffmpeg reads many more; these are
/// the ones people have.
pub const INPUT_PATTERN: &str = "*.jpg;*.jpeg;*.jfif;*.png;*.webp;*.avif;*.heic;*.heif;*.bmp;*.tif;*.tiff;*.gif;*.ico;*.tga;*.jxl;*.jp2;*.dds;*.qoi;*.ppm;*.pgm;*.pbm;*.pcx;*.exr;*.hdr;*.psd;*.svg;\
*.mp4;*.m4v;*.mkv;*.webm;*.mov;*.avi;*.wmv;*.flv;*.mpg;*.mpeg;*.ts;*.mts;*.m2ts;*.vob;*.3gp;*.3g2;*.ogv;*.asf;*.f4v;*.divx;*.mxf;\
*.mp3;*.m4a;*.aac;*.flac;*.wav;*.ogg;*.oga;*.opus;*.wma;*.aif;*.aiff;*.alac;*.ape;*.wv;*.ac3;*.eac3;*.dts;*.amr;*.mka;*.caf;*.au;*.mid";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Png,
    Jpg,
    Webp,
    Avif,
    Bmp,
    Tiff,
    Gif,
    Ico,
    Mp4,
    Mkv,
    Webm,
    Mov,
    Avi,
    Mp3,
    M4a,
    Flac,
    Wav,
    Ogg,
    Opus,
    Aiff,
    Wma,
}

impl Target {
    pub const ALL: [Self; 21] = [
        Self::Png,
        Self::Jpg,
        Self::Webp,
        Self::Avif,
        Self::Bmp,
        Self::Tiff,
        Self::Gif,
        Self::Ico,
        Self::Mp4,
        Self::Mkv,
        Self::Webm,
        Self::Mov,
        Self::Avi,
        Self::Mp3,
        Self::M4a,
        Self::Flac,
        Self::Wav,
        Self::Ogg,
        Self::Opus,
        Self::Aiff,
        Self::Wma,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpg => "jpg",
            Self::Webp => "webp",
            Self::Avif => "avif",
            Self::Bmp => "bmp",
            Self::Tiff => "tif",
            Self::Gif => "gif",
            Self::Ico => "ico",
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
            Self::Mov => "mov",
            Self::Avi => "avi",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Ogg => "ogg",
            Self::Opus => "opus",
            Self::Aiff => "aiff",
            Self::Wma => "wma",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpg => "JPG",
            Self::Webp => "WebP",
            Self::Avif => "AVIF",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
            Self::Gif => "GIF",
            Self::Ico => "ICO",
            Self::Mp4 => "MP4",
            Self::Mkv => "MKV",
            Self::Webm => "WebM",
            Self::Mov => "MOV",
            Self::Avi => "AVI",
            Self::Mp3 => "MP3",
            Self::M4a => "M4A",
            Self::Flac => "FLAC",
            Self::Wav => "WAV",
            Self::Ogg => "OGG",
            Self::Opus => "Opus",
            Self::Aiff => "AIFF",
            Self::Wma => "WMA",
        }
    }

    pub fn family(self) -> Family {
        match self {
            Self::Png | Self::Jpg | Self::Webp | Self::Avif | Self::Bmp | Self::Tiff | Self::Gif | Self::Ico => Family::Image,
            Self::Mp4 | Self::Mkv | Self::Webm | Self::Mov | Self::Avi => Family::Video,
            _ => Family::Audio,
        }
    }
}

/// What ffprobe found in a source file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Probe {
    /// A single picture rather than a clip.
    pub still: bool,
    /// Codec of the first picture stream, cover art included.
    pub picture: Option<String>,
    /// Codec of the first video stream that is not cover art.
    pub video: Option<String>,
    pub audio: Vec<String>,
    pub audio_channels: u32,
    pub audio_bits: u32,
    pub subtitles: bool,
    pub duration: Option<f64>,
}

/// Why a file cannot become the chosen format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mismatch {
    NoPicture,
    NoVideo,
    NoAudio,
}

impl Probe {
    fn parse(json: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let format = value.get("format")?;
        let format_name = format.get("format_name").and_then(serde_json::Value::as_str).unwrap_or_default();
        let mut probe = Self {
            still: format_name == "image2" || format_name.ends_with("_pipe") || format_name == "ico",
            duration: format
                .get("duration")
                .and_then(serde_json::Value::as_str)
                .and_then(|text| text.parse::<f64>().ok())
                .filter(|seconds| seconds.is_finite() && *seconds > 0.0),
            ..Self::default()
        };
        for stream in value.get("streams")?.as_array()? {
            let kind = stream.get("codec_type").and_then(serde_json::Value::as_str).unwrap_or_default();
            let codec = stream.get("codec_name").and_then(serde_json::Value::as_str).unwrap_or("unknown").to_owned();
            let cover = stream
                .get("disposition")
                .and_then(|disposition| disposition.get("attached_pic"))
                .and_then(serde_json::Value::as_i64)
                == Some(1);
            match kind {
                "video" => {
                    probe.picture.get_or_insert_with(|| codec.clone());
                    if !cover && probe.video.is_none() {
                        probe.video = Some(codec);
                    }
                }
                "audio" => {
                    if probe.audio.is_empty() {
                        probe.audio_channels =
                            stream.get("channels").and_then(serde_json::Value::as_u64).unwrap_or(2) as u32;
                        probe.audio_bits = stream
                            .get("bits_per_raw_sample")
                            .and_then(serde_json::Value::as_str)
                            .and_then(|text| text.parse().ok())
                            .unwrap_or(0);
                    }
                    probe.audio.push(codec);
                }
                "subtitle" => probe.subtitles = true,
                _ => {}
            }
        }
        Some(probe)
    }

    pub fn mismatch(&self, target: Target) -> Option<Mismatch> {
        match target.family() {
            Family::Image if self.picture.is_none() => Some(Mismatch::NoPicture),
            Family::Video if self.video.is_none() || self.still => Some(Mismatch::NoVideo),
            Family::Audio if self.audio.is_empty() => Some(Mismatch::NoAudio),
            _ => None,
        }
    }
}

pub fn probe(tools: &Tools, source: &Path, cancel: &CancelToken) -> Result<Probe> {
    let source_text = source.display().to_string();
    let output = crate::process::run(
        &tools.ffprobe(),
        &[
            "-v",
            "quiet",
            "-print_format",
            "json=compact=1",
            "-show_entries",
            "format=format_name,duration:stream=codec_type,codec_name,channels,bits_per_raw_sample:stream_disposition=attached_pic",
            "--",
            &source_text,
        ],
        cancel,
        PROBE_TIMEOUT,
        |_| {},
    )?;
    if output.code != 0 {
        return Err(EngineError::Unsupported(name_of(source)));
    }
    Probe::parse(&output.text).ok_or_else(|| EngineError::Unsupported(name_of(source)))
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

/// Even width and height, which H.264 in 4:2:0 needs.
const EVEN: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";
const GIF_FILTER: &str =
    "fps=15,scale='min(720,iw)':-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4";
const GIF_STILL_FILTER: &str = "split[a][b];[a]palettegen[p];[b][p]paletteuse";
const ANIMATION_FILTER: &str = "fps=15,scale='min(720,iw)':-1:flags=lanczos";
const ICO_FILTER: &str = "scale='min(256,iw)':'min(256,ih)':force_original_aspect_ratio=decrease";

fn h264() -> Vec<String> {
    strings(&["-c:v", "libx264", "-preset", "medium", "-crf", "20", "-pix_fmt", "yuv420p", "-vf", EVEN])
}

fn image_args(target: Target, probe: &Probe) -> Vec<Vec<String>> {
    let animate = !probe.still && probe.video.is_some();
    let mut base = strings(&["-map", "0:v:0", "-an", "-sn", "-dn"]);
    let single = |mut args: Vec<String>, update: bool| {
        args.extend(strings(&["-frames:v", "1"]));
        if update {
            args.extend(strings(&["-update", "1"]));
        }
        args
    };
    match target {
        Target::Png => vec![single([base, strings(&["-c:v", "png"])].concat(), true)],
        Target::Jpg => vec![single([base, strings(&["-c:v", "mjpeg", "-q:v", "2"])].concat(), true)],
        Target::Bmp => vec![single([base, strings(&["-c:v", "bmp"])].concat(), true)],
        Target::Tiff => vec![single([base, strings(&["-c:v", "tiff", "-compression_algo", "deflate"])].concat(), true)],
        Target::Ico => vec![single([base, strings(&["-vf", ICO_FILTER, "-c:v", "png", "-pix_fmt", "rgba"])].concat(), false)],
        Target::Avif => vec![
            single([base.clone(), strings(&["-c:v", "libaom-av1", "-still-picture", "1", "-crf", "26", "-cpu-used", "6"])].concat(), false),
            single([base, strings(&["-c:v", "libaom-av1", "-crf", "26", "-cpu-used", "6"])].concat(), false),
        ],
        Target::Webp if animate => {
            base.extend(strings(&["-vf", ANIMATION_FILTER, "-c:v", "libwebp_anim", "-quality", "80", "-loop", "0"]));
            vec![base]
        }
        Target::Webp => vec![single([base, strings(&["-c:v", "libwebp", "-quality", "90"])].concat(), false)],
        Target::Gif if animate => {
            base.extend(strings(&["-vf", GIF_FILTER, "-loop", "0"]));
            vec![base]
        }
        Target::Gif => vec![single([base, strings(&["-vf", GIF_STILL_FILTER])].concat(), false)],
        _ => Vec::new(),
    }
}

fn all_in(codecs: &[String], allowed: &[&str]) -> bool {
    codecs.iter().all(|codec| allowed.contains(&codec.as_str()))
}

fn video_args(target: Target, probe: &Probe) -> Vec<Vec<String>> {
    let video = probe.video.clone().unwrap_or_default();
    let (video_ok, audio_ok, subtitles): (&[&str], &[&str], Option<&str>) = match target {
        Target::Mp4 => (&["h264", "hevc", "av1", "mpeg4"], &["aac", "mp3", "ac3", "eac3", "alac"], Some("mov_text")),
        Target::Mov => (&["h264", "hevc", "mpeg4", "prores", "mjpeg"], &["aac", "mp3", "alac", "pcm_s16le", "pcm_s24le"], Some("mov_text")),
        Target::Webm => (&["vp8", "vp9", "av1"], &["opus", "vorbis"], None),
        Target::Mkv => (&[], &[], Some("copy")),
        _ => (&[], &[], None),
    };
    let anything = target == Target::Mkv;
    let encode_video = match target {
        Target::Webm => strings(&["-c:v", "libvpx-vp9", "-crf", "32", "-b:v", "0", "-row-mt", "1", "-deadline", "good", "-cpu-used", "4"]),
        Target::Avi => strings(&["-c:v", "mpeg4", "-q:v", "3", "-tag:v", "XVID"]),
        _ => h264(),
    };
    let mut encode_audio = match target {
        Target::Webm => strings(&["-c:a", "libopus", "-b:a", "128k"]),
        Target::Avi => strings(&["-c:a", "libmp3lame", "-q:a", "2"]),
        _ => strings(&["-c:a", "aac", "-b:a", "192k"]),
    };
    if target == Target::Webm && probe.audio_channels > 2 {
        encode_audio.extend(strings(&["-ac", "2"]));
    }
    let tail = |mut args: Vec<String>| {
        if matches!(target, Target::Mp4 | Target::Mov) {
            args.extend(strings(&["-movflags", "+faststart"]));
        }
        args
    };
    let mut attempts = Vec::new();

    let copy_video = anything || video_ok.contains(&video.as_str());
    let copy_audio = anything || all_in(&probe.audio, audio_ok);
    if target != Target::Avi && (copy_video || copy_audio) {
        let mut args = strings(&["-map", "0:V:0", "-map", "0:a?", "-dn"]);
        if let Some(codec) = subtitles.filter(|_| probe.subtitles) {
            args.extend(strings(&["-map", "0:s?", "-c:s", codec]));
        }
        args.extend(if copy_video { strings(&["-c:v", "copy"]) } else { encode_video.clone() });
        args.extend(if copy_audio { strings(&["-c:a", "copy"]) } else { encode_audio.clone() });
        attempts.push(tail(args));
    }
    // Encoding both, without subtitles, works for nearly any source.
    let mut args = strings(&["-map", "0:V:0", "-map", "0:a?", "-sn", "-dn"]);
    args.extend(encode_video);
    args.extend(encode_audio);
    attempts.push(tail(args));
    attempts
}

fn audio_args(target: Target, probe: &Probe) -> Vec<Vec<String>> {
    let codec = probe.audio.first().map(String::as_str).unwrap_or_default();
    let deep = probe.audio_bits > 16;
    let (copy_from, encode): (&[&str], Vec<String>) = match target {
        Target::Mp3 => (&["mp3"], strings(&["-c:a", "libmp3lame", "-q:a", "0"])),
        Target::M4a => (&["aac", "alac"], strings(&["-c:a", "aac", "-b:a", "256k"])),
        Target::Flac => (&["flac"], strings(&["-c:a", "flac"])),
        Target::Wav => (
            &["pcm_s16le", "pcm_s24le", "pcm_f32le"],
            strings(&["-c:a", if deep { "pcm_s24le" } else { "pcm_s16le" }]),
        ),
        Target::Aiff => (&["pcm_s16be", "pcm_s24be"], strings(&["-c:a", if deep { "pcm_s24be" } else { "pcm_s16be" }])),
        Target::Ogg => (&["vorbis"], strings(&["-c:a", "libvorbis", "-q:a", "6"])),
        Target::Opus => {
            let mut args = strings(&["-c:a", "libopus", "-b:a", "160k"]);
            if probe.audio_channels > 2 {
                args.extend(strings(&["-ac", "2"]));
            }
            (&["opus"], args)
        }
        Target::Wma => (&["wmav2"], strings(&["-c:a", "wmav2", "-b:a", "192k"])),
        _ => (&[], Vec::new()),
    };
    let base = strings(&["-map", "0:a:0", "-vn", "-sn", "-dn"]);
    let tail = |mut args: Vec<String>| {
        if target == Target::M4a {
            args.extend(strings(&["-movflags", "+faststart"]));
        }
        args
    };
    let mut attempts = Vec::new();
    if copy_from.contains(&codec) {
        attempts.push(tail([base.clone(), strings(&["-c:a", "copy"])].concat()));
    }
    attempts.push(tail([base, encode].concat()));
    attempts
}

/// The ffmpeg arguments to try in order, without input and output.
pub fn plan(target: Target, probe: &Probe) -> Vec<Vec<String>> {
    match target.family() {
        Family::Image => image_args(target, probe),
        Family::Video => video_args(target, probe),
        Family::Audio => audio_args(target, probe),
    }
}

/// `name.ext`, or `name (2).ext` and up when that is taken.
pub fn output_path(source: &Path, target: Target) -> PathBuf {
    let stem = source.file_stem().map_or_else(|| "converted".into(), |stem| stem.to_string_lossy().into_owned());
    let dir = source.parent().unwrap_or_else(|| Path::new("."));
    let mut candidate = dir.join(format!("{stem}.{}", target.extension()));
    let mut number = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} ({number}).{}", target.extension()));
        number += 1;
    }
    candidate
}

/// Reads `out_time_us=<microseconds>` from ffmpeg's `-progress` output.
fn parse_out_time(line: &str) -> Option<f64> {
    let value = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms="))?;
    value.trim().parse::<f64>().ok().filter(|micros| *micros >= 0.0).map(|micros| micros / 1_000_000.0)
}

/// ffmpeg's own complaint, without the `key=value` progress lines. The
/// first one names the cause; the later ones only report what failed next.
fn error_line(text: &str) -> String {
    text.lines()
        .find(|line| {
            let key = line.split('=').next().unwrap_or_default();
            !(line.contains('=') && key.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
        })
        .unwrap_or_default()
        .to_owned()
}

/// Converts `source` into `target` next to it and returns the new file.
///
/// WindowCraft tries its own code first (see `convert_native`). ffmpeg, when
/// the user installed it, takes what that code cannot do; without it such a
/// file fails with `EngineError::NotFound("ffmpeg")`.
pub fn convert_file(
    ffmpeg: Option<&Tools>,
    source: &Path,
    target: Target,
    cancel: &CancelToken,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf> {
    let output = output_path(source, target);
    let stem = output.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = output.with_file_name(format!("{stem}.wc-tmp.{}", target.extension()));
    let native = crate::convert_native::convert(source, &temporary, target, ffmpeg.is_some(), cancel, &mut progress);
    match native {
        Ok(true) => {
            std::fs::rename(&temporary, &output).map_err(|error| {
                let _ = std::fs::remove_file(&temporary);
                EngineError::io(output.display().to_string(), error)
            })?;
            progress(1.0);
            return Ok(output);
        }
        Ok(false) => {}
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            if matches!(error, EngineError::Cancelled) || ffmpeg.is_none() {
                return Err(error);
            }
        }
    }
    let Some(tools) = ffmpeg else {
        return Err(EngineError::NotFound("ffmpeg".to_owned()));
    };
    convert_with_ffmpeg(tools, source, target, &output, &temporary, cancel, &mut progress)
}

fn convert_with_ffmpeg(
    tools: &Tools,
    source: &Path,
    target: Target,
    output: &Path,
    temporary: &Path,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(f32),
) -> Result<PathBuf> {
    let probe = probe(tools, source, cancel)?;
    if let Some(mismatch) = probe.mismatch(target) {
        return Err(EngineError::Invalid(format!("{mismatch:?}")));
    }
    let source_text = source.display().to_string();
    let temporary_text = temporary.display().to_string();
    let mut last_error = String::new();
    for args in plan(target, &probe) {
        cancel.check()?;
        let mut command = strings(&["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-i"]);
        command.push(source_text.clone());
        command.extend(args);
        command.extend(strings(&["-progress", "pipe:1", "-nostats"]));
        command.push(temporary_text.clone());
        let command: Vec<&str> = command.iter().map(String::as_str).collect();
        let result = crate::process::run(&tools.ffmpeg(), &command, cancel, CONVERT_TIMEOUT, |line| {
            if let (Some(done), Some(total)) = (parse_out_time(line), probe.duration) {
                progress((done / total).clamp(0.0, 1.0) as f32);
            }
        });
        match result {
            Ok(run) if run.code == 0 && temporary.is_file() => {
                std::fs::rename(temporary, output)
                    .map_err(|error| EngineError::io(output.display().to_string(), error))?;
                progress(1.0);
                return Ok(output.to_path_buf());
            }
            Ok(run) => {
                last_error = error_line(&run.text);
                let _ = std::fs::remove_file(temporary);
            }
            Err(error) => {
                let _ = std::fs::remove_file(temporary);
                return Err(error);
            }
        }
    }
    Err(EngineError::Process { program: "ffmpeg".to_owned(), code: 1, detail: last_error })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MKV: &str = r#"{"streams":[{"codec_name":"h264","codec_type":"video","disposition":{"attached_pic":0}},{"codec_name":"opus","codec_type":"audio","channels":6,"disposition":{"attached_pic":0}},{"codec_name":"subrip","codec_type":"subtitle","disposition":{"attached_pic":0}}],"format":{"format_name":"matroska,webm","duration":"12.500000"}}"#;
    const MP3_WITH_COVER: &str = r#"{"streams":[{"codec_name":"mp3","codec_type":"audio","channels":2,"disposition":{"attached_pic":0}},{"codec_name":"mjpeg","codec_type":"video","disposition":{"attached_pic":1}}],"format":{"format_name":"mp3","duration":"200.1"}}"#;
    const PNG: &str = r#"{"streams":[{"codec_name":"png","codec_type":"video","disposition":{"attached_pic":0}}],"format":{"format_name":"png_pipe"}}"#;

    fn joined(attempt: &[String]) -> String {
        attempt.join(" ")
    }

    #[test]
    fn reads_ffprobe_output() {
        let mkv = Probe::parse(MKV).unwrap();
        assert!(!mkv.still && mkv.subtitles);
        assert_eq!(mkv.video.as_deref(), Some("h264"));
        assert_eq!(mkv.audio, ["opus"]);
        assert_eq!(mkv.audio_channels, 6);
        assert_eq!(mkv.duration, Some(12.5));

        let mp3 = Probe::parse(MP3_WITH_COVER).unwrap();
        assert_eq!(mp3.picture.as_deref(), Some("mjpeg"), "cover art can become a picture");
        assert_eq!(mp3.video, None, "but it is not a video");

        let png = Probe::parse(PNG).unwrap();
        assert!(png.still);
        assert_eq!(png.duration, None);
        assert!(Probe::parse("not json").is_none());
    }

    #[test]
    fn rejects_targets_the_source_cannot_fill() {
        let mp3 = Probe::parse(MP3_WITH_COVER).unwrap();
        assert_eq!(mp3.mismatch(Target::Mp4), Some(Mismatch::NoVideo));
        assert_eq!(mp3.mismatch(Target::Png), None);
        assert_eq!(mp3.mismatch(Target::Flac), None);
        let png = Probe::parse(PNG).unwrap();
        assert_eq!(png.mismatch(Target::Mkv), Some(Mismatch::NoVideo));
        assert_eq!(png.mismatch(Target::Mp3), Some(Mismatch::NoAudio));
        assert_eq!(png.mismatch(Target::Jpg), None);
    }

    #[test]
    fn copies_streams_when_the_container_allows_it() {
        let mkv = Probe::parse(MKV).unwrap();
        let to_mkv = plan(Target::Mkv, &mkv);
        assert!(joined(&to_mkv[0]).contains("-c:v copy"));
        assert!(joined(&to_mkv[0]).contains("-c:a copy"));
        assert!(joined(&to_mkv[0]).contains("-c:s copy"));

        // H.264 fits MP4 as it is; Opus is encoded to AAC for players.
        let to_mp4 = plan(Target::Mp4, &mkv);
        let first = joined(&to_mp4[0]);
        assert!(first.contains("-c:v copy") && first.contains("-c:a aac") && first.contains("-c:s mov_text"), "{first}");
        let fallback = joined(&to_mp4[1]);
        assert!(fallback.contains("libx264") && fallback.contains("-sn"), "{fallback}");

        let to_webm = plan(Target::Webm, &mkv);
        let first = joined(&to_webm[0]);
        assert!(first.contains("libvpx-vp9") && first.contains("-c:a copy"), "{first}");

        assert_eq!(plan(Target::Avi, &mkv).len(), 1, "AVI is always encoded");
    }

    #[test]
    fn audio_is_copied_only_into_its_own_format() {
        let mp3 = Probe::parse(MP3_WITH_COVER).unwrap();
        assert!(joined(&plan(Target::Mp3, &mp3)[0]).contains("-c:a copy"));
        let to_flac = plan(Target::Flac, &mp3);
        assert_eq!(to_flac.len(), 1);
        assert!(joined(&to_flac[0]).contains("-c:a flac"));
        let mkv = Probe::parse(MKV).unwrap();
        assert!(joined(&plan(Target::Opus, &mkv)[0]).contains("-c:a copy"));
        assert!(joined(&plan(Target::Mp3, &mkv)[0]).contains("-map 0:a:0 -vn"));
    }

    #[test]
    fn stills_stay_single_frames_and_clips_animate() {
        let png = Probe::parse(PNG).unwrap();
        assert!(joined(&plan(Target::Gif, &png)[0]).contains("-frames:v 1"));
        assert!(joined(&plan(Target::Jpg, &png)[0]).contains("-update 1"));
        let mkv = Probe::parse(MKV).unwrap();
        let gif = joined(&plan(Target::Gif, &mkv)[0]);
        assert!(gif.contains("palettegen") && !gif.contains("-frames:v"), "{gif}");
        assert!(joined(&plan(Target::Webp, &mkv)[0]).contains("libwebp_anim"));
        assert!(joined(&plan(Target::Png, &mkv)[0]).contains("-frames:v 1"), "a clip gives its first frame");
    }

    #[test]
    fn never_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("clip.mp4");
        std::fs::write(&source, b"x").unwrap();
        assert_eq!(output_path(&source, Target::Mkv), dir.path().join("clip.mkv"));
        assert_eq!(output_path(&source, Target::Mp4), dir.path().join("clip (2).mp4"));
        std::fs::write(dir.path().join("clip (2).mp4"), b"x").unwrap();
        assert_eq!(output_path(&source, Target::Mp4), dir.path().join("clip (3).mp4"));
    }

    #[test]
    fn reads_progress_and_errors() {
        assert_eq!(parse_out_time("out_time_us=2500000"), Some(2.5));
        assert_eq!(parse_out_time("out_time_us=N/A"), None);
        assert_eq!(parse_out_time("frame=12"), None);
        let text = "frame=1\n[libx264 @ 0x1] width not divisible by 2\n[out#0 @ 0x2] Nothing was written\nprogress=end";
        assert_eq!(error_line(text), "[libx264 @ 0x1] width not divisible by 2");
    }

    #[test]
    fn every_target_has_a_plan() {
        let clip = Probe::parse(MKV).unwrap();
        let png = Probe::parse(PNG).unwrap();
        for target in Target::ALL {
            let probe = if target.family() == Family::Image { &png } else { &clip };
            assert!(!plan(target, probe).is_empty(), "{target:?}");
        }
    }

    /// Converts generated samples into every format with the real ffmpeg.
    /// Needs an installed converter; run with
    /// `cargo test -p windowcraft-engine live_convert -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs ffmpeg in the WindowCraft tools folder"]
    fn live_convert_every_target() {
        let tools = Tools::in_dir(&crate::tools::tools_dir());
        assert!(tools.has(crate::tools::Tool::Ffmpeg), "install the converter first");
        let dir = tempfile::tempdir().unwrap();
        let cancel = CancelToken::new();
        let make = |name: &str, args: &[&str]| {
            let path = dir.path().join(name);
            let mut command = vec!["-hide_banner", "-loglevel", "error", "-y"];
            command.extend_from_slice(args);
            let path_text = path.display().to_string();
            command.push(&path_text);
            crate::process::run_ok(&tools.ffmpeg(), &command, &cancel, Duration::from_secs(120)).unwrap();
            path
        };
        // Odd sizes on purpose: H.264 needs even ones.
        let picture = make("picture.png", &["-f", "lavfi", "-i", "testsrc2=size=321x241", "-frames:v", "1"]);
        let clip = make(
            "clip.mkv",
            &[
                "-f", "lavfi", "-i", "testsrc2=size=321x241:rate=25:duration=2",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-vf", EVEN, "-c:a", "libopus",
            ],
        );
        let song = make("song.flac", &["-f", "lavfi", "-i", "sine=frequency=330:duration=2:sample_rate=48000"]);
        let mut failures = Vec::new();
        for target in Target::ALL {
            let sources: Vec<&PathBuf> = match target.family() {
                Family::Image => vec![&picture, &clip],
                Family::Video => vec![&clip],
                Family::Audio => vec![&clip, &song],
            };
            for source in sources {
                let started = std::time::Instant::now();
                match convert_file(Some(&tools), source, target, &cancel, |_| {}) {
                    Ok(output) => {
                        let check = probe(&tools, &output, &cancel).unwrap();
                        assert!(check.mismatch(target).is_none(), "{} has the wrong streams", output.display());
                        println!("{:>6} <- {:<12} {:>6} ms", target.label(), name_of(source), started.elapsed().as_millis());
                        std::fs::remove_file(output).unwrap();
                    }
                    Err(error) => failures.push(format!("{} -> {}: {error}", name_of(source), target.label())),
                }
            }
        }
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".wc-tmp."))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        assert!(failures.is_empty(), "{failures:#?}");
    }

}
