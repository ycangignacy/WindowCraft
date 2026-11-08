// Made by ycangignacy
//! Reading and writing ISO base media files: MP4, MOV and M4A.
//!
//! Only regular files are read; fragmented ones (`moof`) go to ffmpeg. The
//! writer puts the index (`moov`) before the data, so the file starts playing
//! before it has fully loaded, and interleaves the tracks in short chunks.

use super::{Codec, Data, Kind, Layout, Movie, Sample, SampleReader, Track};
use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// Reading

struct Atom {
    kind: [u8; 4],
    /// Payload bounds in the file.
    from: u64,
    to: u64,
}

fn atoms(file: &mut File, from: u64, to: u64) -> std::io::Result<Vec<Atom>> {
    let mut found = Vec::new();
    let mut at = from;
    while at + 8 <= to {
        file.seek(SeekFrom::Start(at))?;
        let mut head = [0u8; 16];
        file.read_exact(&mut head[..8])?;
        let small = u64::from(u32::from_be_bytes(head[..4].try_into().unwrap_or_default()));
        let kind: [u8; 4] = head[4..8].try_into().unwrap_or_default();
        let (header, size) = match small {
            0 => (8, to - at),
            1 => {
                file.read_exact(&mut head[8..16])?;
                (16, u64::from_be_bytes(head[8..16].try_into().unwrap_or_default()))
            }
            size => (8, size),
        };
        if size < header || at + size > to {
            break;
        }
        found.push(Atom { kind, from: at + header, to: at + size });
        at += size;
    }
    Ok(found)
}

fn load(file: &mut File, atom: &Atom) -> std::io::Result<Vec<u8>> {
    let len = (atom.to - atom.from).min(256 << 20) as usize;
    let mut data = vec![0u8; len];
    file.seek(SeekFrom::Start(atom.from))?;
    file.read_exact(&mut data)?;
    Ok(data)
}

/// Children of an in-memory box payload, as (kind, payload).
fn children(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap_or_default()) as usize;
        let kind: [u8; 4] = data[at + 4..at + 8].try_into().unwrap_or_default();
        let (header, size) = match size {
            0 => (8, data.len() - at),
            1 if at + 16 <= data.len() => (16, u64::from_be_bytes(data[at + 8..at + 16].try_into().unwrap_or_default()) as usize),
            size => (8, size),
        };
        if size < header || at + size > data.len() {
            break;
        }
        found.push((kind, &data[at + header..at + size]));
        at += size;
    }
    found
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(data).into_iter().find(|(found, _)| found == kind).map(|(_, payload)| payload)
}

fn path<'a>(data: &'a [u8], kinds: &[&[u8; 4]]) -> Option<&'a [u8]> {
    kinds.iter().try_fold(data, |current, kind| child(current, kind))
}

fn be16(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 2).map_or(0, |b| u32::from(u16::from_be_bytes([b[0], b[1]])))
}

