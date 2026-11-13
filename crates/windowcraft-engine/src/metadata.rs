// Made by WindowCraft Dev Team
//! Removes metadata from photos, audio and video.
//!
//! Everything runs on this machine and nothing is sent anywhere. Images, MP3
//! and FLAC are rewritten here block by block: what describes the picture or
//! the sound stays, while camera data, GPS, dates, authors, comments, editing
//! history, thumbnails and embedded cover art go. MP4, MOV, MKV, WebM, WAV,
//! AVI, AIFF, Ogg and ASF are cleaned by `containers`, also without ffmpeg.
//! Only what neither handles, such as FLV, goes to a local ffmpeg, which
//! remuxes it without re-encoding.
//!
//! The source file is never changed. The clean copy lands next to it.

use crate::containers::{self, Container};
use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Jpeg,
    Png,
    Webp,
    Gif,
    Mp3,
    Flac,
    /// Audio or video cleaned in place, see `containers`.
    Container(Container),
    /// Audio or video that only ffmpeg can remux.
    Media,
}

impl Kind {
    pub fn needs_ffmpeg(self) -> bool {
        self == Self::Media
    }
}

/// Extensions that only ffmpeg can clean. The output keeps the same container.
const MEDIA_EXTENSIONS: [&str; 3] = ["flv", "ogv", "ts"];

/// Extensions the file dialog offers.
pub const SUPPORTED_PATTERN: &str = "*.jpg;*.jpeg;*.jfif;*.png;*.webp;*.gif;*.mp3;*.aac;*.flac;*.mp4;*.m4a;*.m4v;*.m4b;*.mov;*.3gp;*.mkv;*.mka;*.webm;*.avi;*.wav;*.aif;*.aiff;*.ogg;*.oga;*.opus;*.wma;*.wmv;*.asf;*.flv";

/// Picks the format from the file contents, then from the extension.
pub fn kind_of(path: &Path, head: &[u8]) -> Option<Kind> {
    if head.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(Kind::Jpeg);
    }
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(Kind::Png);
    }
    if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        return Some(Kind::Webp);
    }
    if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        return Some(Kind::Gif);
    }
    if head.starts_with(b"fLaC") {
        return Some(Kind::Flac);
    }
    if let Some(container) = containers::detect(head) {
        return Some(Kind::Container(container));
    }
    let extension = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match extension.as_str() {
        // Raw AAC carries the same ID3 tags as MP3.
        "mp3" | "aac" => Some(Kind::Mp3),
        "flac" => Some(Kind::Flac),
        other if MEDIA_EXTENSIONS.contains(&other) => Some(Kind::Media),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stripped {
    pub output: PathBuf,
    /// Metadata blocks removed. ffmpeg does not report a count, so it is 0 there.
    pub removed: usize,
}

/// `photo.jpg` becomes `photo-clean.jpg`, or `photo-clean-2.jpg` when taken.
pub fn clean_path(source: &Path) -> PathBuf {
    let stem = source.file_stem().map_or_else(|| "file".into(), |stem| stem.to_string_lossy().into_owned());
    let extension = source.extension().map(|ext| ext.to_string_lossy().into_owned());
    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    let name = |suffix: String| match &extension {
        Some(ext) => format!("{stem}-clean{suffix}.{ext}"),
        None => format!("{stem}-clean{suffix}"),
    };
    let mut candidate = parent.join(name(String::new()));
    let mut counter = 2;
    while candidate.exists() {
        candidate = parent.join(name(format!("-{counter}")));
        counter += 1;
    }
    candidate
}

/// Writes a clean copy of `source` and returns where it went.
pub fn strip_file(source: &Path, ffmpeg: Option<&Path>, cancel: &CancelToken) -> Result<Stripped> {
    let output = clean_path(source);
    let removed = strip_to(source, &output, ffmpeg, cancel)?;
    Ok(Stripped { output, removed })
}

/// Cleans `path` itself, used for files WindowCraft has just created.
pub fn strip_in_place(path: &Path, ffmpeg: Option<&Path>, cancel: &CancelToken) -> Result<usize> {
    strip_to(path, path, ffmpeg, cancel)
}

fn strip_to(source: &Path, output: &Path, ffmpeg: Option<&Path>, cancel: &CancelToken) -> Result<usize> {
    let shown = source.display().to_string();
    let head = read_head(source).map_err(|e| EngineError::io(shown.clone(), e))?;
    let kind = kind_of(source, &head).ok_or_else(|| EngineError::Unsupported(shown.clone()))?;
    if let Kind::Container(container) = kind {
        let temporary = temporary_beside(output);
        match containers::strip(container, source, &temporary, cancel) {
            Ok(removed) => {
                return std::fs::rename(&temporary, output).map(|()| removed).map_err(|e| {
                    let _ = std::fs::remove_file(&temporary);
                    EngineError::io(output.display().to_string(), e)
                });
            }
            Err(EngineError::Unsupported(_)) if ffmpeg.is_some() => {
                let _ = std::fs::remove_file(&temporary);
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(match error {
                    EngineError::Unsupported(_) => EngineError::NotFound("ffmpeg".to_owned()),
                    other => other,
                });
            }
        }
    }
    if matches!(kind, Kind::Media | Kind::Container(_)) {
        let ffmpeg = ffmpeg.ok_or_else(|| EngineError::NotFound("ffmpeg".to_owned()))?;
        remux(ffmpeg, source, output, cancel)?;
        return Ok(0);
    }
    let data = std::fs::read(source).map_err(|e| EngineError::io(shown.clone(), e))?;
    let (clean, removed) = strip_bytes(kind, &data).map_err(|reason| EngineError::Invalid(format!("{shown}: {reason}")))?;
    cancel.check()?;
    write_replacing(output, &clean)?;
    Ok(removed)
}

/// Writes through a temporary file in the same folder, then renames it over
/// `path`. Unlike `atomic_write` it keeps no `.bak`, which would hold the very
/// metadata that was just removed.
fn temporary_beside(path: &Path) -> PathBuf {
    let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".{name}.wc-{}.tmp", std::process::id()))
}

