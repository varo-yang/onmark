//! Persistent native composition across one browser layer and native video.
//!
//! One process owns decode, exact CFR selection, ordered source-over
//! composition, and the selected terminal output. Browser stdin is always
//! backpressured; worker artifacts additionally return canonical RGBA through a
//! capacity-one frame channel, while local video stays entirely inside
//! `FFmpeg`.

use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;

use onmark_core::model::MediaSource;
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
use crate::visual::{NativeMediaSchedule, PixelRegion};
use crate::{DecodedRgba, RawRgbaHash, RenderProfile};

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

/// Exact pixels retained from one canonical distributed-composition frame.
#[derive(Debug)]
pub(crate) struct CanonicalFrame {
    bytes: Box<[u8]>,
    fingerprint: RawRgbaHash,
}

impl CanonicalFrame {
    pub(super) const fn new(bytes: Box<[u8]>, fingerprint: RawRgbaHash) -> Self {
        Self { bytes, fingerprint }
    }

    pub(crate) fn into_parts(self) -> (Box<[u8]>, RawRgbaHash) {
        (self.bytes, self.fingerprint)
    }
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
    pub(crate) source: MediaSource,
    /// Whole-placement schedule sliced by `source_skip` and `frames`.
    pub(crate) schedule: NativeMediaSchedule,
    /// Selected output frames skipped before this unit begins publishing.
    pub(crate) source_skip: u64,
    pub(crate) frames: u64,
}

/// One time-bounded native video placed above browser-owned pixels.
pub(crate) struct BackdropMediaInput {
    pub(crate) path: PathBuf,
    pub(crate) source_frame_rate: WireFrameRate,
    pub(crate) source: MediaSource,
    /// Whole-placement schedule sliced by `source_skip` and `frames`.
    pub(crate) schedule: NativeMediaSchedule,
    pub(crate) source_skip: u64,
    pub(crate) output_start: u64,
    pub(crate) frames: u64,
    pub(crate) source_region: PixelRegion,
    pub(crate) destination_region: PixelRegion,
}

/// Direction and media facts for one native composition process.
pub(crate) enum LayeredInputs {
    /// Sequential full-frame media beneath transparent browser pixels.
    VideoBase(Vec<LayeredMediaInput>),
    /// Time-bounded media above one browser-owned backdrop.
    BrowserBase(Vec<BackdropMediaInput>),
}

impl LayeredInputs {
    pub(super) fn media_count(&self) -> usize {
        match self {
            Self::VideoBase(media) => media.len(),
            Self::BrowserBase(media) => media.len(),
        }
    }
}

