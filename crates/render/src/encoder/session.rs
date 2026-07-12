use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::protocol::WireFrameRate;
use tokio::io::AsyncWriteExt as _;
use tokio::process::{Child, ChildStdin};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout, timeout_at};

use super::error::{EncodeError, EncodeErrorKind};
use super::limits::{EncodeLimits, InvalidFfmpeg};
use super::process::{CapturedStderr, capture_stderr, spawn_ffmpeg};
use crate::EncodedPng;

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

/// Configured `FFmpeg` boundary for one sequential PNG frame stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Ffmpeg {
    executable: PathBuf,
    limits: EncodeLimits,
}

impl Ffmpeg {
    /// Creates a bounded `FFmpeg` boundary.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFfmpeg`] when the executable path is empty.
    pub fn new(
        executable: impl Into<PathBuf>,
        limits: EncodeLimits,
    ) -> Result<Self, InvalidFfmpeg> {
        let executable = executable.into();
        if executable.as_os_str().is_empty() {
            return Err(InvalidFfmpeg::EmptyExecutable);
        }
        Ok(Self { executable, limits })
    }

    pub(crate) const fn max_frames(&self) -> u64 {
        self.limits.max_frames()
    }

    /// Starts one H.264 MP4 encoding session.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] when the destination already exists or `FFmpeg`
    /// cannot be started with piped input and diagnostics.
    pub fn start(
        &self,
        output: impl Into<PathBuf>,
        frame_rate: WireFrameRate,
    ) -> Result<FfmpegSession, EncodeError> {
        let output = output.into();
        if output.exists() {
            return Err(EncodeError::new(
                EncodeErrorKind::OutputExists,
                &output,
                "output already exists",
            ));
        }

        let runtime = Handle::try_current().map_err(|_| {
            EncodeError::new(
                EncodeErrorKind::Spawn,
                &output,
                "FFmpeg encoding requires a Tokio runtime",
            )
        })?;
        let mut child = spawn_ffmpeg(&self.executable, &output, frame_rate)?;
        let Some(stdin) = child.stdin.take() else {
            return Err(EncodeError::new(
                EncodeErrorKind::Spawn,
                &output,
                "FFmpeg started without its configured input pipe",
            ));
        };
        let Some(stderr) = child.stderr.take() else {
            return Err(EncodeError::new(
                EncodeErrorKind::Spawn,
                &output,
                "FFmpeg started without its configured diagnostic pipe",
            ));
        };
        let stderr_limit = self.limits.max_stderr_bytes();
        let stderr = runtime.spawn(capture_stderr(stderr, stderr_limit));

        Ok(FfmpegSession {
            child,
            stdin: Some(stdin),
            stderr: Some(stderr),
            output,
            limits: self.limits,
            deadline: Instant::now() + self.limits.deadline(),
            frames: 0,
            input_bytes: 0,
            reaped: false,
            completed: false,
        })
    }
}

/// One owned `FFmpeg` process accepting a sequential PNG frame stream.
#[derive(Debug)]
pub struct FfmpegSession {
    child: Child,
    stdin: Option<ChildStdin>,
    stderr: Option<JoinHandle<std::io::Result<CapturedStderr>>>,
    output: PathBuf,
    limits: EncodeLimits,
    deadline: Instant,
    frames: u64,
    input_bytes: u64,
    reaped: bool,
    completed: bool,
}

impl FfmpegSession {
    /// Writes one complete PNG frame with backpressure from `FFmpeg` stdin.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] when a configured bound is exceeded, the
    /// process deadline expires, or `FFmpeg` closes its input pipe.
    pub async fn write_frame(&mut self, frame: &EncodedPng) -> Result<(), EncodeError> {
        let next_frame = self.frames.saturating_add(1);
        if next_frame > self.limits.max_frames() {
            return Err(EncodeError::new(
                EncodeErrorKind::FrameLimit,
                &self.output,
                "frame count exceeds the configured limit",
            ));
        }

        let frame_bytes = u64::try_from(frame.as_bytes().len()).map_err(|_| {
            self.error(
                EncodeErrorKind::InputLimit,
                "frame size exceeds the encoder accounting domain",
            )
        })?;
        let next_input_bytes = self.input_bytes.saturating_add(frame_bytes);
        if next_input_bytes > self.limits.max_input_bytes() {
            return Err(EncodeError::new(
                EncodeErrorKind::InputLimit,
                &self.output,
                "frame input exceeds the configured byte budget",
            ));
        }

        let Some(stdin) = self.stdin.as_mut() else {
            return Err(self.error(
                EncodeErrorKind::ProcessControl,
                "FFmpeg input is already closed",
            ));
        };
        timeout_at(self.deadline, stdin.write_all(frame.as_bytes()))
            .await
            .map_err(|_| self.error(EncodeErrorKind::Timeout, "FFmpeg exceeded its deadline"))?
            .map_err(|source| {
                EncodeError::io(
                    EncodeErrorKind::InputWrite,
                    &self.output,
                    "failed to write a frame to FFmpeg",
                    source,
                )
            })?;

        self.frames = next_frame;
        self.input_bytes = next_input_bytes;
        Ok(())
    }

