// Made by WindowCraft Dev Team
//! Reading and writing Matroska and WebM.
//!
//! The reader walks clusters header by header and records where each frame
//! is, without loading the video. The writer lays frames out in clusters that
//! start at video key frames, adds cues so players can seek, and writes no
//! title, date or encoder version.

use super::{Codec, Data, Kind, Layout, Movie, Sample, SampleReader, Track};
use crate::containers::{ebml_id, ebml_size};
use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

const EBML: u32 = 0x1A45_DFA3;
const DOC_TYPE: u32 = 0x4282;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2AD7B1;
const DURATION: u32 = 0x4489;
const MUXING_APP: u32 = 0x4D80;
const WRITING_APP: u32 = 0x5741;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_UID: u32 = 0x73C5;
const TRACK_TYPE: u32 = 0x83;
const FLAG_LACING: u32 = 0x9C;
const CODEC_ID: u32 = 0x86;
const CODEC_PRIVATE: u32 = 0x63A2;
const CODEC_DELAY: u32 = 0x56AA;
const SEEK_PRE_ROLL: u32 = 0x56BB;
const DEFAULT_DURATION: u32 = 0x23E383;
const LANGUAGE: u32 = 0x22B59C;
const CONTENT_ENCODINGS: u32 = 0x6D80;
const VIDEO: u32 = 0xE0;
const PIXEL_WIDTH: u32 = 0xB0;
const PIXEL_HEIGHT: u32 = 0xBA;
const AUDIO: u32 = 0xE1;
const SAMPLING_FREQUENCY: u32 = 0xB5;
const CHANNELS: u32 = 0x9F;
const BIT_DEPTH: u32 = 0x6264;
const CLUSTER: u32 = 0x1F43_B675;
const TIMESTAMP: u32 = 0xE7;
const SIMPLE_BLOCK: u32 = 0xA3;
const BLOCK_GROUP: u32 = 0xA0;
const BLOCK: u32 = 0xA1;
const BLOCK_DURATION: u32 = 0x9B;
const REFERENCE_BLOCK: u32 = 0xFB;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CUE_CLUSTER_POSITION: u32 = 0xF1;
#[cfg(test)]
const VOID: u32 = 0xEC;
/// Elements that can follow a cluster at the top level.
const TOP_LEVEL: [u32; 8] = [CLUSTER, CUES, SEEK_HEAD, INFO, TRACKS, 0x1254_C367, 0x1043_A770, 0x1941_A469];

// ---------------------------------------------------------------------------
// Reading

struct Cursor {
    inner: BufReader<File>,
    position: u64,
    len: u64,
}

impl Cursor {
    fn seek(&mut self, to: u64) -> std::io::Result<()> {
        if to != self.position {
            self.inner.seek_relative(to as i64 - self.position as i64)?;
            self.position = to;
        }
        Ok(())
    }

    fn bytes(&mut self, count: usize) -> std::io::Result<Vec<u8>> {
        let mut buffer = vec![0u8; count];
        self.inner.read_exact(&mut buffer)?;
        self.position += count as u64;
        Ok(buffer)
    }

    /// Reads an element header: ID, payload size (`None` when unknown) and
    /// the payload start.
    fn header(&mut self) -> std::io::Result<Option<(u32, Option<u64>, u64)>> {
        if self.position + 2 > self.len {
            return Ok(None);
        }
        let available = (self.len - self.position).min(12) as usize;
        let start = self.position;
        let head = self.bytes(available)?;
        let Some((id, id_len)) = ebml_id(&head) else { return Ok(None) };
        let Some((size, size_len, unknown)) = ebml_size(&head[id_len..]) else { return Ok(None) };
        let payload = start + (id_len + size_len) as u64;
        self.seek(payload)?;
        Ok(Some((id, (!unknown).then_some(size), payload)))
    }
}

