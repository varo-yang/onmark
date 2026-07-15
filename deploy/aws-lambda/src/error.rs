//! Typed deployment failures without leaking AWS errors into the render engine.
//!
//! Transport context is retained here for observability. Cleanup failures keep
//! both the original operation error and the abort error instead of replacing
//! the cause that initiated recovery.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_render::{
    CaptureEnvironmentId, FrameArtifactError, FrameArtifactId, RenderError, UnitRootError,
};
use tokio::task::JoinError;

use crate::config::ConfigurationError;

/// Typed infrastructure failure at the Lambda deployment boundary.
#[derive(Debug)]
pub(crate) enum DeploymentError {
    Configuration(ConfigurationError),
    S3 {
        operation: &'static str,
        bucket: Box<str>,
        key: Box<str>,
        source: Box<S3Failure>,
    },
    Filesystem {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Request {
        bucket: Box<str>,
        key: Box<str>,
        source: serde_json::Error,
    },
    DownloadLimit {
        role: S3ObjectRole,
        bucket: Box<str>,
        key: Box<str>,
        limit: u64,
    },
    S3IdleTimeout {
        bucket: Box<str>,
        key: Box<str>,
        timeout: Duration,
    },
    InputFiles {
        actual: usize,
        limit: usize,
    },
    InputLength {
        bucket: Box<str>,
        key: Box<str>,
        expected: u64,
        actual: u64,
    },
    WorkerTask(JoinError),
    Materialize(UnitRootError),
    Render(RenderError),
    Artifact(FrameArtifactError),
    CaptureEnvironment {
        requested: CaptureEnvironmentId,
        deployed: CaptureEnvironmentId,
    },
    ArtifactIdentity {
        expected: FrameArtifactId,
        actual: FrameArtifactId,
    },
    InvocationTimeout {
        timeout: Duration,
    },
    MultipartResponse {
        operation: &'static str,
        bucket: Box<str>,
        key: Box<str>,
        message: &'static str,
    },
    PublicationConflicts {
        bucket: Box<str>,
        key: Box<str>,
    },
    MultipartAbort {
        failure: Box<DeploymentError>,
        abort: Box<DeploymentError>,
    },
}

/// The deployment role of one S3 object whose retained bytes are bounded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum S3ObjectRole {
    WorkerRequest,
    WorkerInput,
    ExistingArtifact,
}

impl fmt::Display for S3ObjectRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WorkerRequest => "worker request",
            Self::WorkerInput => "worker input",
            Self::ExistingArtifact => "existing frame artifact",
        })
    }
}

/// Vendor failure while issuing or streaming an S3 operation.
#[derive(Debug)]
pub(crate) enum S3Failure {
    Service(aws_sdk_s3::Error),
    Body(io::Error),
}

impl fmt::Display for S3Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Service(source) => source.fmt(formatter),
            Self::Body(source) => source.fmt(formatter),
        }
    }
}

impl Error for S3Failure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Service(source) => Some(source),
            Self::Body(source) => Some(source),
        }
    }
}

impl DeploymentError {
    pub(crate) fn s3(
        operation: &'static str,
        bucket: &str,
        key: &str,
        source: impl Into<aws_sdk_s3::Error>,
    ) -> Self {
        Self::S3 {
            operation,
            bucket: bucket.into(),
            key: key.into(),
            source: Box::new(S3Failure::Service(source.into())),
        }
    }

    pub(crate) fn s3_body(
        operation: &'static str,
        bucket: &str,
        key: &str,
        source: io::Error,
    ) -> Self {
        Self::S3 {
            operation,
            bucket: bucket.into(),
            key: key.into(),
            source: Box::new(S3Failure::Body(source)),
        }
    }

    pub(crate) fn filesystem(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Filesystem {
            operation,
            path: path.to_owned(),
            source,
        }
    }

    pub(crate) fn request(bucket: &str, key: &str, source: serde_json::Error) -> Self {
        Self::Request {
            bucket: bucket.into(),
            key: key.into(),
            source,
        }
    }

    pub(crate) fn download_limit(role: S3ObjectRole, bucket: &str, key: &str, limit: u64) -> Self {
        Self::DownloadLimit {
            role,
            bucket: bucket.into(),
            key: key.into(),
            limit,
        }
    }

    pub(crate) fn s3_idle_timeout(bucket: &str, key: &str, timeout: Duration) -> Self {
        Self::S3IdleTimeout {
            bucket: bucket.into(),
            key: key.into(),
            timeout,
        }
    }

    pub(crate) const fn input_files(actual: usize, limit: usize) -> Self {
        Self::InputFiles { actual, limit }
    }

    pub(crate) fn input_length(bucket: &str, key: &str, expected: u64, actual: u64) -> Self {
        Self::InputLength {
            bucket: bucket.into(),
            key: key.into(),
            expected,
            actual,
        }
    }

    pub(crate) fn multipart_response(
        operation: &'static str,
        bucket: &str,
        key: &str,
        message: &'static str,
    ) -> Self {
        Self::MultipartResponse {
            operation,
            bucket: bucket.into(),
            key: key.into(),
            message,
        }
    }

