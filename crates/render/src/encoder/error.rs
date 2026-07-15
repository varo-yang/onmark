//! Stable encoder failures with bounded `FFmpeg` diagnostics and retained causes.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use tokio::task::JoinError;

/// Stable category for an `FFmpeg` encoding failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EncodeErrorKind {
    /// The destination already exists.
    OutputExists,
    /// `FFmpeg` could not be started.
    Spawn,
    /// More frames were supplied than configured.
    FrameLimit,
    /// Encoded frame input exceeded its byte budget.
    InputLimit,
    /// No frames were supplied before finishing.
    NoFrames,
    /// A frame could not be written to `FFmpeg`.
    InputWrite,
    /// An encoding operation exceeded its inactivity timeout.
    Timeout,
    /// `FFmpeg` exited unsuccessfully.
    Failed,
    /// `FFmpeg` could not be terminated or reaped reliably.
    ProcessControl,
    /// The bounded stderr drain failed.
    StderrRead,
}

/// Typed `FFmpeg` failure carrying destination and source context.
#[derive(Debug)]
pub struct EncodeError {
    kind: EncodeErrorKind,
    output: PathBuf,
    message: Box<str>,
    source: Option<EncodeErrorSource>,
}

impl EncodeError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(&self) -> EncodeErrorKind {
        self.kind
    }

    /// Returns the output artifact associated with the failure.
    #[must_use]
    pub fn output(&self) -> &Path {
        &self.output
    }

    pub(super) fn new(kind: EncodeErrorKind, output: &Path, message: impl Into<Box<str>>) -> Self {
        Self {
            kind,
            output: output.to_owned(),
            message: message.into(),
            source: None,
        }
    }

    pub(super) fn io(
        kind: EncodeErrorKind,
        output: &Path,
        message: impl Into<Box<str>>,
        source: io::Error,
    ) -> Self {
        Self {
            kind,
            output: output.to_owned(),
            message: message.into(),
            source: Some(EncodeErrorSource::Io(source)),
        }
    }

    pub(super) fn join(output: &Path, source: JoinError) -> Self {
        Self {
            kind: EncodeErrorKind::StderrRead,
            output: output.to_owned(),
            message: "FFmpeg stderr reader terminated unexpectedly".into(),
            source: Some(EncodeErrorSource::Join(source)),
        }
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.output.display(), self.message)
    }
}

impl Error for EncodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source as _)
    }
}

#[derive(Debug)]
enum EncodeErrorSource {
    Io(io::Error),
    Join(JoinError),
}

impl fmt::Display for EncodeErrorSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(source) => source.fmt(formatter),
            Self::Join(source) => source.fmt(formatter),
        }
    }
}

impl Error for EncodeErrorSource {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(source) => source.source(),
            Self::Join(source) => source.source(),
        }
    }
}
