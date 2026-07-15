//! Lambda composition root for one capture request.
//!
//! The handler sequences existing render boundaries; it does not fork compiler,
//! planner, browser, or artifact semantics for deployment.

use std::path::Path;

use onmark_render::{ChromiumSandbox, FrameCaptureExecutor, WorkerCaptureRequest};
use tempfile::TempDir;

use crate::config::Configuration;
use crate::deadline::InvocationDeadline;
use crate::download::InputMaterialization;
use crate::error::DeploymentError;
use crate::invocation::{CaptureInvocation, CaptureResult};
use crate::publication::ArtifactPublication;
use crate::storage::S3Storage;

/// One sequential Lambda capture worker backed by the shared renderer.
#[derive(Clone)]
pub(crate) struct CaptureHandler {
    configuration: Configuration,
    storage: S3Storage,
    capture: FrameCaptureExecutor,
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
        let capture = FrameCaptureExecutor::new(
            configuration.browser_binary().to_owned(),
            ChromiumSandbox::Disabled,
            Configuration::browser_limits(),
        );

        Ok(Self {
            configuration,
            storage,
            capture,
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
        let request = deadline
            .run(self.storage.read_request(invocation.input()))
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
        deadline
            .run(self.storage.materialize_inputs(materialization))
            .await?;

        let unit = deadline
            .run(Self::materialize_unit(request, workspace.path()))
            .await?;
        let artifact_path = workspace.path().join("capture.onmark-frames");
        let artifact = deadline
            .run(self.capture.capture_frame_artifact(
                &unit,
                self.configuration.capture_environment(),
                &artifact_path,
                Configuration::frame_artifact_limits(),
            ))
            .await?;
        deadline.run(artifact.verify()).await?;
        Self::require_artifact_identity(expected_artifact, &artifact)?;

        let publication = ArtifactPublication {
            destination: self.configuration.artifact_destination(),
            artifact: &artifact,
            artifact_limits: Configuration::frame_artifact_limits(),
            workspace: workspace.path(),
            max_download_bytes: Configuration::max_frame_artifact_file_bytes(),
            deadline,
        };
        let (location, status) = self.storage.publish(publication).await?;

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
