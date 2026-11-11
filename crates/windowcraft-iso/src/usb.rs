// Made by ycangignacy
//! USB media creation with a layered disk-destruction guard.
//!
//! `assess` is a pure function over facts read from the OS, so each rejection
//! rule has a unit test. It runs at selection time and again immediately
//! before the first destructive call.

use crate::error::{IsoError, Result};
use crate::fat32;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::*;
use windows::core::GUID;
use windowcraft_engine::EngineError;
use windowcraft_engine::win::{HandleGuard, pcwstr, wide};

const GIB: u64 = 1024 * 1024 * 1024;
const MIN_DISK_BYTES: u64 = 8 * GIB;
const MAX_DISK_BYTES: u64 = 2048 * GIB;
const MAX_PARTITIONS: u32 = 8;
const PARTITION_OFFSET: u64 = 1024 * 1024;
/// Covers the partition table and the boot sector of the old first partition.
const SIGNATURE_WIPE_BYTES: usize = 4 * 1024 * 1024;
const BASIC_DATA: GUID = GUID::from_u128(0xebd0a0a2_b9e5_4433_87c0_68b6b72699c7);
/// `install.wim` chunks stay below the FAT32 file size limit.
pub const SWM_PART_BYTES: u64 = 3_800 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Usb,
    Sata,
    Nvme,
    Scsi,
    Sas,
    Other(i32),
}

impl Bus {
    pub fn label(self) -> String {
        match self {
            Self::Usb => "USB".to_owned(),
            Self::Sata => "SATA".to_owned(),
            Self::Nvme => "NVMe".to_owned(),
            Self::Scsi => "SCSI".to_owned(),
            Self::Sas => "SAS".to_owned(),
            Self::Other(code) => format!("bus {code}"),
        }
    }
}

/// What tells one disk from another of the same model. The disk number
/// alone does not: a stick pulled out and another put in can get the same
/// `PhysicalDriveN`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DiskIdentity {
    pub model: String,
    /// Serial number the device reports, empty when it reports none.
    pub serial: String,
    /// MBR signature or GPT disk GUID, empty on a disk with no partition table.
    pub layout_id: String,
    pub bytes: u64,
}

impl DiskIdentity {
    /// A serial number or a partition table id is there to compare.
    pub fn is_known(&self) -> bool {
        !self.serial.is_empty() || !self.layout_id.is_empty()
    }
}