fn write_replacing(path: &Path, content: &[u8]) -> Result<()> {
    use std::io::Write;
    let temporary = temporary_beside(path);
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| EngineError::io(path.display().to_string(), e))
}

fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::with_capacity(16);
    std::fs::File::open(path)?.take(16).read_to_end(&mut head)?;
    Ok(head)
}

/// Rewrites an in-memory file without its metadata.
pub fn strip_bytes(kind: Kind, data: &[u8]) -> std::result::Result<(Vec<u8>, usize), &'static str> {
    match kind {
        Kind::Jpeg => strip_jpeg(data),
        Kind::Png => strip_png(data),
        Kind::Webp => strip_webp(data),
        Kind::Gif => strip_gif(data),
        Kind::Mp3 => Ok(strip_mp3(data)),
        Kind::Flac => strip_flac(data),
        Kind::Container(_) | Kind::Media => Err("needs a file, not bytes"),
    }
}

fn be16(data: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(u16::from_be_bytes([*data.get(at)?, *data.get(at + 1)?])))
}

fn be32(data: &[u8], at: usize) -> Option<usize> {
    let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_be_bytes(bytes) as usize)
}

fn le32(data: &[u8], at: usize) -> Option<usize> {
    let bytes: [u8; 4] = data.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes) as usize)
}

// ---------------------------------------------------------------------------
// JPEG

/// Keeps the picture, its color profile and the Adobe color transform. Drops
/// EXIF, XMP, IPTC, comments, JFXX and MPF thumbnails, maker segments and
/// anything after the end marker, such as an appended motion-photo video.
/// The EXIF orientation comes back as the only tag, so photos stay upright.
fn strip_jpeg(data: &[u8]) -> std::result::Result<(Vec<u8>, usize), &'static str> {
    const TRUNCATED: &str = "truncated JPEG";
    if !data.starts_with(&[0xFF, 0xD8]) {
        return Err("not a JPEG");
    }
    let mut kept: Vec<&[u8]> = Vec::new();
    let mut removed = 0;
    let mut orientation = None;
    let mut i = 2;
    loop {
        if data.get(i) != Some(&0xFF) {
            return Err("damaged JPEG marker");
        }
        while data.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        let marker = *data.get(i + 1).ok_or(TRUNCATED)?;
        match marker {
            0xD9 => {
                kept.push(&data[i..i + 2]);
                break;
            }
            0x01 | 0xD0..=0xD7 => {
                kept.push(&data[i..i + 2]);
                i += 2;
                continue;
            }
            _ => {}
        }
        let length = be16(data, i + 2).ok_or(TRUNCATED)?;
        if length < 2 {
            return Err("damaged JPEG segment");
        }
        let end = i + 2 + length;
        let segment = data.get(i..end).ok_or(TRUNCATED)?;
        let payload = &segment[4..];
        let keep = match marker {
            0xE0 => payload.starts_with(b"JFIF\0"),
            0xE2 => payload.starts_with(b"ICC_PROFILE\0"),
            0xEE => payload.starts_with(b"Adobe"),
            0xE1 => {
                if payload.starts_with(b"Exif\0\0") {
                    orientation = orientation.or_else(|| exif_orientation(&payload[6..]));
                }
                false
            }
            0xE3..=0xED | 0xEF | 0xFE => false,
            _ => true,
        };
        if keep {
            kept.push(segment);
        } else {
            removed += 1;
        }
        i = end;
        if marker == 0xDA {
            // Entropy-coded data runs to the next marker that is neither a
            // stuffed zero nor a restart marker.
            let start = i;
            loop {
                let byte = *data.get(i).ok_or(TRUNCATED)?;
                if byte == 0xFF {
                    let next = *data.get(i + 1).ok_or(TRUNCATED)?;
                    if next != 0x00 && !(0xD0..=0xD7).contains(&next) && next != 0xFF {
                        break;
                    }
                }
                i += 1;
            }
            kept.push(&data[start..i]);
        }
    }
    if i + 2 < data.len() {
        removed += 1;
    }

    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&[0xFF, 0xD8]);
    let mut rest = kept.as_slice();
    // JFIF stays first, as its specification asks.
    if let Some((first, tail)) = rest.split_first()
        && first.get(1) == Some(&0xE0)
    {
        out.extend_from_slice(first);
        rest = tail;
    }
    if let Some(value) = orientation.filter(|value| (2..=8).contains(value)) {
        out.extend_from_slice(&orientation_exif(value));
    }
    for part in rest {
        out.extend_from_slice(part);
    }
    Ok((out, removed))
}

