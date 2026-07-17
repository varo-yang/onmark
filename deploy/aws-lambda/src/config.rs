//! Environment-owned Lambda policy and its validated resource envelopes.
//!
//! Ambient configuration is read once here; downstream capture, storage, and
//! renderer boundaries receive explicit typed values.

use std::env;
use std::error::Error;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use onmark_render::{
    BrowserLimits, CaptureEnvironmentId, FrameArtifactLimits, InvalidCaptureEnvironmentId,
    UnitRootLimits,
};

use crate::browser::{BrowserArchiveDigest, BrowserPackage, InvalidBrowserArchiveDigest};
use crate::invocation::{InvalidObjectPrefix, ObjectPrefix};

const BROWSER_DEADLINE: Duration = Duration::from_mins(12);
// Leave two minutes of the Lambda 15-minute ceiling for multipart abort and
// runtime response delivery after the worker stops accepting new work.
const INVOCATION_WORK_DEADLINE: Duration = Duration::from_mins(13);
const MAX_CAPTURE_BYTES: usize = 64 * 1024 * 1024;
const MAX_FRAME_ARTIFACT_FRAMES: u64 = 1_000_000;
const MAX_FRAME_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_FRAME_ARTIFACT_FILE_BYTES: u64 = MAX_FRAME_ARTIFACT_BYTES + 1024 * 1024;
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
const MAX_INPUT_FILES: usize = 10_000;
// A publish collision temporarily retains worker input, the renderer's copied
// unit root, the newly captured artifact, and the artifact being verified for
// reuse. These limits cap those four retained groups at six GiB, leaving
// measured headroom inside Lambda's configured ten-GB `/tmp` volume for
// Chromium.
const MAX_INPUT_BYTES: u64 = 1024 * 1024 * 1024;

const S3_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const S3_OPERATION_TIMEOUT: Duration = Duration::from_secs(90);
const S3_OPERATION_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(45);
const S3_BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const S3_MAX_ATTEMPTS: u32 = 3;

const ARTIFACT_BUCKET: &str = "ONMARK_ARTIFACT_BUCKET";
const ARTIFACT_PREFIX: &str = "ONMARK_ARTIFACT_PREFIX";
const BROWSER_ARCHIVE: &str = "ONMARK_BROWSER_ARCHIVE";
const BROWSER_ARCHIVE_SHA256: &str = "ONMARK_BROWSER_ARCHIVE_SHA256";
const BROWSER_BINARY: &str = "ONMARK_BROWSER_BINARY";
const CAPTURE_ENVIRONMENT: &str = "ONMARK_CAPTURE_ENVIRONMENT";

/// Configuration fixed by one Lambda deployment.
///
/// Invocation payloads choose only an immutable worker-input prefix. The
/// browser binary, capture-environment identity, resource limits, and output
/// namespace belong to the deployed artifact and cannot be changed by callers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Configuration {
    artifact_destination: ObjectPrefix,
    browser: BrowserPackage,
    capture_environment: CaptureEnvironmentId,
}

impl Configuration {
    pub(crate) fn from_environment() -> Result<Self, ConfigurationError> {
        let bucket = required(ARTIFACT_BUCKET)?;
        let prefix = env::var(ARTIFACT_PREFIX).unwrap_or_default();
        let artifact_destination =
            ObjectPrefix::new(bucket, prefix).map_err(ConfigurationError::ArtifactPrefix)?;
        let browser = browser_package(
            optional_nonblank(BROWSER_BINARY)?,
            optional_nonblank(BROWSER_ARCHIVE)?,
            optional_nonblank(BROWSER_ARCHIVE_SHA256)?,
        )?;
        let capture_environment = CaptureEnvironmentId::parse(&required(CAPTURE_ENVIRONMENT)?)
            .map_err(ConfigurationError::CaptureEnvironment)?;

        Ok(Self {
            artifact_destination,
            browser,
            capture_environment,
        })
    }

    pub(crate) fn artifact_destination(&self) -> &ObjectPrefix {
        &self.artifact_destination
    }

    pub(crate) const fn browser(&self) -> &BrowserPackage {
        &self.browser
    }