fn be32(data: &[u8], at: usize) -> u32 {
    data.get(at..at + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn be64(data: &[u8], at: usize) -> u64 {
    data.get(at..at + 8).map_or(0, |b| u64::from_be_bytes(b.try_into().unwrap_or_default()))
}

/// Rows of a full-box table after version, flags and count.
fn rows(payload: &[u8], width: usize) -> impl Iterator<Item = &[u8]> {
    let count = be32(payload, 4) as usize;
    payload.get(8..).unwrap_or_default().chunks_exact(width).take(count)
}

fn language(code: u32) -> String {
    if code == 0 || code == 0x7FFF {
        return "und".to_owned();
    }
    let letters: String = [code >> 10 & 31, code >> 5 & 31, code & 31].iter().map(|value| char::from((value + 0x60) as u8)).collect();
    if letters.chars().all(|c| c.is_ascii_lowercase()) { letters } else { "und".to_owned() }
}

/// Codec, setup, width, height, sampling rate, channels and bits per sample.
type Description = (Codec, Vec<u8>, u32, u32, u32, u16, u16);

/// The codec and its setup from the first sample description.
fn describe(entry_kind: [u8; 4], entry: &[u8], kind: Kind) -> Option<Description> {
    let (codec, config_kind, extra): (Codec, &[u8; 4], usize) = match &entry_kind {
        b"avc1" | b"avc3" => (Codec::H264, b"avcC", 78),
        b"hvc1" | b"hev1" => (Codec::Hevc, b"hvcC", 78),
        b"av01" => (Codec::Av1, b"av1C", 78),
        b"vp09" => (Codec::Vp9, b"vpcC", 78),
        b"mp4a" => (Codec::Aac, b"esds", 28),
        b"Opus" => (Codec::Opus, b"dOps", 28),
        b"fLaC" => (Codec::Flac, b"dfLa", 28),
        b"ac-3" => (Codec::Ac3, b"dac3", 28),
        b"alac" => (Codec::Alac, b"alac", 28),
        b".mp3" => (Codec::Mp3, b"esds", 28),
        b"tx3g" => (Codec::Text, b"ftab", 38),
        _ => return None,
    };
    let (mut width, mut height, mut rate, mut channels, mut bits) = (0, 0, 0, 0, 0);
    if kind == Kind::Video {
        width = be16(entry, 24);
        height = be16(entry, 26);
    } else if kind == Kind::Audio {
        let version = be16(entry, 8);
        channels = be16(entry, 16) as u16;
        bits = be16(entry, 18) as u16;
        rate = be32(entry, 24) >> 16;
        // QuickTime sound descriptions grow with their version.
        let extra = match version {
            1 => 16,
            2 => 36,
            _ => 0,
        };
        return finish(codec, config_kind, &entry[(extra + 28).min(entry.len())..], 0, 0, rate, channels, bits);
    }
    finish(codec, config_kind, &entry[extra.min(entry.len())..], width, height, rate, channels, bits)
}

#[allow(clippy::too_many_arguments)]
fn finish(codec: Codec, config_kind: &[u8; 4], boxes: &[u8], width: u32, height: u32, rate: u32, channels: u16, bits: u16) -> Option<Description> {
    // QuickTime keeps the AAC descriptor inside a `wave` box.
    let config = child(boxes, config_kind).or_else(|| child(boxes, b"wave").and_then(|wave| child(wave, config_kind)));
    let (codec, config) = match codec {
        Codec::Aac | Codec::Mp3 => {
            let esds = config?;
            let (object, specific) = es_descriptor(esds.get(4..)?)?;
            match object {
                0x40 | 0x66 | 0x67 | 0x68 => (Codec::Aac, specific),
                0x69 | 0x6B => (Codec::Mp3, Vec::new()),
                _ => return None,
            }
        }
        Codec::Flac => (codec, config?.get(4..)?.to_vec()),
        Codec::Alac => (codec, config?.get(4..)?.to_vec()),
        Codec::Opus => (codec, super::dops_to_opus_head(config?)?),
        Codec::Text => (codec, Vec::new()),
        Codec::Vp9 => (codec, config?.to_vec()),
        _ => (codec, config?.to_vec()),
    };
    Some((codec, config, width, height, rate, channels, bits))
}

/// Object type and DecoderSpecificInfo from an ES_Descriptor.
fn es_descriptor(data: &[u8]) -> Option<(u8, Vec<u8>)> {
    let mut at = 0;
    let mut object = 0;
    let mut specific = Vec::new();
    while at < data.len() {
        let tag = data[at];
        at += 1;
        let mut size = 0usize;
        for _ in 0..4 {
            let byte = *data.get(at)?;
            at += 1;
            size = size << 7 | usize::from(byte & 0x7F);
            if byte & 0x80 == 0 {
                break;
            }
        }
        match tag {
            // ES_Descriptor: ES_ID, flags, then optional fields.
            3 => {
                let flags = *data.get(at + 2)?;
                at += 3;
                if flags & 0x80 != 0 {
                    at += 2;
                }
                if flags & 0x40 != 0 {
                    at += 1 + usize::from(*data.get(at)?);
                }
                if flags & 0x20 != 0 {
                    at += 2;
                }
            }
            4 => {
                object = *data.get(at)?;
                at += 13;
            }
            5 => {
                specific = data.get(at..at + size)?.to_vec();
                at += size;
            }
            _ => at += size,
        }
    }
    Some((object, specific))
}

pub(super) fn read(source: &Path) -> Result<Option<Movie>> {
    let name = source.display().to_string();
    let io = |e: std::io::Error| EngineError::io(name.clone(), e);
    let mut file = File::open(source).map_err(io)?;
    let len = file.metadata().map_err(io)?.len();
    let top = atoms(&mut file, 0, len).map_err(io)?;
    if top.iter().any(|atom| &atom.kind == b"moof") {
        return Ok(None);
    }
    let Some(moov) = top.iter().find(|atom| &atom.kind == b"moov") else { return Ok(None) };
    let brand = top.iter().find(|atom| &atom.kind == b"ftyp").map(|atom| load(&mut file, atom)).transpose().map_err(io)?.unwrap_or_default();
    let layout = if brand.starts_with(b"qt  ") { Layout::Mov } else { Layout::Mp4 };
    let movie = load(&mut file, moov).map_err(io)?;
    let mut tracks = Vec::new();
    for (kind, trak) in children(&movie) {
        if &kind != b"trak" {
            continue;
        }
        let Some(mut track) = read_track(trak) else {
            // A stream that cannot be carried over sends the file to ffmpeg,
            // except data tracks such as timecode, which are dropped.
            if track_is_media(trak) {
                return Ok(None);
            }
            continue;
        };
        if track.kind == Kind::Text {
            // tx3g samples hold a 16-bit length, the text, then style boxes.
            let mut kept = Vec::with_capacity(track.samples.len());
            for sample in track.samples.drain(..) {
                let Data::File { offset, size } = sample.data else { continue };
                let mut bytes = vec![0u8; size as usize];
                file.seek(SeekFrom::Start(offset)).map_err(io)?;
                file.read_exact(&mut bytes).map_err(io)?;
                let length = be16(&bytes, 0) as usize;
                let text = bytes.get(2..2 + length).unwrap_or_default();
                if !text.is_empty() {
                    kept.push(Sample { data: Data::Bytes(text.to_vec()), ..sample });
                }
            }
            track.samples = kept;
        }
        tracks.push(track);
    }
    Ok(Some(Movie { tracks, layout }))
}

fn handler(trak: &[u8]) -> [u8; 4] {
    path(trak, &[b"mdia", b"hdlr"]).and_then(|hdlr| hdlr.get(8..12)).and_then(|kind| kind.try_into().ok()).unwrap_or_default()
}

fn track_is_media(trak: &[u8]) -> bool {
    matches!(&handler(trak), b"vide" | b"soun" | b"sbtl" | b"subt" | b"text")
}

fn read_track(trak: &[u8]) -> Option<Track> {
    let kind = match &handler(trak) {
        b"vide" => Kind::Video,
        b"soun" => Kind::Audio,
        b"sbtl" | b"subt" | b"text" => Kind::Text,
        _ => return None,
    };
    let mdhd = path(trak, &[b"mdia", b"mdhd"])?;
    let (timescale, language_code) = if mdhd.first() == Some(&1) { (be32(mdhd, 20), be16(mdhd, 32)) } else { (be32(mdhd, 12), be16(mdhd, 20)) };
    let table = path(trak, &[b"mdia", b"minf", b"stbl"])?;
    let stsd = child(table, b"stsd")?;
    let entries = children(stsd.get(8..)?);
    let (entry_kind, entry) = *entries.first()?;
    let (codec, config, width, height, rate, channels, bits) = describe(entry_kind, entry, kind)?;
    let mut track = Track::new(kind, codec);
    track.config = config;
    track.width = width;
    track.height = height;
    track.channels = channels;
    track.bits = bits;
    track.rate = rate;
    track.language = language(language_code);
    track.timescale = timescale.max(1);
    if codec == Codec::Aac
        && let Some((rate, channels)) = super::aac_rate_channels(&track.config)
    {
        track.rate = rate;
        track.channels = channels.max(track.channels.min(8));
    }

    // Sample tables.
    let sizes = child(table, b"stsz")?;
    let fixed = be32(sizes, 4);
    let count = be32(sizes, 8) as usize;
    let size_of: Vec<u32> =
        if fixed == 0 { sizes.get(12..)?.as_chunks::<4>().0.iter().take(count).map(|b| u32::from_be_bytes(*b)).collect() } else { vec![fixed; count] };
    let offsets: Vec<u64> = if let Some(stco) = child(table, b"stco") {
        rows(stco, 4).map(|row| u64::from(be32(row, 0))).collect()
    } else {
        rows(child(table, b"co64")?, 8).map(|row| be64(row, 0)).collect()
    };
    let runs: Vec<(u32, u32)> = rows(child(table, b"stsc")?, 12).map(|row| (be32(row, 0), be32(row, 4))).collect();
    let mut deltas: Vec<u32> = Vec::with_capacity(count);
    for row in rows(child(table, b"stts")?, 8) {
        deltas.extend(std::iter::repeat_n(be32(row, 4), be32(row, 0) as usize));
    }
    let mut shifts: Vec<i64> = Vec::new();
    if let Some(ctts) = child(table, b"ctts") {
        let signed = ctts.first() == Some(&1);
        for row in rows(ctts, 8) {
            let raw = be32(row, 4);
            let value = if signed { i64::from(raw as i32) } else { i64::from(raw) };
            shifts.extend(std::iter::repeat_n(value, be32(row, 0) as usize));
        }
    }
    let keys: Option<Vec<u32>> = child(table, b"stss").map(|stss| rows(stss, 4).map(|row| be32(row, 0)).collect());
    // A single edit that skips the start (B-frame delay, AAC priming).
    let skip = path(trak, &[b"edts", b"elst"])
        .and_then(|elst| {
            let version = *elst.first()?;
            let entries: Vec<&[u8]> = rows(elst, if version == 1 { 20 } else { 12 }).collect();
            let entry = entries.iter().find(|entry| if version == 1 { be64(entry, 8) as i64 >= 0 } else { be32(entry, 4) as i32 >= 0 })?;
            Some(if version == 1 { be64(entry, 8) as i64 } else { i64::from(be32(entry, 4) as i32) })
        })
        .unwrap_or(0);

    let mut samples = Vec::with_capacity(count);
    let mut dts = 0i64;
    let mut index = 0usize;
    for (chunk, offset) in offsets.iter().enumerate() {
        let number = chunk as u32 + 1;
        let per_chunk = runs.iter().rev().find(|(first, _)| *first <= number).map_or(0, |(_, count)| *count);
        let mut at = *offset;
        for _ in 0..per_chunk {
            let Some(&size) = size_of.get(index) else { break };
            let delta = deltas.get(index).copied().unwrap_or(0);
            let shift = shifts.get(index).copied().unwrap_or(0);
            let key = keys.as_ref().is_none_or(|keys| keys.binary_search(&(index as u32 + 1)).is_ok());
            samples.push(Sample { data: Data::File { offset: at, size }, pts: dts + shift - skip, duration: delta, key });
            at += u64::from(size);
            dts += i64::from(delta);
            index += 1;
        }
    }
    if samples.len() != count {
        return None;
    }
    track.samples = samples;
    Some(track)
}

// ---------------------------------------------------------------------------
// Writing

fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn full(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![version];
    body.extend_from_slice(&flags.to_be_bytes()[1..]);
    body.extend_from_slice(payload);
    boxed(kind, &body)
}

fn descriptor(tag: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = payload.len();
    out.extend_from_slice(&[0x80 | (len >> 21 & 0x7F) as u8, 0x80 | (len >> 14 & 0x7F) as u8, 0x80 | (len >> 7 & 0x7F) as u8, (len & 0x7F) as u8]);
    out.extend_from_slice(payload);
    out
}

fn esds(object: u8, specific: &[u8], id: u16) -> Vec<u8> {
    let mut config = vec![object, 0x15, 0, 0, 0];
    config.extend_from_slice(&[0; 8]);
    if !specific.is_empty() {
        config.extend_from_slice(&descriptor(5, specific));
    }
    let mut es = id.to_be_bytes().to_vec();
    es.push(0);
    es.extend_from_slice(&descriptor(4, &config));
    es.extend_from_slice(&descriptor(6, &[2]));
    full(b"esds", 0, 0, &descriptor(3, &es))
}

fn language_code(language: &str) -> u16 {
    let letters: Vec<u8> = language.bytes().collect();
    if letters.len() != 3 || !letters.iter().all(u8::is_ascii_lowercase) {
        return 0x55C4;
    }
    letters.iter().fold(0u16, |code, letter| code << 5 | u16::from(letter - 0x60))
}

fn sample_entry(track: &Track, id: u16) -> Option<Vec<u8>> {
    match track.kind {
        Kind::Video => {
            let (kind, config_kind): (&[u8; 4], &[u8; 4]) = match track.codec {
                Codec::H264 => (b"avc1", b"avcC"),
                Codec::Hevc => (b"hvc1", b"hvcC"),
                Codec::Av1 => (b"av01", b"av1C"),
                Codec::Vp9 => (b"vp09", b"vpcC"),
                _ => return None,
            };
            let mut body = vec![0u8; 6];
            body.extend_from_slice(&1u16.to_be_bytes());
            body.extend_from_slice(&[0; 16]);
            body.extend_from_slice(&(track.width as u16).to_be_bytes());
            body.extend_from_slice(&(track.height as u16).to_be_bytes());
            body.extend_from_slice(&0x0048_0000u32.to_be_bytes());
            body.extend_from_slice(&0x0048_0000u32.to_be_bytes());
            body.extend_from_slice(&[0; 4]);
            body.extend_from_slice(&1u16.to_be_bytes());
            body.extend_from_slice(&[0; 32]);
            body.extend_from_slice(&0x0018u16.to_be_bytes());
            body.extend_from_slice(&0xFFFFu16.to_be_bytes());
            // The vpcC payload already starts with its version and flags.
            body.extend_from_slice(&boxed(config_kind, &track.config));
            Some(boxed(kind, &body))
        }
        Kind::Audio => {
            let (kind, config) = match track.codec {
                Codec::Aac => (b"mp4a", esds(0x40, &track.config, id)),
                Codec::Mp3 => (b"mp4a", esds(0x6B, &[], id)),
                Codec::Opus => (b"Opus", boxed(b"dOps", &super::opus_head_to_dops(&track.config)?)),
                Codec::Flac => (b"fLaC", full(b"dfLa", 0, 0, &track.config)),
                Codec::Ac3 => (b"ac-3", boxed(b"dac3", &track.config)),
                Codec::Alac => (b"alac", full(b"alac", 0, 0, &track.config)),
                _ => return None,
            };
            let mut body = vec![0u8; 6];
            body.extend_from_slice(&1u16.to_be_bytes());
            body.extend_from_slice(&[0; 8]);
            body.extend_from_slice(&track.channels.max(1).to_be_bytes());
            body.extend_from_slice(&track.bits.clamp(16, 32).to_be_bytes());
            body.extend_from_slice(&[0; 4]);
            let rate = if track.rate > 0xFFFF { 0 } else { track.rate };
            body.extend_from_slice(&(rate << 16).to_be_bytes());
            body.extend_from_slice(&config);
            Some(boxed(kind, &body))
        }
        Kind::Text => {
            let mut body = vec![0u8; 6];
            body.extend_from_slice(&1u16.to_be_bytes());
            // The same default style ffmpeg writes, so readers add no font tags.
            body.extend_from_slice(&[0, 0, 0, 0, 1, 0xFF, 0, 0, 0, 0xFF]);
            body.extend_from_slice(&[0; 8]);
            body.extend_from_slice(&[0, 0, 0, 0, 0, 1, 0, 0x10, 0xFF, 0xFF, 0xFF, 0xFF]);
            body.extend_from_slice(&boxed(b"ftab", &[0, 1, 0, 1, 5, b'A', b'r', b'i', b'a', b'l']));
            Some(boxed(b"tx3g", &body))
        }
    }
}

/// Run-length rows (count, value).
fn runs(values: impl Iterator<Item = u32>) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for value in values {
        match out.last_mut() {
            Some((count, last)) if *last == value => *count += 1,
            _ => out.push((1, value)),
        }
    }
    out
}

fn table(kind: &[u8; 4], version: u8, rows: &[(u32, u32)]) -> Vec<u8> {
    let mut body = (rows.len() as u32).to_be_bytes().to_vec();
    for (count, value) in rows {
        body.extend_from_slice(&count.to_be_bytes());
        body.extend_from_slice(&value.to_be_bytes());
    }
    full(kind, version, 0, &body)
}

/// One track as prepared for writing.
struct Plan<'a> {
    track: &'a Track,
    /// Samples in decode order; text samples already in tx3g form.
    samples: Vec<Sample>,
    /// Decode time of each sample in track ticks, starting at 0.
    dts: Vec<i64>,
    /// Presentation minus decode time of each sample.
    offsets: Vec<i64>,
    /// Where presentation starts in the media timeline, for the edit list.
    media_time: i64,
    /// Presentation start relative to the movie, in seconds.
    start: f64,
}