/// Children of an in-memory master as (id, payload).
fn elements(data: &[u8]) -> Vec<(u32, &[u8])> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let Some((id, id_len)) = ebml_id(&data[at..]) else { break };
        let Some((size, size_len, unknown)) = ebml_size(&data[at + id_len..]) else { break };
        let start = at + id_len + size_len;
        let end = if unknown { data.len() } else { start.saturating_add(size as usize) };
        if end > data.len() {
            break;
        }
        found.push((id, &data[start..end]));
        at = end;
    }
    found
}

fn uint(data: &[u8]) -> u64 {
    data.iter().take(8).fold(0, |value, byte| value << 8 | u64::from(*byte))
}

fn float(data: &[u8]) -> f64 {
    match data.len() {
        4 => f64::from(f32::from_be_bytes(data.try_into().unwrap_or_default())),
        8 => f64::from_be_bytes(data.try_into().unwrap_or_default()),
        _ => 0.0,
    }
}

fn text(data: &[u8]) -> String {
    String::from_utf8_lossy(data).trim_end_matches('\0').to_owned()
}

/// A track as declared, before its frames are known.
struct Declared {
    number: u64,
    track: Track,
    default_duration: u64,
}

fn declare(entry: &[u8]) -> std::result::Result<Option<Declared>, ()> {
    let fields = elements(entry);
    let get = |id: u32| fields.iter().find(|(field, _)| *field == id).map(|(_, value)| *value);
    if get(CONTENT_ENCODINGS).is_some() {
        // Compressed or encrypted frames are not copied as they are.
        return Err(());
    }
    let number = get(TRACK_NUMBER).map_or(0, uint);
    let kind = match get(TRACK_TYPE).map_or(0, uint) {
        1 => Kind::Video,
        2 => Kind::Audio,
        17 => Kind::Text,
        _ => return Ok(None),
    };
    let codec_id = get(CODEC_ID).map(text).unwrap_or_default();
    let private = get(CODEC_PRIVATE).unwrap_or_default().to_vec();
    let codec = match codec_id.as_str() {
        "V_MPEG4/ISO/AVC" => Codec::H264,
        "V_MPEGH/ISO/HEVC" => Codec::Hevc,
        "V_AV1" => Codec::Av1,
        "V_VP9" => Codec::Vp9,
        "V_VP8" => Codec::Vp8,
        "A_MPEG/L3" => Codec::Mp3,
        "A_OPUS" => Codec::Opus,
        "A_VORBIS" => Codec::Vorbis,
        "A_FLAC" => Codec::Flac,
        "A_AC3" => Codec::Ac3,
        "A_ALAC" => Codec::Alac,
        "S_TEXT/UTF8" => Codec::Text,
        id if id.starts_with("A_AAC") => Codec::Aac,
        _ if kind == Kind::Text => Codec::OtherText,
        _ => return Err(()),
    };
    let mut track = Track::new(kind, codec);
    track.language = get(LANGUAGE).map_or_else(|| "eng".to_owned(), text);
    if let Some(video) = get(VIDEO) {
        let video = elements(video);
        let field = |id: u32| video.iter().find(|(field, _)| *field == id).map_or(0, |(_, value)| uint(value) as u32);
        track.width = field(PIXEL_WIDTH);
        track.height = field(PIXEL_HEIGHT);
    }
    if let Some(audio) = get(AUDIO) {
        let audio = elements(audio);
        let field = |id: u32| audio.iter().find(|(field, _)| *field == id).map(|(_, value)| *value);
        track.rate = field(SAMPLING_FREQUENCY).map_or(8000.0, float).round() as u32;
        track.channels = field(CHANNELS).map_or(1, uint) as u16;
        track.bits = field(BIT_DEPTH).map_or(0, uint) as u16;
    }
    track.config = match codec {
        Codec::Flac => private.strip_prefix(b"fLaC").unwrap_or(&private).to_vec(),
        Codec::Aac if private.is_empty() => super::aac_config(track.rate, track.channels),
        _ => private,
    };
    if codec == Codec::Aac
        && let Some((rate, channels)) = super::aac_rate_channels(&track.config)
    {
        track.rate = rate;
        if channels > 0 {
            track.channels = channels;
        }
    }
    if codec == Codec::Opus {
        track.rate = 48_000;
    }
    Ok(Some(Declared { number, track, default_duration: get(DEFAULT_DURATION).map_or(0, uint) }))
}

