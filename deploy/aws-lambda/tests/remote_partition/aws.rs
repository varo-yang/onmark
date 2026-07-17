//! Black-box AWS CLI boundary for one real conformance run.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use onmark_aws_lambda::{CaptureInvocation, CaptureResult, ObjectPrefix};
use onmark_render::{CaptureEnvironmentId, FrameArtifact, FrameArtifactLimits};
use serde::Deserialize;
use tokio::process::Command;
use tokio::time::timeout;

use super::fixture::CaptureCase;

const AWS_COMMAND_TIMEOUT: Duration = Duration::from_mins(2);
const LAMBDA_INVOCATION_TIMEOUT: Duration = Duration::from_mins(15);
const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

pub(super) struct RemoteEnvironment {
    aws: PathBuf,
    function: OsString,
    bucket: Box<str>,
    input_prefix: Box<str>,
    capture_environment: CaptureEnvironmentId,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl RemoteEnvironment {
    pub(super) fn read() -> Self {
        Self {
            aws: optional_path("ONMARK_AWS", "aws"),
            function: required("ONMARK_REMOTE_FUNCTION"),
            bucket: required_string("ONMARK_REMOTE_INPUT_BUCKET"),
            input_prefix: required_string("ONMARK_REMOTE_INPUT_PREFIX"),
            capture_environment: CaptureEnvironmentId::parse(&required_string(
                "ONMARK_REMOTE_CAPTURE_ENVIRONMENT",
            ))
            .expect("ONMARK_REMOTE_CAPTURE_ENVIRONMENT must be canonical"),
            ffmpeg: optional_path("ONMARK_FFMPEG", "ffmpeg"),
            ffprobe: optional_path("ONMARK_FFPROBE", "ffprobe"),
        }
    }

    pub(super) const fn capture_environment(&self) -> CaptureEnvironmentId {
        self.capture_environment
    }

    pub(super) fn ffmpeg(&self) -> &Path {
        &self.ffmpeg
    }

    pub(super) fn ffprobe(&self) -> &Path {
        &self.ffprobe
    }
}

pub(super) struct AwsConformance<'environment> {
    environment: &'environment RemoteEnvironment,
}

impl<'environment> AwsConformance<'environment> {
    pub(super) const fn new(environment: &'environment RemoteEnvironment) -> Self {
        Self { environment }
    }

    pub(super) async fn capture(&self, workspace: &Path, capture: &CaptureCase) -> FrameArtifact {
        let prefix = format!("{}/{}", self.environment.input_prefix, capture.name());
        self.publish_input(&prefix, capture).await;
        let result = self.invoke(workspace, &prefix, capture).await;
        assert_eq!(
            result.artifact().artifact_id(),
            capture.request().artifact_id()
        );
        assert_eq!(result.artifact().frames(), capture.frames());

        let output = workspace.join(format!("{}.onmark-frames", capture.name()));
        self.download_artifact(&result, &output).await;
        let artifact = FrameArtifact::open(output, artifact_limits(capture.frames()))
            .await
            .expect("the remote artifact envelope is valid");
        artifact
            .verify()
            .await
            .expect("the complete remote artifact checksum is valid");
        artifact
    }

    async fn publish_input(&self, prefix: &str, capture: &CaptureCase) {
        for relative in capture.files() {
            let source = capture.root().join(relative);
            let key = format!("{prefix}/{}", portable_path(relative));
            let arguments = [
                OsString::from("s3api"),
                OsString::from("put-object"),
                OsString::from("--bucket"),
                OsString::from(self.environment.bucket.as_ref()),
                OsString::from("--key"),
                OsString::from(key),
                OsString::from("--body"),
                source.into_os_string(),
                OsString::from("--if-none-match"),
                OsString::from("*"),
            ];
            run_aws(
                self.environment,
                &arguments,
                AWS_COMMAND_TIMEOUT,
                "publish worker input",
            )
            .await;
        }
    }

