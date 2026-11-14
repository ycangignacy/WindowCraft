// Made by Nordan
//! Direct Win32 registry access.
//!
//! Keys are opened with `RegCreateKeyExW`, which never touches sibling values,
//! so creating `...\WindowsUpdate` leaves corporate WSUS settings in place.

use crate::error::{EngineError, Result};
use crate::win::{from_wide, pcwstr, wide};
use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ, KEY_WOW64_64KEY, KEY_WRITE, REG_BINARY, REG_DWORD, REG_EXPAND_SZ, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE, REG_QWORD, REG_SZ,
    REG_VALUE_TYPE, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW,
    RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windowcraft_config::{RegKind, RegistryEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hive {
    LocalMachine,
    CurrentUser,
    Users,
    ClassesRoot,
}

impl Hive {
    fn handle(self) -> HKEY {
        match self {
            Self::LocalMachine => HKEY_LOCAL_MACHINE,
            Self::CurrentUser => HKEY_CURRENT_USER,
            Self::Users => HKEY_USERS,
            Self::ClassesRoot => HKEY_CLASSES_ROOT,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LocalMachine => "HKLM",
            Self::CurrentUser => "HKCU",
            Self::Users => "HKU",
            Self::ClassesRoot => "HKCR",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegPath {
    pub hive: Hive,
    pub subkey: String,
}

impl RegPath {
    pub fn parse(path: &str) -> Result<Self> {
        let (root, subkey) =
            path.split_once('\\').ok_or_else(|| EngineError::Invalid(format!("registry path '{path}'")))?;
        let hive = match root {
            "HKLM" => Hive::LocalMachine,
            "HKCU" => Hive::CurrentUser,
            "HKU" => Hive::Users,
            "HKCR" => Hive::ClassesRoot,
            _ => return Err(EngineError::Invalid(format!("registry hive '{root}'"))),
        };
        if subkey.is_empty() {
            return Err(EngineError::Invalid(format!("registry path '{path}' has no subkey")));
        }
        Ok(Self { hive, subkey: subkey.to_owned() })
    }

    pub fn display(&self) -> String {
        format!("{}\\{}", self.hive.label(), self.subkey)
    }
}

/// Where registry paths resolve. Tests redirect everything under one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryScope {
    System,
    /// Maps `HKLM\X` to `HKCU\<root>\HKLM\X`.
    Sandbox(String),
}

impl RegistryScope {
    fn resolve(&self, path: &RegPath) -> RegPath {
        match self {
            Self::System => path.clone(),
            Self::Sandbox(root) => RegPath {
                hive: Hive::CurrentUser,
                subkey: format!("{root}\\{}\\{}", path.hive.label(), path.subkey),
            },
        }
    }

    pub fn open(&self, path: &RegPath) -> Result<Option<Key>> {
        Key::open(&self.resolve(path))
    }

    pub fn open_for_write(&self, path: &RegPath) -> Result<Key> {
        Key::create(&self.resolve(path))
    }

    pub fn read(&self, path: &RegPath, name: &str) -> Result<Option<RegValue>> {
        match self.open(path)? {
            Some(key) => key.read(name),
            None => Ok(None),
        }
    }

    pub fn write(&self, path: &RegPath, name: &str, value: &RegValue) -> Result<()> {
        self.open_for_write(path)?.write(name, value)
    }

    /// Writes or removes a value. Removing from a missing key is a no-op.
    pub fn write_opt(&self, path: &RegPath, name: &str, value: Option<&RegValue>) -> Result<()> {
        match value {
            Some(value) => self.write(path, name, value),
            None => match Key::open_writable(&self.resolve(path))? {
                Some(key) => key.delete_value(name),
                None => Ok(()),
            },
        }
    }

    pub fn delete_tree(&self, path: &RegPath) -> Result<()> {
        delete_tree(&self.resolve(path))
    }
}

/// Registry data preserved exactly so snapshots restore the original bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum RegValue {
    String(String),
    ExpandString(String),
    DWord(u32),
    QWord(u64),
    Binary(Vec<u8>),
    MultiString(Vec<String>),
    Other { kind: u32, bytes: Vec<u8> },
}

impl RegValue {
    /// Parses catalog text according to the declared type.
    pub fn parse(kind: RegKind, text: &str) -> Result<Self> {
        let invalid = || EngineError::Invalid(format!("'{text}' as {kind:?}"));
        Ok(match kind {
            RegKind::String => Self::String(text.to_owned()),
            RegKind::DWord => Self::DWord(text.parse().map_err(|_| invalid())?),
            RegKind::QWord => Self::QWord(text.parse().map_err(|_| invalid())?),
            RegKind::Binary => {
                if !text.len().is_multiple_of(2) {
                    return Err(invalid());
                }
                let mut bytes = Vec::with_capacity(text.len() / 2);
                for pair in text.as_bytes().as_chunks::<2>().0 {
                    let pair = std::str::from_utf8(pair.as_slice()).map_err(|_| invalid())?;
                    bytes.push(u8::from_str_radix(pair, 16).map_err(|_| invalid())?);
                }
                Self::Binary(bytes)
            }
        })
    }

    pub fn display(&self) -> String {
        match self {
            Self::String(text) | Self::ExpandString(text) => text.clone(),
            Self::DWord(value) => value.to_string(),
            Self::QWord(value) => value.to_string(),
            Self::Binary(bytes) => bytes.iter().map(|byte| format!("{byte:02X}")).collect(),
            Self::MultiString(items) => items.join(" | "),
            Self::Other { kind, bytes } => format!("type {kind}, {} bytes", bytes.len()),
        }
    }

    fn encode(&self) -> (REG_VALUE_TYPE, Vec<u8>) {
        fn utf16(text: &str) -> impl Iterator<Item = u8> + '_ {
            text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes)
        }
        match self {
            Self::String(text) => (REG_SZ, utf16(text).collect()),
            Self::ExpandString(text) => (REG_EXPAND_SZ, utf16(text).collect()),
            Self::DWord(value) => (REG_DWORD, value.to_le_bytes().to_vec()),
            Self::QWord(value) => (REG_QWORD, value.to_le_bytes().to_vec()),
            Self::Binary(bytes) => (REG_BINARY, bytes.clone()),
            Self::MultiString(items) => {
                let mut bytes: Vec<u8> = items.iter().flat_map(|item| utf16(item)).collect();
                bytes.extend_from_slice(&[0, 0]);
                (REG_MULTI_SZ, bytes)
            }
            Self::Other { kind, bytes } => (REG_VALUE_TYPE(*kind), bytes.clone()),
        }
    }

    fn decode(kind: REG_VALUE_TYPE, bytes: &[u8]) -> Self {
        let units = || -> Vec<u16> {
            bytes.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect()
        };
        match kind {
            REG_SZ => Self::String(from_wide(&units())),
            REG_EXPAND_SZ => Self::ExpandString(from_wide(&units())),
            REG_DWORD if bytes.len() >= 4 => Self::DWord(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            REG_QWORD if bytes.len() >= 8 => {
                let mut raw = [0u8; 8];
                raw.copy_from_slice(&bytes[..8]);
                Self::QWord(u64::from_le_bytes(raw))
            }
            REG_BINARY => Self::Binary(bytes.to_vec()),
            REG_MULTI_SZ => {
                let units = units();
                let items = units
                    .split(|&unit| unit == 0)
                    .filter(|item| !item.is_empty())
                    .map(String::from_utf16_lossy)
                    .collect();
                Self::MultiString(items)
            }
            other => Self::Other { kind: other.0, bytes: bytes.to_vec() },
        }
    }

    /// Loose comparison: Windows stores some numeric settings as strings, so
    /// `"1"` and `1` are the same setting for detection purposes.
    pub fn matches(&self, other: &Self) -> bool {
        self == other || self.display().eq_ignore_ascii_case(&other.display())
    }
}

/// An open key, closed on every exit path.
#[derive(Debug)]
pub struct Key {
    handle: HKEY,
    label: String,
}

// SAFETY: Registry handles are process-wide and may be used from any thread.
unsafe impl Send for Key {}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: The handle is owned by this guard and closed exactly once.
        // Nothing useful can be done with a close failure here.
        let _ = unsafe { RegCloseKey(self.handle) };
    }
}