/// The disk about to be erased is the one the user confirmed.
pub fn same_disk(confirmed: &DiskIdentity, now: &DiskIdentity, number: u32) -> Result<()> {
    if !confirmed.is_known() {
        return Err(IsoError::DiskGuard(format!(
            "disk {number} reports no serial number and has no partition table, so it cannot be told apart from another stick of the same model; create a partition on it in Disk Management and choose it again"
        )));
    }
    if confirmed != now {
        return Err(IsoError::DiskGuard(format!(
            "disk {number} is not the disk that was chosen ('{}', {} bytes, now '{}', {} bytes); choose it again",
            confirmed.model, confirmed.bytes, now.model, now.bytes
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFacts {
    pub number: u32,
    pub model: String,
    pub bus: Bus,
    pub identity: DiskIdentity,
    pub bytes: u64,
    pub partitions: u32,
    pub letters: Vec<char>,
    /// The disk hosts the Windows, boot or page-file volume.
    pub is_system: bool,
}

/// Decides whether a disk may be erased. Every layer must pass.
pub fn assess(disk: &DiskFacts, system_drive: char) -> Result<()> {
    let refuse = |reason: String| Err(IsoError::DiskGuard(format!("disk {}: {reason}", disk.number)));

    // Layer 1: the bus. Internal NVMe and SATA disks stop here.
    if disk.bus != Bus::Usb {
        return refuse(format!("it is on {}, not USB", disk.bus.label()));
    }
    // Layer 2: this running system must not live on the disk.
    if disk.is_system {
        return refuse("it hosts the running system".to_owned());
    }
    if disk.letters.iter().any(|letter| letter.eq_ignore_ascii_case(&system_drive)) {
        return refuse(format!("it holds the system drive {system_drive}:"));
    }
    // Layer 3: shape. A USB bridge can hold a large backup disk.
    if disk.bytes < MIN_DISK_BYTES {
        return refuse(format!("{} GB is too small for Windows media", disk.bytes / GIB));
    }
    if disk.bytes > MAX_DISK_BYTES {
        return refuse(format!("{} GB is larger than any install stick", disk.bytes / GIB));
    }
    if disk.partitions > MAX_PARTITIONS {
        return refuse(format!("{} partitions look like a data disk", disk.partitions));
    }
    Ok(())
}

/// Partition length: everything usable, capped so UEFI firmware can read it.
pub fn partition_bytes(disk_bytes: u64) -> u64 {
    let usable = disk_bytes.saturating_sub(2 * PARTITION_OFFSET);
    usable.min(fat32::MAX_VOLUME_BYTES) / PARTITION_OFFSET * PARTITION_OFFSET
}

fn system_drive_letter() -> char {
    std::env::var("SystemDrive").ok().and_then(|text| text.chars().next()).unwrap_or('C')
}

fn open_device(path: &str, access: u32) -> Result<HandleGuard> {
    let name = wide(path);
    // SAFETY: The name is null-terminated and outlives the call.
    let handle = unsafe {
        CreateFileW(
            pcwstr(&name),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .map_err(|error| IsoError::Engine(EngineError::windows("CreateFileW", path, &error)))?;
    Ok(HandleGuard(handle))
}

/// Issues an IOCTL with a typed output. Returns bytes written.
///
/// # Safety
/// `input` and `output` must match the layout the control code expects.
unsafe fn ioctl<I, O>(device: HANDLE, code: u32, input: Option<&I>, output: Option<&mut O>, label: &'static str) -> Result<u32> {
    let mut returned = 0;
    let (in_pointer, in_size) = input.map_or((core::ptr::null(), 0), |value| {
        (core::ptr::from_ref(value).cast::<core::ffi::c_void>(), size_of::<I>() as u32)
    });
    let (out_pointer, out_size) = output.map_or((core::ptr::null_mut(), 0), |value| {
        (core::ptr::from_mut(value).cast::<core::ffi::c_void>(), size_of::<O>() as u32)
    });
    // SAFETY: Pointers come from live references, and the caller vouches for
    // the layouts.
    unsafe {
        DeviceIoControl(
            device,
            code,
            Some(in_pointer),
            in_size,
            Some(out_pointer),
            out_size,
            Some(&mut returned),
            None,
        )
    }
    .map_err(|error| IsoError::Engine(EngineError::windows("DeviceIoControl", label, &error)))?;
    Ok(returned)
}

#[repr(C, align(8))]
struct Buffer<const N: usize>([u8; N]);

/// Bus, model and serial number.
fn query_device(handle: HANDLE) -> Result<(Bus, String, String)> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    let mut buffer = Buffer::<1024>([0; 1024]);
    // SAFETY: The input is the documented query, and the output is a buffer
    // large enough for the variable-length descriptor.
    unsafe { ioctl(handle, IOCTL_STORAGE_QUERY_PROPERTY, Some(&query), Some(&mut buffer), "storage property")? };
    let descriptor = buffer.0.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>();
    // SAFETY: The buffer is aligned and the API filled the fixed header.
    let (bus, product_offset, serial_offset) =
        unsafe { ((*descriptor).BusType, (*descriptor).ProductIdOffset, (*descriptor).SerialNumberOffset) };
    let bus = [
        (BusTypeUsb, Bus::Usb),
        (BusTypeSata, Bus::Sata),
        (BusTypeNvme, Bus::Nvme),
        (BusTypeScsi, Bus::Scsi),
        (BusTypeSas, Bus::Sas),
    ]
    .into_iter()
    .find_map(|(kind, bus_kind)| (kind == bus).then_some(bus_kind))
    .unwrap_or(Bus::Other(bus.0));
    let text_at = |offset: u32| {
        let offset = offset as usize;
        if offset == 0 || offset >= buffer.0.len() {
            return String::new();
        }
        let tail = &buffer.0[offset..];
        let end = tail.iter().position(|&byte| byte == 0).unwrap_or(tail.len());
        String::from_utf8_lossy(&tail[..end]).trim().to_owned()
    };
    Ok((bus, text_at(product_offset), text_at(serial_offset)))
}

fn disk_of_volume(letter: char) -> Option<(u32, u32)> {
    let handle = open_device(&format!(r"\\.\{letter}:"), 0).ok()?;
    let mut extents = Buffer::<512>([0; 512]);
    // SAFETY: The output buffer holds the variable-length extent list.
    unsafe {
        ioctl::<(), _>(handle.raw(), IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, None, Some(&mut extents), "volume extents")
            .ok()?;
    }
    let header = extents.0.as_ptr().cast::<VOLUME_DISK_EXTENTS>();
    // SAFETY: The API filled the header and at least one extent.
    let (count, number) =
        unsafe { ((*header).NumberOfDiskExtents, (*header).Extents[0].DiskNumber) };
    Some((number, count))
}

fn letters_of_disk(number: u32) -> Vec<char> {
    ('A'..='Z')
        .filter(|letter| {
            let root = wide(format!(r"{letter}:\"));
            // SAFETY: The path is null-terminated.
            let kind = unsafe { GetDriveTypeW(pcwstr(&root)) };
            kind > 1 && disk_of_volume(*letter).is_some_and(|(disk, _)| disk == number)
        })
        .collect()
}

fn query_disk(number: u32) -> Option<DiskFacts> {
    let handle = open_device(&format!(r"\\.\PhysicalDrive{number}"), GENERIC_READ.0)
        .or_else(|_| open_device(&format!(r"\\.\PhysicalDrive{number}"), 0))
        .ok()?;
    let (bus, model, serial) = query_device(handle.raw()).ok()?;

    let mut length = GET_LENGTH_INFORMATION::default();
    // SAFETY: The output structure matches this control code.
    let bytes =
        unsafe { ioctl::<(), _>(handle.raw(), IOCTL_DISK_GET_LENGTH_INFO, None, Some(&mut length), "disk length") }
            .map_or(0, |_| length.Length.cast_unsigned());

    let mut layout = Buffer::<8192>([0; 8192]);
    // SAFETY: The buffer holds the variable-length partition list.
    let (partitions, layout_id) =
        unsafe { ioctl::<(), _>(handle.raw(), IOCTL_DISK_GET_DRIVE_LAYOUT_EX, None, Some(&mut layout), "disk layout") }
            .map_or((0, String::new()), |_| {
                let info = layout.0.as_ptr().cast::<DRIVE_LAYOUT_INFORMATION_EX>();
                // SAFETY: The API filled the fixed header, and the union member read
                // matches the partition style it reports.
                unsafe {
                    let style = (*info).PartitionStyle;
                    let id = if style == PARTITION_STYLE_MBR.0 as u32 {
                        format!("mbr:{:08x}", (*info).Anonymous.Mbr.Signature)
                    } else if style == PARTITION_STYLE_GPT.0 as u32 {
                        format!("gpt:{:?}", (*info).Anonymous.Gpt.DiskId)
                    } else {
                        String::new()
                    };
                    ((*info).PartitionCount, id)
                }
            });
    // A blank MBR has signature 0, which every other blank stick shares.
    let layout_id = if layout_id == "mbr:00000000" { String::new() } else { layout_id };

    let letters = letters_of_disk(number);
    let system = system_drive_letter();
    let is_system = letters.iter().any(|letter| letter.eq_ignore_ascii_case(&system));
    let identity = DiskIdentity { model: model.clone(), serial, layout_id, bytes };
    Some(DiskFacts { number, model, bus, identity, bytes, partitions, letters, is_system })
}

/// Lists physical disks. Probing stops at a generous upper bound, because
/// numbering can have gaps.
pub fn disks() -> Vec<DiskFacts> {
    (0..32).filter_map(query_disk).collect()
}

/// Disks that pass every guard.
pub fn eligible_disks() -> Vec<DiskFacts> {
    let system = system_drive_letter();
    disks().into_iter().filter(|disk| assess(disk, system).is_ok()).collect()
}

/// Locks and dismounts a volume until the guard drops.
struct LockedVolume {
    _handle: HandleGuard,
}

fn lock_volume(letter: char) -> Result<LockedVolume> {
    let handle = open_device(&format!(r"\\.\{letter}:"), GENERIC_READ.0 | GENERIC_WRITE.0)?;
    // SAFETY: These FSCTLs take no input or output buffers.
    unsafe {
        ioctl::<(), ()>(handle.raw(), FSCTL_LOCK_VOLUME, None, None, "lock volume")?;
        ioctl::<(), ()>(handle.raw(), FSCTL_DISMOUNT_VOLUME, None, None, "dismount volume")?;
    }
    Ok(LockedVolume { _handle: handle })
}

fn write_gpt(device: HANDLE, disk_bytes: u64) -> Result<u64> {
    let identifier = GUID::new().map_err(|error| IsoError::Engine(EngineError::windows("CoCreateGuid", "disk id", &error)))?;
    let create = CREATE_DISK {
        PartitionStyle: PARTITION_STYLE_GPT,
        Anonymous: CREATE_DISK_0 { Gpt: CREATE_DISK_GPT { DiskId: identifier, MaxPartitionCount: 128 } },
    };
    // SAFETY: Input layouts match these control codes.
    unsafe {
        ioctl::<(), ()>(device, IOCTL_DISK_DELETE_DRIVE_LAYOUT, None, None, "delete layout")?;
        ioctl::<_, ()>(device, IOCTL_DISK_CREATE_DISK, Some(&create), None, "create GPT")?;
        ioctl::<(), ()>(device, IOCTL_DISK_UPDATE_PROPERTIES, None, None, "update properties")?;
    }

    let mut layout = Buffer::<8192>([0; 8192]);
    // SAFETY: The buffer holds the variable-length layout.
    unsafe { ioctl::<(), _>(device, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, None, Some(&mut layout), "read GPT")? };

    let bytes = partition_bytes(disk_bytes);
    let partition_guid =
        GUID::new().map_err(|error| IsoError::Engine(EngineError::windows("CoCreateGuid", "partition id", &error)))?;
    let info = layout.0.as_mut_ptr().cast::<DRIVE_LAYOUT_INFORMATION_EX>();
    // SAFETY: The buffer is aligned, holds the layout read above, and has room
    // for the single entry written here.
    unsafe {
        (*info).PartitionCount = 1;
        let entry = (*info).PartitionEntry.as_mut_ptr();
        *entry = PARTITION_INFORMATION_EX {
            PartitionStyle: PARTITION_STYLE_GPT,
            StartingOffset: PARTITION_OFFSET.cast_signed(),
            PartitionLength: bytes.cast_signed(),
            PartitionNumber: 1,
            RewritePartition: true,
            IsServicePartition: false,
            Anonymous: PARTITION_INFORMATION_EX_0 {
                Gpt: PARTITION_INFORMATION_GPT {
                    PartitionType: BASIC_DATA,
                    PartitionId: partition_guid,
                    Attributes: GPT_ATTRIBUTES(0),
                    Name: {
                        let mut name = [0u16; 36];
                        for (slot, unit) in name.iter_mut().zip("WindowCraft".encode_utf16()) {
                            *slot = unit;
                        }
                        name
                    },
                },
            },
        };
        ioctl::<_, ()>(device, IOCTL_DISK_SET_DRIVE_LAYOUT_EX, Some(&layout), None, "write GPT")?;
        ioctl::<(), ()>(device, IOCTL_DISK_UPDATE_PROPERTIES, None, None, "update properties")?;
    }
    Ok(bytes)
}

fn sector_size(device: HANDLE) -> Result<u32> {
    let mut geometry = DISK_GEOMETRY_EX::default();
    // SAFETY: The output structure matches this control code.
    unsafe { ioctl::<(), _>(device, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, None, Some(&mut geometry), "geometry")? };
    Ok(geometry.Geometry.BytesPerSector)
}

fn find_letter(number: u32) -> Option<char> {
    for _ in 0..50 {
        if let Some(letter) = letters_of_disk(number).first() {
            return Some(*letter);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    None
}

/// Zeroes the start of the disk. A new partition at the same offset would
/// otherwise remount the previous file system, and Windows blocks raw writes
/// into a mounted volume.
fn clear_old_signatures(number: u32) -> Result<()> {
    let mut raw = OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
        .open(format!(r"\\.\PhysicalDrive{number}"))
        .map_err(|error| IsoError::io("physical drive", error))?;
    let zeroes = vec![0u8; SIGNATURE_WIPE_BYTES];
    raw.write_all(&zeroes).map_err(|error| IsoError::io("physical drive", error))
}

/// Erases a USB disk and leaves a single GPT/FAT32 partition. Returns the
/// mounted root. Facts are re-read here, so a device swapped after selection
/// still has to pass the guard, and its serial number and partition table id
/// must match the confirmed ones.
pub fn prepare_disk(number: u32, confirmed: &DiskIdentity, label: &str) -> Result<PathBuf> {
    let disk = query_disk(number).ok_or_else(|| IsoError::DiskGuard(format!("disk {number} is gone")))?;
    assess(&disk, system_drive_letter())?;
    same_disk(confirmed, &disk.identity, number)?;

    let locks: Vec<LockedVolume> = disk.letters.iter().map(|letter| lock_volume(*letter)).collect::<Result<_>>()?;
    clear_old_signatures(number)?;
    let device = open_device(&format!(r"\\.\PhysicalDrive{number}"), GENERIC_READ.0 | GENERIC_WRITE.0)?;
    let bytes = write_gpt(device.raw(), disk.bytes)?;
    let sector = sector_size(device.raw())?;
    drop(device);

    let mut raw = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
        .open(format!(r"\\.\PhysicalDrive{number}"))
        .map_err(|error| IsoError::io("physical drive", error))?;
    let serial = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x5743_0001, |elapsed| elapsed.subsec_nanos());
    fat32::format(&mut raw, PARTITION_OFFSET, bytes, sector, label, serial)?;
    drop(raw);
    drop(locks);

    let device = open_device(&format!(r"\\.\PhysicalDrive{number}"), GENERIC_READ.0 | GENERIC_WRITE.0)?;
    // SAFETY: This control code takes no buffers.
    unsafe { ioctl::<(), ()>(device.raw(), IOCTL_DISK_UPDATE_PROPERTIES, None, None, "update properties")? };
    drop(device);

    let letter = find_letter(number).ok_or_else(|| IsoError::Invalid("the new volume did not mount".to_owned()))?;
    Ok(PathBuf::from(format!(r"{letter}:\")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usb_stick() -> DiskFacts {
        DiskFacts {
            number: 3,
            model: "SanDisk Ultra".to_owned(),
            bus: Bus::Usb,
            identity: identity(),
            bytes: 64 * GIB,
            partitions: 1,
            letters: vec!['F'],
            is_system: false,
        }
    }

    fn identity() -> DiskIdentity {
        DiskIdentity {
            model: "SanDisk Ultra".to_owned(),
            serial: "4C530001230512116054".to_owned(),
            layout_id: "mbr:1a2b3c4d".to_owned(),
            bytes: 64 * GIB,
        }
    }

    #[test]
    fn another_stick_of_the_same_model_is_refused() {
        let chosen = identity();
        assert!(same_disk(&chosen, &chosen.clone(), 3).is_ok());
        let other = DiskIdentity { serial: "4C530001230512116099".to_owned(), ..identity() };
        assert!(matches!(same_disk(&chosen, &other, 3), Err(IsoError::DiskGuard(_))));
        let repartitioned = DiskIdentity { layout_id: "gpt:{0}".to_owned(), ..identity() };
        assert!(same_disk(&chosen, &repartitioned, 3).is_err());
        // Two blank sticks without serial numbers look the same: refused.
        let blank = DiskIdentity { serial: String::new(), layout_id: String::new(), ..identity() };
        assert!(!blank.is_known());
        assert!(same_disk(&blank, &blank.clone(), 3).is_err());
        // A serial number alone, or a partition table id alone, is enough.
        let serial_only = DiskIdentity { layout_id: String::new(), ..identity() };
        assert!(same_disk(&serial_only, &serial_only.clone(), 3).is_ok());
        let layout_only = DiskIdentity { serial: String::new(), ..identity() };
        assert!(same_disk(&layout_only, &layout_only.clone(), 3).is_ok());
    }

    #[test]
    fn a_normal_usb_stick_passes() {
        assert!(assess(&usb_stick(), 'C').is_ok());
    }

    #[test]
    fn internal_buses_are_refused_at_layer_one() {
        for bus in [Bus::Nvme, Bus::Sata, Bus::Scsi, Bus::Sas, Bus::Other(7)] {
            let disk = DiskFacts { bus, ..usb_stick() };
            assert!(matches!(assess(&disk, 'C'), Err(IsoError::DiskGuard(_))), "{bus:?}");
        }
    }

    #[test]
    fn windows_to_go_style_system_disks_are_refused() {
        let disk = DiskFacts { is_system: true, ..usb_stick() };
        assert!(assess(&disk, 'C').is_err());
    }

    #[test]
    fn a_disk_holding_the_system_letter_is_refused_case_insensitively() {
        let disk = DiskFacts { letters: vec!['F', 'c'], ..usb_stick() };
        assert!(assess(&disk, 'C').is_err());
        let disk = DiskFacts { letters: vec!['D'], ..usb_stick() };
        assert!(assess(&disk, 'd').is_err());
    }

    #[test]
    fn size_and_partition_sanity_are_enforced() {
        assert!(assess(&DiskFacts { bytes: 4 * GIB, ..usb_stick() }, 'C').is_err());
        assert!(assess(&DiskFacts { bytes: 4096 * GIB, ..usb_stick() }, 'C').is_err(), "USB backup disk");
        assert!(assess(&DiskFacts { partitions: 12, ..usb_stick() }, 'C').is_err());
        assert!(assess(&DiskFacts { bytes: 0, ..usb_stick() }, 'C').is_err(), "unreadable size must not pass");
    }

    #[test]
    fn partition_is_capped_and_aligned() {
        assert_eq!(partition_bytes(128 * GIB), 32 * GIB);
        let small = partition_bytes(16 * GIB);
        assert!(small < 16 * GIB);
        assert_eq!(small % PARTITION_OFFSET, 0);
    }

    #[test]
    fn this_machine_exposes_no_eligible_system_disk() {
        // Read-only: enumerates disks and confirms the guard rejects whatever
        // hosts the running system.
        let system = system_drive_letter();
        for disk in disks() {
            if disk.letters.contains(&system) {
                assert!(assess(&disk, system).is_err());
            }
        }
    }
}