    async fn invoke(&self, workspace: &Path, prefix: &str, capture: &CaptureCase) -> CaptureResult {
        let invocation = CaptureInvocation::new(
            ObjectPrefix::new(self.environment.bucket.clone(), prefix)
                .expect("the conformance prefix is canonical"),
        );
        let invocation_path = workspace.join(format!("{}-invocation.json", capture.name()));
        let response_path = workspace.join(format!("{}-response.json", capture.name()));
        fs::write(
            &invocation_path,
            serde_json::to_vec(&invocation).expect("the invocation serializes"),
        )
        .expect("the invocation file is writable");

        let arguments = [
            OsString::from("lambda"),
            OsString::from("invoke"),
            OsString::from("--function-name"),
            self.environment.function.clone(),
            OsString::from("--cli-binary-format"),
            OsString::from("raw-in-base64-out"),
            OsString::from("--cli-connect-timeout"),
            OsString::from("10"),
            OsString::from("--cli-read-timeout"),
            OsString::from("900"),
            OsString::from("--payload"),
            OsString::from(format!("fileb://{}", invocation_path.display())),
            response_path.clone().into_os_string(),
        ];
        let output = run_aws(
            self.environment,
            &arguments,
            LAMBDA_INVOCATION_TIMEOUT,
            "invoke capture worker",
        )
        .await;
        let metadata: InvocationMetadata =
            serde_json::from_slice(&output.stdout).expect("AWS invoke metadata is JSON");
        let response = fs::read(response_path).expect("the Lambda response is readable");
        assert_eq!(metadata.status_code, 200);
        if let Some(error) = metadata.function_error {
            panic!(
                "Lambda reported {error}: {}",
                String::from_utf8_lossy(&response),
            );
        }
        serde_json::from_slice(&response).unwrap_or_else(|error| {
            panic!(
                "Lambda response is not a capture result: {error}: {}",
                String::from_utf8_lossy(&response),
            )
        })
    }

    async fn download_artifact(&self, result: &CaptureResult, output: &Path) {
        let artifact = result.artifact();
        let arguments = [
            OsString::from("s3api"),
            OsString::from("get-object"),
            OsString::from("--bucket"),
            OsString::from(artifact.bucket()),
            OsString::from("--key"),
            OsString::from(artifact.key()),
            output.as_os_str().to_owned(),
        ];
        run_aws(
            self.environment,
            &arguments,
            AWS_COMMAND_TIMEOUT,
            "download frame artifact",
        )
        .await;
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct InvocationMetadata {
    status_code: u16,
    #[serde(default)]
    function_error: Option<Box<str>>,
}

async fn run_aws(
    environment: &RemoteEnvironment,
    arguments: &[OsString],
    deadline: Duration,
    operation: &str,
) -> Output {
    let mut command = Command::new(&environment.aws);
    command
        .env("AWS_MAX_ATTEMPTS", "1")
        .args(arguments)
        .kill_on_drop(true);
    let output = timeout(deadline, command.output())
        .await
        .unwrap_or_else(|_| panic!("{operation} exceeded its conformance deadline"))
        .unwrap_or_else(|error| panic!("failed to start AWS CLI for {operation}: {error}"));
    assert!(
        output.status.success(),
        "{operation} failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn artifact_limits(frames: u64) -> FrameArtifactLimits {
    FrameArtifactLimits::new(frames, MAX_ARTIFACT_BYTES, MAX_FRAME_BYTES)
        .expect("the conformance artifact limits are bounded")
}

fn portable_path(path: &Path) -> String {
    path.components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .expect("fixture paths are portable UTF-8")
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn required(variable: &str) -> OsString {
    env::var_os(variable).unwrap_or_else(|| panic!("{variable} is required"))
}

fn required_string(variable: &str) -> Box<str> {
    let value = required(variable)
        .into_string()
        .unwrap_or_else(|_| panic!("{variable} must be UTF-8"))
        .into_boxed_str();
    assert!(!value.trim().is_empty(), "{variable} cannot be blank");
    value
}

fn optional_path(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(fallback))
}
