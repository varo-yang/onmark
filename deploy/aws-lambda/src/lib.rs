#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![deny(clippy::print_stderr, clippy::print_stdout, clippy::unwrap_used)]

//! AWS Lambda adapter for one immutable Onmark worker-frame artifact.
//!
//! The adapter owns AWS invocation and object-storage concerns. It reuses the
//! renderer's portable worker request and never recompiles authored source.

#[cfg(feature = "runtime")]
mod config;
#[cfg(feature = "runtime")]
mod error;
#[cfg(feature = "runtime")]
mod handler;
mod invocation;
#[cfg(feature = "runtime")]
mod storage;

pub use invocation::{
    ArtifactLocation, CaptureInvocation, CaptureInvocationVersion, CaptureResult,
    InvalidObjectPrefix, ObjectPrefix, Publication,
};

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
    let handler = handler::CaptureHandler::from_environment().await?;
    let service = lambda_runtime::service_fn(
        move |event: lambda_runtime::LambdaEvent<CaptureInvocation>| {
            let handler = handler.clone();
            async move {
                handler
                    .handle(event.payload)
                    .await
                    .map_err(|source| Box::new(source) as lambda_runtime::Error)
            }
        },
    );

    lambda_runtime::run(service).await
}
