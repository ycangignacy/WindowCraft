// Made by ycangignacy
//! Removes metadata from audio and video containers without ffmpeg.
//!
//! The streams are never touched. Most containers are cleaned in place on a
//! copy: a metadata block is turned into padding of the same length and its
//! bytes are zeroed, so no offset in the file moves and the old text is gone.
//! Ogg has no padding of its own, so its header pages are written anew.
//!
//! What goes: titles, artists, comments, cover art, chapters, creation dates,
//! encoder names, GPS (QuickTime `©xyz`, Apple location keys), XMP, and timed
//! metadata tracks such as camera GPS logs, whose samples are zeroed too.

use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::os::windows::fs::FileExt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    /// MP4, MOV, M4A, M4V, 3GP.
    IsoBmff,
    /// MKV, MKA, WebM.
    Matroska,
    /// WAV and AVI.
    Riff,
    Aiff,
    /// Ogg Vorbis and Opus.
    Ogg,
    /// WMA and WMV.
    Asf,
}

const ASF_HEADER: [u8; 16] = [0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C];

/// Recognizes a container from the first 16 bytes.
pub fn detect(head: &[u8]) -> Option<Container> {
    if head.len() >= 8 && matches!(&head[4..8], b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip") {
        return Some(Container::IsoBmff);
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some(Container::Matroska);
    }
    if head.len() >= 12 && matches!(&head[..4], b"RIFF" | b"RF64") && matches!(&head[8..12], b"WAVE" | b"AVI ") {
        return Some(Container::Riff);
    }
    if head.len() >= 12 && &head[..4] == b"FORM" && matches!(&head[8..12], b"AIFF" | b"AIFC") {
        return Some(Container::Aiff);
    }
    if head.starts_with(b"OggS") {
        return Some(Container::Ogg);
    }
    if head.starts_with(&ASF_HEADER) {
        return Some(Container::Asf);
    }
    None
}

/// Writes a clean copy of `source` to `target` and returns how many metadata
/// blocks went. `EngineError::Unsupported` means this layout is not handled
/// here and ffmpeg may still manage it.
pub fn strip(container: Container, source: &Path, target: &Path, cancel: &CancelToken) -> Result<usize> {
    let name = source.display().to_string();
    if container == Container::Ogg {
        return strip_ogg(source, target, &name, cancel);
    }
    if source != target {
        std::fs::copy(source, target).map_err(|e| EngineError::io(name.clone(), e))?;
    }
    let file = std::fs::OpenOptions::new().read(true).write(true).open(target).map_err(|e| EngineError::io(name.clone(), e))?;
    let len = file.metadata().map_err(|e| EngineError::io(name.clone(), e))?.len();
    let io = Io { file: &file, len, name: &name };
    let removed = match container {
        Container::IsoBmff => strip_isobmff(&io, cancel),
        Container::Matroska => strip_matroska(&io, cancel),
        Container::Riff => strip_riff(&io),
        Container::Aiff => strip_aiff(&io),
        Container::Asf => strip_asf(&io),
        Container::Ogg => unreachable!("handled above"),
    }?;
    file.sync_all().map_err(|e| EngineError::io(name.clone(), e))?;
    Ok(removed)
}

/// Random access to the copy being cleaned.
struct Io<'a> {
    file: &'a File,
    len: u64,
    name: &'a str,
}

