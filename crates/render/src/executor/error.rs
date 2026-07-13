use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::{BrowserError, EncodeError};

/// Stable category for a local render failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RenderErrorKind {
    /// The browser plan contains no usable output interval.
    InvalidPlan,
    /// The plan exceeds a configured process or protocol limit.
    PlanTooLarge,
    /// A private staging artifact could not be created or published.
    Output,
    /// Chromium or the browser runtime boundary failed.
    Browser,
    /// `FFmpeg` encoding failed.
    Encoder,
    /// The runtime returned a well-formed but unexpected response.
    Protocol,
}

/// Typed failure from the single-process render pipeline.
#[derive(Debug)]
pub struct RenderError {
    kind: RenderErrorKind,
    output: PathBuf,
    message: Box<str>,
    source: Option<Box<RenderErrorSource>>,
}

impl RenderError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(&self) -> RenderErrorKind {
        self.kind
    }

    /// Returns the intended output artifact.
    #[must_use]
    pub fn output(&self) -> &Path {
        &self.output
    }

    pub(super) fn new(kind: RenderErrorKind, output: &Path, message: impl Into<Box<str>>) -> Self {
        Self {
            kind,
            output: output.to_owned(),
            message: message.into(),
            source: None,
        }
    }

    pub(super) fn browser(output: &Path, source: BrowserError) -> Self {
        Self {
            kind: RenderErrorKind::Browser,
            output: output.to_owned(),
            message: "browser execution failed".into(),
            source: Some(Box::new(RenderErrorSource::Browser(source))),
        }
    }

    pub(super) fn encoder(output: &Path, source: EncodeError) -> Self {
        Self {
            kind: RenderErrorKind::Encoder,
            output: output.to_owned(),
            message: "video encoding failed".into(),
            source: Some(Box::new(RenderErrorSource::Encoder(source))),
        }
    }

    pub(super) fn protocol(output: &Path, message: impl Into<Box<str>>) -> Self {
        Self::new(RenderErrorKind::Protocol, output, message)
    }

    pub(super) fn output_io(
        output: &Path,
        message: impl Into<Box<str>>,
        source: io::Error,
    ) -> Self {
        Self {
            kind: RenderErrorKind::Output,
            output: output.to_owned(),
            message: message.into(),
            source: Some(Box::new(RenderErrorSource::Io(source))),
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.output.display(), self.message)
    }
}

impl Error for RenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref().map(|source| source as _)
    }
}

#[derive(Debug)]
enum RenderErrorSource {
    Browser(BrowserError),
    Encoder(EncodeError),
    Io(io::Error),
}

impl fmt::Display for RenderErrorSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Browser(source) => source.fmt(formatter),
            Self::Encoder(source) => source.fmt(formatter),
            Self::Io(source) => source.fmt(formatter),
        }
    }
}

impl Error for RenderErrorSource {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Browser(source) => source.source(),
            Self::Encoder(source) => source.source(),
            Self::Io(source) => source.source(),
        }
    }
}
