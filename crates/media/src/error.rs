use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use onmark_core::model::InvalidDuration;

/// Reason an ffprobe boundary cannot be configured safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidFfprobe {
    /// No executable path was supplied.
    EmptyExecutable,
    /// A zero timeout cannot bound process lifetime.
    ZeroTimeout,
    /// The requested timeout exceeds the fixed process-lifetime ceiling.
    TimeoutTooLong,
    /// A zero-byte limit cannot carry valid probe output.
    ZeroOutputLimit,
    /// The requested capture exceeds the crate's fixed one-MiB ceiling.
    OutputLimitTooLarge,
}

impl fmt::Display for InvalidFfprobe {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyExecutable => "ffprobe executable cannot be empty",
            Self::ZeroTimeout => "ffprobe timeout cannot be zero",
            Self::TimeoutTooLong => "ffprobe timeout cannot exceed ten minutes",
            Self::ZeroOutputLimit => "ffprobe output limit cannot be zero",
            Self::OutputLimitTooLarge => "ffprobe output limit cannot exceed one MiB",
        };
        formatter.write_str(message)
    }
}

impl Error for InvalidFfprobe {}

/// Typed failure from the ffprobe process and response boundary.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProbeError {
    /// The ffprobe process could not be started.
    Spawn(ProbeFailure),
    /// A pipe-reader thread could not be started or completed.
    OutputRead(ProbeFailure),
    /// The process could not be observed or terminated reliably.
    ProcessControl(ProbeFailure),
    /// The process exceeded its configured lifetime.
    Timeout(ProbeFailure),
    /// Captured output exceeded its configured byte limit.
    OutputLimit(ProbeFailure),
    /// ffprobe exited with an unsuccessful status.
    Failed(ProbeFailure),
    /// stdout was not a valid ffprobe JSON response.
    InvalidResponse(ProbeFailure),
    /// ffprobe did not report a format duration.
    MissingDuration(ProbeFailure),
    /// The reported duration was not an exact supported decimal.
    InvalidDuration(ProbeFailure),
    /// The selected video stream lacks normalized format or timing facts.
    InvalidVideo(ProbeFailure),
}