impl Io<'_> {
    fn damaged(&self, what: &str) -> EngineError {
        EngineError::Invalid(format!("{}: {what}", self.name))
    }

    fn read(&self, at: u64, buffer: &mut [u8]) -> Result<()> {
        let mut done = 0;
        while done < buffer.len() {
            let read = self.file.seek_read(&mut buffer[done..], at + done as u64).map_err(|e| EngineError::io(self.name, e))?;
            if read == 0 {
                return Err(self.damaged("the file ends too early"));
            }
            done += read;
        }
        Ok(())
    }

    fn read_vec(&self, at: u64, len: u64) -> Result<Vec<u8>> {
        if at.saturating_add(len) > self.len || len > 512 << 20 {
            return Err(self.damaged("a block runs past the end"));
        }
        let mut buffer = vec![0; len as usize];
        self.read(at, &mut buffer)?;
        Ok(buffer)
    }

    fn write(&self, at: u64, data: &[u8]) -> Result<()> {
        let mut done = 0;
        while done < data.len() {
            let written = self.file.seek_write(&data[done..], at + done as u64).map_err(|e| EngineError::io(self.name, e))?;
            done += written;
        }
        Ok(())
    }

    fn zero(&self, at: u64, len: u64) -> Result<()> {
        let end = at.saturating_add(len).min(self.len);
        let block = vec![0u8; 1 << 16];
        let mut position = at;
        while position < end {
            let step = (end - position).min(block.len() as u64) as usize;
            self.write(position, &block[..step])?;
            position += step as u64;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ISO base media (MP4, MOV, M4A)

const XMP_UUID: [u8; 16] = [0xBE, 0x7A, 0xCF, 0xCB, 0x97, 0xA9, 0x42, 0xE8, 0x9C, 0x71, 0x99, 0x94, 0x91, 0xE3, 0xAF, 0xAC];

#[derive(Debug, Clone, Copy)]
struct Atom {
    start: u64,
    header: u64,
    size: u64,
    kind: [u8; 4],
}

impl Atom {
    fn payload(&self) -> (u64, u64) {
        (self.start + self.header, self.start + self.size)
    }
}

fn atoms(io: &Io, start: u64, end: u64) -> Result<Vec<Atom>> {
    let mut found = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let mut head = [0u8; 8];
        io.read(at, &mut head)?;
        let small = u64::from(u32::from_be_bytes([head[0], head[1], head[2], head[3]]));
        let kind = [head[4], head[5], head[6], head[7]];
        let (header, size) = match small {
            0 => (8, end - at),
            1 => {
                let mut large = [0u8; 8];
                io.read(at + 8, &mut large)?;
                (16, u64::from_be_bytes(large))
            }
            size => (8, size),
        };
        if size < header || at + size > end {
            // Some recorders leave a few stray bytes at the end.
            if found.is_empty() {
                return Err(io.damaged("a box has an impossible size"));
            }
            break;
        }
        found.push(Atom { start: at, header, size, kind });
        at += size;
    }
    Ok(found)
}

/// Turns a box into `free` and zeroes what it held.
fn free_atom(io: &Io, atom: &Atom) -> Result<()> {
    io.write(atom.start + 4, b"free")?;
    let (from, to) = atom.payload();
    io.zero(from, to - from)
}

fn is_xmp(io: &Io, atom: &Atom) -> Result<bool> {
    if atom.size < atom.header + 16 {
        return Ok(false);
    }
    let mut id = [0u8; 16];
    io.read(atom.start + atom.header, &mut id)?;
    Ok(id == XMP_UUID)
}

/// `mvhd`, `tkhd` and `mdhd` carry creation and modification times.
fn zero_times(io: &Io, atom: &Atom) -> Result<()> {
    let (from, to) = atom.payload();
    if to - from < 4 {
        return Ok(());
    }
    let mut version = [0u8; 1];
    io.read(from, &mut version)?;
    let width = if version[0] == 1 { 16 } else { 8 };
    if to - from >= 4 + width {
        io.zero(from + 4, width)?;
    }
    Ok(())
}

fn strip_isobmff(io: &Io, cancel: &CancelToken) -> Result<usize> {
    let top = atoms(io, 0, io.len)?;
    let Some(movie) = top.iter().find(|atom| &atom.kind == b"moov") else {
        // HEIF and AVIF pictures keep the image itself in a top-level `meta`.
        return Err(EngineError::Unsupported(format!("{}: no movie box", io.name)));
    };
    let mut removed = 0;
    for atom in &top {
        cancel.check()?;
        match &atom.kind {
            b"meta" => {
                free_atom(io, atom)?;
                removed += 1;
            }
            b"uuid" if is_xmp(io, atom)? => {
                free_atom(io, atom)?;
                removed += 1;
            }
            // Padding sometimes holds what an editor wrote there before.
            b"free" | b"skip" | b"wide" => {
                let (from, to) = atom.payload();
                io.zero(from, to - from)?;
            }
            _ => {}
        }
    }
    let (from, to) = movie.payload();
    for child in atoms(io, from, to)? {
        cancel.check()?;
        match &child.kind {
            b"udta" | b"meta" => {
                free_atom(io, &child)?;
                removed += 1;
            }
            b"uuid" if is_xmp(io, &child)? => {
                free_atom(io, &child)?;
                removed += 1;
            }
            b"mvhd" => zero_times(io, &child)?,
            b"trak" => {
                if track_handler(io, &child)? == Some(*b"meta") {
                    zero_samples(io, &child)?;
                    free_atom(io, &child)?;
                    removed += 1;
                } else {
                    removed += strip_track(io, &child)?;
                }
            }
            _ => {}
        }
    }
    Ok(removed)
}

fn find_atom(io: &Io, parent: &Atom, path: &[&[u8; 4]]) -> Result<Option<Atom>> {
    let mut current = *parent;
    for kind in path {
        let (from, to) = current.payload();
        match atoms(io, from, to)?.into_iter().find(|atom| &atom.kind == *kind) {
            Some(atom) => current = atom,
            None => return Ok(None),
        }
    }
    Ok(Some(current))
}

fn track_handler(io: &Io, track: &Atom) -> Result<Option<[u8; 4]>> {
    let Some(handler) = find_atom(io, track, &[b"mdia", b"hdlr"])? else { return Ok(None) };
    let (from, to) = handler.payload();
    if to - from < 12 {
        return Ok(None);
    }
    let mut kind = [0u8; 4];
    io.read(from + 8, &mut kind)?;
    Ok(Some(kind))
}

fn strip_track(io: &Io, track: &Atom) -> Result<usize> {
    let mut removed = 0;
    let (from, to) = track.payload();
    for child in atoms(io, from, to)? {
        match &child.kind {
            b"tkhd" => zero_times(io, &child)?,
            b"udta" | b"meta" => {
                free_atom(io, &child)?;
                removed += 1;
            }
            b"mdia" => {
                if let Some(header) = find_atom(io, &child, &[b"mdhd"])? {
                    zero_times(io, &header)?;
                }
                // Handler names are free text; YouTube writes its name and a date there.
                for path in [&[b"hdlr"][..], &[b"minf", b"hdlr"][..]] {
                    if let Some(handler) = find_atom(io, &child, path)? {
                        let (from, to) = handler.payload();
                        if to - from > 24 {
                            io.zero(from + 24, to - from - 24)?;
                        }
                    }
                }
                if let Some(descriptions) = find_atom(io, &child, &[b"minf", b"stbl", b"stsd"])? {
                    let video = track_handler(io, track)? == Some(*b"vide");
                    let (from, to) = descriptions.payload();
                    for entry in atoms(io, from + 8, to)? {
                        let (start, end) = entry.payload();
                        // QuickTime puts a vendor code here, for example FFMP.
                        if end - start >= 16 {
                            io.zero(start + 12, 4)?;
                        }
                        // The compressor name says which encoder and version made it.
                        if video && end - start >= 74 {
                            io.zero(start + 42, 32)?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(removed)
}

/// Zeroes the samples of a timed metadata track, such as a GPS log, so the
/// readings do not stay behind in `mdat` once the track is gone.
fn zero_samples(io: &Io, track: &Atom) -> Result<()> {
    let Some(table) = find_atom(io, track, &[b"mdia", b"minf", b"stbl"])? else { return Ok(()) };
    let (from, to) = table.payload();
    let children = atoms(io, from, to)?;
    let load = |kind: &[u8; 4]| -> Result<Option<Vec<u8>>> {
        match children.iter().find(|atom| &atom.kind == kind) {
            Some(atom) => {
                let (from, to) = atom.payload();
                io.read_vec(from, to - from).map(Some)
            }
            None => Ok(None),
        }
    };
    let (Some(sizes), Some(chunks)) = (load(b"stsz")?, load(b"stsc")?) else { return Ok(()) };
    let offsets: Vec<u64> = if let Some(stco) = load(b"stco")? {
        table_entries(&stco, 4).map(|entry| u64::from(u32::from_be_bytes(entry.try_into().unwrap_or_default()))).collect()
    } else if let Some(co64) = load(b"co64")? {
        table_entries(&co64, 8).map(|entry| u64::from_be_bytes(entry.try_into().unwrap_or_default())).collect()
    } else {
        return Ok(());
    };
    let fixed = sizes.get(4..8).map_or(0, |b| u64::from(u32::from_be_bytes(b.try_into().unwrap_or_default())));
    let sample_sizes: Vec<u64> = if fixed == 0 {
        sizes.get(12..).unwrap_or_default().as_chunks::<4>().0.iter().map(|b| u64::from(u32::from_be_bytes(*b))).collect()
    } else {
        let count = sizes.get(8..12).map_or(0, |b| u32::from_be_bytes(b.try_into().unwrap_or_default()));
        vec![fixed; count.min(50_000_000) as usize]
    };
    let runs: Vec<(u32, u32)> = table_entries(&chunks, 12)
        .map(|entry| {
            let first = u32::from_be_bytes(entry[0..4].try_into().unwrap_or_default());
            let per_chunk = u32::from_be_bytes(entry[4..8].try_into().unwrap_or_default());
            (first, per_chunk)
        })
        .collect();
    let mut sample = 0usize;
    for (index, offset) in offsets.iter().enumerate() {
        let number = index as u32 + 1;
        let per_chunk = runs.iter().rev().find(|(first, _)| *first <= number).map_or(0, |(_, count)| *count) as usize;
        let end = (sample + per_chunk).min(sample_sizes.len());
        let bytes: u64 = sample_sizes[sample.min(end)..end].iter().sum();
        if *offset < io.len {
            io.zero(*offset, bytes)?;
        }
        sample = end;
    }
    Ok(())
}

/// Entries of a full box table: version and flags, a count, then the rows.
fn table_entries(payload: &[u8], width: usize) -> impl Iterator<Item = &[u8]> {
    let count = payload.get(4..8).map_or(0, |b| u32::from_be_bytes(b.try_into().unwrap_or_default()) as usize);
    payload.get(8..).unwrap_or_default().chunks_exact(width).take(count)
}

// ---------------------------------------------------------------------------
// Matroska and WebM

const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TAGS: u32 = 0x1254_C367;
const CHAPTERS: u32 = 0x1043_A770;
const ATTACHMENTS: u32 = 0x1941_A469;
const ATTACHED_FILE: u32 = 0x61A7;
const FILE_MIME: u32 = 0x4660;
const CLUSTER: u32 = 0x1F43_B675;
const CRC32: u32 = 0xBF;
const VOID: u8 = 0xEC;
/// Info children that name, date or identify the file.
const INFO_DROP: [u32; 6] = [0x7BA9, 0x4461, 0x7384, 0x73A4, 0x003C_83AB, 0x003E_83BB];
/// MuxingApp and WritingApp must stay, so their text is blanked instead.
const INFO_BLANK: [u32; 2] = [0x4D80, 0x5741];
const TRACK_NAME: u32 = 0x536E;

/// An element ID keeps its length marker, as the specification writes them.
pub(crate) fn ebml_id(data: &[u8]) -> Option<(u32, usize)> {
    let first = *data.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 4 || data.len() < len {
        return None;
    }
    Some((data[..len].iter().fold(0u32, |value, byte| value << 8 | u32::from(*byte)), len))
}

/// A size: its value, its length, and whether it is "unknown".
pub(crate) fn ebml_size(data: &[u8]) -> Option<(u64, usize, bool)> {
    let first = *data.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || data.len() < len {
        return None;
    }
    let mask = if len == 8 { 0 } else { 0xFFu8 >> len };
    let mut value = u64::from(first & mask);
    let mut all_ones = first & mask == mask;
    for byte in &data[1..len] {
        value = value << 8 | u64::from(*byte);
        all_ones &= *byte == 0xFF;
    }
    Some((value, len, all_ones))
}

#[derive(Debug, Clone, Copy)]
struct Element {
    id: u32,
    start: u64,
    header: u64,
    /// `None` for an element of unknown size.
    size: Option<u64>,
}

fn element_at(io: &Io, at: u64) -> Result<Option<Element>> {
    if at + 2 > io.len {
        return Ok(None);
    }
    let mut head = [0u8; 12];
    let available = (io.len - at).min(12) as usize;
    io.read(at, &mut head[..available])?;
    let Some((id, id_len)) = ebml_id(&head[..available]) else { return Err(io.damaged("bad element ID")) };
    let Some((size, size_len, unknown)) = ebml_size(&head[id_len..available]) else {
        return Err(io.damaged("bad element size"));
    };
    Ok(Some(Element { id, start: at, header: (id_len + size_len) as u64, size: (!unknown).then_some(size) }))
}

/// The header of a Void element that spans `len` bytes in total.
fn void_header(len: usize) -> Vec<u8> {
    if len - 2 < 127 {
        vec![VOID, 0x80 | (len - 2) as u8]
    } else {
        let mut header = vec![VOID, 0x01];
        header.extend_from_slice(&((len - 9) as u64).to_be_bytes()[1..]);
        header
    }
}

/// Overwrites `buffer` with one Void element of exactly its length.
fn write_void(buffer: &mut [u8]) {
    buffer.fill(0);
    let header = void_header(buffer.len());
    buffer[..header.len()].copy_from_slice(&header);
}

/// Children of an in-memory master, as (id, start, header, total length).
fn children_in(data: &[u8], from: usize) -> Vec<(u32, usize, usize, usize)> {
    let mut found = Vec::new();
    let mut at = from;
    while at < data.len() {
        let Some((id, id_len)) = ebml_id(&data[at..]) else { break };
        let Some((size, size_len, unknown)) = ebml_size(&data[at + id_len..]) else { break };
        let header = id_len + size_len;
        let total = header as u64 + size;
        if unknown || at as u64 + total > data.len() as u64 {
            break;
        }
        found.push((id, at, header, total as usize));
        at += total as usize;
    }
    found
}

fn crc32_ieee(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { crc >> 1 ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Refreshes a master's CRC-32 child after its contents changed.
fn refresh_crc(master: &mut [u8], header: usize) {
    let children = children_in(master, header);
    if let Some(&(CRC32, start, crc_header, total)) = children.first()
        && total == crc_header + 4
    {
        let crc = crc32_ieee(&master[start + total..]);
        master[start + crc_header..start + total].copy_from_slice(&crc.to_le_bytes());
    }
}

/// Cleans an Info, Tracks, Attachments or SeekHead master held in memory.
fn clean_master(master: &mut [u8], header: usize, id: u32, gone: &[u32]) -> usize {
    let mut removed = 0;
    for (child, start, child_header, total) in children_in(master, header) {
        let range = start..start + total;
        match id {
            INFO if INFO_DROP.contains(&child) => {
                write_void(&mut master[range]);
                removed += 1;
            }
            INFO if INFO_BLANK.contains(&child) => master[start + child_header..start + total].fill(0),
            TRACKS if child == TRACK_ENTRY => {
                for (field, field_start, _, field_total) in children_in(&master[range.clone()], child_header) {
                    if field == TRACK_NAME {
                        let at = start + field_start;
                        write_void(&mut master[at..at + field_total]);
                        removed += 1;
                    }
                }
            }
            ATTACHMENTS if child == ATTACHED_FILE => {
                let picture = children_in(&master[range.clone()], child_header).into_iter().any(|(field, at, field_header, field_total)| {
                    field == FILE_MIME && master[start + at + field_header..start + at + field_total].starts_with(b"image/")
                });
                // Fonts for subtitles stay; pictures are cover art.
                if picture {
                    write_void(&mut master[range]);
                    removed += 1;
                }
            }
            SEEK_HEAD if child == SEEK => {
                let target = children_in(&master[range.clone()], child_header).into_iter().find_map(|(field, at, field_header, field_total)| {
                    (field == SEEK_ID).then(|| ebml_id(&master[start + at + field_header..start + at + field_total]).map(|(id, _)| id)).flatten()
                });
                if target.is_some_and(|target| gone.contains(&target)) {
                    write_void(&mut master[range]);
                }
            }
            _ => {}
        }
    }
    refresh_crc(master, header);
    removed
}

fn strip_matroska(io: &Io, cancel: &CancelToken) -> Result<usize> {
    let ebml = element_at(io, 0)?.ok_or_else(|| io.damaged("empty file"))?;
    let segment_at = ebml.start + ebml.header + ebml.size.ok_or_else(|| io.damaged("EBML header without size"))?;
    let segment = element_at(io, segment_at)?.filter(|element| element.id == SEGMENT).ok_or_else(|| io.damaged("no segment"))?;
    let data_start = segment.start + segment.header;
    let data_end = segment.size.map_or(io.len, |size| (data_start + size).min(io.len));

    // Walk the top level; clusters of unknown size end the walk, and the
    // SeekHead points to what lies beyond them.
    let mut found: Vec<Element> = Vec::new();
    let mut at = data_start;
    while at < data_end {
        cancel.check()?;
        let Some(element) = element_at(io, at)? else { break };
        let Some(size) = element.size else { break };
        if element.id != CLUSTER {
            found.push(element);
        }
        at = element.start + element.header + size;
    }
    let mut seen_heads = 0;
    let mut index = 0;
    while index < found.len() {
        let element = found[index];
        index += 1;
        if element.id != SEEK_HEAD || seen_heads > 4 {
            continue;
        }
        seen_heads += 1;
        let data = io.read_vec(element.start, element.header + element.size.unwrap_or(0))?;
        for (child, start, header, total) in children_in(&data, element.header as usize) {
            if child != SEEK {
                continue;
            }
            let fields = children_in(&data[start..start + total], header);
            let position = fields.iter().find(|field| field.0 == SEEK_POSITION).map(|&(_, at, field_header, field_total)| {
                data[start + at + field_header..start + at + field_total].iter().fold(0u64, |value, byte| value << 8 | u64::from(*byte))
            });
            if let Some(position) = position
                && !found.iter().any(|known| known.start == data_start + position)
                && let Some(target) = element_at(io, data_start + position)?
                && target.size.is_some()
                && target.id != CLUSTER
            {
                found.push(target);
            }
        }
    }

    let mut removed = 0;
    let mut gone = Vec::new();
    for element in found.iter().filter(|element| matches!(element.id, TAGS | CHAPTERS)) {
        let total = element.header + element.size.unwrap_or(0);
        let head = void_header(total as usize);
        io.write(element.start, &head)?;
        io.zero(element.start + head.len() as u64, total - head.len() as u64)?;
        gone.push(element.id);
        removed += 1;
    }
    for element in found.iter().filter(|element| matches!(element.id, INFO | TRACKS | ATTACHMENTS | SEEK_HEAD)) {
        cancel.check()?;
        let total = element.header + element.size.unwrap_or(0);
        let mut data = io.read_vec(element.start, total)?;
        let before = data.clone();
        removed += clean_master(&mut data, element.header as usize, element.id, &gone);
        if data != before {
            io.write(element.start, &data)?;
        }
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// RIFF: WAV and AVI

/// WAV chunks with tags, broadcast descriptions, XMP and the like.
const RIFF_DROP: [&[u8; 4]; 10] = [b"id3 ", b"ID3 ", b"bext", b"iXML", b"_PMX", b"cart", b"DISP", b"axml", b"umid", b"IDIT"];

fn le32_at(data: &[u8]) -> u64 {
    u64::from(u32::from_le_bytes(data[..4].try_into().unwrap_or_default()))
}

/// Renames a chunk to JUNK, which every RIFF reader skips, and zeroes it.
fn junk_chunk(io: &Io, at: u64, size: u64) -> Result<()> {
    io.write(at, b"JUNK")?;
    io.zero(at + 8, size)
}

fn strip_riff(io: &Io) -> Result<usize> {
    let mut head = [0u8; 12];
    io.read(0, &mut head)?;
    let end = (8 + le32_at(&head[4..])).min(io.len);
    walk_riff(io, 12, end, 0)
}

fn walk_riff(io: &Io, start: u64, end: u64, depth: u32) -> Result<usize> {
    let mut removed = 0;
    let mut at = start;
    while at + 8 <= end {
        let mut head = [0u8; 12];
        let available = (end - at).min(12) as usize;
        io.read(at, &mut head[..available])?;
        let id: [u8; 4] = head[..4].try_into().unwrap_or_default();
        let size = le32_at(&head[4..]);
        if &id == b"data" && size == 0xFFFF_FFFF {
            break;
        }
        if at + 8 + size > end {
            break;
        }
        if RIFF_DROP.contains(&&id) || &id == b"strn" {
            junk_chunk(io, at, size)?;
            removed += 1;
        } else if &id == b"LIST" && available == 12 {
            match &head[8..12] {
                b"INFO" => {
                    junk_chunk(io, at, size)?;
                    removed += 1;
                }
                // AVI keeps a date and stream names inside its header lists.
                b"hdrl" | b"strl" if depth < 3 => removed += walk_riff(io, at + 12, at + 8 + size, depth + 1)?,
                _ => {}
            }
        }
        at += 8 + size + (size & 1);
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// AIFF

const AIFF_DROP: [&[u8; 4]; 7] = [b"NAME", b"AUTH", b"(c) ", b"ANNO", b"COMT", b"ID3 ", b"id3 "];

fn strip_aiff(io: &Io) -> Result<usize> {
    let mut head = [0u8; 12];
    io.read(0, &mut head)?;
    let end = (8 + u64::from(u32::from_be_bytes(head[4..8].try_into().unwrap_or_default()))).min(io.len);
    let mut removed = 0;
    let mut at = 12;
    while at + 8 <= end {
        let mut chunk = [0u8; 8];
        io.read(at, &mut chunk)?;
        let id: [u8; 4] = chunk[..4].try_into().unwrap_or_default();
        let size = u64::from(u32::from_be_bytes(chunk[4..8].try_into().unwrap_or_default()));
        if at + 8 + size > end {
            break;
        }
        if AIFF_DROP.contains(&&id) {
            // Readers skip chunks they do not know; FLLR is Apple's filler.
            io.write(at, b"FLLR")?;
            io.zero(at + 8, size)?;
            removed += 1;
        }
        at += 8 + size + (size & 1);
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// ASF: WMA and WMV

const ASF_CONTENT_DESCRIPTION: [u8; 16] = [0x33, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C];
const ASF_EXTENDED_CONTENT: [u8; 16] = [0x40, 0xA4, 0xD0, 0xD2, 0x07, 0xE3, 0xD2, 0x11, 0x97, 0xF0, 0x00, 0xA0, 0xC9, 0x5E, 0xA8, 0x50];
const ASF_HEADER_EXTENSION: [u8; 16] = [0xB5, 0x03, 0xBF, 0x5F, 0x2E, 0xA9, 0xCF, 0x11, 0x8E, 0xE3, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65];
const ASF_METADATA: [u8; 16] = [0xEA, 0xCB, 0xF8, 0xC5, 0xAF, 0x5B, 0x77, 0x48, 0x84, 0x67, 0xAA, 0x8C, 0x44, 0xFA, 0x4C, 0xCA];
const ASF_METADATA_LIBRARY: [u8; 16] = [0x94, 0x1C, 0x23, 0x44, 0x98, 0x94, 0xD1, 0x49, 0xA1, 0x41, 0x1D, 0x13, 0x4E, 0x45, 0x70, 0x54];
const ASF_FILE_PROPERTIES: [u8; 16] = [0xA1, 0xDC, 0xAB, 0x8C, 0x47, 0xA9, 0xCF, 0x11, 0x8E, 0xE4, 0x00, 0xC0, 0x0C, 0x20, 0x53, 0x65];
const ASF_PADDING: [u8; 16] = [0x74, 0xD4, 0x06, 0x18, 0xDF, 0xCA, 0x09, 0x45, 0xA4, 0xBA, 0x9A, 0xAB, 0xCB, 0x96, 0xAA, 0xE8];

/// Objects as (guid, start, total length) between `from` and `to`.
fn asf_objects(data: &[u8], from: usize, to: usize) -> Vec<([u8; 16], usize, usize)> {
    let mut found = Vec::new();
    let mut at = from;
    while at + 24 <= to {
        let guid: [u8; 16] = data[at..at + 16].try_into().unwrap_or_default();
        let size = u64::from_le_bytes(data[at + 16..at + 24].try_into().unwrap_or_default());
        if size < 24 || at as u64 + size > to as u64 {
            break;
        }
        found.push((guid, at, size as usize));
        at += size as usize;
    }
    found
}

fn pad_object(data: &mut [u8], at: usize, len: usize) {
    data[at..at + 16].copy_from_slice(&ASF_PADDING);
    data[at + 24..at + len].fill(0);
}

fn strip_asf(io: &Io) -> Result<usize> {
    let mut head = [0u8; 24];
    io.read(0, &mut head)?;
    let size = u64::from_le_bytes(head[16..24].try_into().unwrap_or_default());
    let mut header = io.read_vec(0, size)?;
    let mut removed = 0;
    for (guid, at, len) in asf_objects(&header, 30, header.len()) {
        match guid {
            ASF_CONTENT_DESCRIPTION | ASF_EXTENDED_CONTENT => {
                pad_object(&mut header, at, len);
                removed += 1;
            }
            // The creation date sits after the file ID and the file size.
            ASF_FILE_PROPERTIES if len >= 56 => header[at + 48..at + 56].fill(0),
            ASF_HEADER_EXTENSION if len >= 46 => {
                for (inner, inner_at, inner_len) in asf_objects(&header, at + 46, at + len) {
                    if inner == ASF_METADATA || inner == ASF_METADATA_LIBRARY {
                        pad_object(&mut header, inner_at, inner_len);
                        removed += 1;
                    }
                }
            }
            _ => {}
        }
    }
    io.write(0, &header)?;
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Ogg: Vorbis and Opus

struct Page {
    flags: u8,
    granule: u64,
    serial: u32,
    sequence: u32,
    lacing: Vec<u8>,
    body: Vec<u8>,
}

impl Page {
    fn read(reader: &mut impl Read) -> std::io::Result<Option<Self>> {
        let mut head = [0u8; 27];
        match reader.read_exact(&mut head) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }
        if &head[..4] != b"OggS" {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "lost Ogg page sync"));
        }
        let mut lacing = vec![0u8; usize::from(head[26])];
        reader.read_exact(&mut lacing)?;
        let mut body = vec![0u8; lacing.iter().map(|value| usize::from(*value)).sum()];
        reader.read_exact(&mut body)?;
        Ok(Some(Self {
            flags: head[5],
            granule: u64::from_le_bytes(head[6..14].try_into().unwrap_or_default()),
            serial: u32::from_le_bytes(head[14..18].try_into().unwrap_or_default()),
            sequence: u32::from_le_bytes(head[18..22].try_into().unwrap_or_default()),
            lacing,
            body,
        }))
    }

    fn write(&self, writer: &mut impl Write) -> std::io::Result<()> {
        let mut bytes = Vec::with_capacity(27 + self.lacing.len() + self.body.len());
        bytes.extend_from_slice(b"OggS");
        bytes.push(0);
        bytes.push(self.flags);
        bytes.extend_from_slice(&self.granule.to_le_bytes());
        bytes.extend_from_slice(&self.serial.to_le_bytes());
        bytes.extend_from_slice(&self.sequence.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        bytes.push(self.lacing.len() as u8);
        bytes.extend_from_slice(&self.lacing);
        bytes.extend_from_slice(&self.body);
        let crc = ogg_crc(&bytes);
        bytes[22..26].copy_from_slice(&crc.to_le_bytes());
        writer.write_all(&bytes)
    }

    /// Whole packets in this page, plus whether the last one goes on.
    fn packets(&self) -> (Vec<&[u8]>, bool) {
        let mut packets = Vec::new();
        let mut start = 0;
        let mut length = 0;
        for value in &self.lacing {
            length += usize::from(*value);
            if *value < 255 {
                packets.push(&self.body[start..start + length]);
                start += length;
                length = 0;
            }
        }
        let open = self.lacing.last() == Some(&255);
        if open {
            packets.push(&self.body[start..start + length]);
        }
        (packets, open)
    }
}

/// One complete Ogg page holding whole packets. They must fit in 255 segments.
pub(crate) fn ogg_page(flags: u8, granule: u64, serial: u32, sequence: u32, packets: &[&[u8]]) -> Vec<u8> {
    let mut lacing = Vec::new();
    let mut body = Vec::new();
    for packet in packets {
        let mut rest = packet.len();
        while rest >= 255 {
            lacing.push(255);
            rest -= 255;
        }
        lacing.push(rest as u8);
        body.extend_from_slice(packet);
    }
    let mut bytes = Vec::new();
    // Writing to a Vec cannot fail.
    let _ = Page { flags, granule, serial, sequence, lacing, body }.write(&mut bytes);
    bytes
}

fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for byte in data {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { crc << 1 ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

/// Lays header packets out in pages that end where the last packet ends.
fn header_pages(packets: &[Vec<u8>], serial: u32, first_sequence: u32) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut lacing = Vec::new();
    let mut body = Vec::new();
    // Set when the page being filled starts in the middle of a packet.
    let mut continued = false;
    let flush = |lacing: &mut Vec<u8>, body: &mut Vec<u8>, continued: bool, pages: &mut Vec<Page>| {
        pages.push(Page {
            flags: u8::from(continued),
            granule: 0,
            serial,
            sequence: first_sequence + pages.len() as u32,
            lacing: std::mem::take(lacing),
            body: std::mem::take(body),
        });
    };
    for packet in packets {
        let mut rest = packet.as_slice();
        let mut started = false;
        loop {
            if lacing.len() == 255 {
                flush(&mut lacing, &mut body, continued, &mut pages);
                continued = started;
            }
            let take = rest.len().min(255);
            lacing.push(take as u8);
            body.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            started = true;
            if take < 255 {
                break;
            }
            if rest.is_empty() {
                // A packet of an exact multiple of 255 ends with a 0 segment.
                if lacing.len() == 255 {
                    flush(&mut lacing, &mut body, continued, &mut pages);
                    continued = true;
                }
                lacing.push(0);
                break;
            }
        }
    }
    if !lacing.is_empty() {
        flush(&mut lacing, &mut body, continued, &mut pages);
    }
    pages
}

fn strip_ogg(source: &Path, target: &Path, name: &str, cancel: &CancelToken) -> Result<usize> {
    let unsupported = || EngineError::Unsupported(format!("{name}: Ogg layout"));
    let input = File::open(source).map_err(|e| EngineError::io(name, e))?;
    let mut reader = BufReader::with_capacity(1 << 20, input);
    let io_error = |e: std::io::Error| EngineError::io(name, e);

    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut partial: Vec<u8> = Vec::new();
    let mut needed = 0;
    let mut serial = 0;
    let mut used_pages = 0u32;
    while needed == 0 || packets.len() < needed {
        let page = Page::read(&mut reader).map_err(io_error)?.ok_or_else(unsupported)?;
        if used_pages == 0 {
            serial = page.serial;
        } else if page.serial != serial {
            return Err(unsupported());
        }
        used_pages += 1;
        let (pieces, open) = page.packets();
        let count = pieces.len();
        for (index, piece) in pieces.into_iter().enumerate() {
            partial.extend_from_slice(piece);
            let last = index + 1 == count;
            if !(last && open) {
                packets.push(std::mem::take(&mut partial));
            }
        }
        if needed == 0 {
            let first = packets.first().ok_or_else(unsupported)?;
            needed = if first.starts_with(b"\x01vorbis") {
                3
            } else if first.starts_with(b"OpusHead") {
                2
            } else {
                return Err(unsupported());
            };
        }
        if packets.len() > needed || (packets.len() == needed && open) {
            // Audio shares a page with the headers, which the format forbids.
            return Err(unsupported());
        }
    }
    let opus = packets[0].starts_with(b"OpusHead");
    let empty: Vec<u8> = if opus { b"OpusTags\0\0\0\0\0\0\0\0".to_vec() } else { b"\x03vorbis\0\0\0\0\0\0\0\0\x01".to_vec() };
    let removed = usize::from(packets[1].len() > empty.len());
    packets[1] = empty;

    let output = File::create(target).map_err(|e| EngineError::io(target.display().to_string(), e))?;
    let mut writer = BufWriter::with_capacity(1 << 20, output);
    let mut first = header_pages(&packets[..1], serial, 0);
    first[0].flags = 0x02;
    let rest = header_pages(&packets[1..], serial, 1);
    let new_header_pages = 1 + rest.len() as u32;
    for page in first.iter().chain(&rest) {
        page.write(&mut writer).map_err(io_error)?;
    }
    let mut count = 0u32;
    while let Some(mut page) = Page::read(&mut reader).map_err(io_error)? {
        count += 1;
        if count.is_multiple_of(4096) {
            cancel.check()?;
        }
        if page.serial != serial {
            return Err(unsupported());
        }
        page.sequence = page.sequence - used_pages + new_header_pages;
        page.write(&mut writer).map_err(io_error)?;
    }
    writer.flush().map_err(io_error)?;
    writer.into_inner().map_err(|e| io_error(e.into_error()))?.sync_all().map_err(io_error)?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(container: Container, bytes: &[u8]) -> (Vec<u8>, usize) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("in.bin");
        let target = dir.path().join("out.bin");
        std::fs::write(&source, bytes).unwrap();
        let removed = strip(container, &source, &target, &CancelToken::new()).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), bytes, "the source must stay as it was");
        (std::fs::read(target).unwrap(), removed)
    }

    fn atom(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|window| window == needle)
    }

    #[test]
    fn mp4_loses_tags_dates_and_gps_but_keeps_its_layout() {
        let mut mvhd = vec![0u8; 100];
        mvhd[4..8].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        let ilst = atom(b"ilst", &atom(b"\xA9nam", b"Secret title"));
        let udta = atom(b"udta", &[atom(b"\xA9xyz", b"+52.2297+021.0122/"), atom(b"meta", &[vec![0; 4], ilst].concat())].concat());
        let hdlr = atom(b"hdlr", &[vec![0; 8], b"vide".to_vec(), vec![0; 12], b"VideoHandler\0".to_vec()].concat());
        let trak = atom(b"trak", &[atom(b"tkhd", &[0u8; 84]), atom(b"mdia", &[atom(b"mdhd", &[0u8; 24]), hdlr].concat())].concat());
        let moov = atom(b"moov", &[atom(b"mvhd", &mvhd), trak, udta].concat());
        let file = [atom(b"ftyp", b"isom\0\0\x02\0isomiso2"), atom(b"mdat", b"FRAMEDATA"), moov].concat();

        let (clean, removed) = run(Container::IsoBmff, &file);
        assert_eq!(clean.len(), file.len(), "nothing moves");
        assert_eq!(removed, 1);
        assert!(!contains(&clean, b"Secret title") && !contains(&clean, b"+52.2297"));
        assert!(!contains(&clean, &0x1234_5678u32.to_be_bytes()), "the creation time is zeroed");
        assert!(contains(&clean, b"FRAMEDATA") && contains(&clean, b"vide"));
        assert!(!contains(&clean, b"VideoHandler"), "handler names are blanked");
    }

    #[test]
    fn mp4_gps_track_is_removed_with_its_samples() {
        let stbl = atom(
            b"stbl",
            &[
                atom(b"stsz", &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 9]),
                atom(b"stsc", &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]),
                atom(b"stco", &[0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]),
            ]
            .concat(),
        );
        let hdlr = atom(b"hdlr", &[vec![0; 8], b"meta".to_vec(), vec![0; 12]].concat());
        let trak = atom(b"trak", &atom(b"mdia", &[hdlr, atom(b"minf", &stbl)].concat()));
        let ftyp = atom(b"ftyp", b"isom\0\0\x02\0");
        let mdat_at = ftyp.len() + 8;
        let mut file = [ftyp, atom(b"mdat", b"GPSLOG123"), atom(b"moov", &trak)].concat();
        // Point the chunk offset at the GPS bytes inside mdat.
        let offset_at = file.len() - 4;
        file[offset_at..].copy_from_slice(&(mdat_at as u32).to_be_bytes());

        let (clean, removed) = run(Container::IsoBmff, &file);
        assert_eq!(removed, 1);
        assert!(!contains(&clean, b"GPSLOG123"));
        assert!(!contains(&clean, b"trak"));
    }

    #[test]
    fn heif_pictures_are_left_to_ffmpeg() {
        let file = [atom(b"ftyp", b"heic\0\0\0\0mif1"), atom(b"meta", b"items")].concat();
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("p.heic");
        std::fs::write(&source, &file).unwrap();
        let result = strip(Container::IsoBmff, &source, &dir.path().join("o.heic"), &CancelToken::new());
        assert!(matches!(result, Err(EngineError::Unsupported(_))));
    }

    fn ebml(id: u32, payload: &[u8]) -> Vec<u8> {
        let mut out: Vec<u8> = id.to_be_bytes().into_iter().skip_while(|byte| *byte == 0).collect();
        out.push(0x01);
        out.extend_from_slice(&(payload.len() as u64).to_be_bytes()[1..]);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn matroska_loses_title_tags_and_track_names() {
        let info = ebml(INFO, &[ebml(0x2AD7B1, &[0x0F, 0x42, 0x40]), ebml(0x7BA9, b"Secret title"), ebml(0x4D80, b"Lavf61.7.100")].concat());
        let tracks = ebml(TRACKS, &ebml(TRACK_ENTRY, &[ebml(0xD7, &[1]), ebml(TRACK_NAME, b"Director commentary")].concat()));
        let tags = ebml(TAGS, &ebml(0x7373, b"ARTIST=Jan Kowalski"));
        let cluster = ebml(CLUSTER, &ebml(0xE7, &[0]));
        let segment = ebml(SEGMENT, &[info, tracks, cluster, tags].concat());
        let file = [ebml(0x1A45_DFA3, &ebml(0x4282, b"matroska")), segment].concat();

        let (clean, removed) = run(Container::Matroska, &file);
        assert_eq!(clean.len(), file.len());
        assert_eq!(removed, 3);
        for secret in [&b"Secret title"[..], b"Lavf61", b"Director commentary", b"Jan Kowalski"] {
            assert!(!contains(&clean, secret), "{}", String::from_utf8_lossy(secret));
        }
        assert!(contains(&clean, b"matroska"));
        // The cleaned file still parses from start to end.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.mkv");
        std::fs::write(&path, &clean).unwrap();
        let file = File::open(&path).unwrap();
        let io = Io { file: &file, len: clean.len() as u64, name: "c" };
        assert_eq!(strip_matroska(&io, &CancelToken::new()).unwrap(), 0);
    }

    #[test]
    fn void_elements_fill_any_length() {
        for len in [2usize, 3, 128, 129, 130, 5000] {
            let mut buffer = vec![0xAAu8; len];
            write_void(&mut buffer);
            let (id, id_len) = ebml_id(&buffer).unwrap();
            let (size, size_len, unknown) = ebml_size(&buffer[id_len..]).unwrap();
            assert_eq!(id, u32::from(VOID));
            assert!(!unknown);
            assert_eq!(id_len + size_len + size as usize, len, "length {len}");
        }
    }

    fn riff_chunk(id: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    #[test]
    fn wav_info_list_becomes_junk() {
        let info = riff_chunk(b"LIST", &[b"INFO".to_vec(), riff_chunk(b"IART", b"Jan Kowalski")].concat());
        let body = [b"WAVE".to_vec(), riff_chunk(b"fmt ", &[1, 0, 1, 0, 0x44, 0xAC, 0, 0, 0x88, 0x58, 1, 0, 2, 0, 16, 0]), info, riff_chunk(b"data", &[1, 2, 3, 4])].concat();
        let file = riff_chunk(b"RIFF", &body);
        let (clean, removed) = run(Container::Riff, &file);
        assert_eq!(removed, 1);
        assert!(!contains(&clean, b"Jan Kowalski") && contains(&clean, b"JUNK"));
        assert!(contains(&clean, &[1, 2, 3, 4]));
    }

    #[test]
    fn aiff_name_and_author_go() {
        let chunk = |id: &[u8; 4], payload: &[u8]| [id.to_vec(), (payload.len() as u32).to_be_bytes().to_vec(), payload.to_vec()].concat();
        let body = [b"AIFF".to_vec(), chunk(b"COMM", &[0; 18]), chunk(b"NAME", b"Secret"), chunk(b"AUTH", b"Jan!"), chunk(b"SSND", &[0; 12])].concat();
        let file = [b"FORM".to_vec(), (body.len() as u32).to_be_bytes().to_vec(), body].concat();
        let (clean, removed) = run(Container::Aiff, &file);
        assert_eq!(removed, 2);
        assert!(!contains(&clean, b"Secret") && !contains(&clean, b"Jan!"));
    }

    #[test]
    fn asf_content_description_becomes_padding() {
        let mut description = ASF_CONTENT_DESCRIPTION.to_vec();
        let text = b"T\0i\0t\0l\0e\0";
        description.extend_from_slice(&((24 + text.len()) as u64).to_le_bytes());
        description.extend_from_slice(text);
        let mut header = ASF_HEADER.to_vec();
        header.extend_from_slice(&((30 + description.len()) as u64).to_le_bytes());
        header.extend_from_slice(&[1, 0, 0, 0, 1, 2]);
        header.extend_from_slice(&description);
        let (clean, removed) = run(Container::Asf, &header);
        assert_eq!(removed, 1);
        assert!(!contains(&clean, text) && contains(&clean, &ASF_PADDING));
    }

    #[test]
    fn ogg_comments_are_rewritten_and_pages_renumbered() {
        let id = b"OpusHead\x01\x02\x38\x01\x80\xBB\0\0\0\0\0".to_vec();
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&11u32.to_le_bytes());
        tags.extend_from_slice(b"libopus 1.4");
        tags.extend_from_slice(&1u32.to_le_bytes());
        let comment = vec![b'X'; 70_000];
        tags.extend_from_slice(&(comment.len() as u32).to_le_bytes());
        tags.extend_from_slice(&comment);
        let audio = vec![0x55u8; 40];
        let mut file = Vec::new();
        let mut pages = header_pages(&[id], 7, 0);
        pages[0].flags = 2;
        pages.extend(header_pages(&[tags], 7, 1));
        let audio_sequence = pages.len() as u32;
        pages.push(Page { flags: 4, granule: 960, serial: 7, sequence: audio_sequence, lacing: vec![40], body: audio.clone() });
        for page in &pages {
            page.write(&mut file).unwrap();
        }
        assert_eq!(pages.len(), 4, "the long comment spans pages");
        assert_eq!((pages[1].flags, pages[2].flags), (0, 1), "the second comment page continues the packet");

        let (clean, removed) = run(Container::Ogg, &file);
        assert_eq!(removed, 1);
        assert!(!contains(&clean, b"libopus") && !contains(&clean, &comment[..50]));
        let mut reader = clean.as_slice();
        let mut sequence = 0;
        while let Some(page) = Page::read(&mut reader).unwrap() {
            assert_eq!(page.sequence, sequence);
            sequence += 1;
            // Every page must carry a valid checksum.
            let mut bytes = Vec::new();
            page.write(&mut bytes).unwrap();
            assert!(contains(&clean, &bytes));
        }
        assert_eq!(sequence, 3);
    }

    #[test]
    fn detects_every_container() {
        assert_eq!(detect(b"\0\0\0\x20ftypisom\0\0\0\0"), Some(Container::IsoBmff));
        assert_eq!(detect(&[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0]), Some(Container::Matroska));
        assert_eq!(detect(b"RIFF\0\0\0\0WAVEfmt "), Some(Container::Riff));
        assert_eq!(detect(b"RIFF\0\0\0\0AVI LIST"), Some(Container::Riff));
        assert_eq!(detect(b"RIFF\0\0\0\0WEBPVP8 "), None);
        assert_eq!(detect(b"FORM\0\0\0\0AIFFCOMM"), Some(Container::Aiff));
        assert_eq!(detect(b"OggS\0\x02\0\0\0\0\0\0\0\0\0\0"), Some(Container::Ogg));
        assert_eq!(detect(&ASF_HEADER), Some(Container::Asf));
        assert_eq!(detect(b"FLV\x01\x05\0\0\0\x09"), None);
    }
}
