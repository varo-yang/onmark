//! Persistent native composition of one transparent browser layer over video.
//!
//! One process owns decode, exact CFR selection, source-over composition,
//! canonical RGBA fingerprinting, and optional final encoding. Its stdin and
//! stdout advance under per-frame backpressure, so neither side can accumulate
//! an unbounded frame queue.

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use onmark_core::protocol::WireFrameRate;
use tokio::io::AsyncWriteExt as _;
use tokio::process::{Child, ChildStdin};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::error::{EncodeError, EncodeErrorKind};
use super::layered_process::{frame_bytes, read_frames, spawn, take_pipe, validate_job};
use super::limits::EncodeLimits;
use super::process::{CapturedStderr, capture_stderr};
use super::session::{EncodedVideo, with_stderr};
use crate::{EncodedPng, RawRgbaHash, RenderProfile};

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);
const FRAME_READER_FAILURE: TaskFailure = TaskFailure {
    kind: EncodeErrorKind::FrameRead,
    io: "failed to read layered FFmpeg frame output",
    join: "layered frame reader terminated unexpectedly",
    timeout: "layered frame reader missed its cleanup deadline",
};
const STDERR_READER_FAILURE: TaskFailure = TaskFailure {
    kind: EncodeErrorKind::StderrRead,
    io: "failed to read layered FFmpeg stderr",
    join: "layered FFmpeg stderr reader terminated unexpectedly",
    timeout: "layered FFmpeg stderr reader missed its cleanup deadline",
};

/// Output retained from each canonical composed frame.
#[derive(Debug)]
pub(crate) enum CanonicalFrame {
    Fingerprint(RawRgbaHash),
    Pixels {
        bytes: Box<[u8]>,
        fingerprint: RawRgbaHash,
    },
}

/// Whether the compositor publishes video or returns lossless worker pixels.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LayeredOutput {
    Video(PathBuf),
    Frames,
}

impl LayeredOutput {
    pub(super) fn retains_pixels(&self) -> bool {
        matches!(self, Self::Frames)
    }

    pub(super) fn video_path(&self) -> Option<&Path> {
        match self {
            Self::Video(path) => Some(path),
            Self::Frames => None,
        }
    }
}

/// Checked facts required to start one native composition stream.
pub(crate) struct LayeredMediaInput {
    pub(crate) path: PathBuf,
    pub(crate) source_frame_rate: WireFrameRate,
    pub(crate) frames: u64,
}

/// Checked facts required to start one native composition stream.
pub(crate) struct LayeredJob {
    pub(crate) media: Vec<LayeredMediaInput>,
    pub(crate) output_frame_rate: WireFrameRate,
    pub(crate) frames: u64,
    pub(crate) profile: RenderProfile,
    pub(crate) destination: LayeredOutput,
    pub(crate) diagnostic_path: PathBuf,
}

impl LayeredJob {
    pub(super) fn frame_count(&self) -> u64 {
        self.frames
    }
}

/// One owned native decode/composition process.
pub(crate) struct LayeredSession {
    child: Child,
    input: Option<ChildStdin>,
    frames: mpsc::Receiver<CanonicalFrame>,
    frame_reader: Option<JoinHandle<io::Result<()>>>,
    stderr: Option<JoinHandle<io::Result<CapturedStderr>>>,
    destination: LayeredOutput,
    diagnostic_path: PathBuf,
    limits: EncodeLimits,
    expected_frames: u64,
    submitted_frames: u64,
    input_bytes: u64,
    reaped: bool,
    completed: bool,
}

/// Terminal artifact produced by the chosen layered destination.
pub(crate) enum LayeredCompletion {
    Video(EncodedVideo),
    Frames,
}

impl LayeredSession {
    pub(crate) fn start(
        executable: &Path,
        limits: EncodeLimits,
        job: LayeredJob,
    ) -> Result<Self, EncodeError> {
        validate_job(&job, limits)?;
        let runtime = Handle::try_current().map_err(|_| {
            EncodeError::new(
                EncodeErrorKind::Spawn,
                &job.diagnostic_path,
                "layered composition requires a Tokio runtime",
            )
        })?;
        let mut child = spawn(executable, &job)?;
        let input = take_pipe(child.stdin.take(), &job.diagnostic_path, "input")?;
        let stdout = take_pipe(child.stdout.take(), &job.diagnostic_path, "frame output")?;
        let stderr = take_pipe(
            child.stderr.take(),
            &job.diagnostic_path,
            "diagnostic output",
        )?;
        let expected_frames = job.frame_count();
        let frame_bytes = frame_bytes(job.profile, &job.diagnostic_path)?;
        let (frame_sender, frames) = mpsc::channel(1);
        let retains_pixels = job.destination.retains_pixels();
        let frame_reader = runtime.spawn(read_frames(
            stdout,
            frame_bytes,
            expected_frames,
            retains_pixels,
            frame_sender,
        ));
        let stderr = runtime.spawn(capture_stderr(stderr, limits.max_stderr_bytes()));

        Ok(Self {
            child,
            input: Some(input),
            frames,
            frame_reader: Some(frame_reader),
            stderr: Some(stderr),
            destination: job.destination,
            diagnostic_path: job.diagnostic_path,
            limits,
            expected_frames,
            submitted_frames: 0,
            input_bytes: 0,
            reaped: false,
            completed: false,
        })
    }

