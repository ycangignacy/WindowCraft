// Made by ycangignacy
//! Pure-Rust FAT32 formatter.
//!
//! Windows refuses to format FAT32 above 32 GB and the undocumented `fmifs`
//! route is unstable, so the volume is written directly. Writing to any
//! `Write + Seek` target also lets tests verify the exact on-disk layout.

use crate::error::{IsoError, Result};
use std::io::{Seek, SeekFrom, Write};

pub const MAX_VOLUME_BYTES: u64 = 32 * 1024 * 1024 * 1024;
/// FAT32 is defined by cluster count, not by size.
const MIN_CLUSTERS: u64 = 65_525;
const RESERVED_SECTORS: u32 = 32;
const FAT_COUNT: u32 = 2;
const ROOT_CLUSTER: u32 = 2;
const CHUNK_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub total_sectors: u32,
    pub fat_sectors: u32,
    pub data_clusters: u32,
}

impl Layout {
    pub fn data_start_sector(&self) -> u32 {
        RESERVED_SECTORS + FAT_COUNT * self.fat_sectors
    }
}

fn cluster_sectors(bytes: u64, bytes_per_sector: u32) -> u32 {
    let cluster_bytes: u64 = match bytes {
        b if b <= 260 * 1024 * 1024 => 512,
        b if b <= 8 * 1024 * 1024 * 1024 => 4 * 1024,
        b if b <= 16 * 1024 * 1024 * 1024 => 8 * 1024,
        _ => 16 * 1024,
    };
    (cluster_bytes / u64::from(bytes_per_sector)).max(1) as u32
}

/// Computes the layout, sizing the FAT for every data cluster.
pub fn plan(bytes: u64, bytes_per_sector: u32) -> Result<Layout> {
    if !matches!(bytes_per_sector, 512 | 4096) {
        return Err(IsoError::Invalid(format!("unsupported sector size {bytes_per_sector}")));
    }
    if bytes > MAX_VOLUME_BYTES {
        return Err(IsoError::Invalid("FAT32 volumes are capped at 32 GB".to_owned()));
    }
    let sector = u64::from(bytes_per_sector);
    let sectors_per_cluster = cluster_sectors(bytes, bytes_per_sector);
    let total_sectors = bytes / sector;
    let entries_per_sector = sector / 4;
    let spc = u64::from(sectors_per_cluster);
    // Solves `F * E >= C + 2` with `C = (T - R - 2F) / spc`. The two extra FAT
    // entries are reserved and never map to a data cluster.
    let divisor = entries_per_sector * spc + u64::from(FAT_COUNT);
    let fat_sectors = (total_sectors.saturating_sub(u64::from(RESERVED_SECTORS)) + 2 * spc).div_ceil(divisor);
    let metadata = u64::from(RESERVED_SECTORS) + u64::from(FAT_COUNT) * fat_sectors;
    let data_sectors = total_sectors.saturating_sub(metadata);
    let data_clusters = data_sectors / u64::from(sectors_per_cluster);
    if data_clusters < MIN_CLUSTERS {
        return Err(IsoError::Invalid(format!("{bytes} bytes is too small for FAT32")));
    }
    let layout = Layout {
        bytes_per_sector,
        sectors_per_cluster,
        total_sectors: total_sectors as u32,
        fat_sectors: fat_sectors as u32,
        data_clusters: data_clusters as u32,
    };
    Ok(layout)
}