impl ProbeError {
    /// Returns the artifact that was being probed.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.failure().path()
    }

    pub(crate) fn spawn(path: &Path, source: io::Error) -> Self {
        Self::Spawn(ProbeFailure::with_source(
            path,
            "failed to start ffprobe",
            ProbeErrorSource::Io(source),
        ))
    }

    pub(crate) fn output_reader(path: &Path, stream: Stream, source: io::Error) -> Self {
        Self::OutputRead(ProbeFailure::with_source(
            path,
            format!("failed to start the ffprobe {stream} reader"),
            ProbeErrorSource::Io(source),
        ))
    }

    pub(crate) fn output_join(path: &Path, stream: Stream) -> Self {
        Self::OutputRead(ProbeFailure::new(
            path,
            format!("ffprobe {stream} reader terminated unexpectedly"),
        ))
    }

    pub(crate) fn output_io(path: &Path, stream: Stream, source: io::Error) -> Self {
        Self::OutputRead(ProbeFailure::with_source(
            path,
            format!("failed to read ffprobe {stream}"),
            ProbeErrorSource::Io(source),
        ))
    }

    pub(crate) fn process_control(path: &Path, action: &str, source: io::Error) -> Self {
        Self::ProcessControl(ProbeFailure::with_source(
            path,
            format!("failed to {action} ffprobe"),
            ProbeErrorSource::Io(source),
        ))
    }

    pub(crate) fn timeout(path: &Path, timeout: Duration) -> Self {
        Self::Timeout(ProbeFailure::new(
            path,
            format!("ffprobe exceeded its {} ms timeout", timeout.as_millis()),
        ))
    }

    pub(crate) fn output_limit(path: &Path, stream: Stream, limit: usize) -> Self {
        Self::OutputLimit(ProbeFailure::new(
            path,
            format!("ffprobe {stream} exceeded its {limit}-byte capture limit"),
        ))
    }

    pub(crate) fn failed(path: &Path, status: ExitStatus, stderr: &[u8], truncated: bool) -> Self {
        let suffix = if truncated { " [truncated]" } else { "" };
        let stderr = String::from_utf8_lossy(stderr);
        let stderr = stderr.trim_ascii_end();
        let message = if stderr.is_empty() {
            format!("ffprobe exited with {status}{suffix}")
        } else {
            format!("ffprobe exited with {status}: {stderr}{suffix}")
        };

        Self::Failed(ProbeFailure::new(path, message))
    }

    pub(crate) fn invalid_response(path: &Path, source: serde_json::Error) -> Self {
        Self::InvalidResponse(ProbeFailure::with_source(
            path,
            "ffprobe emitted invalid JSON",
            ProbeErrorSource::Json(source),
        ))
    }

    pub(crate) fn missing_duration(path: &Path) -> Self {
        Self::MissingDuration(ProbeFailure::new(
            path,
            "ffprobe response contains no format duration",
        ))
    }

    pub(crate) fn invalid_duration(path: &Path, duration: &str, source: InvalidDuration) -> Self {
        Self::InvalidDuration(ProbeFailure::with_source(
            path,
            format!("ffprobe duration {duration:?} is invalid"),
            ProbeErrorSource::Duration(source),
        ))
    }

    pub(crate) fn invalid_video(path: &Path, detail: impl fmt::Display) -> Self {
        Self::InvalidVideo(ProbeFailure::new(
            path,
            format!("ffprobe video metadata is invalid: {detail}"),
        ))
    }

    pub(crate) fn invalid_video_duration(
        path: &Path,
        duration: &str,
        source: InvalidDuration,
    ) -> Self {
        Self::InvalidVideo(ProbeFailure::with_source(
            path,
            format!("ffprobe video stream duration {duration:?} is invalid"),
            ProbeErrorSource::Duration(source),
        ))
    }

    const fn failure(&self) -> &ProbeFailure {
        match self {
            Self::Spawn(failure)
            | Self::OutputRead(failure)
            | Self::ProcessControl(failure)
            | Self::Timeout(failure)
            | Self::OutputLimit(failure)
            | Self::Failed(failure)
            | Self::InvalidResponse(failure)
            | Self::MissingDuration(failure)
            | Self::InvalidDuration(failure)
            | Self::InvalidVideo(failure) => failure,
        }
    }
}

impl fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.failure().fmt(formatter)
    }
}

impl Error for ProbeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.failure().source()
    }
}

/// Artifact context carried by one typed probe error.
#[derive(Debug)]
pub struct ProbeFailure {
    path: PathBuf,
    message: Box<str>,
    source: Option<ProbeErrorSource>,
}

impl ProbeFailure {
    fn new(path: &Path, message: impl Into<Box<str>>) -> Self {
        Self {
            path: path.to_owned(),
            message: message.into(),
            source: None,
        }
    }

    fn with_source(path: &Path, message: impl Into<Box<str>>, source: ProbeErrorSource) -> Self {
        Self {
            path: path.to_owned(),
            message: message.into(),
            source: Some(source),
        }
    }

    /// Returns the artifact associated with this failure.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.message)
    }
}

impl Error for ProbeFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(ProbeErrorSource::as_error)
    }
}

#[derive(Debug)]
enum ProbeErrorSource {
    Io(io::Error),
    Json(serde_json::Error),
    Duration(InvalidDuration),
}

impl ProbeErrorSource {
    fn as_error(&self) -> &(dyn Error + 'static) {
        match self {
            Self::Io(source) => source,
            Self::Json(source) => source,
            Self::Duration(source) => source,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Stream {
    Stdout,
    Stderr,
}

impl fmt::Display for Stream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        };
        formatter.write_str(name)
    }
}