fn plan(track: &Track, origin: f64) -> Plan<'_> {
    let mut samples = track.samples.clone();
    if track.kind == Kind::Text {
        // Text tracks play back to back: gaps become empty samples.
        let mut filled = Vec::with_capacity(samples.len() * 2);
        let mut clock = samples.first().map_or(0, |sample| sample.pts);
        for sample in samples {
            if sample.pts > clock {
                filled.push(Sample { data: Data::Bytes(vec![0, 0]), pts: clock, duration: (sample.pts - clock) as u32, key: true });
            }
            let mut bytes = (sample.data.len() as u16).to_be_bytes().to_vec();
            if let Data::Bytes(text) = &sample.data {
                bytes.extend_from_slice(text);
            }
            clock = clock.max(sample.pts + i64::from(sample.duration));
            filled.push(Sample { data: Data::Bytes(bytes), ..sample });
        }
        samples = filled;
    }
    let raw: Vec<i64> = if track.kind == Kind::Video {
        super::decode_times(&samples)
    } else {
        samples.iter().map(|sample| sample.pts).collect()
    };
    let base = raw.first().copied().unwrap_or(0);
    let offsets: Vec<i64> = samples.iter().zip(&raw).map(|(sample, dts)| sample.pts - dts).collect();
    let dts: Vec<i64> = raw.iter().map(|value| value - base).collect();
    let first_pts = samples.iter().map(|sample| sample.pts).min().unwrap_or(0);
    // Media before the movie origin, such as AAC priming, is skipped by the edit.
    let early = ((origin - track.seconds(first_pts)).max(0.0) * f64::from(track.timescale)).round() as i64;
    Plan { track, samples, dts, offsets, media_time: first_pts - base + early, start: (track.seconds(first_pts) - origin).max(0.0) }
}

