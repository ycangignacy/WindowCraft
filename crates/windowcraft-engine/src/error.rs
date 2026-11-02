// Made by ycangignacy
use thiserror::Error;

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{operation} failed for {target}: {message} (0x{code:08X})")]
    Win32 { operation: &'static str, target: String, code: u32, message: String },
    #[error("administrator rights are required for {0}")]
    NeedsElevation(String),
    #[error("service '{0}' is protected and cannot be disabled")]
    ProtectedService(String),
    #[error("invalid value: {0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("unsupported on this system: {0}")]
    Unsupported(String),
    #[error("{program} exited with code {code}: {detail}")]
    Process { program: String, code: i32, detail: String },
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("snapshot store: {0}")]
    Store(String),
    #[error("WindowCraft is already using the snapshot database. Close the other instance and try again.")]
    StoreBusy,
    #[error("network: {0}")]
    Network(String),
    #[error("checksum mismatch for {file}: expected {expected}, got {actual}")]
    Checksum { file: String, expected: String, actual: String },
    #[error("{target}: {source}")]
    Io {
        target: String,
        #[source]
        source: std::io::Error,
    },
}

impl EngineError {
    pub fn win32(operation: &'static str, target: impl Into<String>, code: u32) -> Self {
        let target = target.into();
        if code == 5 {
            return Self::NeedsElevation(format!("{operation} on {target}"));
        }
        Self::Win32 { operation, target, code, message: windows::core::HRESULT::from_win32(code).message() }
    }

    pub fn windows(operation: &'static str, target: impl Into<String>, error: &windows::core::Error) -> Self {
        let target = target.into();
        let code = error.code().0.cast_unsigned();
        if code == 0x8007_0005 {
            return Self::NeedsElevation(format!("{operation} on {target}"));
        }
        Self::Win32 { operation, target, code, message: error.message() }
    }

    pub fn io(target: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io { target: target.into(), source }
    }

    /// Absent objects are often expected, for example an uninstalled service.
    pub fn is_not_found(&self) -> bool {
        match self {
            Self::NotFound(_) => true,
            // 0x800F080C is CBS_E_UNKNOWN_UPDATE: this edition has no such feature.
            Self::Win32 { code, .. } => *code == 0x800F_080C || matches!(*code & 0xFFFF, 2 | 3 | 1060 | 1168),
            Self::Io { source, .. } => source.kind() == std::io::ErrorKind::NotFound,
            _ => false,
        }
    }
}

impl From<redb::Error> for EngineError {
    fn from(error: redb::Error) -> Self {
        Self::Store(error.to_string())
    }
}

macro_rules! store_error {
    ($($source:ty),+) => {
        $(impl From<$source> for EngineError {
            fn from(error: $source) -> Self {
                Self::Store(error.to_string())
            }
        })+
    };
}

store_error!(
    redb::DatabaseError,
    redb::TransactionError,
    redb::TableError,
    redb::StorageError,
    redb::CommitError,
    serde_json::Error
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn access_denied_win32_and_hresult_codes_map_to_needs_elevation() {
        let from_code = EngineError::win32("RegSetValueExW", r"HKLM\Test", 5);
        assert!(matches!(&from_code, EngineError::NeedsElevation(detail) if detail == r"RegSetValueExW on HKLM\Test"));
        assert_eq!(from_code.to_string(), r"administrator rights are required for RegSetValueExW on HKLM\Test");

        let hresult = windows::core::HRESULT(0x8007_0005_u32.cast_signed());
        let win_err = windows::core::Error::from_hresult(hresult);
        let from_win = EngineError::windows("OpenServiceW", "DiagTrack", &win_err);
        assert!(matches!(&from_win, EngineError::NeedsElevation(detail) if detail == "OpenServiceW on DiagTrack"));
    }

    #[test]
    fn non_elevation_win32_and_windows_errors_preserve_operation_target_and_hex_code() {
        let err = EngineError::win32("RegOpenKeyExW", r"HKLM\Missing", 2);
        let text = err.to_string();
        assert!(text.starts_with(r"RegOpenKeyExW failed for HKLM\Missing: "), "{text}");
        assert!(text.ends_with("(0x00000002)"), "{text}");

        let hresult = windows::core::HRESULT(0x800F_080C_u32.cast_signed());
        let win_err = windows::core::Error::from_hresult(hresult);
        let err = EngineError::windows("DismEnableFeature", "Recall", &win_err);
        let text = err.to_string();
        assert!(text.starts_with("DismEnableFeature failed for Recall: "), "{text}");
        assert!(text.ends_with("(0x800F080C)"), "{text}");
    }

    #[test]
    fn is_not_found_recognizes_absent_win32_cbs_and_io_errors_only() {
        assert!(EngineError::NotFound("item".into()).is_not_found());
        for code in [2, 3, 1060, 1168, 0x8007_0002, 0x8007_0003, 0x8007_0424, 0x8007_0490, 0x800F_080C] {
            let err = EngineError::Win32 { operation: "op", target: "t".into(), code, message: String::new() };
            assert!(err.is_not_found(), "0x{code:08X} must count as not found");
        }
        let other_win32 =
            EngineError::Win32 { operation: "op", target: "t".into(), code: 0x8000_4005, message: String::new() };
        assert!(!other_win32.is_not_found());

        let io_missing = EngineError::io("file.txt", std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(io_missing.is_not_found());
        assert!(io_missing.source().is_some());

        let io_denied = EngineError::io("file.txt", std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        assert!(!io_denied.is_not_found());
        assert!(!EngineError::Cancelled.is_not_found());
    }

    #[test]
    fn display_and_store_conversions_format_expected_messages() {
        assert_eq!(
            EngineError::ProtectedService("RpcSs".into()).to_string(),
            "service 'RpcSs' is protected and cannot be disabled"
        );
        assert_eq!(
            EngineError::Process { program: "dism.exe".into(), code: 87, detail: "bad arg".into() }.to_string(),
            "dism.exe exited with code 87: bad arg"
        );
        assert_eq!(
            EngineError::Checksum { file: "win.iso".into(), expected: "aaa".into(), actual: "bbb".into() }.to_string(),
            "checksum mismatch for win.iso: expected aaa, got bbb"
        );
        assert_eq!(
            EngineError::StoreBusy.to_string(),
            "WindowCraft is already using the snapshot database. Close the other instance and try again."
        );

        let json_err = serde_json::from_str::<serde_json::Value>("{bad").unwrap_err();
        let converted: EngineError = json_err.into();
        assert!(matches!(&converted, EngineError::Store(msg) if !msg.is_empty()));
        assert!(converted.to_string().starts_with("snapshot store: "));

        let redb_err: EngineError = redb::StorageError::Corrupted("header".into()).into();
        assert!(matches!(redb_err, EngineError::Store(_)));
        let general_redb: EngineError = redb::Error::Corrupted("page".into()).into();
        assert!(matches!(general_redb, EngineError::Store(_)));
    }
}