/// Checked facts required to start one native composition stream.
pub(crate) struct LayeredJob {
    pub(crate) inputs: LayeredInputs,
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

struct FrameOutput {
    receiver: mpsc::Receiver<CanonicalFrame>,
    reader: JoinHandle<io::Result<()>>,
}

fn start_frame_output(
    runtime: &Handle,
    child: &mut Child,
    job: &LayeredJob,
) -> Result<Option<FrameOutput>, EncodeError> {
    if !job.destination.retains_pixels() {
        return Ok(None);
    }

    let stdout = take_pipe(child.stdout.take(), &job.diagnostic_path, "frame output")?;
    let frame_bytes = frame_bytes(job.profile, &job.diagnostic_path)?;
    let (sender, receiver) = mpsc::channel(1);
    let reader = runtime.spawn(read_frames(stdout, frame_bytes, job.frame_count(), sender));

    Ok(Some(FrameOutput { receiver, reader }))
}

/// One owned native decode/composition process.
pub(crate) struct LayeredSession {
    child: Child,
    input: Option<ChildStdin>,
    frame_output: Option<FrameOutput>,
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
    Frames(CanonicalFrame),
}

impl LayeredSession {
    pub(crate) fn start(
        executable: &Path,
        limits: EncodeLimits,
        job: LayeredJob,
        profile: super::profile::EncodeProfile,
    ) -> Result<Self, EncodeError> {
        validate_job(&job, limits)?;
        let runtime = Handle::try_current().map_err(|_| {
            EncodeError::new(
                EncodeErrorKind::Spawn,
                &job.diagnostic_path,
                "layered composition requires a Tokio runtime",
            )
        })?;
        let mut child = spawn(executable, &job, limits.video_encoder_threads(), profile)?;
        let input = take_pipe(child.stdin.take(), &job.diagnostic_path, "input")?;
        let stderr = take_pipe(
            child.stderr.take(),
            &job.diagnostic_path,
            "diagnostic output",
        )?;
        let expected_frames = job.frame_count();
        let frame_output = start_frame_output(&runtime, &mut child, &job)?;
        let stderr = runtime.spawn(capture_stderr(stderr, limits.max_stderr_bytes()));

        Ok(Self {
            child,
            input: Some(input),
            frame_output,
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

    pub(crate) async fn write_artifact_frame(
        &mut self,
        foreground: &DecodedRgba,
    ) -> Result<Option<CanonicalFrame>, EncodeError> {
        if !self.destination.retains_pixels() {
            return Err(self.error(
                EncodeErrorKind::FrameRead,
                "video composition cannot return canonical frame pixels",
            ));
        }
        self.submit_foreground(foreground).await?;

        // FFmpeg framesync releases a foreground only after seeing the next
        // timestamp. Keep that single-frame lookahead explicit and bounded.
        if self.submitted_frames == 1 {
            return Ok(None);
        }
        self.receive_frame().await.map(Some)
    }

    async fn submit_foreground(&mut self, foreground: &DecodedRgba) -> Result<(), EncodeError> {
        self.check_input(foreground)?;
        self.write_foreground(foreground).await?;
        self.submitted_frames += 1;
        self.input_bytes += u64::try_from(foreground.as_bytes().len())
            .expect("the checked foreground size fits the encoder accounting domain");
        Ok(())
    }

    pub(crate) async fn write_video_frame(
        &mut self,
        foreground: &DecodedRgba,
    ) -> Result<(), EncodeError> {
        if self.destination.retains_pixels() {
            return Err(self.error(
                EncodeErrorKind::FrameRead,
                "frame-artifact composition cannot publish encoded video",
            ));
        }
        self.submit_foreground(foreground).await
    }

    async fn write_foreground(&mut self, foreground: &DecodedRgba) -> Result<(), EncodeError> {
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
        let Some(output) = &mut self.frame_output else {
            return Err(self.error(
                EncodeErrorKind::FrameRead,
                "layered composition has no canonical frame output",
            ));
        };
        let frame = match timeout(self.limits.inactivity_timeout(), output.receiver.recv()).await {
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
        let final_frame = if self.destination.retains_pixels() {
            Some(self.receive_frame().await?)
        } else {
            None
        };
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

        let completion = match (&self.destination, final_frame) {
            (LayeredOutput::Video(path), None) => LayeredCompletion::Video(
                EncodedVideo::completed(path.to_owned(), self.submitted_frames),
            ),
            (LayeredOutput::Frames, Some(frame)) => LayeredCompletion::Frames(frame),
            (LayeredOutput::Video(_), Some(_)) => {
                return Err(self.error(
                    EncodeErrorKind::FrameRead,
                    "local layered composition unexpectedly retained frame pixels",
                ));
            }
            (LayeredOutput::Frames, None) => {
                return Err(self.error(
                    EncodeErrorKind::FrameRead,
                    "layered worker composition did not retain final frame pixels",
                ));
            }
        };
        self.completed = true;
        Ok(completion)
    }

    pub(crate) async fn abort(mut self) -> Result<(), EncodeError> {
        self.input.take();
        let process = self.abort_process().await;
        let frames = self.finish_frame_output().await;
        let stderr = self.abort_stderr().await;

        process?;
        frames?;
        stderr
    }

    async fn abort_process(&mut self) -> Result<(), EncodeError> {
        if self.reaped {
            return Ok(());
        }

        let _ = self.child.start_kill();
        match timeout(CLEANUP_TIMEOUT, self.child.wait()).await {
            Ok(Ok(_)) => {
                self.reaped = true;
                Ok(())
            }
            Ok(Err(source)) => Err(EncodeError::io(
                EncodeErrorKind::ProcessControl,
                &self.diagnostic_path,
                "failed to reap aborted layered FFmpeg composition",
                source,
            )),
            Err(_) => Err(self.error(
                EncodeErrorKind::ProcessControl,
                "aborted layered FFmpeg composition missed its cleanup deadline",
            )),
        }
    }

    async fn abort_stderr(&mut self) -> Result<(), EncodeError> {
        if self.stderr.is_none() {
            return Ok(());
        }
        self.finish_stderr().await.map(drop)
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
        let frame_result = self.finish_frame_output().await;
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

    fn check_input(&self, foreground: &DecodedRgba) -> Result<(), EncodeError> {
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

    async fn finish_frame_output(&mut self) -> Result<(), EncodeError> {
        let Some(output) = self.frame_output.take() else {
            return Ok(());
        };
        finish_task(output.reader, &self.diagnostic_path, FRAME_READER_FAILURE).await
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
        if let Some(output) = self.frame_output.take() {
            output.reader.abort();
            let _ = output.reader.await;
        }
        if let Some(stderr) = self.stderr.take() {
            stderr.abort();
            let _ = stderr.await;
        }
    }

    async fn early_frame_end(&mut self) -> EncodeError {
        let frame_error = self.finish_frame_output().await.err();
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
        if let Some(output) = self.frame_output.take() {
            output.reader.abort();
            let _ = output.reader.await;
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
        if let Some(output) = self.frame_output.take() {
            output.reader.abort();
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
