use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Stable category for one unit-root materialization failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum UnitRootErrorKind {
    /// The declared bundle identity does not match its canonical payload list.
    BundleIdentity,
    /// The complete unit exceeds its configured file count.
    FileLimit,
    /// The complete unit exceeds its configured retained bytes.
    ByteLimit,
    /// A source path is a symlink or not a regular file.
    InvalidSource,
    /// Copied bytes differ in size from the trusted manifest.
    SizeMismatch,
    /// Copied bytes differ from their frozen SHA-256 identity.
    DigestMismatch,
    /// Two materialized assets claim the same frozen identity.
    DuplicateAsset,
    /// The browser entry cannot become a local file URL.
    InvalidEntry,
    /// A filesystem operation failed.
    Io,
}

/// Typed failure carrying the source path involved in materialization.
#[derive(Debug)]
pub struct UnitRootError {
    kind: UnitRootErrorKind,
    path: PathBuf,
    message: Box<str>,
    source: Option<io::Error>,
}

impl UnitRootError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(&self) -> UnitRootErrorKind {
        self.kind
    }

    /// Returns the bundle or asset path associated with the failure.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn without_source(
        kind: UnitRootErrorKind,
        path: &Path,
        message: impl Into<Box<str>>,
    ) -> Self {
        Self {
            kind,
            path: path.to_owned(),
            message: message.into(),
            source: None,
        }
    }

    pub(super) fn io(path: &Path, message: impl Into<Box<str>>, source: io::Error) -> Self {
        Self {
            kind: UnitRootErrorKind::Io,
            path: path.to_owned(),
            message: message.into(),
            source: Some(source),
        }
    }
}

impl fmt::Display for UnitRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.message)
    }
}

impl Error for UnitRootError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source as _)
    }
}