    /// Closes frame input and observes the final MP4 result.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] when no frames were supplied, the process or
    /// stderr reader fails, the deadline expires, or `FFmpeg` exits unsuccessfully.
    pub async fn finish(mut self) -> Result<EncodedVideo, EncodeError> {
        if self.frames == 0 {
            self.terminate().await;
            return Err(self.error(EncodeErrorKind::NoFrames, "no frames were supplied"));
        }

        self.stdin.take();
        let status = match timeout_at(self.deadline, self.child.wait()).await {
            Ok(Ok(status)) => {
                self.reaped = true;
                status
            }
            Ok(Err(source)) => {
                self.terminate().await;
                return Err(EncodeError::io(
                    EncodeErrorKind::ProcessControl,
                    &self.output,
                    "failed to wait for FFmpeg",
                    source,
                ));
            }
            Err(_) => {
                self.terminate().await;
                return Err(self.error(EncodeErrorKind::Timeout, "FFmpeg exceeded its deadline"));
            }
        };
        let stderr = self.finish_stderr().await?;

        if !status.success() {
            return Err(self.failed(&status.to_string(), &stderr));
        }

        self.completed = true;
        Ok(EncodedVideo {
            path: self.output.clone(),
            frames: self.frames,
        })
    }

    fn error(&self, kind: EncodeErrorKind, message: &'static str) -> EncodeError {
        EncodeError::new(kind, &self.output, message)
    }

    fn failed(&self, status: &str, stderr: &CapturedStderr) -> EncodeError {
        let suffix = if stderr.truncated { " [truncated]" } else { "" };
        let stderr = String::from_utf8_lossy(&stderr.bytes);
        let stderr = stderr.trim_ascii_end();
        let message = if stderr.is_empty() {
            format!("FFmpeg exited with {status}{suffix}")
        } else {
            format!("FFmpeg exited with {status}: {stderr}{suffix}")
        };
        EncodeError::new(EncodeErrorKind::Failed, &self.output, message)
    }

    async fn finish_stderr(&mut self) -> Result<CapturedStderr, EncodeError> {
        let Some(mut stderr) = self.stderr.take() else {
            return Err(self.error(
                EncodeErrorKind::StderrRead,
                "FFmpeg stderr reader is already closed",
            ));
        };
        match timeout(CLEANUP_TIMEOUT, &mut stderr).await {
            Ok(Ok(Ok(captured))) => Ok(captured),
            Ok(Ok(Err(source))) => Err(EncodeError::io(
                EncodeErrorKind::StderrRead,
                &self.output,
                "failed to read FFmpeg stderr",
                source,
            )),
            Ok(Err(source)) => Err(EncodeError::join(&self.output, source)),
            Err(_) => {
                stderr.abort();
                let _ = stderr.await;
                Err(self.error(
                    EncodeErrorKind::StderrRead,
                    "FFmpeg stderr reader missed its cleanup deadline",
                ))
            }
        }
    }

    async fn terminate(&mut self) {
        self.stdin.take();
        let _ = self.child.start_kill();
        if matches!(timeout(CLEANUP_TIMEOUT, self.child.wait()).await, Ok(Ok(_))) {
            self.reaped = true;
        }
        if let Some(stderr) = self.stderr.take() {
            stderr.abort();
            let _ = stderr.await;
        }
    }
}

impl Drop for FfmpegSession {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.start_kill();
        }
        if let Some(stderr) = self.stderr.take() {
            stderr.abort();
        }
        if !self.completed {
            let _ = std::fs::remove_file(&self.output);
        }
    }
}

/// Completed encoded video artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedVideo {
    path: PathBuf,
    frames: u64,
}

impl EncodedVideo {
    /// Returns the completed MP4 path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the number of frames accepted by the encoder.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    pub(crate) fn published_at(self, path: PathBuf) -> Self {
        Self {
            path,
            frames: self.frames,
        }
    }
}