const MOVIE_TIMESCALE: u32 = 1000;

impl Plan<'_> {
    /// Sample lengths in decode order; the last keeps its own length.
    fn deltas(&self) -> Vec<u32> {
        let mut deltas: Vec<u32> = self.dts.windows(2).map(|pair| (pair[1] - pair[0]).max(0) as u32).collect();
        let last = self.samples.last().map_or(0, |sample| sample.duration);
        deltas.push(if last > 0 { last } else { deltas.last().copied().unwrap_or(0) });
        deltas
    }

    fn media_duration(&self) -> u64 {
        self.deltas().iter().map(|delta| u64::from(*delta)).sum()
    }

    /// Length as shown, in movie ticks, without the late start.
    fn shown(&self) -> u64 {
        let ticks = self.media_duration() as i64 - self.media_time;
        (self.track.seconds(ticks.max(0)) * f64::from(MOVIE_TIMESCALE)).round() as u64
    }

    fn late(&self) -> u64 {
        (self.start * f64::from(MOVIE_TIMESCALE)).round() as u64
    }
}

fn track_box(plan: &Plan, id: u32, layout: Layout, chunk_offsets: &[u64], chunk_sizes: &[u32], large: bool) -> Option<Vec<u8>> {
    let track = plan.track;
    let samples = &plan.samples;
    let deltas = plan.deltas();
    let media_duration = plan.media_duration();
    let movie_duration = plan.late() + plan.shown();

    let mut stbl = Vec::new();
    let mut stsd = 1u32.to_be_bytes().to_vec();
    stsd.extend_from_slice(&sample_entry(track, id as u16)?);
    stbl.extend_from_slice(&full(b"stsd", 0, 0, &stsd));
    stbl.extend_from_slice(&table(b"stts", 0, &runs(deltas.iter().copied())));
    if plan.offsets.iter().any(|offset| *offset != 0) {
        let negative = plan.offsets.iter().any(|offset| *offset < 0);
        let rows = runs(plan.offsets.iter().map(|offset| *offset as i32 as u32));
        stbl.extend_from_slice(&table(b"ctts", u8::from(negative), &rows));
    }
    if track.kind == Kind::Video && samples.iter().any(|sample| !sample.key) {
        let keys: Vec<u32> = samples.iter().enumerate().filter(|(_, sample)| sample.key).map(|(index, _)| index as u32 + 1).collect();
        let mut body = (keys.len() as u32).to_be_bytes().to_vec();
        for key in keys {
            body.extend_from_slice(&key.to_be_bytes());
        }
        stbl.extend_from_slice(&full(b"stss", 0, 0, &body));
    }
    // Chunk runs: first chunk, samples per chunk, description 1.
    let mut stsc_rows: Vec<(u32, u32)> = Vec::new();
    for (index, per_chunk) in chunk_sizes.iter().enumerate() {
        if stsc_rows.last().is_none_or(|(_, last)| last != per_chunk) {
            stsc_rows.push((index as u32 + 1, *per_chunk));
        }
    }
    let mut stsc = (stsc_rows.len() as u32).to_be_bytes().to_vec();
    for (first, per_chunk) in stsc_rows {
        stsc.extend_from_slice(&first.to_be_bytes());
        stsc.extend_from_slice(&per_chunk.to_be_bytes());
        stsc.extend_from_slice(&1u32.to_be_bytes());
    }
    stbl.extend_from_slice(&full(b"stsc", 0, 0, &stsc));
    let mut stsz = 0u32.to_be_bytes().to_vec();
    stsz.extend_from_slice(&(samples.len() as u32).to_be_bytes());
    for sample in samples {
        stsz.extend_from_slice(&sample.data.len().to_be_bytes());
    }
    stbl.extend_from_slice(&full(b"stsz", 0, 0, &stsz));
    let mut stco = (chunk_offsets.len() as u32).to_be_bytes().to_vec();
    for offset in chunk_offsets {
        if large {
            stco.extend_from_slice(&offset.to_be_bytes());
        } else {
            stco.extend_from_slice(&(*offset as u32).to_be_bytes());
        }
    }
    stbl.extend_from_slice(&full(if large { b"co64" } else { b"stco" }, 0, 0, &stco));

    let (handler, header): (&[u8; 4], Vec<u8>) = match track.kind {
        Kind::Video => (b"vide", full(b"vmhd", 0, 1, &[0; 8])),
        Kind::Audio => (b"soun", full(b"smhd", 0, 0, &[0; 4])),
        Kind::Text if layout == Layout::Mov => (b"sbtl", full(b"nmhd", 0, 0, &[])),
        Kind::Text => (b"text", full(b"nmhd", 0, 0, &[])),
    };
    let dinf = boxed(b"dinf", &full(b"dref", 0, 0, &[&1u32.to_be_bytes()[..], &full(b"url ", 0, 1, &[])].concat()));
    let minf = boxed(b"minf", &[header, dinf, boxed(b"stbl", &stbl)].concat());
    let mut mdhd = vec![0u8; 8];
    mdhd.extend_from_slice(&track.timescale.to_be_bytes());
    mdhd.extend_from_slice(&(media_duration.min(u64::from(u32::MAX)) as u32).to_be_bytes());
    mdhd.extend_from_slice(&language_code(&track.language).to_be_bytes());
    mdhd.extend_from_slice(&[0, 0]);
    let mut hdlr = vec![0u8; 4];
    hdlr.extend_from_slice(handler);
    hdlr.extend_from_slice(&[0; 12]);
    hdlr.push(0);
    let mdia = boxed(b"mdia", &[full(b"mdhd", 0, 0, &mdhd), full(b"hdlr", 0, 0, &hdlr), minf].concat());

    let mut tkhd = vec![0u8; 8];
    tkhd.extend_from_slice(&id.to_be_bytes());
    tkhd.extend_from_slice(&[0; 4]);
    tkhd.extend_from_slice(&(movie_duration.min(u64::from(u32::MAX)) as u32).to_be_bytes());
    tkhd.extend_from_slice(&[0; 12]);
    tkhd.extend_from_slice(&(if track.kind == Kind::Audio { 0x0100u16 } else { 0 }).to_be_bytes());
    tkhd.extend_from_slice(&[0, 0]);
    for value in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        tkhd.extend_from_slice(&value.to_be_bytes());
    }
    tkhd.extend_from_slice(&(track.width << 16).to_be_bytes());
    tkhd.extend_from_slice(&(track.height << 16).to_be_bytes());

    // Edit list: an empty edit for a late start, then the media from the
    // first sample that is shown.
    let mut edits: Vec<(u32, i32)> = Vec::new();
    if plan.late() > 0 {
        edits.push((plan.late() as u32, -1));
    }
    edits.push((plan.shown().min(u64::from(u32::MAX)) as u32, plan.media_time.clamp(0, i64::from(i32::MAX)) as i32));
    let mut elst = (edits.len() as u32).to_be_bytes().to_vec();
    for (duration, media_time) in edits {
        elst.extend_from_slice(&duration.to_be_bytes());
        elst.extend_from_slice(&media_time.to_be_bytes());
        elst.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    }
    let edts = boxed(b"edts", &full(b"elst", 0, 0, &elst));
    Some(boxed(b"trak", &[full(b"tkhd", 0, 3, &tkhd), edts, mdia].concat()))
}

