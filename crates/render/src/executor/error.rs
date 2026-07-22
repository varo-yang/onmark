//! Translation of browser, encoder, artifact, and output failures at execution.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use onmark_core::protocol::ProtocolFailure;

use crate::{BrowserError, EncodeError, FrameArtifactError};

// A valid wire failure can name 256 resources. Keep terminal diagnostics
// actionable without allowing one browser response to dominate them.
const DISPLAYED_PENDING_RESOURCE_LIMIT: usize = 8;

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
    /// `FFmpeg` visual encoding or audio mixing failed.
    Encoder,
    /// A durable worker frame artifact could not be published or verified.
    Artifact,
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
            message: "FFmpeg execution failed".into(),
            source: Some(Box::new(RenderErrorSource::Encoder(source))),
        }
    }

    pub(super) fn artifact(output: &Path, source: FrameArtifactError) -> Self {
        Self {
            kind: RenderErrorKind::Artifact,
            output: output.to_owned(),
            message: "worker frame artifact failed".into(),
            source: Some(Box::new(RenderErrorSource::Artifact(source))),
        }
    }

    pub(super) fn protocol(output: &Path, message: impl Into<Box<str>>) -> Self {
        Self::new(RenderErrorKind::Protocol, output, message)
    }

    pub(super) fn runtime_failure(output: &Path, failure: &ProtocolFailure) -> Self {
        Self::protocol(output, runtime_failure_message(failure))
    }

    pub(super) fn with_disposal_failure(mut self, disposal: Self) -> Self {
        let primary = self.source.take();
        self.message =
            format!("{}; browser runtime disposal also failed", self.message).into_boxed_str();
        self.source = Some(Box::new(RenderErrorSource::Disposal {
            primary,
            disposal: Box::new(disposal),
        }));
        self
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

fn runtime_failure_message(failure: &ProtocolFailure) -> Box<str> {
    let mut message = format!("browser runtime failed: {}", failure.message());
    let mut resources = failure.pending_resources();
    let resource_count = resources.len();
    if resource_count == 0 {
        return message.into();
    }

    message.push_str("; pending resources: ");
    for (index, resource) in resources
        .by_ref()
        .take(DISPLAYED_PENDING_RESOURCE_LIMIT)
        .enumerate()
    {
        if index > 0 {
            message.push_str(", ");
        }
        message.push_str(resource);
    }
    if resource_count > DISPLAYED_PENDING_RESOURCE_LIMIT {
        message.push_str("; and ");
        message.push_str(&(resource_count - DISPLAYED_PENDING_RESOURCE_LIMIT).to_string());
        message.push_str(" more");
    }

    message.into()
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
    Artifact(FrameArtifactError),
    Io(io::Error),
    Disposal {
        primary: Option<Box<RenderErrorSource>>,
        disposal: Box<RenderError>,
    },
}

impl fmt::Display for RenderErrorSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Browser(source) => source.fmt(formatter),
            Self::Encoder(source) => source.fmt(formatter),
            Self::Artifact(source) => source.fmt(formatter),
            Self::Io(source) => source.fmt(formatter),
            Self::Disposal { primary, disposal } => match primary {
                Some(primary) => write!(formatter, "{primary}; disposal failure: {disposal}"),
                None => write!(formatter, "disposal failure: {disposal}"),
            },
        }
    }
}

impl Error for RenderErrorSource {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Browser(source) => source.source(),
            Self::Encoder(source) => source.source(),
            Self::Artifact(source) => source.source(),
            Self::Io(source) => source.source(),
            Self::Disposal { primary, disposal } => match primary {
                Some(primary) => Some(primary.as_ref()),
                None => Some(disposal.as_ref()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use onmark_core::protocol::{ProtocolFailure, ProtocolFailureCode};

    use super::{DISPLAYED_PENDING_RESOURCE_LIMIT, RenderError};

    #[test]
    fn runtime_failure_reports_pending_resources() {
        let failure = ProtocolFailure::new(
            ProtocolFailureCode::ReadinessTimeout,
            "a required resource did not stabilize",
            vec!["video:hero".into(), "font:inter".into()],
        )
        .expect("the fixture failure must satisfy the protocol contract");

        let error = RenderError::runtime_failure(Path::new("output.mp4"), &failure);

        assert_eq!(
            error.to_string(),
            "output.mp4: browser runtime failed: a required resource did not stabilize; \
             pending resources: video:hero, font:inter",
        );
    }

    #[test]
    fn runtime_failure_bounds_pending_resource_rendering() {
        let resources = (0..=DISPLAYED_PENDING_RESOURCE_LIMIT)
            .map(|index| format!("resource:{index}").into())
            .collect();
        let failure = ProtocolFailure::new(
            ProtocolFailureCode::ReadinessTimeout,
            "a required resource did not stabilize",
            resources,
        )
        .expect("the fixture failure must satisfy the protocol contract");

        let error = RenderError::runtime_failure(Path::new("output.mp4"), &failure);

        assert!(error.to_string().ends_with("resource:7; and 1 more"));
    }
}
