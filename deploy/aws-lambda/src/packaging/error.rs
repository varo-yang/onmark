//! Typed failure surface for Lambda release packaging.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) enum PackageError {
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InvalidOptions(Box<str>),
    InvalidInput {
        path: PathBuf,
        reason: &'static str,
    },
    OutputExists(PathBuf),
    EntryLimit {
        actual: usize,
        limit: usize,
    },
    ExpandedLimit {
        actual: u64,
        limit: u64,
    },
    CompressedLimit {
        actual: u64,
        limit: u64,
    },
    PackageLimit {
        actual: u64,
        limit: u64,
    },
    Json(serde_json::Error),
    Zip(zip::result::ZipError),
}

impl PackageError {
    pub(super) fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_owned(),
            source,
        }
    }

    pub(super) fn invalid_input(path: impl Into<PathBuf>, reason: &'static str) -> Self {
        Self::InvalidInput {
            path: path.into(),
            reason,
        }
    }
}

impl fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation, path, ..
            } => write!(formatter, "failed to {operation} {}", path.display()),
            Self::InvalidOptions(message) => write!(
                formatter,
                "{message}; expected --bootstrap <path> --browser-root <path> --output <directory>",
            ),
            Self::InvalidInput { path, reason } => {
                write!(formatter, "invalid {reason}: {}", path.display())
            }
            Self::OutputExists(path) => write!(
                formatter,
                "package output {} already exists; refusing to overwrite it",
                path.display(),
            ),
            Self::EntryLimit { actual, limit } => write!(
                formatter,
                "browser payload contains {actual} files, exceeding its {limit}-entry limit",
            ),
            Self::ExpandedLimit { actual, limit } => write!(
                formatter,
                "browser payload contains {actual} bytes, exceeding its {limit}-byte expanded limit",
            ),
            Self::CompressedLimit { actual, limit } => write!(
                formatter,
                "browser archive contains {actual} bytes, exceeding its {limit}-byte compressed limit",
            ),
            Self::PackageLimit { actual, limit } => write!(
                formatter,
                "Lambda package expands to {actual} bytes, exceeding its {limit}-byte release limit",
            ),
            Self::Json(_) => formatter.write_str("failed to serialize package manifest"),
            Self::Zip(_) => formatter.write_str("failed to write deterministic Lambda ZIP"),
        }
    }
}

impl Error for PackageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(source) => Some(source),
            Self::Zip(source) => Some(source),
            Self::InvalidOptions(_)
            | Self::InvalidInput { .. }
            | Self::OutputExists(_)
            | Self::EntryLimit { .. }
            | Self::ExpandedLimit { .. }
            | Self::CompressedLimit { .. }
            | Self::PackageLimit { .. } => None,
        }
    }
}
