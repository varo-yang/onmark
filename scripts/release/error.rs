//! Typed failures shared by the deterministic desktop artifact assemblers.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) enum PackageError {
    InvalidOptions(Box<str>),
    InvalidInput {
        path: PathBuf,
        reason: &'static str,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Json(serde_json::Error),
    OutputExists(PathBuf),
    PackageLimit {
        actual: u64,
        limit: u64,
    },
}

impl PackageError {
    pub(super) fn invalid_input(path: &Path, reason: &'static str) -> Self {
        Self::InvalidInput {
            path: path.to_path_buf(),
            reason,
        }
    }

    pub(super) fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }
}

impl fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOptions(reason) => write!(formatter, "invalid release options: {reason}"),
            Self::InvalidInput { path, reason } => {
                write!(
                    formatter,
                    "invalid release input {}: {reason}",
                    path.display()
                )
            }
            Self::Io {
                operation, path, ..
            } => write!(formatter, "cannot {operation} {}", path.display()),
            Self::Json(_) => formatter.write_str("cannot process desktop release metadata"),
            Self::OutputExists(path) => {
                write!(
                    formatter,
                    "release output already exists: {}",
                    path.display()
                )
            }
            Self::PackageLimit { actual, limit } => write!(
                formatter,
                "desktop release artifact contains {actual} bytes; limit is {limit} bytes"
            ),
        }
    }
}

impl Error for PackageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(source) => Some(source),
            Self::InvalidOptions(_)
            | Self::InvalidInput { .. }
            | Self::OutputExists(_)
            | Self::PackageLimit { .. } => None,
        }
    }
}

impl From<serde_json::Error> for PackageError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json(source)
    }
}
