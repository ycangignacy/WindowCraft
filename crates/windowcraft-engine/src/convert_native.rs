// Made by Nordan
//! Conversions WindowCraft does itself, without ffmpeg.
//!
//! Pictures go through the `image` crate. Sound is decoded by Symphonia and
//! written as WAV or AIFF by the code below, as FLAC by `flacenc` and as MP3
//! by LAME. Every function answers `Ok(false)` when the job is outside what it
//! can do, so the caller can hand the file to ffmpeg instead.

use crate::convert::{Family, Target};
use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use std::fs::File;
use std::io::{BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

/// Converts `source` into `output` when that can be done here.
pub fn convert(source: &Path, output: &Path, target: Target, lossy_elsewhere: bool, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<bool> {
    match target.family() {
        Family::Image => convert_image(source, output, target, lossy_elsewhere),
        Family::Audio => convert_audio(source, output, target, cancel, progress),
        Family::Video => crate::remux::remux(source, output, target, cancel, progress),
    }
}

fn io_error(path: &Path) -> impl Fn(std::io::Error) -> EngineError + '_ {
    move |error| EngineError::io(path.display().to_string(), error)
}

fn image_error(path: &Path) -> impl Fn(image::ImageError) -> EngineError + '_ {
    move |error| EngineError::Invalid(format!("{}: {error}", path.display()))
}

// ---------------------------------------------------------------------------
// Pictures

fn convert_image(source: &Path, output: &Path, target: Target, lossy_elsewhere: bool) -> Result<bool> {
    use image::{DynamicImage, ImageDecoder, ImageFormat};
    let encodable = matches!(target, Target::Png | Target::Jpg | Target::Webp | Target::Bmp | Target::Tiff | Target::Gif | Target::Ico);
    if !encodable {
        return Ok(false);
    }
    let Ok(reader) = image::ImageReader::open(source).and_then(image::ImageReader::with_guessed_format) else {
        return Ok(false);
    };
    let Some(format) = reader.format() else { return Ok(false) };
    // The image crate writes WebP without loss only. A JPEG would grow, so a
    // lossy encoder elsewhere does better when it is there.
    if target == Target::Webp && format == ImageFormat::Jpeg && lossy_elsewhere {
        return Ok(false);
    }
    if format == ImageFormat::Gif && matches!(target, Target::Gif | Target::Webp) && animated_gif(source) {
        return Ok(false);
    }
    let mut decoder = match reader.into_decoder() {
        Ok(decoder) => decoder,
        Err(_) => return Ok(false),
    };
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut picture = DynamicImage::from_decoder(decoder).map_err(image_error(source))?;
    picture.apply_orientation(orientation);

    let file = File::create(output).map_err(io_error(output))?;
    let mut writer = BufWriter::new(file);
    let fail = image_error(output);
    let alpha = picture.color().has_alpha();
    match target {
        Target::Png => {
            let picture = match picture {
                DynamicImage::ImageRgb32F(_) | DynamicImage::ImageRgba32F(_) => DynamicImage::ImageRgba16(picture.to_rgba16()),
                other => other,
            };
            picture.write_with_encoder(image::codecs::png::PngEncoder::new(&mut writer)).map_err(fail)?;
        }
        Target::Jpg => {
            let flat = flatten_on_white(&picture);
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 90).encode_image(&flat).map_err(fail)?;
        }
        Target::Bmp | Target::Tiff | Target::Webp => {
            let picture = if alpha { DynamicImage::ImageRgba8(picture.to_rgba8()) } else { DynamicImage::ImageRgb8(picture.to_rgb8()) };
            match target {
                Target::Bmp => picture.write_with_encoder(image::codecs::bmp::BmpEncoder::new(&mut writer)),
                Target::Tiff => picture.write_with_encoder(image::codecs::tiff::TiffEncoder::new(&mut writer)),
                _ => picture.write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(&mut writer)),
            }
            .map_err(fail)?;
        }
        Target::Gif => {
            let mut encoder = image::codecs::gif::GifEncoder::new_with_speed(&mut writer, 10);
            encoder.encode_frame(image::Frame::new(picture.to_rgba8())).map_err(fail)?;
        }
        Target::Ico => {
            let icon = if picture.width() > 256 || picture.height() > 256 {
                picture.resize(256, 256, image::imageops::FilterType::Lanczos3)
            } else {
                picture
            };
            icon.to_rgba8().write_with_encoder(image::codecs::ico::IcoEncoder::new(&mut writer)).map_err(fail)?;
        }
        _ => return Ok(false),
    }
    writer.flush().map_err(io_error(output))?;
    Ok(true)
}