/// Reads tag 0x0112 from IFD0 of a TIFF-structured EXIF block.
fn exif_orientation(tiff: &[u8]) -> Option<u16> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |at: usize| -> Option<u16> {
        let bytes = [*tiff.get(at)?, *tiff.get(at + 1)?];
        Some(if little { u16::from_le_bytes(bytes) } else { u16::from_be_bytes(bytes) })
    };
    let u32_at = |at: usize| -> Option<usize> {
        let bytes: [u8; 4] = tiff.get(at..at + 4)?.try_into().ok()?;
        Some((if little { u32::from_le_bytes(bytes) } else { u32::from_be_bytes(bytes) }) as usize)
    };
    let ifd = u32_at(4)?;
    let count = usize::from(u16_at(ifd)?);
    for index in 0..count.min(512) {
        let entry = ifd + 2 + index * 12;
        if u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 {
            return u16_at(entry + 8);
        }
    }
    None
}

/// An APP1 segment holding nothing but the orientation tag.
fn orientation_exif(value: u16) -> Vec<u8> {
    let mut segment = vec![0xFF, 0xE1, 0x00, 0x22];
    segment.extend_from_slice(b"Exif\0\0MM\0\x2A\0\0\0\x08");
    segment.extend_from_slice(&[0x00, 0x01, 0x01, 0x12, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01]);
    segment.extend_from_slice(&value.to_be_bytes());
    segment.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    segment
}

// ---------------------------------------------------------------------------
// PNG

/// Chunks that shape the picture. Text, time, EXIF, C2PA and every private
/// chunk are left out.
const PNG_KEEP: [&[u8; 4]; 17] = [
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"cHRM", b"gAMA", b"iCCP", b"sBIT", b"sRGB", b"cICP", b"mDCV", b"cLLI",
    b"bKGD", b"pHYs", b"acTL", b"fcTL",
];

fn strip_png(data: &[u8]) -> std::result::Result<(Vec<u8>, usize), &'static str> {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIGNATURE) {
        return Err("not a PNG");
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(SIGNATURE);
    let mut removed = 0;
    let mut i = SIGNATURE.len();
    loop {
        let length = be32(data, i).ok_or("truncated PNG")?;
        let end = i.checked_add(12 + length).ok_or("damaged PNG")?;
        let chunk = data.get(i..end).ok_or("truncated PNG")?;
        let kind = &chunk[4..8];
        if PNG_KEEP.iter().any(|keep| keep.as_slice() == kind) || kind == b"fdAT" {
            out.extend_from_slice(chunk);
        } else {
            removed += 1;
        }
        i = end;
        if kind == b"IEND" {
            break;
        }
    }
    if i < data.len() {
        removed += 1;
    }
    Ok((out, removed))
}

// ---------------------------------------------------------------------------
// WebP

const WEBP_KEEP: [&[u8; 4]; 7] = [b"VP8 ", b"VP8L", b"VP8X", b"ALPH", b"ANIM", b"ANMF", b"ICCP"];