/// Sizes of the frames in a laced block, from the bytes after its flags.
fn lace_sizes(lacing: u8, body: &[u8], total: usize) -> Option<(Vec<usize>, usize)> {
    let count = usize::from(*body.first()?) + 1;
    let mut at = 1;
    let mut sizes = Vec::with_capacity(count);
    match lacing {
        // Xiph: each size is a run of 255s and a final byte.
        1 => {
            for _ in 0..count - 1 {
                let mut size = 0;
                loop {
                    let byte = *body.get(at)?;
                    at += 1;
                    size += usize::from(byte);
                    if byte != 255 {
                        break;
                    }
                }
                sizes.push(size);
            }
        }
        // EBML: a size, then signed differences.
        3 => {
            let (first, len, _) = ebml_size(body.get(at..)?)?;
            at += len;
            sizes.push(first as usize);
            for _ in 1..count - 1 {
                let (raw, len, _) = ebml_size(body.get(at..)?)?;
                at += len;
                let bias = (1i64 << (7 * len - 1)) - 1;
                let next = *sizes.last()? as i64 + raw as i64 - bias;
                sizes.push(usize::try_from(next).ok()?);
            }
        }
        // Fixed: equal parts.
        2 => {
            let each = total.checked_sub(at)? / count;
            sizes.extend(std::iter::repeat_n(each, count - 1));
        }
        _ => return None,
    }
    let used: usize = sizes.iter().sum();
    sizes.push(total.checked_sub(at + used)?);
    Some((sizes, at))
}