    pub(crate) const fn capture_environment(&self) -> CaptureEnvironmentId {
        self.capture_environment
    }

    pub(crate) fn browser_limits() -> BrowserLimits {
        BrowserLimits::new(BROWSER_DEADLINE, MAX_CAPTURE_BYTES)
            .expect("the Lambda browser policy stays within the renderer safety envelope")
    }

    pub(crate) fn frame_artifact_limits() -> FrameArtifactLimits {
        FrameArtifactLimits::new(
            MAX_FRAME_ARTIFACT_FRAMES,
            MAX_FRAME_ARTIFACT_BYTES,
            MAX_FRAME_BYTES,
        )
        .expect("the Lambda artifact policy stays within the renderer safety envelope")
    }

    pub(crate) fn unit_root_limits() -> UnitRootLimits {
        UnitRootLimits::new(MAX_INPUT_FILES, MAX_INPUT_BYTES)
            .expect("the Lambda input policy stays within the renderer safety envelope")
    }

    pub(crate) const fn max_input_bytes() -> u64 {
        MAX_INPUT_BYTES
    }

    pub(crate) const fn max_input_files() -> usize {
        MAX_INPUT_FILES
    }

    pub(crate) const fn max_frame_artifact_file_bytes() -> u64 {
        MAX_FRAME_ARTIFACT_FILE_BYTES
    }

    pub(crate) const fn invocation_work_deadline() -> Duration {
        INVOCATION_WORK_DEADLINE
    }

    pub(crate) const fn s3_transport_limits() -> S3TransportLimits {
        S3TransportLimits {
            connect_timeout: S3_CONNECT_TIMEOUT,
            operation_timeout: S3_OPERATION_TIMEOUT,
            operation_attempt_timeout: S3_OPERATION_ATTEMPT_TIMEOUT,
            body_idle_timeout: S3_BODY_IDLE_TIMEOUT,
            max_attempts: S3_MAX_ATTEMPTS,
        }
    }
}

/// Fixed network budgets for one deployment-owned S3 client.
///
/// Service requests and streamed bodies are separate operations in the AWS
/// SDK, so the body needs its own progress deadline after the SDK's request
/// timeout has produced a response stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct S3TransportLimits {
    connect_timeout: Duration,
    operation_timeout: Duration,
    operation_attempt_timeout: Duration,
    body_idle_timeout: Duration,
    max_attempts: u32,
}

impl S3TransportLimits {
    pub(crate) fn timeout_configuration(self) -> aws_config::timeout::TimeoutConfig {
        aws_config::timeout::TimeoutConfig::builder()
            .connect_timeout(self.connect_timeout)
            .operation_timeout(self.operation_timeout)
            .operation_attempt_timeout(self.operation_attempt_timeout)
            .build()
    }

    pub(crate) fn retry_configuration(self) -> aws_config::retry::RetryConfig {
        aws_config::retry::RetryConfig::standard().with_max_attempts(self.max_attempts)
    }

    pub(crate) const fn body_idle_timeout(self) -> Duration {
        self.body_idle_timeout
    }
}

/// Reason a Lambda deployment cannot establish its fixed execution boundary.
#[derive(Debug)]
pub(crate) enum ConfigurationError {
    Missing(&'static str),
    Blank(&'static str),
    MissingBrowser,
    ConflictingBrowser,
    IncompleteBrowserArchive,
    BrowserArchiveDigest(InvalidBrowserArchiveDigest),
    ArtifactPrefix(InvalidObjectPrefix),
    CaptureEnvironment(InvalidCaptureEnvironmentId),
}

impl fmt::Display for ConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(
                formatter,
                "required Lambda environment variable {name} is missing"
            ),
            Self::Blank(name) => write!(
                formatter,
                "required Lambda environment variable {name} must not be blank"
            ),
            Self::MissingBrowser => formatter.write_str(
                "Lambda requires either ONMARK_BROWSER_BINARY or ONMARK_BROWSER_ARCHIVE",
            ),
            Self::ConflictingBrowser => formatter.write_str(
                "ONMARK_BROWSER_BINARY and ONMARK_BROWSER_ARCHIVE are mutually exclusive",
            ),
            Self::IncompleteBrowserArchive => formatter.write_str(
                "ONMARK_BROWSER_ARCHIVE and ONMARK_BROWSER_ARCHIVE_SHA256 must be set together",
            ),
            Self::BrowserArchiveDigest(source) => source.fmt(formatter),
            Self::ArtifactPrefix(source) => source.fmt(formatter),
            Self::CaptureEnvironment(source) => source.fmt(formatter),
        }
    }
}