fn strip_webp(data: &[u8]) -> std::result::Result<(Vec<u8>, usize), &'static str> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err("not a WebP");
    }
    let riff_end = (le32(data, 4).ok_or("truncated WebP")? + 8).min(data.len());
    let mut body = Vec::with_capacity(data.len());
    body.extend_from_slice(b"WEBP");
    let mut removed = 0;
    let mut i = 12;
    while i + 8 <= riff_end {
        let size = le32(data, i + 4).ok_or("truncated WebP")?;
        let padded = size + (size & 1);
        let end = (i + 8 + padded).min(riff_end);
        let chunk = data.get(i..end).ok_or("truncated WebP")?;
        let kind = &chunk[..4];
        if WEBP_KEEP.iter().any(|keep| keep.as_slice() == kind) {
            let start = body.len();
            body.extend_from_slice(chunk);
            if kind == b"VP8X" && body.len() > start + 8 {
                // The EXIF and XMP flags must match the chunks that remain.
                body[start + 8] &= !(0x08 | 0x04);
            }
        } else {
            removed += 1;
        }
        i = end;
    }
    if riff_end < data.len() {
        removed += 1;
    }
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&u32::try_from(body.len()).map_err(|_| "WebP too large")?.to_le_bytes());
    out.extend_from_slice(&body);
    Ok((out, removed))
}

// ---------------------------------------------------------------------------
// GIF

fn gif_sub_blocks(data: &[u8], mut i: usize) -> std::result::Result<usize, &'static str> {
    loop {
        let size = usize::from(*data.get(i).ok_or("truncated GIF")?);
        i += 1 + size;
        if size == 0 {
            return Ok(i);
        }
    }
}

/// Keeps frames, timing and the loop count. Drops comments and application
/// blocks such as XMP.
fn strip_gif(data: &[u8]) -> std::result::Result<(Vec<u8>, usize), &'static str> {
    if data.len() < 13 || !(data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")) {
        return Err("not a GIF");
    }
    let flags = data[10];
    let mut i = 13;
    if flags & 0x80 != 0 {
        i += 3 << ((flags & 0x07) + 1);
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(data.get(..i).ok_or("truncated GIF")?);
    let mut removed = 0;
    loop {
        match *data.get(i).ok_or("truncated GIF")? {
            0x3B => {
                out.push(0x3B);
                i += 1;
                break;
            }
            0x2C => {
                let descriptor_flags = *data.get(i + 9).ok_or("truncated GIF")?;
                let mut end = i + 10;
                if descriptor_flags & 0x80 != 0 {
                    end += 3 << ((descriptor_flags & 0x07) + 1);
                }
                end = gif_sub_blocks(data, end + 1)?;
                out.extend_from_slice(data.get(i..end).ok_or("truncated GIF")?);
                i = end;
            }
            0x21 => {
                let label = *data.get(i + 1).ok_or("truncated GIF")?;
                let end = gif_sub_blocks(data, i + 2)?;
                let block = data.get(i..end).ok_or("truncated GIF")?;
                let keep = match label {
                    0xF9 | 0x01 => true,
                    0xFF => {
                        let id = block.get(3..14).unwrap_or_default();
                        id == b"NETSCAPE2.0" || id == b"ANIMEXTS1.0"
                    }
                    _ => false,
                };
                if keep {
                    out.extend_from_slice(block);
                } else {
                    removed += 1;
                }
                i = end;
            }
            _ => return Err("damaged GIF block"),
        }
    }
    if i < data.len() {
        removed += 1;
    }
    Ok((out, removed))
}

// ---------------------------------------------------------------------------
// MP3

/// Drops ID3v2 tags at the start and ID3v1, Lyrics3 and APE tags at the end.
fn strip_mp3(data: &[u8]) -> (Vec<u8>, usize) {
    let mut removed = 0;
    let mut start = 0;
    while data.len() >= start + 10 && &data[start..start + 3] == b"ID3" {
        let size = data[start + 6..start + 10].iter().fold(0usize, |acc, byte| (acc << 7) | usize::from(byte & 0x7F));
        let footer = if data[start + 5] & 0x10 != 0 { 10 } else { 0 };
        start = (start + 10 + size + footer).min(data.len());
        removed += 1;
    }
    let mut end = data.len();
    loop {
        let tail = &data[start..end];
        if tail.len() >= 128 && tail[tail.len() - 128..].starts_with(b"TAG") {
            end -= 128;
            if end - start >= 227 && data[end - 227..].starts_with(b"TAG+") {
                end -= 227;
            }
            removed += 1;
            continue;
        }
        if tail.len() >= 32 && tail[tail.len() - 32..].starts_with(b"APETAGEX") {
            let footer = end - 32;
            let size = le32(data, footer + 12).unwrap_or(0);
            let has_header = le32(data, footer + 20).is_some_and(|flags| flags & 0x8000_0000 != 0);
            // The size counts the items and the footer, not the header. A size
            // below 32 or a tag longer than what is left is damaged: keep the
            // bytes and stop, so every pass of the loop moves `end` back.
            let total = size.checked_add(if has_header { 32 } else { 0 });
            match total {
                Some(total) if size >= 32 && total <= end - start => {
                    end -= total;
                    removed += 1;
                    continue;
                }
                _ => break,
            }
        }
        if tail.len() >= 15 && &tail[tail.len() - 9..] == b"LYRICS200" {
            let digits = std::str::from_utf8(&tail[tail.len() - 15..tail.len() - 9]).ok();
            if let Some(size) = digits.and_then(|text| text.parse::<usize>().ok()) {
                end = end.saturating_sub(size + 15).max(start);
                removed += 1;
                continue;
            }
        }
        break;
    }
    (data[start..end].to_vec(), removed)
}