pub(super) fn read(source: &Path) -> Result<Option<Movie>> {
    let name = source.display().to_string();
    let io = |e: std::io::Error| EngineError::io(name.clone(), e);
    let file = File::open(source).map_err(io)?;
    let len = file.metadata().map_err(io)?.len();
    let mut cursor = Cursor { inner: BufReader::with_capacity(1 << 20, file), position: 0, len };

    let Some((EBML, Some(size), payload)) = cursor.header().map_err(io)? else { return Ok(None) };
    let header = cursor.bytes(size as usize).map_err(io)?;
    let webm = elements(&header).iter().any(|(id, value)| *id == DOC_TYPE && text(value) == "webm");
    cursor.seek(payload + size).map_err(io)?;
    let Some((SEGMENT, segment_size, data_start)) = cursor.header().map_err(io)? else { return Ok(None) };
    let data_end = segment_size.map_or(len, |size| (data_start + size).min(len));

    let mut scale = 1_000_000u64;
    let mut declared: Vec<Declared> = Vec::new();
    let mut frames: Vec<Vec<Sample>> = Vec::new();
    let mut durations: Vec<Vec<u64>> = Vec::new();
    let mut laced_vorbis = false;
    let mut position = data_start;
    'top: while position < data_end {
        cursor.seek(position).map_err(io)?;
        let Some((id, size, payload)) = cursor.header().map_err(io)? else { break };
        match id {
            INFO => {
                let info = cursor.bytes(size.unwrap_or(0) as usize).map_err(io)?;
                if let Some((_, value)) = elements(&info).into_iter().find(|(field, _)| *field == TIMESTAMP_SCALE) {
                    scale = uint(value).max(1);
                }
            }
            TRACKS => {
                let tracks = cursor.bytes(size.unwrap_or(0) as usize).map_err(io)?;
                for (field, entry) in elements(&tracks) {
                    if field != TRACK_ENTRY {
                        continue;
                    }
                    match declare(entry) {
                        Ok(Some(track)) => declared.push(track),
                        Ok(None) => {}
                        Err(()) => return Ok(None),
                    }
                }
                frames = vec![Vec::new(); declared.len()];
                durations = vec![Vec::new(); declared.len()];
            }
            CLUSTER => {
                let end = size.map_or(data_end, |size| (payload + size).min(data_end));
                let mut cluster_time = 0i64;
                let mut at = payload;
                while at < end {
                    cursor.seek(at).map_err(io)?;
                    let Some((child, child_size, child_payload)) = cursor.header().map_err(io)? else { break 'top };
                    // A cluster of unknown size ends where the next top-level element starts.
                    if size.is_none() && TOP_LEVEL.contains(&child) {
                        position = at;
                        continue 'top;
                    }
                    let Some(child_size) = child_size else { return Ok(None) };
                    match child {
                        TIMESTAMP => cluster_time = uint(&cursor.bytes(child_size as usize).map_err(io)?) as i64,
                        SIMPLE_BLOCK | BLOCK_GROUP => {
                            let (block_at, block_size, mut duration, mut key) = if child == SIMPLE_BLOCK {
                                (child_payload, child_size, 0, None)
                            } else {
                                let group = cursor.bytes(child_size as usize).map_err(io)?;
                                let mut found = None;
                                let mut duration = 0;
                                let mut referenced = false;
                                let mut offset = 0u64;
                                for (field, value) in elements(&group) {
                                    let field_start = value.as_ptr() as usize - group.as_ptr() as usize;
                                    match field {
                                        BLOCK => {
                                            found = Some(child_payload + field_start as u64);
                                            offset = value.len() as u64;
                                        }
                                        BLOCK_DURATION => duration = uint(value),
                                        REFERENCE_BLOCK => referenced = true,
                                        _ => {}
                                    }
                                }
                                let Some(found) = found else {
                                    at = child_payload + child_size;
                                    continue;
                                };
                                (found, offset, duration, Some(!referenced))
                            };
                            cursor.seek(block_at).map_err(io)?;
                            let peek = cursor.bytes(block_size.min(16) as usize).map_err(io)?;
                            let Some((number, number_len, _)) = ebml_size(&peek) else { return Ok(None) };
                            let Some(index) = declared.iter().position(|track| track.number == number) else {
                                at = child_payload + child_size;
                                continue;
                            };
                            // Track number, a 16-bit time and the flags byte.
                            let Some(body_len) = block_size.checked_sub(number_len as u64 + 3).filter(|_| peek.len() >= number_len + 3)
                            else {
                                return Err(EngineError::Invalid(format!("{name}: damaged Matroska block")));
                            };
                            let body_len = body_len as usize;
                            let relative = i16::from_be_bytes([peek[number_len], peek[number_len + 1]]);
                            let flags = peek[number_len + 2];
                            let header = number_len + 3;
                            let timestamp = cluster_time + i64::from(relative);
                            if key.is_none() {
                                key = Some(flags & 0x80 != 0);
                            }
                            let lacing = flags >> 1 & 3;
                            let body_at = block_at + header as u64;
                            if lacing == 0 {
                                frames[index].push(Sample { data: Data::File { offset: body_at, size: body_len as u32 }, pts: timestamp, duration: 0, key: key.unwrap_or(true) });
                            } else {
                                if declared[index].track.codec == Codec::Vorbis {
                                    laced_vorbis = true;
                                }
                                cursor.seek(body_at).map_err(io)?;
                                let lace_head = cursor.bytes(body_len.min(4096)).map_err(io)?;
                                let Some((sizes, used)) = lace_sizes(lacing, &lace_head, body_len) else { return Ok(None) };
                                let mut offset = body_at + used as u64;
                                for size in sizes {
                                    frames[index].push(Sample { data: Data::File { offset, size: size as u32 }, pts: timestamp, duration: 0, key: true });
                                    offset += size as u64;
                                }
                                duration = 0;
                            }
                            durations[index].push(duration);
                        }
                        _ => {}
                    }
                    at = child_payload + child_size;
                }
                position = end;
                continue;
            }
            _ => {}
        }
        let Some(size) = size else { break };
        position = payload + size;
    }
    if declared.is_empty() || laced_vorbis {
        return Ok(None);
    }

    // Timestamps are in units of `scale` nanoseconds.
    let ticks_per_second = (1_000_000_000 / scale).max(1) as u32;
    let mut reader = SampleReader::open(source)?;
    let mut tracks = Vec::new();
    for ((declared, samples), block_durations) in declared.into_iter().zip(frames).zip(durations) {
        let mut track = declared.track;
        track.timescale = ticks_per_second;
        track.samples = samples;
        match track.kind {
            Kind::Text => {
                // Subtitles are small; keep their text in memory.
                let mut kept = Vec::with_capacity(track.samples.len());
                let mut buffer = Vec::new();
                for (sample, duration) in track.samples.drain(..).zip(block_durations) {
                    reader.read(&sample.data, &mut buffer)?;
                    kept.push(Sample { data: Data::Bytes(buffer.clone()), duration: duration as u32, ..sample });
                }
                track.samples = kept;
            }
            Kind::Video => {
                let frame = declared.default_duration * u64::from(ticks_per_second) / 1_000_000_000;
                for sample in &mut track.samples {
                    sample.duration = frame as u32;
                }
                if track.codec == Codec::Vp9 && track.config.is_empty() {
                    let first = track.samples.iter().find(|sample| sample.key).map(|sample| sample.data.clone());
                    let bytes = match first {
                        Some(data) => reader.peek(&data, 64)?,
                        None => Vec::new(),
                    };
                    track.config = super::vp9_config(&bytes, track.width);
                }
            }
            Kind::Audio => {
                if track.codec == Codec::Ac3 && track.config.is_empty() {
                    let Some(first) = track.samples.first().map(|sample| sample.data.clone()) else { continue };
                    track.config = super::ac3_config(&reader.peek(&first, 16)?).unwrap_or_default();
                }
                if track.codec != Codec::Vorbis && track.rate > 0 {
                    // Sound is timed in samples, which keeps it exact.
                    for sample in &mut track.samples {
                        sample.pts = sample.pts * i64::from(track.rate) / i64::from(ticks_per_second);
                    }
                    track.timescale = track.rate;
                    super::audio_durations(&mut track, &mut reader)?;
                }
            }
        }
        tracks.push(track);
    }
    Ok(Some(Movie { tracks, layout: if webm { Layout::Webm } else { Layout::Mkv } }))
}