fn put16(buffer: &mut [u8], offset: usize, value: u16) {
    buffer[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put32(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn volume_label(label: &str) -> [u8; 11] {
    let mut field = [b' '; 11];
    let cleaned = label.bytes().filter(u8::is_ascii_graphic).map(|byte| byte.to_ascii_uppercase());
    for (slot, byte) in field.iter_mut().zip(cleaned) {
        *slot = byte;
    }
    field
}

fn boot_sector(layout: &Layout, label: &str, serial: u32) -> Vec<u8> {
    let mut sector = vec![0u8; layout.bytes_per_sector as usize];
    sector[..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    sector[3..11].copy_from_slice(b"MSWIN4.1");
    put16(&mut sector, 11, layout.bytes_per_sector as u16);
    sector[13] = layout.sectors_per_cluster as u8;
    put16(&mut sector, 14, RESERVED_SECTORS as u16);
    sector[16] = FAT_COUNT as u8;
    sector[21] = 0xF8;
    put16(&mut sector, 24, 63);
    put16(&mut sector, 26, 255);
    put32(&mut sector, 32, layout.total_sectors);
    put32(&mut sector, 36, layout.fat_sectors);
    put32(&mut sector, 44, ROOT_CLUSTER);
    put16(&mut sector, 48, 1);
    put16(&mut sector, 50, 6);
    sector[64] = 0x80;
    sector[66] = 0x29;
    put32(&mut sector, 67, serial);
    sector[71..82].copy_from_slice(&volume_label(label));
    sector[82..90].copy_from_slice(b"FAT32   ");
    sector[510] = 0x55;
    sector[511] = 0xAA;
    sector
}

fn fs_info(layout: &Layout) -> Vec<u8> {
    let mut sector = vec![0u8; layout.bytes_per_sector as usize];
    put32(&mut sector, 0, 0x4161_5252);
    put32(&mut sector, 484, 0x6141_7272);
    put32(&mut sector, 488, layout.data_clusters - 1);
    put32(&mut sector, 492, ROOT_CLUSTER + 1);
    sector[510] = 0x55;
    sector[511] = 0xAA;
    sector
}

fn seek<W: Seek>(device: &mut W, base: u64, sector: u64, size: u32) -> Result<()> {
    device
        .seek(SeekFrom::Start(base + sector * u64::from(size)))
        .map(drop)
        .map_err(|e| IsoError::io("volume", e))
}

fn write_zeroes<W: Write>(device: &mut W, bytes: u64) -> Result<()> {
    let chunk = vec![0u8; CHUNK_BYTES];
    let mut remaining = bytes;
    while remaining > 0 {
        let step = remaining.min(CHUNK_BYTES as u64) as usize;
        device.write_all(&chunk[..step]).map_err(|e| IsoError::io("volume", e))?;
        remaining -= step as u64;
    }
    Ok(())
}

/// Writes an empty FAT32 file system at `base`, a sector-aligned offset.
pub fn format<W: Write + Seek>(device: &mut W, base: u64, bytes: u64, sector_size: u32, label: &str, serial: u32) -> Result<Layout> {
    let layout = plan(bytes, sector_size)?;
    let sector = u64::from(sector_size);

    // Clear reserved sectors, both FATs and the root cluster so stale data on
    // a reused stick can never appear as directory entries.
    seek(device, base, 0, sector_size)?;
    let metadata = u64::from(layout.data_start_sector()) + u64::from(layout.sectors_per_cluster);
    write_zeroes(device, metadata * sector)?;

    let boot = boot_sector(&layout, label, serial);
    let info = fs_info(&layout);
    for (position, data) in [(0, &boot), (1, &info), (6, &boot), (7, &info)] {
        seek(device, base, position, sector_size)?;
        device.write_all(data).map_err(|e| IsoError::io("volume", e))?;
    }

    let mut first_fat_sector = vec![0u8; sector_size as usize];
    put32(&mut first_fat_sector, 0, 0x0FFF_FFF8);
    put32(&mut first_fat_sector, 4, 0x0FFF_FFFF);
    put32(&mut first_fat_sector, 8, 0x0FFF_FFFF);
    for copy in 0..FAT_COUNT {
        seek(device, base, u64::from(RESERVED_SECTORS + copy * layout.fat_sectors), sector_size)?;
        device.write_all(&first_fat_sector).map_err(|e| IsoError::io("volume", e))?;
    }

    let mut root = vec![0u8; sector_size as usize];
    root[..11].copy_from_slice(&volume_label(label));
    root[11] = 0x08;
    seek(device, base, u64::from(layout.data_start_sector()), sector_size)?;
    device.write_all(&root).map_err(|e| IsoError::io("volume", e))?;

    device.flush().map_err(|e| IsoError::io("volume", e))?;
    Ok(layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn read32(buffer: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(buffer[offset..offset + 4].try_into().unwrap())
    }

    fn read16(buffer: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(buffer[offset..offset + 2].try_into().unwrap())
    }

    #[test]
    fn formats_a_volume_with_a_valid_layout() {
        let size = 64 * 1024 * 1024;
        let mut device = Cursor::new(vec![0xCCu8; size as usize]);
        let layout = format(&mut device, 0, size, 512, "WC_USB", 0xABCD_1234).unwrap();
        let bytes = device.into_inner();

        assert_eq!(&bytes[3..11], b"MSWIN4.1");
        assert_eq!(read16(&bytes, 11), 512);
        assert_eq!(bytes[13], 1);
        assert_eq!(read16(&bytes, 14), 32);
        assert_eq!(bytes[16], 2);
        assert_eq!(read32(&bytes, 32), layout.total_sectors);
        assert_eq!(read32(&bytes, 36), layout.fat_sectors);
        assert_eq!(read32(&bytes, 44), 2);
        assert_eq!(read32(&bytes, 67), 0xABCD_1234);
        assert_eq!(&bytes[71..82], b"WC_USB     ");
        assert_eq!(&bytes[82..90], b"FAT32   ");
        assert_eq!(&bytes[510..512], &[0x55, 0xAA]);

        // Backups
        assert_eq!(&bytes[512 * 6..512 * 7], &bytes[..512]);
        assert_eq!(&bytes[512 * 7..512 * 8], &bytes[512..1024]);

        // FSInfo
        assert_eq!(read32(&bytes, 512), 0x4161_5252);
        assert_eq!(read32(&bytes, 512 + 488), layout.data_clusters - 1);

        // Both FATs mark the root cluster as end of chain
        for copy in 0..2 {
            let fat = (32 + copy * layout.fat_sectors) as usize * 512;
            assert_eq!(read32(&bytes, fat), 0x0FFF_FFF8);
            assert_eq!(read32(&bytes, fat + 8), 0x0FFF_FFFF);
            assert_eq!(read32(&bytes, fat + 12), 0, "the next cluster must be free");
        }

        // Root directory: label entry, then clean space
        let root = layout.data_start_sector() as usize * 512;
        assert_eq!(&bytes[root..root + 11], b"WC_USB     ");
        assert_eq!(bytes[root + 11], 0x08);
        assert_eq!(bytes[root + 32], 0, "stale data must not read as an entry");
    }

    #[test]
    fn fat_is_large_enough_for_every_cluster() {
        for size in [GIB, 4 * GIB, 8 * GIB, 16 * GIB, 32 * GIB] {
            for sector in [512, 4096] {
                let layout = plan(size, sector).unwrap();
                let entries = u64::from(layout.fat_sectors) * u64::from(sector) / 4;
                assert!(entries >= u64::from(layout.data_clusters) + 2, "{size} bytes");
                assert!(u64::from(layout.data_clusters) >= MIN_CLUSTERS);
            }
        }
    }

    #[test]
    fn formatting_at_an_offset_leaves_earlier_bytes_alone() {
        let base = 1024 * 1024u64;
        let size = 64 * 1024 * 1024;
        let mut device = Cursor::new(vec![0x5Au8; (base + size) as usize]);
        format(&mut device, base, size, 512, "X", 1).unwrap();
        let bytes = device.into_inner();
        assert!(bytes[..base as usize].iter().all(|&byte| byte == 0x5A));
        assert_eq!(&bytes[base as usize + 82..base as usize + 90], b"FAT32   ");
    }

    #[test]
    fn enforces_size_limits() {
        assert!(plan(32 * GIB + 1, 512).is_err(), "above the UEFI compatibility cap");
        assert!(plan(1024 * 1024, 512).is_err(), "too few clusters to be FAT32");
        assert!(plan(GIB, 1024).is_err(), "unsupported sector size");
    }

    #[test]
    fn labels_are_sanitized() {
        assert_eq!(&volume_label("win 11!"), b"WIN11!     ");
        assert_eq!(&volume_label("averyveryverylong"), b"AVERYVERYVE");
    }
}