// ---------------------------------------------------------------------------
// FLAC

/// Keeps STREAMINFO, SEEKTABLE and CUESHEET. Drops Vorbis comments, pictures,
/// application blocks and padding.
fn strip_flac(data: &[u8]) -> std::result::Result<(Vec<u8>, usize), &'static str> {
    let (data, mut removed) = {
        // Some taggers put an ID3v2 block in front of the stream.
        let (rest, count) = strip_mp3(data);
        (rest, count)
    };
    if !data.starts_with(b"fLaC") {
        return Err("not a FLAC stream");
    }
    let mut blocks: Vec<&[u8]> = Vec::new();
    let mut i = 4;
    loop {
        let header = *data.get(i).ok_or("truncated FLAC")?;
        let length = (usize::from(*data.get(i + 1).ok_or("truncated FLAC")?) << 16)
            | be16(&data, i + 2).ok_or("truncated FLAC")?;
        let end = i + 4 + length;
        let block = data.get(i..end).ok_or("truncated FLAC")?;
        if matches!(header & 0x7F, 0 | 3 | 5) {
            blocks.push(block);
        } else {
            removed += 1;
        }
        i = end;
        if header & 0x80 != 0 {
            break;
        }
    }
    if blocks.first().is_none_or(|block| block[0] & 0x7F != 0) {
        return Err("FLAC without STREAMINFO");
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(b"fLaC");
    let last = blocks.len() - 1;
    for (index, block) in blocks.iter().enumerate() {
        let flag = if index == last { 0x80 } else { 0x00 };
        out.push((block[0] & 0x7F) | flag);
        out.extend_from_slice(&block[1..]);
    }
    out.extend_from_slice(&data[i..]);
    Ok((out, removed))
}

// ---------------------------------------------------------------------------
// ffmpeg