// ---------------------------------------------------------------------------
// Writing

fn id_bytes(id: u32) -> Vec<u8> {
    id.to_be_bytes().into_iter().skip_while(|byte| *byte == 0).collect()
}

fn size_bytes(size: u64) -> Vec<u8> {
    let mut len = 1;
    while len < 8 && size >= (1u64 << (7 * len)) - 1 {
        len += 1;
    }
    let mut out = size.to_be_bytes()[8 - len..].to_vec();
    out[0] |= 0x80 >> (len - 1);
    out
}

fn element(id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = id_bytes(id);
    out.extend_from_slice(&size_bytes(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn uint_element(id: u32, value: u64) -> Vec<u8> {
    let bytes: Vec<u8> = value.to_be_bytes().into_iter().skip_while(|byte| *byte == 0).collect();
    element(id, if bytes.is_empty() { &[0] } else { &bytes })
}

fn float_element(id: u32, value: f64) -> Vec<u8> {
    element(id, &value.to_be_bytes())
}

/// A Void element of exactly `len` bytes.
fn void(len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    crate::containers::write_void(&mut out);
    out
}

fn codec_id(track: &Track) -> Option<(&'static str, Vec<u8>)> {
    Some(match track.codec {
        Codec::H264 => ("V_MPEG4/ISO/AVC", track.config.clone()),
        Codec::Hevc => ("V_MPEGH/ISO/HEVC", track.config.clone()),
        Codec::Av1 => ("V_AV1", track.config.clone()),
        Codec::Vp9 => ("V_VP9", Vec::new()),
        Codec::Vp8 => ("V_VP8", Vec::new()),
        Codec::Aac => ("A_AAC", track.config.clone()),
        Codec::Mp3 => ("A_MPEG/L3", Vec::new()),
        Codec::Opus => ("A_OPUS", track.config.clone()),
        Codec::Vorbis => ("A_VORBIS", track.config.clone()),
        Codec::Flac => ("A_FLAC", [b"fLaC".as_slice(), &track.config].concat()),
        Codec::Ac3 => ("A_AC3", Vec::new()),
        Codec::Alac => ("A_ALAC", track.config.clone()),
        Codec::Text => ("S_TEXT/UTF8", Vec::new()),
        Codec::OtherText => return None,
    })
}

fn track_entry(track: &Track, number: u64) -> Result<Vec<u8>> {
    let (id, private) = codec_id(track).ok_or_else(|| EngineError::Unsupported("subtitle format".to_owned()))?;
    let mut body = uint_element(TRACK_NUMBER, number);
    body.extend_from_slice(&uint_element(TRACK_UID, number));
    body.extend_from_slice(&uint_element(TRACK_TYPE, match track.kind {
        Kind::Video => 1,
        Kind::Audio => 2,
        Kind::Text => 17,
    }));
    body.extend_from_slice(&uint_element(FLAG_LACING, 0));
    body.extend_from_slice(&element(LANGUAGE, track.language.as_bytes()));
    body.extend_from_slice(&element(CODEC_ID, id.as_bytes()));
    if !private.is_empty() {
        body.extend_from_slice(&element(CODEC_PRIVATE, &private));
    }
    if track.codec == Codec::Opus && track.config.len() >= 12 {
        let pre_skip = u64::from(u16::from_le_bytes([track.config[10], track.config[11]]));
        body.extend_from_slice(&uint_element(CODEC_DELAY, pre_skip * 1_000_000_000 / 48_000));
        body.extend_from_slice(&uint_element(SEEK_PRE_ROLL, 80_000_000));
    }
    match track.kind {
        Kind::Video => {
            let video = [uint_element(PIXEL_WIDTH, u64::from(track.width)), uint_element(PIXEL_HEIGHT, u64::from(track.height))].concat();
            body.extend_from_slice(&element(VIDEO, &video));
        }
        Kind::Audio => {
            let mut audio = float_element(SAMPLING_FREQUENCY, f64::from(track.rate));
            audio.extend_from_slice(&uint_element(CHANNELS, u64::from(track.channels.max(1))));
            if track.bits > 0 && track.codec == Codec::Flac {
                audio.extend_from_slice(&uint_element(BIT_DEPTH, u64::from(track.bits)));
            }
            body.extend_from_slice(&element(AUDIO, &audio));
        }
        Kind::Text => {}
    }
    Ok(element(TRACK_ENTRY, &body))
}

/// One frame ready to be placed in a cluster.
struct Frame<'a> {
    track: usize,
    sample: &'a Sample,
    /// Decode order key and presentation time, in milliseconds.
    order: i64,
    time: i64,
}

const CLUSTER_MS: i64 = 2000;

pub(super) fn write<W: Write + Seek>(movie: &Movie, reader: &mut SampleReader, out: &mut W, webm: bool, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<()> {
    let fail = |e: std::io::Error| EngineError::io("output", e);
    let origin = super::origin(movie);
    let ms = |track: &Track, ticks: i64| ((track.seconds(ticks) - origin) * 1000.0).round() as i64;

    let mut frames: Vec<Frame> = Vec::new();
    for (index, track) in movie.tracks.iter().enumerate() {
        let order: Vec<i64> = if track.kind == Kind::Video { super::decode_times(&track.samples) } else { track.samples.iter().map(|sample| sample.pts).collect() };
        for (sample, decode) in track.samples.iter().zip(order) {
            frames.push(Frame { track: index, sample, order: ms(track, decode), time: ms(track, sample.pts).max(0) });
        }
    }
    // Stable, so each track keeps its own decode order.
    frames.sort_by_key(|frame| frame.order);
    let duration = movie.tracks.iter().map(|track| (track.span().1 - origin) * 1000.0).fold(0.0, f64::max);
    let video = movie.tracks.iter().position(|track| track.kind == Kind::Video);

    // Header, then a segment of unknown size patched at the end.
    let header = [
        uint_element(0x4286, 1),
        uint_element(0x42F7, 1),
        uint_element(0x42F2, 4),
        uint_element(0x42F3, 8),
        element(DOC_TYPE, if webm { b"webm" } else { b"matroska" }),
        uint_element(0x4287, 4),
        uint_element(0x4285, 2),
    ]
    .concat();
    out.write_all(&element(EBML, &header)).map_err(fail)?;
    out.write_all(&id_bytes(SEGMENT)).map_err(fail)?;
    let segment_size_at = out.stream_position().map_err(fail)?;
    out.write_all(&[0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).map_err(fail)?;
    let data_start = out.stream_position().map_err(fail)?;
    const SEEK_SPACE: usize = 160;
    out.write_all(&void(SEEK_SPACE)).map_err(fail)?;

    let info_at = out.stream_position().map_err(fail)? - data_start;
    let info = [
        uint_element(TIMESTAMP_SCALE, 1_000_000),
        element(MUXING_APP, b"WindowCraft"),
        element(WRITING_APP, b"WindowCraft"),
        float_element(DURATION, duration),
    ]
    .concat();
    out.write_all(&element(INFO, &info)).map_err(fail)?;
    let tracks_at = out.stream_position().map_err(fail)? - data_start;
    let mut entries = Vec::new();
    for (index, track) in movie.tracks.iter().enumerate() {
        entries.extend_from_slice(&track_entry(track, index as u64 + 1)?);
    }
    out.write_all(&element(TRACKS, &entries)).map_err(fail)?;

    let mut cues = Vec::new();
    let mut cluster: Vec<u8> = Vec::new();
    let mut cluster_time = 0i64;
    let mut cluster_open = false;
    let mut buffer = Vec::new();
    let total = frames.len().max(1);
    for (count, frame) in frames.iter().enumerate() {
        if count % 512 == 0 {
            cancel.check()?;
            progress(count as f32 / total as f32);
        }
        let track = &movie.tracks[frame.track];
        let starts_group = match video {
            Some(video) => frame.track == video && frame.sample.key,
            None => true,
        };
        let relative = frame.time - cluster_time;
        let full = !(-32_768..=32_767).contains(&relative) || cluster.len() > 32 << 20;
        if !cluster_open || full || (starts_group && frame.time - cluster_time >= CLUSTER_MS) {
            if cluster_open {
                out.write_all(&element(CLUSTER, &cluster)).map_err(fail)?;
            }
            cluster_time = frame.time;
            cluster = uint_element(TIMESTAMP, cluster_time as u64);
            cluster_open = true;
            let position = out.stream_position().map_err(fail)? - data_start;
            let cued = video.is_none_or(|video| frame.track == video && frame.sample.key);
            if cued {
                let positions = [uint_element(CUE_TRACK, frame.track as u64 + 1), uint_element(CUE_CLUSTER_POSITION, position)].concat();
                let point = [uint_element(CUE_TIME, frame.time as u64), element(CUE_TRACK_POSITIONS, &positions)].concat();
                cues.extend_from_slice(&element(CUE_POINT, &point));
            }
        }
        reader.read(&frame.sample.data, &mut buffer)?;
        let relative = (frame.time - cluster_time) as i16;
        let mut block = vec![0x80 | (frame.track as u8 + 1)];
        block.extend_from_slice(&relative.to_be_bytes());
        if track.kind == Kind::Text {
            block.push(0);
            block.extend_from_slice(&buffer);
            let duration = (track.seconds(i64::from(frame.sample.duration)) * 1000.0).round() as u64;
            let group = [element(BLOCK, &block), uint_element(BLOCK_DURATION, duration)].concat();
            cluster.extend_from_slice(&element(BLOCK_GROUP, &group));
        } else {
            block.push(if frame.sample.key || track.kind == Kind::Audio { 0x80 } else { 0 });
            block.extend_from_slice(&buffer);
            cluster.extend_from_slice(&element(SIMPLE_BLOCK, &block));
        }
    }
    if cluster_open {
        out.write_all(&element(CLUSTER, &cluster)).map_err(fail)?;
    }
    let cues_at = out.stream_position().map_err(fail)? - data_start;
    out.write_all(&element(CUES, &cues)).map_err(fail)?;
    let end = out.stream_position().map_err(fail)?;

    // The SeekHead goes into the space kept for it.
    let seek = |id: u32, position: u64| element(SEEK, &[element(SEEK_ID, &id_bytes(id)), uint_element(SEEK_POSITION, position)].concat());
    let head = element(SEEK_HEAD, &[seek(INFO, info_at), seek(TRACKS, tracks_at), seek(CUES, cues_at)].concat());
    let mut space = head.clone();
    space.extend_from_slice(&void(SEEK_SPACE - head.len()));
    out.seek(SeekFrom::Start(data_start)).map_err(fail)?;
    out.write_all(&space).map_err(fail)?;
    let mut size = (end - data_start).to_be_bytes();
    size[0] = 0x01;
    out.seek(SeekFrom::Start(segment_size_at)).map_err(fail)?;
    out.write_all(&size).map_err(fail)?;
    out.seek(SeekFrom::Start(end)).map_err(fail)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_the_shortest_form() {
        assert_eq!(size_bytes(0), [0x80]);
        assert_eq!(size_bytes(126), [0xFE]);
        assert_eq!(size_bytes(127), [0x40, 0x7F]);
        assert_eq!(size_bytes(16_382), [0x7F, 0xFE]);
        for value in [0u64, 1, 126, 127, 5000, 1 << 30] {
            let bytes = size_bytes(value);
            assert_eq!(ebml_size(&bytes), Some((value, bytes.len(), false)));
        }
    }

    #[test]
    fn laced_frames_are_split() {
        // Xiph lacing: three frames of 300, 10 and the rest.
        let body = [2, 255, 45, 10];
        let (sizes, used) = lace_sizes(1, &body, 4 + 300 + 10 + 7).unwrap();
        assert_eq!(sizes, vec![300, 10, 7]);
        assert_eq!(used, 4);
        // Fixed lacing: four equal frames.
        let (sizes, used) = lace_sizes(2, &[3], 1 + 400).unwrap();
        assert_eq!((sizes, used), (vec![100, 100, 100, 100], 1));
        // EBML lacing: 500, then +2, then the rest.
        let mut body = vec![2];
        body.extend_from_slice(&size_bytes(500));
        body.push(0x80 | (63 + 2));
        let used = body.len();
        let (sizes, _) = lace_sizes(3, &body, used + 500 + 502 + 9).unwrap();
        assert_eq!(sizes, vec![500, 502, 9]);
    }

    #[test]
    fn a_cut_simple_block_is_an_error() {
        let entry = [uint_element(TRACK_NUMBER, 1), uint_element(TRACK_TYPE, 1), element(CODEC_ID, b"V_VP8")].concat();
        let tracks = element(TRACKS, &element(TRACK_ENTRY, &entry));
        // Track 1, then one byte of the 16-bit time and no flags.
        let cluster = element(CLUSTER, &[uint_element(TIMESTAMP, 0), element(SIMPLE_BLOCK, &[0x81, 0x00])].concat());
        let mut data = element(EBML, &element(DOC_TYPE, b"matroska"));
        data.extend_from_slice(&element(SEGMENT, &[tracks, cluster].concat()));

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cut.mkv");
        std::fs::write(&path, &data).unwrap();
        assert!(matches!(read(&path), Err(EngineError::Invalid(_))));
    }

    #[test]
    fn void_fills_the_space_exactly() {
        for len in [2usize, 100, 128, 129, 160] {
            let bytes = void(len);
            let (id, id_len) = ebml_id(&bytes).unwrap();
            let (size, size_len, _) = ebml_size(&bytes[id_len..]).unwrap();
            assert_eq!(id, VOID);
            assert_eq!(id_len + size_len + size as usize, len);
        }
    }
}
