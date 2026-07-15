use std::path::Path;

use onmark_render::{ChromiumSandbox, FrameCaptureExecutor, WorkerCaptureRequest};
use tempfile::TempDir;

use crate::config::Configuration;
use crate::error::DeploymentError;
use crate::invocation::{CaptureInvocation, CaptureResult};
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
        let workspace = TempDir::new_in("/tmp").map_err(|source| {
            DeploymentError::filesystem("create worker workspace", Path::new("/tmp"), source)
        })?;
        // Generated AWS operation futures are deliberately heap-pinned at
        // phase boundaries, so one long-lived Lambda handler does not retain
        // their full state machines on its stack.
        let request = Box::pin(self.storage.read_request(invocation.input())).await?;
        self.require_capture_environment(&request)?;
        let expected_artifact = request.artifact_id();
        Box::pin(self.storage.materialize_inputs(
            invocation.input(),
            &request,
            workspace.path(),
            Configuration::max_input_files(),
            Configuration::max_input_bytes(),
        ))
        .await?;

        let unit = Self::materialize_unit(request, workspace.path()).await?;
        let artifact_path = workspace.path().join("capture.onmark-frames");
        let artifact = self
            .capture
            .capture_frame_artifact(
                &unit,
                self.configuration.capture_environment(),
                &artifact_path,
                Configuration::frame_artifact_limits(),
            )
            .await?;
        artifact.verify().await?;
        Self::require_artifact_identity(expected_artifact, &artifact)?;

        let (location, publication) = Box::pin(self.storage.publish(
            self.configuration.artifact_destination(),
            &artifact,
            Configuration::frame_artifact_limits(),
            workspace.path(),
            Configuration::max_frame_artifact_file_bytes(),
        ))
        .await?;

        Ok(CaptureResult::new(location, publication))
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