fn animated_gif(path: &Path) -> bool {
    use image::AnimationDecoder;
    let Ok(file) = File::open(path) else { return false };
    image::codecs::gif::GifDecoder::new(BufReader::new(file)).is_ok_and(|decoder| decoder.into_frames().take(2).count() > 1)
}

/// JPEG has no transparency; see-through pixels go on white, as viewers show them.
fn flatten_on_white(picture: &image::DynamicImage) -> image::RgbImage {
    if !picture.color().has_alpha() {
        return picture.to_rgb8();
    }
    let rgba = picture.to_rgba8();
    image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let blend = |channel: u8| ((u32::from(channel) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255) as u8;
        image::Rgb([blend(r), blend(g), blend(b)])
    })
}

// ---------------------------------------------------------------------------
// Sound

use symphonia::core::audio::{Channels, SampleBuffer};
use symphonia::core::codecs::{CODEC_TYPE_ALAC, CODEC_TYPE_FLAC, CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// An open audio track, ready to hand out interleaved samples.
struct Sound {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    rate: u32,
    channels: usize,
    layout: Option<Channels>,
    /// 24 for lossless sources deeper than 16 bits, otherwise 16.
    bits: u32,
    frames: Option<u64>,
    position: u64,
}

impl Sound {
    fn open(path: &Path) -> Option<Self> {
        let file = File::open(path).ok()?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
            hint.with_extension(extension);
        }
        let probed = symphonia::default::get_probe()
            .format(&hint, stream, &FormatOptions { enable_gapless: true, ..FormatOptions::default() }, &MetadataOptions::default())
            .ok()?;
        let format = probed.format;
        let codecs = symphonia::default::get_codecs();
        let track = format
            .tracks()
            .iter()
            .find(|track| track.codec_params.codec != CODEC_TYPE_NULL && track.codec_params.sample_rate.is_some() && codecs.get_codec(track.codec_params.codec).is_some())?;
        let params = track.codec_params.clone();
        let decoder = codecs.make(&params, &DecoderOptions::default()).ok()?;
        let pcm = codecs.get_codec(params.codec).is_some_and(|codec| codec.short_name.starts_with("pcm"));
        let lossless = pcm || matches!(params.codec, CODEC_TYPE_FLAC | CODEC_TYPE_ALAC);
        let bits = if lossless && params.bits_per_sample.unwrap_or(16) > 16 { 24 } else { 16 };
        Some(Self {
            track: track.id,
            rate: params.sample_rate?,
            channels: params.channels.map_or(0, Channels::count),
            layout: params.channels,
            bits,
            frames: params.n_frames,
            format,
            decoder,
            position: 0,
        })
    }

    /// The next decoded block as interleaved samples in -1.0..=1.0, or `None`
    /// at the end.
    fn next(&mut self) -> Result<Option<Vec<f32>>> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(SymphoniaError::ResetRequired) => return Ok(None),
                Err(error) => return Err(EngineError::Invalid(error.to_string())),
            };
            if packet.track_id() != self.track {
                continue;
            }
            self.position = packet.ts();
            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    let spec = *decoded.spec();
                    if self.channels == 0 {
                        self.channels = spec.channels.count();
                        self.layout = Some(spec.channels);
                    }
                    let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                    buffer.copy_interleaved_ref(decoded);
                    return Ok(Some(buffer.samples().to_vec()));
                }
                // A damaged packet is skipped, as players do.
                Err(SymphoniaError::DecodeError(_)) => {}
                Err(error) => return Err(EngineError::Invalid(error.to_string())),
            }
        }
    }

    fn share(&self) -> f32 {
        self.frames.filter(|frames| *frames > 0).map_or(0.0, |frames| (self.position as f32 / frames as f32).clamp(0.0, 1.0))
    }
}

