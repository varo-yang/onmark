mod error;
mod materializer;

use std::error::Error;
use std::fmt;
use std::path::Path;

use onmark_core::protocol::BundleManifest;
use tempfile::TempDir;
use url::Url;

pub use error::{UnitRootError, UnitRootErrorKind};

use crate::MaterializedAsset;

const MAX_FILES: usize = 100_000;
const MAX_BYTES: u64 = 1 << 40;

/// Explicit retained-storage limits for one private execution root.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UnitRootLimits {
    max_files: usize,
    max_bytes: u64,
}

impl UnitRootLimits {
    /// Creates one bounded unit-root policy.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidUnitRootLimits`] when a bound is zero or exceeds the
    /// fixed local-render safety envelope.
    pub const fn new(max_files: usize, max_bytes: u64) -> Result<Self, InvalidUnitRootLimits> {
        if max_files == 0 {
            return Err(InvalidUnitRootLimits::ZeroFiles);
        }
        if max_files > MAX_FILES {
            return Err(InvalidUnitRootLimits::TooManyFiles);
        }
        if max_bytes == 0 {
            return Err(InvalidUnitRootLimits::ZeroBytes);
        }
        if max_bytes > MAX_BYTES {
            return Err(InvalidUnitRootLimits::TooManyBytes);
        }
        Ok(Self {
            max_files,
            max_bytes,
        })
    }

    const fn max_files(self) -> usize {
        self.max_files
    }

    const fn max_bytes(self) -> u64 {
        self.max_bytes
    }
}

/// Reason unit-root resource limits cannot bound local materialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidUnitRootLimits {
    /// No retained file may be created.
    ZeroFiles,
    /// The requested file count exceeds the fixed safety ceiling.
    TooManyFiles,
    /// No retained payload bytes may be written.
    ZeroBytes,
    /// The requested byte budget exceeds one tebibyte.
    TooManyBytes,
}

impl fmt::Display for InvalidUnitRootLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroFiles => "unit root file limit must be positive",
            Self::TooManyFiles => "unit root file limit exceeds the safety ceiling",
            Self::ZeroBytes => "unit root byte limit must be positive",
            Self::TooManyBytes => "unit root byte limit exceeds the safety ceiling",
        })
    }
}

impl Error for InvalidUnitRootLimits {}

/// Private verified filesystem root retained for one local render lifetime.
#[derive(Debug)]
pub struct UnitRoot {
    directory: TempDir,
    entry_url: Url,
}

impl UnitRoot {
    /// Materializes one presentation bundle and its frozen assets.
    ///
    /// Payloads are copied rather than linked so later source-path mutation
    /// cannot change bytes already admitted into this private execution root.
    ///
    /// # Errors
    ///
    /// Returns [`UnitRootError`] when identities, source files, resource
    /// limits, or filesystem operations violate the execution contract.
    pub fn materialize<'a>(
        bundle_directory: &Path,
        manifest: &BundleManifest,
        assets: impl IntoIterator<Item = &'a MaterializedAsset>,
        limits: UnitRootLimits,
    ) -> Result<Self, UnitRootError> {
        let directory =
            materializer::materialize(bundle_directory, manifest, assets.into_iter(), limits)?;
        let entry = directory.path().join(manifest.entry_point());
        let entry_url = Url::from_file_path(&entry).map_err(|()| {
            UnitRootError::without_source(
                UnitRootErrorKind::InvalidEntry,
                &entry,
                "unit-root entry cannot be represented as a file URL",
            )
        })?;

        Ok(Self {
            directory,
            entry_url,
        })
    }

    /// Returns the owned private filesystem root.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.directory.path()
    }

    /// Returns the browser URL of the verified presentation entry.
    #[must_use]
    pub const fn entry_url(&self) -> &Url {
        &self.entry_url
    }
}