fn file_type(movie: &Movie, layout: Layout) -> Vec<u8> {
    let (major, minor, mut compatible): (&[u8; 4], u32, Vec<&[u8; 4]>) = match layout {
        Layout::Mov => (b"qt  ", 0x2005_0300, vec![b"qt  "]),
        Layout::M4a => (b"M4A ", 0, vec![b"M4A ", b"mp42", b"isom"]),
        _ => (b"isom", 512, vec![b"isom", b"iso2", b"mp41"]),
    };
    if layout == Layout::Mp4 {
        if movie.tracks.iter().any(|track| track.codec == Codec::H264) {
            compatible.push(b"avc1");
        }
        if movie.tracks.iter().any(|track| track.codec == Codec::Av1) {
            compatible.push(b"av01");
        }
    }
    let mut body = major.to_vec();
    body.extend_from_slice(&minor.to_be_bytes());
    for brand in compatible {
        body.extend_from_slice(brand);
    }
    boxed(b"ftyp", &body)
}

/// Builds the whole `moov` for the given chunk layout.
fn movie_box(plans: &[Plan], layout: Layout, chunks: &[Vec<(u64, u32)>], large: bool) -> Result<Vec<u8>> {
    let duration = plans.iter().map(|plan| plan.late() + plan.shown()).max().unwrap_or(0);
    let mut mvhd = vec![0u8; 8];
    mvhd.extend_from_slice(&MOVIE_TIMESCALE.to_be_bytes());
    mvhd.extend_from_slice(&(duration.min(u64::from(u32::MAX)) as u32).to_be_bytes());
    mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
    mvhd.extend_from_slice(&[0; 10]);
    for value in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        mvhd.extend_from_slice(&value.to_be_bytes());
    }
    mvhd.extend_from_slice(&[0; 24]);
    mvhd.extend_from_slice(&(plans.len() as u32 + 1).to_be_bytes());
    let mut body = full(b"mvhd", 0, 0, &mvhd);
    for (index, plan) in plans.iter().enumerate() {
        let offsets: Vec<u64> = chunks[index].iter().map(|(offset, _)| *offset).collect();
        let sizes: Vec<u32> = chunks[index].iter().map(|(_, count)| *count).collect();
        let trak = track_box(plan, index as u32 + 1, layout, &offsets, &sizes, large)
            .ok_or_else(|| EngineError::Unsupported("a stream MP4 cannot hold".to_owned()))?;
        body.extend_from_slice(&trak);
    }
    Ok(boxed(b"moov", &body))
}

