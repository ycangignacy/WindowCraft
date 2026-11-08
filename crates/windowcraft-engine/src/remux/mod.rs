// Made by ycangignacy
//! Moves video and sound between containers without re-encoding.
//!
//! MP4, MOV and M4A are read and written by `mp4`, MKV and WebM by `mkv`, and
//! Opus sound goes into Ogg here. The streams themselves are copied byte for
//! byte, so nothing loses quality and the work is as fast as the disk. When a
//! stream does not fit the target container (VP8 in MP4, Vorbis in MP4, ASS
//! subtitles in MP4 and the like) the answer is `Ok(false)` and ffmpeg, if
//! installed, takes over.

mod mkv;
mod mp4;

use crate::convert::Target;
use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Video,
    Audio,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Codec {
    H264,
    Hevc,
    Av1,
    Vp9,
    Vp8,
    Aac,
    Mp3,
    Opus,
    Vorbis,
    Flac,
    Ac3,
    Alac,
    /// Plain UTF-8 subtitle text, as SRT carries it.
    Text,
    /// Subtitles only Matroska can keep, such as ASS or PGS.
    OtherText,
}

/// Where a sample's bytes are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Data {
    File { offset: u64, size: u32 },
    Bytes(Vec<u8>),
}

impl Data {
    fn len(&self) -> u32 {
        match self {
            Self::File { size, .. } => *size,
            Self::Bytes(bytes) => bytes.len() as u32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sample {
    pub data: Data,
    /// Presentation time in track ticks.
    pub pts: i64,
    /// Length in track ticks; 0 when unknown.
    pub duration: u32,
    pub key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Track {
    pub kind: Kind,
    pub codec: Codec,
    /// Codec setup in its MP4 form: avcC, hvcC, av1C and vpcC payloads, the
    /// AAC AudioSpecificConfig, OpusHead, FLAC metadata blocks without the
    /// `fLaC` marker, the dac3 payload, the ALAC cookie, or the three Vorbis
    /// headers in Matroska's Xiph lacing.
    pub config: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    /// ISO 639-2 code, "und" when unknown.
    pub language: String,
    /// Ticks per second for `pts` and `duration`.
    pub timescale: u32,
    pub samples: Vec<Sample>,
}

impl Track {
    pub(crate) fn new(kind: Kind, codec: Codec) -> Self {
        Self {
            kind,
            codec,
            config: Vec::new(),
            width: 0,
            height: 0,
            rate: 0,
            channels: 0,
            bits: 0,
            language: "und".to_owned(),
            timescale: 1000,
            samples: Vec::new(),
        }
    }

    fn seconds(&self, ticks: i64) -> f64 {
        ticks as f64 / f64::from(self.timescale.max(1))
    }

    /// First and last presentation time in seconds, the last including its duration.
    fn span(&self) -> (f64, f64) {
        let first = self.samples.iter().map(|sample| sample.pts).min().unwrap_or(0);
        let last = self.samples.iter().map(|sample| sample.pts + i64::from(sample.duration)).max().unwrap_or(0);
        (self.seconds(first), self.seconds(last))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    Mp4,
    Mov,
    M4a,
    Mkv,
    Webm,
}

/// What a source file holds. Samples point into `source`.
pub(crate) struct Movie {
    pub tracks: Vec<Track>,
    pub layout: Layout,
}

fn read_movie(source: &Path) -> Result<Option<Movie>> {
    let mut head = [0u8; 12];
    let mut file = File::open(source).map_err(|e| EngineError::io(source.display().to_string(), e))?;
    let read = file.read(&mut head).map_err(|e| EngineError::io(source.display().to_string(), e))?;
    if read >= 8 && matches!(&head[4..8], b"ftyp" | b"moov" | b"mdat" | b"free" | b"wide") {
        return mp4::read(source);
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return mkv::read(source);
    }
    Ok(None)
}

fn fits(layout: Layout, track: &Track) -> bool {
    use Codec::*;
    match layout {
        Layout::Mp4 => matches!(track.codec, H264 | Hevc | Av1 | Vp9 | Aac | Mp3 | Opus | Flac | Ac3 | Alac | Text),
        Layout::Mov => matches!(track.codec, H264 | Hevc | Aac | Mp3 | Ac3 | Alac | Text),
        Layout::M4a => matches!(track.codec, Aac | Alac),
        Layout::Mkv => track.codec != OtherText,
        Layout::Webm => matches!(track.codec, Vp8 | Vp9 | Av1 | Opus | Vorbis),
    }
}

/// Rewraps the streams of `source` into the container of `target` when that
/// can be done here. `Ok(false)` leaves the file to ffmpeg.
pub fn remux(source: &Path, output: &Path, target: Target, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<bool> {
    let layout = match target {
        Target::Mp4 => Layout::Mp4,
        Target::Mov => Layout::Mov,
        Target::Mkv => Layout::Mkv,
        Target::Webm => Layout::Webm,
        _ => return Ok(false),
    };
    let Some(mut movie) = read_movie(source)? else { return Ok(false) };
    if !movie.tracks.iter().any(|track| track.kind == Kind::Video) {
        return Ok(false);
    }
    // Already in that container: the copy is the conversion.
    if movie.layout == layout {
        std::fs::copy(source, output).map_err(|e| EngineError::io(output.display().to_string(), e))?;
        return Ok(true);
    }
    // Subtitles WebM cannot hold are left out; any other stream that does not
    // fit sends the whole file to ffmpeg.
    if layout == Layout::Webm {
        movie.tracks.retain(|track| track.kind != Kind::Text);
    }
    if !movie.tracks.iter().all(|track| fits(layout, track)) {
        return Ok(false);
    }
    write(&movie, source, output, layout, cancel, progress)?;
    Ok(true)
}

/// Copies the first suitable sound track into M4A or Ogg Opus.
pub fn audio_only(source: &Path, output: &Path, target: Target, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<bool> {
    let Some(movie) = read_movie(source)? else { return Ok(false) };
    let wanted = |track: &&Track| match target {
        Target::M4a => matches!(track.codec, Codec::Aac | Codec::Alac),
        Target::Opus => track.codec == Codec::Opus,
        _ => false,
    };
    let Some(track) = movie.tracks.iter().find(wanted).cloned() else { return Ok(false) };
    if target == Target::Opus {
        write_ogg_opus(&track, source, output, cancel, progress)?;
    } else {
        let movie = Movie { tracks: vec![track], layout: Layout::M4a };
        write(&movie, source, output, Layout::M4a, cancel, progress)?;
    }
    Ok(true)
}

fn write(movie: &Movie, source: &Path, output: &Path, layout: Layout, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<()> {
    let mut reader = SampleReader::open(source)?;
    let file = File::create(output).map_err(|e| EngineError::io(output.display().to_string(), e))?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    match layout {
        Layout::Mkv | Layout::Webm => mkv::write(movie, &mut reader, &mut writer, layout == Layout::Webm, cancel, progress)?,
        _ => mp4::write(movie, &mut reader, &mut writer, layout, cancel, progress)?,
    }
    let file = writer.into_inner().map_err(|e| EngineError::io(output.display().to_string(), e.into_error()))?;
    file.sync_all().map_err(|e| EngineError::io(output.display().to_string(), e))
}

/// Reads sample bytes from the source, cheaply when they come in order.
pub(crate) struct SampleReader {
    reader: BufReader<File>,
    position: u64,
    name: String,
}

impl SampleReader {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        let name = path.display().to_string();
        let file = File::open(path).map_err(|e| EngineError::io(name.clone(), e))?;
        Ok(Self { reader: BufReader::with_capacity(1 << 20, file), position: 0, name })
    }

    pub(crate) fn read(&mut self, data: &Data, into: &mut Vec<u8>) -> Result<()> {
        into.clear();
        match data {
            Data::Bytes(bytes) => into.extend_from_slice(bytes),
            Data::File { offset, size } => {
                if *offset != self.position {
                    let delta = *offset as i64 - self.position as i64;
                    self.reader.seek_relative(delta).map_err(|e| EngineError::io(self.name.clone(), e))?;
                }
                into.resize(*size as usize, 0);
                self.reader.read_exact(into).map_err(|e| EngineError::io(self.name.clone(), e))?;
                self.position = offset + u64::from(*size);
            }
        }
        Ok(())
    }

    /// The first `count` bytes of a sample, for codecs whose length is in the
    /// packet itself.
    pub(crate) fn peek(&mut self, data: &Data, count: usize) -> Result<Vec<u8>> {
        match data {
            Data::Bytes(bytes) => Ok(bytes.iter().take(count).copied().collect()),
            Data::File { offset, size } => {
                let take = count.min(*size as usize);
                let mut buffer = vec![0u8; take];
                if *offset != self.position {
                    self.reader.seek(SeekFrom::Start(*offset)).map_err(|e| EngineError::io(self.name.clone(), e))?;
                }
                self.reader.read_exact(&mut buffer).map_err(|e| EngineError::io(self.name.clone(), e))?;
                self.position = offset + take as u64;
                Ok(buffer)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Codec details shared by both containers

/// Samples per channel in one Opus packet, from its TOC byte.
pub(crate) fn opus_samples(packet: &[u8]) -> u32 {
    let Some(&toc) = packet.first() else { return 0 };
    let config = toc >> 3;
    // Frame length in 48 kHz samples.
    let frame = match config {
        0..=11 => [480, 960, 1920, 2880][usize::from(config & 3)],
        12..=15 => [480, 960][usize::from(config & 1)],
        _ => [120, 240, 480, 960][usize::from(config & 3)],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => u32::from(packet.get(1).map_or(0, |count| count & 0x3F)),
    };
    frame * frames
}

/// Samples per AAC frame: 1024, or 960 when the config says so.
pub(crate) fn aac_frame(config: &[u8]) -> u32 {
    let mut bits = Bits::new(config);
    let mut object = bits.take(5);
    if object == 31 {
        object = 32 + bits.take(6);
    }
    let index = bits.take(4);
    if index == 15 {
        bits.take(24);
    }
    bits.take(4);
    let _ = object;
    if bits.take(1) == 1 { 960 } else { 1024 }
}

/// Sampling rate and channel count from an AAC AudioSpecificConfig.
pub(crate) fn aac_rate_channels(config: &[u8]) -> Option<(u32, u16)> {
    const RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    let mut bits = Bits::new(config);
    if bits.take(5) == 31 {
        bits.take(6);
    }
    let index = bits.take(4);
    let rate = if index == 15 { bits.take(24) } else { *RATES.get(index as usize)? };
    let channels = bits.take(4) as u16;
    Some((rate, channels))
}

/// Builds an AudioSpecificConfig for AAC-LC.
pub(crate) fn aac_config(rate: u32, channels: u16) -> Vec<u8> {
    const RATES: [u32; 13] = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350];
    let index = RATES.iter().position(|known| *known == rate).unwrap_or(4) as u16;
    let value: u16 = 2 << 11 | index << 7 | (channels & 0xF) << 3;
    value.to_be_bytes().to_vec()
}

/// Samples per MP3 frame from its header.
pub(crate) fn mp3_frame(header: &[u8]) -> u32 {
    match header.get(1).map(|byte| byte >> 3 & 3) {
        Some(3) => 1152,
        Some(_) => 576,
        None => 1152,
    }
}

/// Samples in a FLAC frame from its header.
pub(crate) fn flac_frame(header: &[u8]) -> u32 {
    let Some(&byte) = header.get(2) else { return 0 };
    match byte >> 4 {
        1 => 192,
        code @ 2..=5 => 576 << (code - 2),
        code @ 8..=15 => 256 << (code - 8),
        // 6 and 7 store the size after the UTF-8 number; it is rare.
        _ => 0,
    }
}

/// The dac3 payload from the first AC-3 frame.
pub(crate) fn ac3_config(frame: &[u8]) -> Option<Vec<u8>> {
    if frame.get(..2)? != [0x0B, 0x77] {
        return None;
    }
    let mut bits = Bits::new(frame.get(4..)?);
    let fscod = bits.take(2);
    let frmsizecod = bits.take(6);
    let bsid = bits.take(5);
    let bsmod = bits.take(3);
    let acmod = bits.take(3);
    if acmod & 1 != 0 && acmod != 1 {
        bits.take(2);
    }
    if acmod & 4 != 0 {
        bits.take(2);
    }
    if acmod == 2 {
        bits.take(2);
    }
    let lfeon = bits.take(1);
    let packed = fscod << 22 | bsid << 17 | bsmod << 14 | acmod << 11 | lfeon << 10 | (frmsizecod >> 1) << 5;
    Some(packed.to_be_bytes()[1..].to_vec())
}

/// A vpcC payload from the first VP9 key frame.
pub(crate) fn vp9_config(frame: &[u8], width: u32) -> Vec<u8> {
    let mut bits = Bits::new(frame);
    let mut profile = 0;
    let mut depth = 8;
    let mut subsampling = 1;
    let mut full_range = 0;
    if bits.take(2) == 2 {
        let low = bits.take(1);
        profile = bits.take(1) << 1 | low;
        if profile == 3 {
            bits.take(1);
        }
        let shown_existing = bits.take(1) == 1;
        let key = bits.take(1) == 0;
        bits.take(2);
        if !shown_existing && key && bits.take(24) == 0x49_8342 {
            if profile >= 2 {
                depth = if bits.take(1) == 1 { 12 } else { 10 };
            }
            let space = bits.take(3);
            if space == 7 {
                full_range = 1;
                if profile & 1 == 1 {
                    subsampling = 3;
                }
            } else {
                full_range = bits.take(1);
                if profile & 1 == 1 {
                    let x = bits.take(1);
                    let y = bits.take(1);
                    subsampling = match (x, y) {
                        (1, 0) => 2,
                        (0, 0) => 3,
                        _ => 1,
                    };
                }
            }
        }
    }
    let level: u8 = match width {
        0..=1279 => 31,
        1280..=1920 => 41,
        _ => 51,
    };
    vec![1, 0, 0, 0, profile as u8, level, (depth << 4 | subsampling << 1 | full_range) as u8, 2, 2, 2, 0, 0]
}

/// OpusHead to the MP4 dOps layout: no magic, version 0, big-endian fields.
pub(crate) fn opus_head_to_dops(head: &[u8]) -> Option<Vec<u8>> {
    if head.len() < 19 || &head[..8] != b"OpusHead" {
        return None;
    }
    let mut out = vec![0, head[9]];
    out.extend_from_slice(&u16::from_le_bytes([head[10], head[11]]).to_be_bytes());
    out.extend_from_slice(&u32::from_le_bytes([head[12], head[13], head[14], head[15]]).to_be_bytes());
    out.extend_from_slice(&i16::from_le_bytes([head[16], head[17]]).to_be_bytes());
    out.push(head[18]);
    out.extend_from_slice(&head[19..]);
    Some(out)
}

pub(crate) fn dops_to_opus_head(dops: &[u8]) -> Option<Vec<u8>> {
    if dops.len() < 11 {
        return None;
    }
    let mut out = b"OpusHead".to_vec();
    out.push(1);
    out.push(dops[1]);
    out.extend_from_slice(&u16::from_be_bytes([dops[2], dops[3]]).to_le_bytes());
    out.extend_from_slice(&u32::from_be_bytes([dops[4], dops[5], dops[6], dops[7]]).to_le_bytes());
    out.extend_from_slice(&i16::from_be_bytes([dops[8], dops[9]]).to_le_bytes());
    out.push(dops[10]);
    out.extend_from_slice(&dops[11..]);
    Some(out)
}

/// Big-endian bit reader for codec headers.
pub(crate) struct Bits<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    pub(crate) fn take(&mut self, count: u32) -> u32 {
        let mut value = 0u32;
        for _ in 0..count {
            let byte = self.data.get(self.position / 8).copied().unwrap_or(0);
            let bit = byte >> (7 - self.position % 8) & 1;
            value = value << 1 | u32::from(bit);
            self.position += 1;
        }
        value
    }
}

/// Where the output timeline starts, in source seconds: the earliest track
/// start, but never before 0, so encoder priming stays hidden as it was.
pub(crate) fn origin(movie: &Movie) -> f64 {
    let first = movie.tracks.iter().filter(|track| !track.samples.is_empty()).map(|track| track.span().0.max(0.0)).fold(f64::INFINITY, f64::min);
    if first.is_finite() { first } else { 0.0 }
}

/// Decode times for samples in decode order, derived from their presentation
/// times: the sorted times, moved back until none is after its own sample.
pub(crate) fn decode_times(samples: &[Sample]) -> Vec<i64> {
    let mut sorted: Vec<i64> = samples.iter().map(|sample| sample.pts).collect();
    sorted.sort_unstable();
    let delay = sorted.iter().zip(samples).map(|(early, sample)| early - sample.pts).max().unwrap_or(0).max(0);
    sorted.iter().map(|early| early - delay).collect()
}

/// Fills in sample durations for sound whose container gives only times.
pub(crate) fn audio_durations(track: &mut Track, reader: &mut SampleReader) -> Result<()> {
    let constant = match track.codec {
        Codec::Aac => Some(aac_frame(&track.config)),
        Codec::Ac3 => Some(1536),
        Codec::Alac => Some(4096),
        _ => None,
    };
    let mut position: Option<i64> = None;
    for index in 0..track.samples.len() {
        let length = match (constant, track.codec) {
            (Some(length), _) => length,
            (None, Codec::Opus) => opus_samples(&reader.peek(&track.samples[index].data, 2)?),
            (None, Codec::Mp3) => mp3_frame(&reader.peek(&track.samples[index].data, 4)?),
            (None, Codec::Flac) => flac_frame(&reader.peek(&track.samples[index].data, 4)?),
            _ => 0,
        };
        let sample = &mut track.samples[index];
        sample.duration = length;
        // Sound plays back to back, so each packet starts where the last ended.
        let start = *position.get_or_insert(sample.pts);
        sample.pts = start;
        position = Some(start + i64::from(length));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Ogg Opus

fn write_ogg_opus(track: &Track, source: &Path, output: &Path, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<()> {
    use crate::containers::ogg_page;
    let fail = |e: std::io::Error| EngineError::io(output.display().to_string(), e);
    let mut reader = SampleReader::open(source)?;
    let mut writer = BufWriter::with_capacity(1 << 20, File::create(output).map_err(fail)?);
    let head = if track.config.starts_with(b"OpusHead") {
        track.config.clone()
    } else {
        return Err(EngineError::Invalid("Opus without OpusHead".to_owned()));
    };
    let serial = 0x5743_4F50;
    let mut sequence = 0;
    writer.write_all(&ogg_page(0x02, 0, serial, sequence, &[head.as_slice()])).map_err(fail)?;
    sequence += 1;
    writer.write_all(&ogg_page(0, 0, serial, sequence, &[b"OpusTags\0\0\0\0\0\0\0\0".as_slice()])).map_err(fail)?;
    sequence += 1;
    let pre_skip = u64::from(u16::from_le_bytes([head[10], head[11]]));
    let mut granule = pre_skip;
    let mut page: Vec<Vec<u8>> = Vec::new();
    let mut segments = 0;
    let count = track.samples.len();
    let mut buffer = Vec::new();
    for (index, sample) in track.samples.iter().enumerate() {
        if index % 256 == 0 {
            cancel.check()?;
            progress(index as f32 / count.max(1) as f32);
        }
        reader.read(&sample.data, &mut buffer)?;
        let needed = buffer.len() / 255 + 1;
        if segments + needed > 255 && !page.is_empty() {
            let packets: Vec<&[u8]> = page.iter().map(Vec::as_slice).collect();
            writer.write_all(&ogg_page(0, granule, serial, sequence, &packets)).map_err(fail)?;
            sequence += 1;
            page.clear();
            segments = 0;
        }
        granule += u64::from(opus_samples(&buffer));
        segments += needed;
        page.push(buffer.clone());
    }
    let packets: Vec<&[u8]> = page.iter().map(Vec::as_slice).collect();
    writer.write_all(&ogg_page(0x04, granule, serial, sequence, &packets)).map_err(fail)?;
    let file = writer.into_inner().map_err(|e| fail(e.into_error()))?;
    file.sync_all().map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_packet_lengths_come_from_the_toc() {
        assert_eq!(opus_samples(&[0xFC]), 960, "CELT 20 ms");
        assert_eq!(opus_samples(&[0x78]), 960 * 2 / 2, "SILK 20 ms");
        assert_eq!(opus_samples(&[0xFD]), 1920, "two 20 ms frames");
        assert_eq!(opus_samples(&[0xFF, 0x03]), 2880, "three 20 ms frames");
    }

    #[test]
    fn aac_configs_round_trip() {
        let config = aac_config(44_100, 2);
        assert_eq!(config, [0x12, 0x10]);
        assert_eq!(aac_rate_channels(&config), Some((44_100, 2)));
        assert_eq!(aac_frame(&config), 1024);
        assert_eq!(aac_frame(&[0x12, 0x14]), 960);
    }

    #[test]
    fn opus_head_and_dops_convert_both_ways() {
        let head = b"OpusHead\x01\x02\x38\x01\x80\xBB\0\0\0\0\0".to_vec();
        let dops = opus_head_to_dops(&head).unwrap();
        assert_eq!(dops, [0, 2, 0x01, 0x38, 0, 0, 0xBB, 0x80, 0, 0, 0]);
        assert_eq!(dops_to_opus_head(&dops).unwrap(), head);
    }

    #[test]
    fn ac3_config_reads_the_first_frame() {
        // 48 kHz, 192 kbps, 2/0 stereo, no LFE.
        let frame = [0x0B, 0x77, 0, 0, 0x14, 0x40, 0x43, 0xFF];
        let config = ac3_config(&frame).unwrap();
        let value = u32::from_be_bytes([0, config[0], config[1], config[2]]);
        assert_eq!(value >> 22, 0, "48 kHz");
        assert_eq!(value >> 17 & 0x1F, 8, "bsid");
        assert_eq!(value >> 11 & 7, 2, "acmod 2/0");
        assert_eq!(value >> 5 & 0x1F, 10, "192 kbps");
    }

    #[test]
    fn vp9_config_reads_profile_and_depth() {
        // Profile 0 key frame: marker 10, profile bits 00, show_existing 0,
        // frame_type 0, show 1, error_res 0, sync code, color space 2, range 0.
        let frame = [0b1000_0010, 0x49, 0x83, 0x42, 0b0100_0000];
        let config = vp9_config(&frame, 1920);
        assert_eq!(config[4], 0, "profile");
        assert_eq!(config[5], 41, "level for 1080p");
        assert_eq!(config[6] >> 4, 8, "bit depth");
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::tools::{self, Extension, Tools};
    use std::time::Duration;

    fn ffmpeg(tools: &Tools, args: &[&str]) {
        let mut command = vec!["-hide_banner", "-loglevel", "error", "-y"];
        command.extend_from_slice(args);
        crate::process::run_ok(&tools.ffmpeg(), &command, &CancelToken::new(), Duration::from_secs(300)).unwrap();
    }

    /// Codec and packet count of every stream, as ffprobe reads them.
    fn streams(tools: &Tools, path: &Path) -> Vec<(String, u64)> {
        let text = crate::process::run_ok(
            &tools.ffprobe(),
            &["-v", "error", "-count_packets", "-show_entries", "stream=codec_name,nb_read_packets", "-of", "csv=p=0", &path.display().to_string()],
            &CancelToken::new(),
            Duration::from_secs(120),
        )
        .unwrap();
        text.lines()
            .filter_map(|line| {
                let (codec, count) = line.split_once(',')?;
                Some((codec.to_owned(), count.trim().parse().ok()?))
            })
            .collect()
    }

    /// The subtitles of a file as SRT text, through ffmpeg.
    fn subtitles(tools: &Tools, path: &Path) -> String {
        crate::process::run_ok(
            &tools.ffmpeg(),
            &["-v", "error", "-i", &path.display().to_string(), "-map", "0:s:0", "-f", "srt", "-"],
            &CancelToken::new(),
            Duration::from_secs(60),
        )
        .unwrap_or_default()
    }

    fn decodes_cleanly(tools: &Tools, path: &Path) -> String {
        let output = crate::process::run(
            &tools.ffmpeg(),
            &["-v", "error", "-i", &path.display().to_string(), "-f", "null", "-"],
            &CancelToken::new(),
            Duration::from_secs(300),
            |_| {},
        )
        .unwrap();
        if output.code == 0 { output.text } else { format!("exit {}: {}", output.code, output.text) }
    }

    /// Builds real files with ffmpeg, rewraps them here and checks the result
    /// with ffmpeg again. Downloads ffmpeg (about 90 MB) when it is missing:
    /// `cargo test -p windowcraft-engine live_remux -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs ffmpeg from GitHub"]
    fn live_remux_round_trips() {
        let cancel = CancelToken::new();
        let tools = tools::install(Extension::Ffmpeg, &cancel, |_| {}).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| dir.path().join(name).display().to_string();
        std::fs::write(dir.path().join("sub.srt"), "1\n00:00:00,500 --> 00:00:01,200\nHello\n\n2\n00:00:01,500 --> 00:00:01,900\nWorld\n").unwrap();
        let video = ["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=2"];
        let sound = ["-f", "lavfi", "-i", "sine=frequency=440:duration=2:sample_rate=48000"];
        let sources: Vec<(&str, Vec<&str>)> = vec![
            ("h264-aac.mp4", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
            ("h264-aac.mov", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"]),
            ("h264-opus.mkv", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "libopus"]),
            ("h264-mp3.mkv", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "libmp3lame"]),
            ("h264-flac.mkv", vec!["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "flac"]),
            ("hevc-ac3.mkv", vec!["-c:v", "libx265", "-pix_fmt", "yuv420p", "-x265-params", "log-level=none", "-c:a", "ac3"]),
            ("vp9-opus.webm", vec!["-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-c:a", "libopus"]),
            ("av1-opus.webm", vec!["-c:v", "libaom-av1", "-cpu-used", "8", "-crf", "40", "-c:a", "libopus"]),
        ];
        for (name, codecs) in &sources {
            let mut args: Vec<&str> = video.to_vec();
            args.extend_from_slice(&sound);
            args.extend_from_slice(codecs);
            let target = path(name);
            args.push(&target);
            ffmpeg(&tools, &args);
        }
        // Subtitles in an MP4 and an MKV.
        let srt = path("sub.srt");
        ffmpeg(&tools, &["-i", &path("h264-aac.mp4"), "-i", &srt, "-map", "0", "-map", "1", "-c", "copy", "-c:s", "mov_text", &path("subs.mp4")]);
        ffmpeg(&tools, &["-i", &path("h264-opus.mkv"), "-i", &srt, "-map", "0", "-map", "1", "-c", "copy", "-c:s", "srt", &path("subs.mkv")]);

        let mut failures = Vec::new();
        let mut checked = 0;
        let names: Vec<&str> = sources.iter().map(|(name, _)| *name).chain(["subs.mp4", "subs.mkv"]).collect();
        for name in names {
            let source = dir.path().join(name);
            let before = streams(&tools, &source);
            for target in [Target::Mp4, Target::Mov, Target::Mkv, Target::Webm, Target::M4a, Target::Opus] {
                let output = dir.path().join(format!("{name}.out.{}", target.extension()));
                let done = if matches!(target, Target::M4a | Target::Opus) {
                    audio_only(&source, &output, target, &cancel, &mut |_| {})
                } else {
                    remux(&source, &output, target, &cancel, &mut |_| {})
                };
                match done {
                    Ok(false) => println!("{name:>14} -> {:<5} left to ffmpeg", target.label()),
                    Ok(true) => {
                        checked += 1;
                        let after = streams(&tools, &output);
                        let errors = decodes_cleanly(&tools, &output);
                        let expected: Vec<&(String, u64)> = match target {
                            Target::M4a | Target::Opus => before.iter().filter(|(codec, _)| matches!(codec.as_str(), "aac" | "opus")).collect(),
                            Target::Webm => before.iter().filter(|(codec, _)| !matches!(codec.as_str(), "subrip" | "mov_text")).collect(),
                            _ => before.iter().collect(),
                        };
                        // Subtitle tracks differ in how many empty fillers sit
                        // between lines, so their text and times are compared.
                        let text = |codec: &str| matches!(codec, "subrip" | "mov_text");
                        let mut same = expected.len() == after.len()
                            && expected.iter().zip(&after).all(|(old, new)| if text(&old.0) { text(&new.0) } else { old == &new });
                        if after.iter().any(|(codec, _)| text(codec)) {
                            let lines = subtitles(&tools, &output);
                            same &= lines.contains("00:00:00,500 --> 00:00:01,200
Hello") && lines.contains("00:00:01,500 --> 00:00:01,900
World");
                            if !same {
                                println!("{lines}");
                            }
                        }
                        println!("{name:>14} -> {:<5} {after:?} {}", target.label(), if errors.is_empty() { "ok" } else { &errors });
                        if !same || !errors.is_empty() {
                            failures.push(format!("{name} -> {}: before {before:?}, after {after:?}, {errors}", target.label()));
                        }
                    }
                    Err(error) => failures.push(format!("{name} -> {}: {error}", target.label())),
                }
            }
        }
        assert!(checked > 20, "only {checked} conversions ran here");
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
