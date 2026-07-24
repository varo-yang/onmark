//! Lambda composition root for one capture request.
//!
//! The handler sequences existing render boundaries; it does not fork compiler,
//! planner, browser, or artifact semantics for deployment.

use std::future::Future;
use std::path::Path;
use std::time::Instant;

use onmark_render::{Ffmpeg, FrameCaptureMetrics, WorkerCaptureRequest};
use tempfile::TempDir;

use crate::browser::BrowserRuntime;
use crate::config::Configuration;
use crate::deadline::InvocationDeadline;
use crate::download::InputMaterialization;
use crate::error::DeploymentError;
use crate::invocation::{CaptureInvocation, CaptureResult};
use crate::publication::ArtifactPublication;
use crate::storage::S3Storage;

const PACKAGED_FFMPEG: &str = "/var/task/ffmpeg";

/// One sequential Lambda capture worker backed by the shared renderer.
pub(crate) struct CaptureHandler {
    configuration: Configuration,
    storage: S3Storage,
    browser: BrowserRuntime,
}

impl CaptureHandler {
    pub(crate) async fn from_environment() -> Result<Self, DeploymentError> {
        let configuration = Configuration::from_environment()?;
        let transport = Configuration::s3_transport_limits();
        let sdk = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .retry_config(transport.retry_configuration())
            .timeout_config(transport.timeout_configuration())
            .load()
            .await;
        let storage = S3Storage::new(aws_sdk_s3::Client::new(&sdk), transport);
        let browser = BrowserRuntime::new(
            configuration.browser().clone(),
            Configuration::browser_limits(),
            Ffmpeg::new(PACKAGED_FFMPEG, Configuration::encode_limits())
                .expect("the deployed FFmpeg path and limits are fixed and nonempty"),
        );

        Ok(Self {
            configuration,
            storage,
            browser,
        })
    }

    pub(crate) async fn handle(
        &self,
        invocation: CaptureInvocation,
    ) -> Result<CaptureResult, DeploymentError> {
        let deadline = InvocationDeadline::after(Configuration::invocation_work_deadline());
        let workspace = TempDir::new_in("/tmp").map_err(|source| {
            DeploymentError::filesystem("create worker workspace", Path::new("/tmp"), source)
        })?;
        // The deadline owns each large phase future on the heap so one
        // long-lived Lambda handler does not retain every state machine.
        let request = InvocationPhase::ReadRequest
            .measure(deadline.run(self.storage.read_request(invocation.input())))
            .await?;
        self.require_capture_environment(&request)?;
        let expected_artifact = request.artifact_id();
        let materialization = InputMaterialization {
            source: invocation.input(),
            request: &request,
            workspace: workspace.path(),
            max_files: Configuration::max_input_files(),
            max_bytes: Configuration::max_input_bytes(),
        };
        InvocationPhase::MaterializeInputs
            .measure(deadline.run(self.storage.materialize_inputs(materialization)))
            .await?;

        let unit = InvocationPhase::MaterializeUnit
            .measure(deadline.run(Self::materialize_unit(request, workspace.path())))
            .await?;
        let capture = InvocationPhase::PrepareBrowser
            .measure(deadline.run(self.browser.executor()))
            .await?;
        let artifact_path = workspace.path().join("capture.onmark-frames");
        let capture_report = InvocationPhase::CaptureArtifact
            .measure(deadline.run(capture.capture_frame_artifact_report(
                &unit,
                self.configuration.capture_environment(),
                &artifact_path,
                Configuration::frame_artifact_limits(),
            )))
            .await?;
        if let Some(metrics) = capture_report.metrics() {
            log_capture_metrics(metrics);
        }
        let artifact = capture_report.into_artifact();
        InvocationPhase::VerifyArtifact
            .measure(deadline.run(artifact.verify()))
            .await?;
        Self::require_artifact_identity(expected_artifact, &artifact)?;

        let publication = ArtifactPublication {
            destination: self.configuration.artifact_destination(),
            artifact: &artifact,
            artifact_limits: Configuration::frame_artifact_limits(),
            workspace: workspace.path(),
            max_download_bytes: Configuration::max_frame_artifact_file_bytes(),
            deadline,
        };
        let (location, status) = InvocationPhase::PublishArtifact
            .measure(self.storage.publish(publication))
            .await?;

        Ok(CaptureResult::new(location, status))
    }

    fn require_capture_environment(
        &self,
        request: &WorkerCaptureRequest,
    ) -> Result<(), DeploymentError> {
        let deployed = self.configuration.capture_environment();
        let requested = request.capture_environment();
        if requested != deployed {
            return Err(DeploymentError::CaptureEnvironment {
                requested,
                deployed,
            });
        }
        Ok(())
    }

    async fn materialize_unit(
        request: WorkerCaptureRequest,
        workspace: &Path,
    ) -> Result<onmark_render::ExecutableUnit, DeploymentError> {
        let workspace = workspace.to_owned();
        let limits = Configuration::unit_root_limits();

        tokio::task::spawn_blocking(move || request.materialize_in(&workspace, &workspace, limits))
            .await
            .map_err(DeploymentError::WorkerTask)?
            .map_err(DeploymentError::Materialize)
    }

    fn require_artifact_identity(
        expected: onmark_render::FrameArtifactId,
        artifact: &onmark_render::FrameArtifact,
    ) -> Result<(), DeploymentError> {
        let actual = artifact.id();
        if actual != expected {
            return Err(DeploymentError::ArtifactIdentity { expected, actual });
        }
        Ok(())
    }
}

fn log_capture_metrics(metrics: FrameCaptureMetrics) {
    lambda_runtime::tracing::info!(
        frames = metrics.frames(),
        browser_captures = metrics.browser_captures(),
        browser_capture_commands = metrics.browser_capture_commands(),
        launch_ms = metrics.launch().as_millis(),
        runtime_setup_ms = metrics.runtime_setup().as_millis(),
        seek_ms = metrics.seek().as_millis(),
        readback_ms = metrics.readback().as_millis(),
        pixel_processing_ms = metrics.pixel_processing().as_millis(),
        confirm_ms = metrics.confirm().as_millis(),
        write_ms = metrics.write().as_millis(),
        shutdown_ms = metrics.shutdown().as_millis(),
        "browser capture cost attributed",
    );
}

#[derive(Clone, Copy, Debug)]
enum InvocationPhase {
    ReadRequest,
    MaterializeInputs,
    MaterializeUnit,
    PrepareBrowser,
    CaptureArtifact,
    VerifyArtifact,
    PublishArtifact,
}

impl InvocationPhase {
    async fn measure<T, E>(self, future: impl Future<Output = Result<T, E>>) -> Result<T, E> {
        let started = Instant::now();
        let result = future.await;
        let elapsed = started.elapsed();
        lambda_runtime::tracing::info!(
            phase = self.name(),
            elapsed_ms = elapsed.as_millis(),
            succeeded = result.is_ok(),
            "capture worker phase finished",
        );
        result
    }

    const fn name(self) -> &'static str {
        match self {
            Self::ReadRequest => "read_request",
            Self::MaterializeInputs => "materialize_inputs",
            Self::MaterializeUnit => "materialize_unit",
            Self::PrepareBrowser => "prepare_browser",
            Self::CaptureArtifact => "capture_artifact",
            Self::VerifyArtifact => "verify_artifact",
            Self::PublishArtifact => "publish_artifact",
        }
    }
}