    pub(crate) async fn write_frame(
        &mut self,
        foreground: &EncodedPng,
    ) -> Result<CanonicalFrame, EncodeError> {
        self.check_input(foreground)?;
        self.write_foreground(foreground).await?;
        let frame = self.receive_frame().await?;

        self.submitted_frames += 1;
        self.input_bytes += u64::try_from(foreground.as_bytes().len())
            .expect("the checked foreground size fits the encoder accounting domain");
        Ok(frame)
    }

    pub(crate) async fn write_video_frame(
        &mut self,
        foreground: &EncodedPng,
    ) -> Result<(), EncodeError> {
        match self.write_frame(foreground).await? {
            CanonicalFrame::Fingerprint(_fingerprint) => Ok(()),
            CanonicalFrame::Pixels { .. } => Err(self.error(
                EncodeErrorKind::FrameRead,
                "local layered composition unexpectedly retained frame pixels",
            )),
        }
    }

    async fn write_foreground(&mut self, foreground: &EncodedPng) -> Result<(), EncodeError> {
        let Some(input) = self.input.as_mut() else {
            return Err(self.error(
                EncodeErrorKind::ProcessControl,
                "layered composition input is already closed",
            ));
        };
        let write = timeout(
            self.limits.inactivity_timeout(),
            input.write_all(foreground.as_bytes()),
        )
        .await;
        match write {
            Ok(Ok(())) => Ok(()),
            Ok(Err(source)) => Err(self.input_write_failure(source).await),
            Err(_) => Err(self
                .process_failure(
                    EncodeErrorKind::Timeout,
                    "layered composition input made no progress before its inactivity timeout",
                )
                .await),
        }
    }

    async fn receive_frame(&mut self) -> Result<CanonicalFrame, EncodeError> {
        let frame = match timeout(self.limits.inactivity_timeout(), self.frames.recv()).await {
            Ok(Some(frame)) => frame,
            Ok(None) => return Err(self.early_frame_end().await),
            Err(_) => {
                return Err(self
                    .process_failure(
                        EncodeErrorKind::Timeout,
                        "layered frame output made no progress before its inactivity timeout",
                    )
                    .await);
            }
        };
        Ok(frame)
    }

    pub(crate) async fn finish(mut self) -> Result<LayeredCompletion, EncodeError> {
        if self.submitted_frames != self.expected_frames {
            self.terminate().await;
            return Err(self.error(
                EncodeErrorKind::NoFrames,
                "layered composition did not receive its planned frame count",
            ));
        }

        self.input.take();
        let status = self.wait_for_exit().await?;
        let stderr = self.finish_process_output().await?;
        if !status.success() {
            let message = with_stderr(
                &format!("layered FFmpeg composition exited with {status}"),
                &stderr,
            );
            return Err(EncodeError::new(
                EncodeErrorKind::Failed,
                &self.diagnostic_path,
                message,
            ));
        }

        self.completed = true;
        Ok(match self.destination.video_path() {
            Some(path) => LayeredCompletion::Video(EncodedVideo::completed(
                path.to_owned(),
                self.submitted_frames,
            )),
            None => LayeredCompletion::Frames,
        })
    }

    async fn wait_for_exit(&mut self) -> Result<ExitStatus, EncodeError> {
        match timeout(self.limits.inactivity_timeout(), self.child.wait()).await {
            Ok(Ok(status)) => {
                self.reaped = true;
                Ok(status)
            }
            Ok(Err(source)) => {
                let message = self
                    .stop_with_diagnostics("failed to wait for layered FFmpeg composition")
                    .await;
                Err(EncodeError::io(
                    EncodeErrorKind::ProcessControl,
                    &self.diagnostic_path,
                    message,
                    source,
                ))
            }
            Err(_) => Err(self
                .process_failure(
                    EncodeErrorKind::Timeout,
                    "layered composition missed its finalization deadline",
                )
                .await),
        }
    }

    async fn finish_process_output(&mut self) -> Result<CapturedStderr, EncodeError> {
        let frame_result = self.finish_frame_reader().await;
        let stderr_result = self.finish_stderr().await;
        if let Err(source) = frame_result {
            let message = observed_failure(source.message(), stderr_result.ok().as_ref());
            return Err(EncodeError::new(
                EncodeErrorKind::FrameRead,
                &self.diagnostic_path,
                message,
            ));
        }
        stderr_result
    }