fn convert_audio(source: &Path, output: &Path, target: Target, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<bool> {
    if matches!(target, Target::M4a | Target::Opus) {
        // AAC and Opus are copied out of their container; WindowCraft has no
        // encoder for them.
        return crate::remux::audio_only(source, output, target, cancel, progress);
    }
    if !matches!(target, Target::Mp3 | Target::Flac | Target::Wav | Target::Aiff) {
        return Ok(false);
    }
    let Some(mut sound) = Sound::open(source) else { return Ok(false) };
    match target {
        Target::Mp3 => write_mp3(&mut sound, output, cancel, progress)?,
        Target::Flac => write_flac(&mut sound, output, cancel, progress)?,
        Target::Wav => write_pcm(&mut sound, output, false, cancel, progress)?,
        _ => write_pcm(&mut sound, output, true, cancel, progress)?,
    }
    Ok(true)
}

fn to_int(sample: f32, bits: u32) -> i32 {
    let scale = ((1i64 << (bits - 1)) - 1) as f32;
    (sample * scale).round().clamp(-scale - 1.0, scale) as i32
}

/// WAV (little-endian) or AIFF (big-endian) PCM. The sizes are filled in once
/// the length is known.
fn write_pcm(sound: &mut Sound, output: &Path, aiff: bool, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<()> {
    let fail = io_error(output);
    let mut writer = BufWriter::with_capacity(1 << 20, File::create(output).map_err(&fail)?);
    let header = if aiff { 54 } else { 44 };
    writer.write_all(&vec![0u8; header]).map_err(&fail)?;
    let bytes_per_sample = (sound.bits / 8) as usize;
    let mut data_bytes = 0u64;
    let mut count = 0u32;
    while let Some(block) = sound.next()? {
        count += 1;
        if count.is_multiple_of(64) {
            cancel.check()?;
            progress(sound.share());
        }
        let mut bytes = Vec::with_capacity(block.len() * bytes_per_sample);
        for sample in block {
            let value = to_int(sample, sound.bits).to_le_bytes();
            if aiff {
                bytes.extend(value[..bytes_per_sample].iter().rev());
            } else {
                bytes.extend_from_slice(&value[..bytes_per_sample]);
            }
        }
        data_bytes += bytes.len() as u64;
        writer.write_all(&bytes).map_err(&fail)?;
    }
    if sound.channels == 0 {
        return Err(EngineError::Invalid(format!("{}: no sound decoded", output.display())));
    }
    if data_bytes % 2 == 1 {
        writer.write_all(&[0]).map_err(&fail)?;
    }
    let mut file = writer.into_inner().map_err(|e| fail(e.into_error()))?;
    let channels = sound.channels as u16;
    let frames = data_bytes / (bytes_per_sample as u64 * u64::from(channels));
    let mut head = Vec::with_capacity(header);
    if aiff {
        let form = 4 + 26 + 16 + data_bytes + data_bytes % 2;
        head.extend_from_slice(b"FORM");
        head.extend_from_slice(&(form as u32).to_be_bytes());
        head.extend_from_slice(b"AIFFCOMM");
        head.extend_from_slice(&18u32.to_be_bytes());
        head.extend_from_slice(&channels.to_be_bytes());
        head.extend_from_slice(&(frames as u32).to_be_bytes());
        head.extend_from_slice(&(sound.bits as u16).to_be_bytes());
        head.extend_from_slice(&extended(f64::from(sound.rate)));
        head.extend_from_slice(b"SSND");
        head.extend_from_slice(&((data_bytes + 8) as u32).to_be_bytes());
        head.extend_from_slice(&[0; 8]);
    } else {
        let block_align = channels * bytes_per_sample as u16;
        head.extend_from_slice(b"RIFF");
        head.extend_from_slice(&((36 + data_bytes + data_bytes % 2) as u32).to_le_bytes());
        head.extend_from_slice(b"WAVEfmt ");
        head.extend_from_slice(&16u32.to_le_bytes());
        head.extend_from_slice(&1u16.to_le_bytes());
        head.extend_from_slice(&channels.to_le_bytes());
        head.extend_from_slice(&sound.rate.to_le_bytes());
        head.extend_from_slice(&(sound.rate * u32::from(block_align)).to_le_bytes());
        head.extend_from_slice(&block_align.to_le_bytes());
        head.extend_from_slice(&(sound.bits as u16).to_le_bytes());
        head.extend_from_slice(b"data");
        head.extend_from_slice(&(data_bytes as u32).to_le_bytes());
    }
    if data_bytes > u64::from(u32::MAX) - 64 {
        return Err(EngineError::Invalid(format!("{}: longer than WAV and AIFF can hold", output.display())));
    }
    file.seek(SeekFrom::Start(0)).map_err(&fail)?;
    file.write_all(&head).map_err(&fail)?;
    file.sync_all().map_err(&fail)
}

/// The 80-bit extended float AIFF uses for the sample rate.
fn extended(value: f64) -> [u8; 10] {
    let mut out = [0u8; 10];
    if value <= 0.0 {
        return out;
    }
    let exponent = value.log2().floor() as i32;
    let mantissa = (value / 2f64.powi(exponent) * (1u64 << 63) as f64) as u64;
    out[..2].copy_from_slice(&((exponent + 16383) as u16).to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// Feeds decoded sound to flacenc block by block.
struct FlacFeed<'a> {
    sound: &'a mut Sound,
    pending: Vec<i32>,
    cancel: &'a CancelToken,
    progress: &'a mut dyn FnMut(f32),
    error: Option<EngineError>,
    blocks: u32,
}

impl flacenc::source::Source for FlacFeed<'_> {
    fn channels(&self) -> usize {
        self.sound.channels
    }

    fn bits_per_sample(&self) -> usize {
        self.sound.bits as usize
    }

    fn sample_rate(&self) -> usize {
        self.sound.rate as usize
    }

    fn read_samples<F: flacenc::source::Fill>(&mut self, block_size: usize, dest: &mut F) -> std::result::Result<usize, flacenc::error::SourceError> {
        let wanted = block_size * self.sound.channels;
        while self.pending.len() < wanted && self.error.is_none() {
            self.blocks += 1;
            if self.blocks.is_multiple_of(64) {
                if let Err(error) = self.cancel.check() {
                    self.error = Some(error);
                    break;
                }
                (self.progress)(self.sound.share());
            }
            match self.sound.next() {
                Ok(Some(block)) => self.pending.extend(block.into_iter().map(|sample| to_int(sample, self.sound.bits))),
                Ok(None) => break,
                Err(error) => self.error = Some(error),
            }
        }
        let take = self.pending.len().min(wanted);
        let take = take - take % self.sound.channels.max(1);
        let chunk: Vec<i32> = self.pending.drain(..take).collect();
        dest.fill_interleaved(&chunk)?;
        Ok(take / self.sound.channels.max(1))
    }
}

fn write_flac(sound: &mut Sound, output: &Path, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<()> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;
    if sound.channels == 0 || sound.channels > 8 {
        return Err(EngineError::Unsupported(format!("{} channels in FLAC", sound.channels)));
    }
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, error)| EngineError::Invalid(format!("FLAC settings: {error:?}")))?;
    let block_size = config.block_size;
    let mut feed = FlacFeed { sound, pending: Vec::new(), cancel, progress, error: None, blocks: 0 };
    let stream = flacenc::encode_with_fixed_block_size(&config, &mut feed, block_size);
    if let Some(error) = feed.error.take() {
        return Err(error);
    }
    let mut stream = stream.map_err(|error| EngineError::Invalid(format!("FLAC: {error:?}")))?;
    // flacenc counts the short last block as the minimum. The reference
    // encoder does not, and readers take unequal sizes for variable blocking.
    stream
        .stream_info_mut()
        .set_block_sizes(block_size, block_size)
        .map_err(|error| EngineError::Invalid(format!("FLAC: {error:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|error| EngineError::Invalid(format!("FLAC: {error:?}")))?;
    std::fs::write(output, sink.as_slice()).map_err(io_error(output))
}

/// Folds surround down to stereo with the usual ITU weights.
fn downmix(block: &[f32], channels: usize, layout: Option<Channels>) -> Vec<f32> {
    let order: Vec<Channels> = layout.map(|layout| layout.iter().collect()).unwrap_or_default();
    let weight = |index: usize| -> (f32, f32) {
        match order.get(index) {
            Some(&channel) if channel == Channels::FRONT_LEFT => (1.0, 0.0),
            Some(&channel) if channel == Channels::FRONT_RIGHT => (0.0, 1.0),
            Some(&channel) if channel == Channels::FRONT_CENTRE => (0.707, 0.707),
            Some(&channel) if channel == Channels::LFE1 => (0.0, 0.0),
            Some(&channel) if channel == Channels::REAR_LEFT || channel == Channels::SIDE_LEFT || channel == Channels::FRONT_LEFT_CENTRE => (0.707, 0.0),
            Some(&channel) if channel == Channels::REAR_RIGHT || channel == Channels::SIDE_RIGHT || channel == Channels::FRONT_RIGHT_CENTRE => (0.0, 0.707),
            _ if index == 0 => (1.0, 0.0),
            _ if index == 1 => (0.0, 1.0),
            _ => (0.5, 0.5),
        }
    };
    let weights: Vec<(f32, f32)> = (0..channels).map(weight).collect();
    let total_left: f32 = weights.iter().map(|w| w.0).sum::<f32>().max(1.0);
    let total_right: f32 = weights.iter().map(|w| w.1).sum::<f32>().max(1.0);
    let mut out = Vec::with_capacity(block.len() / channels * 2);
    for frame in block.chunks_exact(channels) {
        let (mut left, mut right) = (0.0, 0.0);
        for (sample, (to_left, to_right)) in frame.iter().zip(&weights) {
            left += sample * to_left;
            right += sample * to_right;
        }
        out.push(left / total_left);
        out.push(right / total_right);
    }
    out
}

/// MP3 through LAME at its top variable-bitrate setting (V0).
fn write_mp3(sound: &mut Sound, output: &Path, cancel: &CancelToken, progress: &mut dyn FnMut(f32)) -> Result<()> {
    use mp3lame_encoder::{Builder, FlushGap, InterleavedPcm, MonoPcm, Quality, VbrMode};
    let lame = |error: &dyn std::fmt::Display| EngineError::Invalid(format!("LAME: {error}"));
    let fail = io_error(output);
    // The first block tells how many channels there really are.
    let first = sound.next()?.ok_or_else(|| EngineError::Invalid(format!("{}: no sound decoded", output.display())))?;
    let channels = sound.channels;
    if channels == 0 {
        return Err(EngineError::Invalid(format!("{}: no channels", output.display())));
    }
    let out_channels: u8 = if channels == 1 { 1 } else { 2 };
    let mut builder = Builder::new().ok_or_else(|| EngineError::Invalid("LAME could not start".to_owned()))?;
    builder.set_num_channels(out_channels).map_err(|e| lame(&e))?;
    builder.set_sample_rate(sound.rate).map_err(|e| lame(&e))?;
    builder.set_vbr_mode(VbrMode::Mtrh).map_err(|e| lame(&e))?;
    builder.set_vbr_quality(Quality::Best).map_err(|e| lame(&e))?;
    builder.set_quality(Quality::NearBest).map_err(|e| lame(&e))?;
    builder.set_to_write_vbr_tag(true).map_err(|e| lame(&e))?;
    let mut encoder = builder.build().map_err(|e| lame(&e))?;

    let mut file = File::create(output).map_err(&fail)?;
    let mut writer = BufWriter::with_capacity(1 << 20, &mut file);
    let mut buffer: Vec<u8> = Vec::new();
    let mut block = Some(first);
    let mut count = 0u32;
    while let Some(samples) = block {
        count += 1;
        if count.is_multiple_of(64) {
            cancel.check()?;
            progress(sound.share());
        }
        let samples = if channels > 2 { downmix(&samples, channels, sound.layout) } else { samples };
        let frames = samples.len() / usize::from(out_channels);
        buffer.clear();
        buffer.reserve(mp3lame_encoder::max_required_buffer_size(frames));
        if out_channels == 1 {
            encoder.encode_to_vec(MonoPcm(samples.as_slice()), &mut buffer).map_err(|e| lame(&e))?;
        } else {
            encoder.encode_to_vec(InterleavedPcm(samples.as_slice()), &mut buffer).map_err(|e| lame(&e))?;
        }
        writer.write_all(&buffer).map_err(&fail)?;
        block = sound.next()?;
    }
    buffer.clear();
    buffer.reserve(7200);
    encoder.flush_to_vec::<FlushGap>(&mut buffer).map_err(|e| lame(&e))?;
    writer.write_all(&buffer).map_err(&fail)?;
    writer.flush().map_err(&fail)?;
    drop(writer);
    // The Xing/LAME frame at the start gets the real length now, so players
    // can seek and show the duration of a variable-bitrate file.
    let mut tag = Vec::with_capacity(encoder.lame_tag_size());
    if encoder.lame_tag_encode_to_vec(&mut tag).is_some() {
        file.seek(SeekFrom::Start(0)).map_err(&fail)?;
        file.write_all(&tag).map_err(&fail)?;
    }
    file.sync_all().map_err(&fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_wav(path: &Path, rate: u32, channels: u16, seconds: f32) {
        let frames = (rate as f32 * seconds) as usize;
        let mut data = Vec::new();
        for index in 0..frames {
            let value = ((index as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 12000.0) as i16;
            for _ in 0..channels {
                data.extend_from_slice(&value.to_le_bytes());
            }
        }
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
        file.extend_from_slice(b"WAVEfmt ");
        file.extend_from_slice(&16u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&channels.to_le_bytes());
        file.extend_from_slice(&rate.to_le_bytes());
        file.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        file.extend_from_slice(&(channels * 2).to_le_bytes());
        file.extend_from_slice(&16u16.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&(data.len() as u32).to_le_bytes());
        file.extend_from_slice(&data);
        std::fs::write(path, file).unwrap();
    }

    fn convert_to(source: &Path, target: Target) -> std::path::PathBuf {
        let output = source.with_extension(format!("out.{}", target.extension()));
        assert!(convert(source, &output, target, false, &CancelToken::new(), &mut |_| {}).unwrap(), "{target:?} must be done here");
        output
    }

    #[test]
    fn sound_round_trips_through_every_native_format() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("tone.wav");
        tone_wav(&wav, 44_100, 2, 1.0);
        for target in [Target::Flac, Target::Aiff, Target::Wav, Target::Mp3] {
            let output = convert_to(&wav, target);
            let mut sound = Sound::open(&output).unwrap_or_else(|| panic!("{target:?} output must decode"));
            let mut samples = 0usize;
            while let Some(block) = sound.next().unwrap() {
                samples += block.len();
            }
            assert_eq!(sound.channels, 2, "{target:?}");
            assert_eq!(sound.rate, 44_100, "{target:?}");
            // MP3 works in frames of 1152; a reader trims the encoder delay
            // from the LAME header, so only the last frame may differ.
            let expected = 44_100 * 2;
            if target == Target::Mp3 {
                assert!(samples.abs_diff(expected) <= 2 * 1152, "{samples}");
            } else {
                assert_eq!(samples, expected, "{target:?}");
            }
        }
    }

    #[test]
    fn mp3_starts_with_a_vbr_header() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("tone.wav");
        tone_wav(&wav, 48_000, 1, 2.0);
        let mp3 = std::fs::read(convert_to(&wav, Target::Mp3)).unwrap();
        assert_eq!(mp3[0], 0xFF, "no ID3 tag in front");
        assert!(mp3.windows(4).take(200).any(|window| window == b"Xing" || window == b"Info"));
    }

    #[test]
    fn surround_folds_to_stereo_without_clipping() {
        let layout = Channels::FRONT_LEFT | Channels::FRONT_RIGHT | Channels::FRONT_CENTRE | Channels::LFE1 | Channels::REAR_LEFT | Channels::REAR_RIGHT;
        let frame = [1.0f32; 6];
        let stereo = downmix(&frame, 6, Some(layout));
        assert_eq!(stereo.len(), 2);
        assert!(stereo.iter().all(|sample| *sample <= 1.0 && *sample > 0.9));
    }

    #[test]
    fn aiff_rate_is_written_as_extended_float() {
        assert_eq!(extended(44_100.0), [0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
        assert_eq!(extended(48_000.0), [0x40, 0x0E, 0xBB, 0x80, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn pictures_convert_and_keep_their_size() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("p.png");
        let mut picture = image::RgbaImage::new(300, 120);
        for (x, _, pixel) in picture.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x % 256) as u8, 40, 200, if x < 150 { 0 } else { 255 }]);
        }
        picture.save(&png).unwrap();
        for target in [Target::Jpg, Target::Webp, Target::Bmp, Target::Tiff, Target::Gif, Target::Png, Target::Ico] {
            let output = convert_to(&png, target);
            let back = image::open(&output).unwrap_or_else(|error| panic!("{target:?}: {error}"));
            let expected = if target == Target::Ico { (256, 102) } else { (300, 120) };
            assert_eq!((back.width(), back.height()), expected, "{target:?}");
        }
        // Transparent pixels become white in a JPEG.
        let jpeg = image::open(png.with_extension("out.jpg")).unwrap().to_rgb8();
        assert!(jpeg.get_pixel(10, 10).0.iter().all(|channel| *channel > 240));
    }

    #[test]
    fn exif_orientation_is_applied() {
        let dir = tempfile::tempdir().unwrap();
        let jpg = dir.path().join("turned.jpg");
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes).encode_image(&image::RgbImage::new(40, 20)).unwrap();
        // Insert an EXIF segment that says "rotate 90 degrees clockwise".
        let exif = crate::metadata::tests_support::orientation_segment(6);
        bytes.splice(2..2, exif);
        std::fs::write(&jpg, &bytes).unwrap();
        let back = image::open(convert_to(&jpg, Target::Png)).unwrap();
        assert_eq!((back.width(), back.height()), (20, 40));
    }

    #[test]
    fn leaves_what_it_cannot_do_to_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("p.png");
        image::RgbImage::new(4, 4).save(&png).unwrap();
        let output = dir.path().join("p.avif");
        assert!(!convert(&png, &output, Target::Avif, false, &CancelToken::new(), &mut |_| {}).unwrap());
        assert!(!convert(&png, &output, Target::Opus, false, &CancelToken::new(), &mut |_| {}).unwrap());
        let jpg = dir.path().join("p.jpg");
        image::RgbImage::new(4, 4).save(&jpg).unwrap();
        assert!(!convert(&jpg, &dir.path().join("p.webp"), Target::Webp, true, &CancelToken::new(), &mut |_| {}).unwrap());
        assert!(convert(&jpg, &dir.path().join("p.webp"), Target::Webp, false, &CancelToken::new(), &mut |_| {}).unwrap());
    }
}