fn check(status: WIN32_ERROR, operation: &'static str, target: &str) -> Result<()> {
    if status == ERROR_SUCCESS { Ok(()) } else { Err(EngineError::win32(operation, target, status.0)) }
}

impl Key {
    pub fn open(path: &RegPath) -> Result<Option<Self>> {
        Self::open_with(path, KEY_READ | KEY_WOW64_64KEY)
    }

    pub fn open_writable(path: &RegPath) -> Result<Option<Self>> {
        // Only what a value write needs. KEY_ALL_ACCESS also asks for WRITE_DAC
        // and WRITE_OWNER, which keys with narrower ACLs refuse.
        Self::open_with(path, KEY_READ | KEY_WRITE | KEY_WOW64_64KEY)
    }

    fn open_with(path: &RegPath, access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Result<Option<Self>> {
        let subkey = wide(&path.subkey);
        let mut handle = HKEY::default();
        // SAFETY: `subkey` is null-terminated and outlives the call, and
        // `handle` is a valid out pointer.
        let status = unsafe { RegOpenKeyExW(path.hive.handle(), pcwstr(&subkey), None, access, &mut handle) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status, "RegOpenKeyExW", &path.display())?;
        Ok(Some(Self { handle, label: path.display() }))
    }

    /// Opens or creates a key, including missing parents. Never clears
    /// existing values.
    pub fn create(path: &RegPath) -> Result<Self> {
        let subkey = wide(&path.subkey);
        let mut handle = HKEY::default();
        // SAFETY: All pointers reference locals that outlive the call.
        let status = unsafe {
            RegCreateKeyExW(
                path.hive.handle(),
                pcwstr(&subkey),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
                None,
                &mut handle,
                None,
            )
        };
        check(status, "RegCreateKeyExW", &path.display())?;
        Ok(Self { handle, label: path.display() })
    }

    pub fn read(&self, name: &str) -> Result<Option<RegValue>> {
        let wide_name = wide(name);
        let mut size = 0u32;
        let mut kind = REG_VALUE_TYPE::default();
        // Registry data can change between the size query and the read, so
        // the query loops until the buffer is large enough.
        for _ in 0..8 {
            // SAFETY: A null data pointer with a valid size pointer requests
            // the required size only.
            let status = unsafe {
                RegQueryValueExW(self.handle, pcwstr(&wide_name), None, Some(&mut kind), None, Some(&mut size))
            };
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            check(status, "RegQueryValueExW", &self.label)?;

            let mut bytes = vec![0u8; size as usize];
            let mut written = size;
            // SAFETY: `bytes` holds `written` bytes and both out pointers are valid.
            let status = unsafe {
                RegQueryValueExW(
                    self.handle,
                    pcwstr(&wide_name),
                    None,
                    Some(&mut kind),
                    Some(bytes.as_mut_ptr()),
                    Some(&mut written),
                )
            };
            if status == ERROR_MORE_DATA {
                continue;
            }
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            check(status, "RegQueryValueExW", &self.label)?;
            bytes.truncate(written as usize);
            return Ok(Some(RegValue::decode(kind, &bytes)));
        }
        Err(EngineError::Invalid(format!("value {name} kept changing under {}", self.label)))
    }

    pub fn write(&self, name: &str, value: &RegValue) -> Result<()> {
        let wide_name = wide(name);
        let (kind, bytes) = value.encode();
        // SAFETY: The name and data buffers are valid for the call.
        let status = unsafe { RegSetValueExW(self.handle, pcwstr(&wide_name), None, kind, Some(&bytes)) };
        check(status, "RegSetValueExW", &format!("{}\\{name}", self.label))
    }

    pub fn delete_value(&self, name: &str) -> Result<()> {
        let wide_name = wide(name);
        // SAFETY: The name buffer is valid for the call.
        let status = unsafe { RegDeleteValueW(self.handle, pcwstr(&wide_name)) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(status, "RegDeleteValueW", &format!("{}\\{name}", self.label))
    }

    pub fn subkeys(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let mut buffer = [0u16; 256];
        for index in 0u32.. {
            let mut length = buffer.len() as u32;
            // SAFETY: `buffer` holds `length` UTF-16 units.
            let status = unsafe {
                RegEnumKeyExW(
                    self.handle,
                    index,
                    Some(windows::core::PWSTR(buffer.as_mut_ptr())),
                    &mut length,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            check(status, "RegEnumKeyExW", &self.label)?;
            names.push(from_wide(&buffer[..length as usize]));
        }
        Ok(names)
    }

    pub fn value_names(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let mut buffer = vec![0u16; 16_384];
        for index in 0u32.. {
            let mut length = buffer.len() as u32;
            // SAFETY: `buffer` holds `length` UTF-16 units. Type and data are
            // not requested.
            let status = unsafe {
                RegEnumValueW(
                    self.handle,
                    index,
                    Some(windows::core::PWSTR(buffer.as_mut_ptr())),
                    &mut length,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            check(status, "RegEnumValueW", &self.label)?;
            names.push(from_wide(&buffer[..length as usize]));
        }
        Ok(names)
    }
}

/// Deletes a key only if it has no subkeys. Never recursive.
pub fn delete_empty_key(path: &RegPath) -> Result<()> {
    let subkey = wide(&path.subkey);
    // SAFETY: `subkey` is null-terminated and outlives the call.
    let status = unsafe { RegDeleteKeyW(path.hive.handle(), pcwstr(&subkey)) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status, "RegDeleteKeyW", &path.display())
}

fn delete_tree(path: &RegPath) -> Result<()> {
    let subkey = wide(&path.subkey);
    // SAFETY: `subkey` is null-terminated and outlives the call.
    let status = unsafe { RegDeleteTreeW(path.hive.handle(), pcwstr(&subkey)) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status, "RegDeleteTreeW", &path.display())
}

/// One registry change after resolving catalog text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegTarget {
    pub path: RegPath,
    pub name: String,
    pub value: Option<RegValue>,
}

/// Turns catalog text into a typed target. `None` means "delete this value".
pub fn resolve_value(entry: &RegistryEntry, text: Option<&str>, ram_kb: u64) -> Result<RegTarget> {
    use windowcraft_config::{RAM_KB_TOKEN, REMOVE_ENTRY};
    let value = match text {
        None | Some(REMOVE_ENTRY) => None,
        Some(RAM_KB_TOKEN) => Some(RegValue::DWord(u32::try_from(ram_kb).unwrap_or(u32::MAX))),
        Some(text) => Some(RegValue::parse(entry.kind, text)?),
    };
    Ok(RegTarget { path: RegPath::parse(&entry.path)?, name: entry.name.clone(), value })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Sandbox;

    fn sample() -> RegPath {
        RegPath::parse(r"HKLM\SOFTWARE\Policies\Sample").unwrap()
    }

    #[test]
    fn path_parsing() {
        assert_eq!(sample().hive, Hive::LocalMachine);
        assert!(RegPath::parse("HKXX\\A").is_err());
        assert!(RegPath::parse("HKLM").is_err());
        assert!(RegPath::parse("HKLM\\").is_err());
        assert_eq!(RegPath::parse(r"HKU\.Default\Control Panel").unwrap().hive, Hive::Users);
    }

    #[test]
    fn every_value_type_roundtrips() {
        let sandbox = Sandbox::new("regtypes");
        let path = sample();
        let values = [
            RegValue::String("Grüße".into()),
            RegValue::String(String::new()),
            RegValue::ExpandString("%SystemRoot%\\x".into()),
            RegValue::DWord(0xDEAD_BEEF),
            RegValue::QWord(u64::MAX),
            RegValue::Binary(vec![144, 18, 3, 128, 16, 0, 0, 0]),
            RegValue::Binary(Vec::new()),
            RegValue::MultiString(vec!["a".into(), "b c".into()]),
        ];
        for (index, value) in values.iter().enumerate() {
            let name = format!("v{index}");
            sandbox.scope.write(&path, &name, value).unwrap();
            assert_eq!(sandbox.scope.read(&path, &name).unwrap().as_ref(), Some(value));
        }
    }

    #[test]
    fn missing_keys_and_values_read_as_none() {
        let sandbox = Sandbox::new("regmissing");
        let path = RegPath::parse(r"HKLM\No\Such\Key").unwrap();
        assert_eq!(sandbox.scope.read(&path, "x").unwrap(), None);
        sandbox.scope.write_opt(&path, "x", None).unwrap();
        assert!(sandbox.scope.open(&path).unwrap().is_none(), "removal must not create keys");
    }

    #[test]
    fn creating_a_key_preserves_sibling_values() {
        let sandbox = Sandbox::new("regsiblings");
        let path = RegPath::parse(r"HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate").unwrap();
        let wsus = RegValue::String("https://wsus.corp.example".into());
        sandbox.scope.write(&path, "WUServer", &wsus).unwrap();

        sandbox.scope.write(&path, "ExcludeWUDriversInQualityUpdate", &RegValue::DWord(1)).unwrap();
        drop(sandbox.scope.open_for_write(&path).unwrap());

        assert_eq!(sandbox.scope.read(&path, "WUServer").unwrap(), Some(wsus));
    }

    #[test]
    fn default_value_uses_the_empty_name() {
        let sandbox = Sandbox::new("regdefault");
        let path = sample();
        sandbox.scope.write(&path, "", &RegValue::String(String::new())).unwrap();
        assert_eq!(sandbox.scope.read(&path, "").unwrap(), Some(RegValue::String(String::new())));
        sandbox.scope.write_opt(&path, "", None).unwrap();
        assert_eq!(sandbox.scope.read(&path, "").unwrap(), None);
    }

    #[test]
    fn enumeration_and_tree_delete() {
        let sandbox = Sandbox::new("regenum");
        let root = sample();
        let child = RegPath { hive: root.hive, subkey: format!("{}\\Child", root.subkey) };
        sandbox.scope.write(&child, "a", &RegValue::DWord(1)).unwrap();
        sandbox.scope.write(&child, "b", &RegValue::DWord(2)).unwrap();

        let key = sandbox.scope.open(&root).unwrap().unwrap();
        assert_eq!(key.subkeys().unwrap(), ["Child"]);
        let child_key = sandbox.scope.open(&child).unwrap().unwrap();
        assert_eq!(child_key.value_names().unwrap().len(), 2);
        drop((key, child_key));

        sandbox.scope.delete_tree(&root).unwrap();
        assert!(sandbox.scope.open(&root).unwrap().is_none());
        sandbox.scope.delete_tree(&root).unwrap();
    }

    #[test]
    fn value_parsing() {
        assert_eq!(RegValue::parse(RegKind::DWord, "32").unwrap(), RegValue::DWord(32));
        assert_eq!(RegValue::parse(RegKind::Binary, "9012").unwrap(), RegValue::Binary(vec![0x90, 0x12]));
        assert!(RegValue::parse(RegKind::Binary, "901").is_err());
        assert!(RegValue::parse(RegKind::Binary, "ZZ").is_err());
        assert!(RegValue::parse(RegKind::DWord, "-1").is_err());
        assert!(RegValue::DWord(1).matches(&RegValue::String("1".into())));
        assert!(!RegValue::DWord(1).matches(&RegValue::DWord(2)));
    }

    #[test]
    fn large_ram_is_clamped_to_dword() {
        let entry = RegistryEntry {
            path: r"HKLM\A\B".into(),
            name: "x".into(),
            kind: RegKind::DWord,
            value: None,
            original: None,
            default_on: None,
            values: None,
        };
        let target = resolve_value(&entry, Some("{ram_kb}"), u64::MAX).unwrap();
        assert_eq!(target.value, Some(RegValue::DWord(u32::MAX)));
        assert_eq!(resolve_value(&entry, Some("<RemoveEntry>"), 0).unwrap().value, None);
    }
}