/// Copies every audio, video and subtitle stream into the same container
/// without global, stream or chapter metadata and without an encoder tag.
fn remux(ffmpeg: &Path, source: &Path, output: &Path, cancel: &CancelToken) -> Result<()> {
    let extension = source.extension().map(|ext| ext.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let temporary = output.with_extension(format!("wc-tmp.{extension}"));
    let source_text = source.display().to_string();
    let temporary_text = temporary.display().to_string();
    let mut args = vec![
        "-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-i", source_text.as_str(),
        "-map", "0:v?", "-map", "0:a?", "-map", "0:s?", "-dn", "-c", "copy",
        "-map_metadata", "-1", "-map_metadata:s:v", "-1", "-map_metadata:s:a", "-1", "-map_metadata:s:s", "-1",
        "-map_chapters", "-1", "-fflags", "+bitexact", "-flags:v", "+bitexact", "-flags:a", "+bitexact",
    ];
    if matches!(extension.as_str(), "mp4" | "m4a" | "m4v" | "mov" | "3gp") {
        args.extend(["-movflags", "+faststart"]);
    }
    args.push(temporary_text.as_str());
    let result = crate::process::run_ok(ffmpeg, &args, cancel, Duration::from_secs(4 * 3600))
        .and_then(|_| std::fs::rename(&temporary, output).map_err(|e| EngineError::io(output.display().to_string(), e)));
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
pub(crate) mod tests_support {
    /// An APP1 segment holding only an EXIF orientation.
    pub fn orientation_segment(value: u16) -> Vec<u8> {
        super::orientation_exif(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_with(segments: &[&[u8]]) -> Vec<u8> {
        let mut data = vec![0xFF, 0xD8];
        for segment in segments {
            data.extend_from_slice(segment);
        }
        // A minimal scan: SOS header, two bytes of entropy data with a
        // stuffed 0xFF, then EOI.
        data.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x04, 0x01, 0x00, 0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD9]);
        data
    }

    fn app(marker: u8, payload: &[u8]) -> Vec<u8> {
        let mut segment = vec![0xFF, marker];
        segment.extend_from_slice(&u16::try_from(payload.len() + 2).unwrap().to_be_bytes());
        segment.extend_from_slice(payload);
        segment
    }

    fn exif_with_orientation(value: u16) -> Vec<u8> {
        let mut payload = b"Exif\0\0II\x2A\0\x08\0\0\0".to_vec();
        payload.extend_from_slice(&2u16.to_le_bytes());
        // A GPS pointer the stripper must drop along with everything else.
        payload.extend_from_slice(&[0x25, 0x88, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00]);
        payload.extend_from_slice(&[0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00]);
        payload.extend_from_slice(&value.to_le_bytes());
        payload.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        payload
    }

    #[test]
    fn jpeg_loses_exif_xmp_and_comments_but_keeps_picture_and_orientation() {
        let jfif = app(0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
        let exif = app(0xE1, &exif_with_orientation(6));
        let xmp = app(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>GPS</x:xmpmeta>");
        let icc = app(0xE2, b"ICC_PROFILE\0\x01\x01data");
        let iptc = app(0xED, b"Photoshop 3.0\0author");
        let comment = app(0xFE, b"secret comment");
        let quant = app(0xDB, &[0u8; 65]);
        let mut data = jpeg_with(&[&jfif, &exif, &xmp, &icc, &iptc, &comment, &quant]);
        data.extend_from_slice(b"appended motion photo");
        let (clean, removed) = strip_bytes(Kind::Jpeg, &data).unwrap();
        assert_eq!(removed, 5);
        let text = String::from_utf8_lossy(&clean);
        for gone in ["xmpmeta", "author", "secret", "appended"] {
            assert!(!text.contains(gone), "{gone} survived");
        }
        assert!(clean.windows(11).any(|w| w == b"ICC_PROFILE"));
        assert!(clean.ends_with(&[0xFF, 0xD9]));
        assert_eq!(&clean[2..4], &[0xFF, 0xE0], "JFIF stays first");
        let exif_at = clean.windows(6).position(|w| w == b"Exif\0\0").unwrap();
        assert_eq!(exif_orientation(&clean[exif_at + 6..]), Some(6));
        assert_eq!(clean.len(), data.len() - exif.len() - xmp.len() - iptc.len() - comment.len() - 21 + 36);
    }

    #[test]
    fn upright_jpeg_gets_no_exif_at_all() {
        let exif = app(0xE1, &exif_with_orientation(1));
        let (clean, removed) = strip_bytes(Kind::Jpeg, &jpeg_with(&[&exif])).unwrap();
        assert_eq!(removed, 1);
        assert!(!clean.windows(4).any(|w| w == b"Exif"));
    }

    #[test]
    fn progressive_jpeg_keeps_tables_between_scans() {
        let mut data = jpeg_with(&[]);
        data.truncate(data.len() - 2);
        data.extend_from_slice(&app(0xC4, &[0u8; 17]));
        data.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x04, 0x01, 0x00, 0x56, 0xFF, 0xD3, 0x78, 0xFF, 0xD9]);
        let (clean, removed) = strip_bytes(Kind::Jpeg, &data).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(clean, data);
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]);
        chunk
    }

    #[test]
    fn png_keeps_image_chunks_only() {
        let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, body) in [
            (b"IHDR", &[0u8; 13][..]),
            (b"tEXt", b"Author\0me"),
            (b"eXIf", b"MM"),
            (b"iTXt", b"XML:com.adobe.xmp"),
            (b"tIME", &[0u8; 7]),
            (b"caBX", b"c2pa"),
            (b"IDAT", b"pixels"),
            (b"IEND", b""),
        ] {
            data.extend_from_slice(&png_chunk(kind, body));
        }
        let (clean, removed) = strip_bytes(Kind::Png, &data).unwrap();
        assert_eq!(removed, 5);
        let text = String::from_utf8_lossy(&clean);
        assert!(text.contains("IHDR") && text.contains("IDAT") && text.contains("IEND"));
        assert!(!text.contains("Author") && !text.contains("xmp") && !text.contains("c2pa"));
    }

    #[test]
    fn webp_drops_exif_and_xmp_and_clears_their_flags() {
        let mut body = b"WEBP".to_vec();
        body.extend_from_slice(b"VP8X");
        body.extend_from_slice(&10u32.to_le_bytes());
        body.extend_from_slice(&[0x0C, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        body.extend_from_slice(b"VP8L");
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(&[1, 2, 3, 0]);
        body.extend_from_slice(b"EXIF");
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(b"gps!");
        body.extend_from_slice(b"XMP ");
        body.extend_from_slice(&5u32.to_le_bytes());
        body.extend_from_slice(b"xmp!!\0");
        let mut data = b"RIFF".to_vec();
        data.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        data.extend_from_slice(&body);
        let (clean, removed) = strip_bytes(Kind::Webp, &data).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(le32(&clean, 4).unwrap() + 8, clean.len());
        assert_eq!(clean[20] & 0x0C, 0);
        assert!(!String::from_utf8_lossy(&clean).contains("gps"));
    }

    #[test]
    fn gif_keeps_frames_and_loop_but_not_comments() {
        let mut data = b"GIF89a\x01\0\x01\0\x80\0\0".to_vec();
        data.extend_from_slice(&[0, 0, 0, 255, 255, 255]);
        data.extend_from_slice(b"\x21\xFF\x0BNETSCAPE2.0\x03\x01\0\0\0");
        data.extend_from_slice(b"\x21\xFE\x07comment\0");
        data.extend_from_slice(b"\x21\xFF\x0BXMP DataXMP\x03abc\0");
        data.extend_from_slice(b"\x21\xF9\x04\0\0\0\0\0");
        data.extend_from_slice(b"\x2C\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0");
        data.push(0x3B);
        let (clean, removed) = strip_bytes(Kind::Gif, &data).unwrap();
        assert_eq!(removed, 2);
        let text = String::from_utf8_lossy(&clean);
        assert!(text.contains("NETSCAPE2.0"));
        assert!(!text.contains("comment") && !text.contains("XMP"));
        assert_eq!(clean.last(), Some(&0x3B));
    }

    #[test]
    fn mp3_loses_id3v2_id3v1_and_ape() {
        let mut data = b"ID3\x04\0\0\0\0\0\x05TIT2x".to_vec();
        data.extend_from_slice(b"\xFF\xFBaudio");
        let mut ape = b"APETAGEX".to_vec();
        ape.extend_from_slice(&2000u32.to_le_bytes());
        ape.extend_from_slice(&32u32.to_le_bytes());
        ape.extend_from_slice(&[0u8; 16]);
        data.extend_from_slice(&ape);
        let mut v1 = b"TAG".to_vec();
        v1.resize(128, b'x');
        data.extend_from_slice(&v1);
        let (clean, removed) = strip_bytes(Kind::Mp3, &data).unwrap();
        assert_eq!(removed, 3);
        assert_eq!(clean, b"\xFF\xFBaudio");
    }

    fn ape_footer(size: u32, flags: u32) -> Vec<u8> {
        let mut ape = b"APETAGEX".to_vec();
        ape.extend_from_slice(&2000u32.to_le_bytes());
        ape.extend_from_slice(&size.to_le_bytes());
        ape.extend_from_slice(&0u32.to_le_bytes());
        ape.extend_from_slice(&flags.to_le_bytes());
        ape.extend_from_slice(&[0u8; 8]);
        ape
    }

    #[test]
    fn mp3_ape_footer_with_size_zero_ends_and_keeps_the_bytes() {
        let mut data = b"\xFF\xFBaudio".to_vec();
        data.extend_from_slice(&ape_footer(0, 0));
        let (clean, removed) = strip_bytes(Kind::Mp3, &data).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(clean, data);
    }

    #[test]
    fn mp3_ape_footer_larger_than_the_file_ends_and_keeps_the_bytes() {
        let mut data = b"\xFF\xFBaudio".to_vec();
        data.extend_from_slice(&ape_footer(u32::MAX, 0x8000_0000));
        let (clean, removed) = strip_bytes(Kind::Mp3, &data).unwrap();
        assert_eq!(removed, 0);
        assert_eq!(clean, data);
    }

    #[test]
    fn mp3_valid_ape_tag_with_header_is_removed() {
        let mut data = b"\xFF\xFBaudio".to_vec();
        let mut header = b"APETAGEX".to_vec();
        header.resize(32, 0);
        data.extend_from_slice(&header);
        data.extend_from_slice(b"item");
        data.extend_from_slice(&ape_footer(36, 0x8000_0000));
        let (clean, removed) = strip_bytes(Kind::Mp3, &data).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(clean, b"\xFF\xFBaudio");
    }

    #[test]
    fn flac_keeps_streaminfo_and_marks_it_last() {
        let mut data = b"fLaC".to_vec();
        data.extend_from_slice(&[0x00, 0, 0, 34]);
        data.extend_from_slice(&[7u8; 34]);
        data.extend_from_slice(&[0x04, 0, 0, 6]);
        data.extend_from_slice(b"ARTIST");
        data.extend_from_slice(&[0x86, 0, 0, 3]);
        data.extend_from_slice(b"jpg");
        data.extend_from_slice(b"\xFF\xF8frames");
        let (clean, removed) = strip_bytes(Kind::Flac, &data).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(clean[4], 0x80);
        assert!(clean.ends_with(b"\xFF\xF8frames"));
        assert!(!String::from_utf8_lossy(&clean).contains("ARTIST"));
    }

    #[test]
    fn clean_copy_never_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.jpg");
        assert_eq!(clean_path(&source), dir.path().join("photo-clean.jpg"));
        std::fs::write(dir.path().join("photo-clean.jpg"), b"x").unwrap();
        assert_eq!(clean_path(&source), dir.path().join("photo-clean-2.jpg"));
    }

    #[test]
    fn strip_file_writes_a_copy_and_leaves_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.jpg");
        let original = jpeg_with(&[&app(0xFE, b"comment")]);
        std::fs::write(&source, &original).unwrap();
        let result = strip_file(&source, None, &CancelToken::new()).unwrap();
        assert_eq!(result.removed, 1);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert!(!String::from_utf8_lossy(&std::fs::read(result.output).unwrap()).contains("comment"));
    }

    /// Cleans copies of real files and checks that ffmpeg still decodes them.
    /// Run with `WINDOWCRAFT_LIVE_STRIP=<folder>` after the YouTube downloader
    /// has fetched ffmpeg: `cargo test -p windowcraft-engine live_strip -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a folder of sample files and the fetched ffmpeg"]
    fn live_strip_keeps_files_decodable() {
        let folder = std::path::PathBuf::from(std::env::var("WINDOWCRAFT_LIVE_STRIP").expect("WINDOWCRAFT_LIVE_STRIP"));
        let ffmpeg = crate::tools::installed_ffmpeg().expect("ffmpeg");
        let dir = tempfile::tempdir().unwrap();
        let cancel = CancelToken::new();
        let mut checked = 0;
        for entry in std::fs::read_dir(folder).unwrap().filter_map(std::result::Result::ok) {
            let source = dir.path().join(entry.file_name());
            if std::fs::copy(entry.path(), &source).is_err() || kind_of(&source, &read_head(&source).unwrap()).is_none() {
                continue;
            }
            // Only what WindowCraft cannot clean itself may use ffmpeg.
            let kind = kind_of(&source, &read_head(&source).unwrap()).unwrap();
            let result = strip_file(&source, (kind == Kind::Media).then_some(ffmpeg.as_path()), &cancel).unwrap();
            let before = std::fs::metadata(&source).unwrap().len();
            let after = std::fs::metadata(&result.output).unwrap().len();
            let output = result.output.display().to_string();
            crate::process::run_ok(&ffmpeg, &["-v", "error", "-i", &output, "-f", "null", "-"], &cancel, Duration::from_secs(120))
                .unwrap_or_else(|error| panic!("{output} does not decode: {error}"));
            let clean = std::fs::read(&result.output).unwrap();
            let text = String::from_utf8_lossy(&clean);
            assert!(!text.contains("xmpmeta") && !text.contains("ns.adobe.com"), "XMP left in {output}");
            for secret in ["Secret title", "Jan Kowalski", "GPS 52.2", "+52.2297", "Director commentary"] {
                assert!(!text.contains(secret), "{secret} left in {output}");
            }
            let probe = ffmpeg.with_file_name("ffprobe.exe");
            let tags = crate::process::run_ok(&probe, &["-v", "error", "-show_entries", "format_tags:stream_tags", "-of", "compact", &output], &cancel, Duration::from_secs(60)).unwrap_or_default();
            println!("    {}", tags.replace('\n', " | "));
            assert!(!tags.contains("2024-05-06"), "creation date left in {output}");
            let orientation = clean
                .windows(6)
                .position(|window| window == b"Exif\0\0")
                .and_then(|at| exif_orientation(&clean[at + 6..]));
            println!(
                "{}: {before} -> {after} bytes, {} blocks removed, orientation {orientation:?}",
                entry.file_name().to_string_lossy(),
                result.removed
            );
            checked += 1;
        }
        assert!(checked > 0);
    }

    #[test]
    fn rare_video_without_ffmpeg_is_reported_not_guessed() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("clip.flv");
        std::fs::write(&source, b"FLV\x01\x05\0\0\0\x09").unwrap();
        assert!(matches!(strip_file(&source, None, &CancelToken::new()), Err(EngineError::NotFound(_))));
        // An MP4 without a movie box cannot be cleaned here either.
        let broken = dir.path().join("clip.mp4");
        std::fs::write(&broken, b"\0\0\0\x18ftypmp42\0\0\0\0mp42isom").unwrap();
        assert!(matches!(strip_file(&broken, None, &CancelToken::new()), Err(EngineError::NotFound(_))));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2, "no temporary file is left");
    }
}
