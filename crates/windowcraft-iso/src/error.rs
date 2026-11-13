// Made by WindowCraft Dev Team
use thiserror::Error;
use windowcraft_engine::EngineError;

pub type Result<T> = std::result::Result<T, IsoError>;

#[derive(Debug, Error)]
pub enum IsoError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("{operation} failed for {target} (error {code})")]
    Wim { operation: &'static str, target: String, code: u32 },
    #[error("{0}")]
    Invalid(String),
    #[error("refusing to touch this disk: {0}")]
    DiskGuard(String),
    #[error("{target}: {source}")]
    Io {
        target: String,
        #[source]
        source: std::io::Error,
    },
}

impl IsoError {
    pub fn io(target: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io { target: target.into(), source }
    }
}