    pub(crate) const fn invocation_timeout(timeout: Duration) -> Self {
        Self::InvocationTimeout { timeout }
    }

    pub(crate) fn multipart_abort(failure: Self, abort: Self) -> Self {
        Self::MultipartAbort {
            failure: Box::new(failure),
            abort: Box::new(abort),
        }
    }
}

impl fmt::Display for DeploymentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(source) => source.fmt(formatter),
            Self::S3 {
                operation,
                bucket,
                key,
                ..
            } => write!(formatter, "failed to {operation} s3://{bucket}/{key}"),
            Self::Filesystem {
                operation, path, ..
            } => write!(formatter, "failed to {operation} {}", path.display()),
            Self::Request { bucket, key, .. } => {
                write!(
                    formatter,
                    "failed to parse worker request s3://{bucket}/{key}"
                )
            }
            Self::DownloadLimit {
                role,
                bucket,
                key,
                limit,
            } => write!(
                formatter,
                "{role} s3://{bucket}/{key} exceeds the {limit}-byte deployment limit"
            ),
            Self::S3IdleTimeout {
                bucket,
                key,
                timeout,
            } => write!(
                formatter,
                "s3://{bucket}/{key} did not produce bytes within {timeout:?}"
            ),
            Self::InputFiles { actual, limit } => write!(
                formatter,
                "worker input declares {actual} files, exceeding the {limit}-file deployment limit"
            ),
            Self::InputLength {
                bucket,
                key,
                expected,
                actual,
            } => write!(
                formatter,
                "worker input s3://{bucket}/{key} has {actual} bytes, expected {expected}"
            ),
            Self::WorkerTask(_) => {
                formatter.write_str("worker materialization task did not complete")
            }
            Self::Materialize(source) => source.fmt(formatter),
            Self::Render(source) => source.fmt(formatter),
            Self::Artifact(source) => source.fmt(formatter),
            Self::CaptureEnvironment {
                requested,
                deployed,
            } => write!(
                formatter,
                "worker request requires capture environment {requested}, but this Lambda image provides {deployed}"
            ),
            Self::ArtifactIdentity { expected, actual } => write!(
                formatter,
                "frame artifact identity {actual} does not match requested identity {expected}"
            ),
            Self::InvocationTimeout { timeout } => {
                write!(
                    formatter,
                    "Lambda capture exceeded its {timeout:?} deadline"
                )
            }
            Self::MultipartResponse {
                operation,
                bucket,
                key,
                message,
            } => write!(
                formatter,
                "{operation} s3://{bucket}/{key} returned no {message}"
            ),
            Self::PublicationConflicts { bucket, key } => write!(
                formatter,
                "conditional publication repeatedly conflicted for s3://{bucket}/{key}"
            ),
            Self::MultipartAbort { failure, abort } => write!(
                formatter,
                "multipart publication failed ({failure}); abort also failed ({abort})"
            ),
        }
    }
}

impl Error for DeploymentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Configuration(source) => Some(source),
            Self::S3 { source, .. } => Some(source.as_ref()),
            Self::Filesystem { source, .. } => Some(source),
            Self::Request { source, .. } => Some(source),
            Self::WorkerTask(source) => Some(source),
            Self::Materialize(source) => Some(source),
            Self::Render(source) => Some(source),
            Self::Artifact(source) => Some(source),
            Self::MultipartAbort { failure, .. } => Some(failure),
            Self::DownloadLimit { .. }
            | Self::S3IdleTimeout { .. }
            | Self::InputFiles { .. }
            | Self::InputLength { .. }
            | Self::CaptureEnvironment { .. }
            | Self::ArtifactIdentity { .. }
            | Self::InvocationTimeout { .. }
            | Self::MultipartResponse { .. }
            | Self::PublicationConflicts { .. } => None,
        }
    }
}

impl From<ConfigurationError> for DeploymentError {
    fn from(source: ConfigurationError) -> Self {
        Self::Configuration(source)
    }
}

impl From<UnitRootError> for DeploymentError {
    fn from(source: UnitRootError) -> Self {
        Self::Materialize(source)
    }
}

impl From<RenderError> for DeploymentError {
    fn from(source: RenderError) -> Self {
        Self::Render(source)
    }
}

impl From<FrameArtifactError> for DeploymentError {
    fn from(source: FrameArtifactError) -> Self {
        Self::Artifact(source)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;
    use std::time::Duration;

    use super::DeploymentError;

    #[test]
    fn preserves_publication_and_abort_failures_together() {
        let failure = DeploymentError::invocation_timeout(Duration::from_secs(1));
        let abort = DeploymentError::invocation_timeout(Duration::from_secs(2));
        let error = DeploymentError::multipart_abort(failure, abort);

        assert!(matches!(error, DeploymentError::MultipartAbort { .. }));
        assert!(error.source().is_some());
        assert!(error.to_string().contains("abort also failed"));
    }
}