    fn check_input(&self, foreground: &EncodedPng) -> Result<(), EncodeError> {
        if self.submitted_frames >= self.expected_frames {
            return Err(self.error(
                EncodeErrorKind::FrameLimit,
                "layered composition received more frames than planned",
            ));
        }
        let bytes = u64::try_from(foreground.as_bytes().len()).map_err(|_| {
            self.error(
                EncodeErrorKind::InputLimit,
                "foreground frame exceeds the encoder accounting domain",
            )
        })?;
        let total = self.input_bytes.checked_add(bytes).ok_or_else(|| {
            self.error(
                EncodeErrorKind::InputLimit,
                "foreground input exceeds the encoder accounting domain",
            )
        })?;
        if total > self.limits.max_input_bytes() {
            return Err(self.error(
                EncodeErrorKind::InputLimit,
                "foreground input exceeds the configured byte budget",
            ));
        }
        Ok(())
    }

    fn error(&self, kind: EncodeErrorKind, message: &'static str) -> EncodeError {
        EncodeError::new(kind, &self.diagnostic_path, message)
    }

    async fn finish_frame_reader(&mut self) -> Result<(), EncodeError> {
        let Some(reader) = self.frame_reader.take() else {
            return Err(self.error(
                EncodeErrorKind::FrameRead,
                "layered frame reader is already closed",
            ));
        };
        finish_task(reader, &self.diagnostic_path, FRAME_READER_FAILURE).await
    }

    async fn finish_stderr(&mut self) -> Result<CapturedStderr, EncodeError> {
        let Some(stderr) = self.stderr.take() else {
            return Err(self.error(
                EncodeErrorKind::StderrRead,
                "layered FFmpeg stderr reader is already closed",
            ));
        };
        finish_task(stderr, &self.diagnostic_path, STDERR_READER_FAILURE).await
    }

    async fn terminate(&mut self) {
        self.stop_child().await;
        if let Some(reader) = self.frame_reader.take() {
            reader.abort();
            let _ = reader.await;
        }
        if let Some(stderr) = self.stderr.take() {
            stderr.abort();
            let _ = stderr.await;
        }
    }

    async fn early_frame_end(&mut self) -> EncodeError {
        let frame_error = self.finish_frame_reader().await.err();
        self.stop_child().await;
        let stderr = self.finish_stderr().await.ok();
        let message = frame_error
            .as_ref()
            .map_or("FFmpeg ended the composed-frame stream early", |source| {
                source.message()
            });
        let message = observed_failure(message, stderr.as_ref());
        EncodeError::new(EncodeErrorKind::FrameRead, &self.diagnostic_path, message)
    }

    async fn input_write_failure(&mut self, source: io::Error) -> EncodeError {
        let message = self
            .stop_with_diagnostics("failed to write a foreground frame to FFmpeg")
            .await;
        EncodeError::io(
            EncodeErrorKind::InputWrite,
            &self.diagnostic_path,
            message,
            source,
        )
    }

    async fn process_failure(
        &mut self,
        kind: EncodeErrorKind,
        message: &'static str,
    ) -> EncodeError {
        let message = self.stop_with_diagnostics(message).await;
        EncodeError::new(kind, &self.diagnostic_path, message)
    }

    async fn stop_with_diagnostics(&mut self, message: &str) -> String {
        self.stop_child().await;
        if let Some(reader) = self.frame_reader.take() {
            reader.abort();
            let _ = reader.await;
        }
        let stderr = self.finish_stderr().await.ok();
        observed_failure(message, stderr.as_ref())
    }

    async fn stop_child(&mut self) {
        self.input.take();
        let _ = self.child.start_kill();
        if matches!(timeout(CLEANUP_TIMEOUT, self.child.wait()).await, Ok(Ok(_))) {
            self.reaped = true;
        }
    }
}

fn observed_failure(message: &str, stderr: Option<&CapturedStderr>) -> String {
    stderr.map_or_else(|| message.to_owned(), |stderr| with_stderr(message, stderr))
}

async fn finish_task<T>(
    mut task: JoinHandle<io::Result<T>>,
    output: &Path,
    failure: TaskFailure,
) -> Result<T, EncodeError> {
    match timeout(CLEANUP_TIMEOUT, &mut task).await {
        Ok(Ok(Ok(value))) => Ok(value),
        Ok(Ok(Err(source))) => Err(EncodeError::io(failure.kind, output, failure.io, source)),
        Ok(Err(source)) => Err(EncodeError::new(
            failure.kind,
            output,
            format!("{}: {source}", failure.join),
        )),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err(EncodeError::new(failure.kind, output, failure.timeout))
        }
    }
}

#[derive(Clone, Copy)]
struct TaskFailure {
    kind: EncodeErrorKind,
    io: &'static str,
    join: &'static str,
    timeout: &'static str,
}

impl Drop for LayeredSession {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.start_kill();
        }
        if let Some(reader) = self.frame_reader.take() {
            reader.abort();
        }
        if let Some(stderr) = self.stderr.take() {
            stderr.abort();
        }
        if !self.completed
            && let Some(path) = self.destination.video_path()
        {
            let _ = std::fs::remove_file(path);
        }
    }
}