/// Seconds of media per interleaved chunk.
const CHUNK_SECONDS: f64 = 0.5;

pub(super) fn write(movie: &Movie, reader: &mut SampleReader, out: &mut impl Write, layout: Layout, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<()> {
    let origin = super::origin(movie);
    let plans: Vec<Plan> = movie.tracks.iter().filter(|track| !track.samples.is_empty()).map(|track| plan(track, origin)).collect();

    // Interleave: each time slice holds one chunk per track.
    let mut order: Vec<(usize, std::ops::Range<usize>)> = Vec::new();
    let mut next = vec![0usize; plans.len()];
    let mut slice = 1u32;
    while next.iter().zip(&plans).any(|(position, plan)| *position < plan.samples.len()) {
        let end = f64::from(slice) * CHUNK_SECONDS;
        for (index, plan) in plans.iter().enumerate() {
            let from = next[index];
            let mut to = from;
            while to < plan.samples.len() && plan.track.seconds(plan.dts[to]) + plan.start < end {
                to += 1;
            }
            if to > from {
                order.push((index, from..to));
                next[index] = to;
            }
        }
        slice += 1;
    }
    let chunk_bytes = |index: usize, range: &std::ops::Range<usize>| -> u64 {
        plans[index].samples[range.clone()].iter().map(|sample| u64::from(sample.data.len())).sum()
    };
    let total: u64 = order.iter().map(|(index, range)| chunk_bytes(*index, range)).sum();
    let large = total > u64::from(u32::MAX) - (64 << 20);

    let layout_chunks = |base: u64| -> Vec<Vec<(u64, u32)>> {
        let mut chunks = vec![Vec::new(); plans.len()];
        let mut at = base;
        for (index, range) in &order {
            chunks[*index].push((at, range.len() as u32));
            at += chunk_bytes(*index, range);
        }
        chunks
    };
    let ftyp = file_type(movie, layout);
    let mdat_header = if large { 16 } else { 8 };
    let probe = movie_box(&plans, layout, &layout_chunks(0), large)?;
    let base = (ftyp.len() + probe.len() + mdat_header) as u64;
    let moov = movie_box(&plans, layout, &layout_chunks(base), large)?;
    debug_assert_eq!(moov.len(), probe.len());

    let fail = |e: std::io::Error| EngineError::io("output", e);
    out.write_all(&ftyp).map_err(fail)?;
    out.write_all(&moov).map_err(fail)?;
    if large {
        out.write_all(&1u32.to_be_bytes()).map_err(fail)?;
        out.write_all(b"mdat").map_err(fail)?;
        out.write_all(&(total + 16).to_be_bytes()).map_err(fail)?;
    } else {
        out.write_all(&((total + 8) as u32).to_be_bytes()).map_err(fail)?;
        out.write_all(b"mdat").map_err(fail)?;
    }
    let mut buffer = Vec::new();
    let mut written = 0u64;
    for (count, (index, range)) in order.iter().enumerate() {
        if count % 16 == 0 {
            cancel.check()?;
            progress(written as f32 / total.max(1) as f32);
        }
        for sample in &plans[*index].samples[range.clone()] {
            reader.read(&sample.data, &mut buffer)?;
            out.write_all(&buffer).map_err(fail)?;
            written += buffer.len() as u64;
        }
    }
    Ok(())
}