impl Error for ConfigurationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Missing(_)
            | Self::Blank(_)
            | Self::MissingBrowser
            | Self::ConflictingBrowser
            | Self::IncompleteBrowserArchive => None,
            Self::BrowserArchiveDigest(source) => Some(source),
            Self::ArtifactPrefix(source) => Some(source),
            Self::CaptureEnvironment(source) => Some(source),
        }
    }
}

fn required(name: &'static str) -> Result<String, ConfigurationError> {
    env::var(name).map_err(|_| ConfigurationError::Missing(name))
}

fn optional_nonblank(name: &'static str) -> Result<Option<String>, ConfigurationError> {
    env::var(name)
        .ok()
        .map(|value| nonblank(name, value))
        .transpose()
}

fn browser_package(
    binary: Option<String>,
    archive: Option<String>,
    digest: Option<String>,
) -> Result<BrowserPackage, ConfigurationError> {
    match (binary, archive, digest) {
        (Some(binary), None, None) => Ok(BrowserPackage::expanded(PathBuf::from(binary))),
        (None, Some(archive), Some(digest)) => {
            let digest = BrowserArchiveDigest::parse(&digest)
                .map_err(ConfigurationError::BrowserArchiveDigest)?;
            Ok(BrowserPackage::archive(PathBuf::from(archive), digest))
        }
        (None, None, None) => Err(ConfigurationError::MissingBrowser),
        (Some(_), Some(_), _) | (Some(_), None, Some(_)) => {
            Err(ConfigurationError::ConflictingBrowser)
        }
        (None, Some(_), None) | (None, None, Some(_)) => {
            Err(ConfigurationError::IncompleteBrowserArchive)
        }
    }
}

fn nonblank(name: &'static str, value: String) -> Result<String, ConfigurationError> {
    if value.trim().is_empty() {
        return Err(ConfigurationError::Blank(name));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{BROWSER_BINARY, Configuration, ConfigurationError, browser_package, nonblank};

    #[test]
    fn rejects_a_blank_browser_binary() {
        let error = nonblank(BROWSER_BINARY, String::from(" \t"))
            .expect_err("a browser binary cannot be blank");

        assert!(matches!(error, ConfigurationError::Blank(BROWSER_BINARY)));
    }

    #[test]
    fn requires_one_complete_browser_source() {
        assert!(matches!(
            browser_package(None, None, None),
            Err(ConfigurationError::MissingBrowser),
        ));
        assert!(matches!(
            browser_package(
                Some("/browser".into()),
                Some("/browser.tar.zst".into()),
                None
            ),
            Err(ConfigurationError::ConflictingBrowser),
        ));
        assert!(matches!(
            browser_package(None, Some("/browser.tar.zst".into()), None),
            Err(ConfigurationError::IncompleteBrowserArchive),
        ));
    }

    #[test]
    fn defines_bounded_s3_transport_limits() {
        let limits = Configuration::s3_transport_limits();
        let timeouts = limits.timeout_configuration();
        let retry = limits.retry_configuration();

        assert_eq!(timeouts.connect_timeout(), Some(Duration::from_secs(5)));
        assert_eq!(timeouts.operation_timeout(), Some(Duration::from_secs(90)));
        assert_eq!(
            timeouts.operation_attempt_timeout(),
            Some(Duration::from_secs(45))
        );
        assert_eq!(limits.body_idle_timeout(), Duration::from_secs(30));
        assert_eq!(retry.max_attempts(), 3);
    }

    #[test]
    fn reserves_cleanup_time_below_the_lambda_ceiling() {
        assert_eq!(
            Configuration::invocation_work_deadline(),
            Duration::from_mins(13)
        );
    }
}
