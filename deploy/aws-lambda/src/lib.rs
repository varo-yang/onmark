#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

//! AWS Lambda adapter for one immutable Onmark worker-frame artifact.
//!
//! The adapter owns AWS invocation and object-storage concerns. It reuses the
//! renderer's portable worker request and never recompiles authored source.

#[cfg(feature = "runtime")]
use std::sync::Arc;

#[cfg(feature = "runtime")]
mod browser;
#[cfg(feature = "runtime")]
mod config;
#[cfg(feature = "runtime")]
mod deadline;
#[cfg(feature = "runtime")]
mod download;
#[cfg(feature = "runtime")]
mod error;
#[cfg(feature = "runtime")]
mod handler;
#[cfg(any(feature = "runtime", feature = "schema"))]
mod invocation;
#[cfg(feature = "runtime")]
mod publication;
#[cfg(feature = "runtime")]
mod storage;

#[cfg(feature = "runtime")]
use error::ReportedDeploymentError;

#[cfg(any(feature = "runtime", feature = "schema"))]
pub use invocation::{
    ArtifactLocation, CaptureInvocation, CaptureInvocationVersion, CaptureResult,
    InvalidObjectPrefix, ObjectPrefix, Publication,
};

/// Canonical executable name at the root of a Lambda browser archive.
pub const BROWSER_ARCHIVE_EXECUTABLE: &str = "chrome-headless-shell";

/// Maximum accepted compressed size of one Lambda browser archive.
pub const BROWSER_ARCHIVE_MAX_COMPRESSED_BYTES: u64 = 128 * 1024 * 1024;

/// Maximum number of entries accepted from one Lambda browser archive.
pub const BROWSER_ARCHIVE_MAX_ENTRIES: usize = 64;

/// Maximum expanded payload accepted from one Lambda browser archive.
pub const BROWSER_ARCHIVE_MAX_EXPANDED_BYTES: u64 = 320 * 1024 * 1024;

/// Starts the sequential Lambda runtime for immutable frame capture.
///
/// Lambda may reuse one process, but this adapter intentionally processes one
/// browser capture at a time. Parallelism belongs to separately invoked
/// workers until a coordinator has explicit resource and lease ownership.
///
/// # Errors
///
/// Returns a Lambda-runtime failure when deployment configuration, AWS
/// transport, worker materialization, browser capture, or artifact publication
/// cannot complete.
#[cfg(feature = "runtime")]
pub async fn run() -> Result<(), lambda_runtime::Error> {
    let handler = Arc::new(handler::CaptureHandler::from_environment().await?);
    let service = lambda_runtime::service_fn(
        move |event: lambda_runtime::LambdaEvent<CaptureInvocation>| {
            handle_invocation(Arc::clone(&handler), event)
        },
    );

    Box::pin(lambda_runtime::run(service)).await
}

#[cfg(feature = "runtime")]
async fn handle_invocation(
    handler: Arc<handler::CaptureHandler>,
    event: lambda_runtime::LambdaEvent<CaptureInvocation>,
) -> Result<CaptureResult, lambda_runtime::Error> {
    Box::pin(handler.handle(event.payload))
        .await
        .map_err(|source| Box::new(ReportedDeploymentError::new(source)) as lambda_runtime::Error)
}
