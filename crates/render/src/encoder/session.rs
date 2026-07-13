use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::protocol::WireFrameRate;
use tokio::io::AsyncWriteExt as _;
use tokio::process::{Child, ChildStdin};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio::time::timeout;

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
    /// encoder inactivity timeout expires, or `FFmpeg` closes its input pipe.
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
        let write_result = timeout(
            self.limits.inactivity_timeout(),
            stdin.write_all(frame.as_bytes()),
        )
        .await;
        match write_result {
            Ok(Ok(())) => {}
            Ok(Err(source)) => return Err(self.input_write_failure(source).await),
            Err(_) => {
                // A cancelled write may have transferred a PNG prefix. Make
                // the session terminal before that partial frame can be reused.
                self.terminate().await;
                return Err(self.error(
                    EncodeErrorKind::Timeout,
                    "FFmpeg input made no progress before its inactivity timeout",
                ));
            }
        }

        self.frames = next_frame;
        self.input_bytes = next_input_bytes;
        Ok(())
    }

    /// Closes frame input and observes the final MP4 result.
    ///
    /// # Errors
    ///
    /// Returns [`EncodeError`] when no frames were supplied, the process or
    /// stderr reader fails, the inactivity timeout expires, or `FFmpeg` exits
    /// unsuccessfully.
    pub async fn finish(mut self) -> Result<EncodedVideo, EncodeError> {
        if self.frames == 0 {
            self.terminate().await;
            return Err(self.error(EncodeErrorKind::NoFrames, "no frames were supplied"));
        }

        self.stdin.take();
        let status = match timeout(self.limits.inactivity_timeout(), self.child.wait()).await {
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
                return Err(self.error(
                    EncodeErrorKind::Timeout,
                    "FFmpeg finalization made no progress before its inactivity timeout",
                ));
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
        let message = with_stderr(&format!("FFmpeg exited with {status}"), stderr);
        EncodeError::new(EncodeErrorKind::Failed, &self.output, message)
    }

    async fn input_write_failure(&mut self, source: std::io::Error) -> EncodeError {
        // The input pipe is unusable. Reaping the child closes stderr so the
        // bounded reader can return the encoder's actual rejection reason.
        self.stop_child().await;
        let message = match self.finish_stderr().await {
            Ok(stderr) => with_stderr("failed to write a frame to FFmpeg", &stderr),
            Err(stderr_error) => format!(
                "failed to write a frame to FFmpeg; diagnostics unavailable: {stderr_error}"
            ),
        };
        EncodeError::io(EncodeErrorKind::InputWrite, &self.output, message, source)
    }

    async fn finish_stderr(&mut self) -> Result<CapturedStderr, EncodeError> {
        let Some(mut stderr) = self.stderr.take() else {
            return Err(self.error(
                EncodeErrorKind::StderrRead,
                "FFmpeg stderr reader is already closed",
            ));
        };
        let Ok(joined) = timeout(CLEANUP_TIMEOUT, &mut stderr).await else {
            stderr.abort();
            let _ = stderr.await;
            return Err(self.error(
                EncodeErrorKind::StderrRead,
                "FFmpeg stderr reader missed its cleanup deadline",
            ));
        };
        let capture_result = joined.map_err(|source| EncodeError::join(&self.output, source))?;
        capture_result.map_err(|source| {
            EncodeError::io(
                EncodeErrorKind::StderrRead,
                &self.output,
                "failed to read FFmpeg stderr",
                source,
            )
        })
    }

    async fn terminate(&mut self) {
        self.stop_child().await;
        if let Some(stderr) = self.stderr.take() {
            stderr.abort();
            let _ = stderr.await;
        }
    }

    async fn stop_child(&mut self) {
        self.stdin.take();
        let _ = self.child.start_kill();
        if matches!(timeout(CLEANUP_TIMEOUT, self.child.wait()).await, Ok(Ok(_))) {
            self.reaped = true;
        }
    }
}

fn with_stderr(message: &str, stderr: &CapturedStderr) -> String {
    let suffix = if stderr.truncated { " [truncated]" } else { "" };
    let stderr = String::from_utf8_lossy(&stderr.bytes);
    let stderr = stderr.trim_ascii_end();
    if stderr.is_empty() {
        format!("{message}{suffix}")
    } else {
        format!("{message}: {stderr}{suffix}")
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use onmark_core::model::FrameRate;
    use onmark_core::protocol::WireFrameRate;
    use tempfile::{TempDir, tempdir};
    use tokio::time::sleep;

    use super::{EncodeError, EncodeErrorKind, EncodeLimits, Ffmpeg, FfmpegSession};
    use crate::EncodedPng;

    #[tokio::test]
    async fn translates_a_failed_encoder_and_removes_its_partial_output() {
        let fixture = EncoderFixture::new("failed.mp4", Duration::from_secs(1), 4_096);
        let error = fixture.finish().await;

        assert_eq!(error.kind(), EncodeErrorKind::Failed);
        assert!(error.to_string().contains("encoder rejected the stream"));
        assert!(!fixture.output().exists());
    }

    #[tokio::test]
    async fn retains_only_the_bounded_encoder_diagnostic_tail() {
        let fixture = EncoderFixture::new("failed-tail.mp4", Duration::from_secs(1), 64);
        let error = fixture.finish().await;
        let message = error.to_string();

        assert_eq!(error.kind(), EncodeErrorKind::Failed);
        assert!(message.contains("final encoder failure"));
        assert!(message.contains("[truncated]"));
        assert!(!fixture.output().exists());
    }

    #[tokio::test]
    async fn retains_encoder_diagnostics_when_frame_input_breaks() {
        let fixture = EncoderFixture::new("write-failed.mp4", Duration::from_secs(1), 4_096);
        let mut session = fixture.start();
        session
            .write_frame(&EncodedPng::new(vec![0]))
            .await
            .expect("the pipe may accept one frame while the fixture exits");
        sleep(Duration::from_millis(30)).await;

        let error = session
            .write_frame(&EncodedPng::new(vec![0]))
            .await
            .expect_err("the exited fixture must close its input pipe");

        assert_eq!(error.kind(), EncodeErrorKind::InputWrite);
        assert!(error.to_string().contains("decoder rejected the PNG frame"));
        drop(session);
        assert!(!fixture.output().exists());
    }

    #[tokio::test]
    async fn browser_time_does_not_consume_encoder_inactivity_budget() {
        let fixture = EncoderFixture::new("failed.mp4", Duration::from_millis(20), 4_096);
        let mut session = fixture.start();
        sleep(Duration::from_millis(40)).await;

        session
            .write_frame(&EncodedPng::new(vec![0]))
            .await
            .expect("an immediately accepted write gets a fresh inactivity budget");
        let error = session
            .finish()
            .await
            .expect_err("the fixture must report its intentional failure");

        assert_eq!(error.kind(), EncodeErrorKind::Failed);
        assert!(error.to_string().contains("encoder rejected the stream"));
    }

    #[tokio::test]
    async fn terminates_an_encoder_that_misses_its_inactivity_timeout() {
        let fixture = EncoderFixture::new("slow.mp4", Duration::from_millis(30), 4_096);
        let error = fixture.finish().await;

        assert_eq!(error.kind(), EncodeErrorKind::Timeout);
        assert!(!fixture.output().exists());
    }

    struct EncoderFixture {
        directory: TempDir,
        output_name: &'static str,
        ffmpeg: Ffmpeg,
    }

    impl EncoderFixture {
        fn new(
            output_name: &'static str,
            inactivity_timeout: Duration,
            stderr_limit: usize,
        ) -> Self {
            let directory = tempdir().expect("the fixture directory must be available");
            let limits = EncodeLimits::new(inactivity_timeout, 2, 2, stderr_limit)
                .expect("the fixture limits are bounded");
            let ffmpeg = Ffmpeg::new(fixture_executable(), limits)
                .expect("the fixture executable path is present");

            Self {
                directory,
                output_name,
                ffmpeg,
            }
        }

        async fn finish(&self) -> EncodeError {
            let mut session = self.start();
            session
                .write_frame(&EncodedPng::new(vec![0]))
                .await
                .expect("the fixture encoder must accept one byte");
            session
                .finish()
                .await
                .expect_err("the fixture encoder must fail")
        }

        fn start(&self) -> FfmpegSession {
            let rate = FrameRate::new(30, 1).expect("the fixture frame rate is valid");
            self.ffmpeg
                .start(self.output(), WireFrameRate::from(rate))
                .expect("the fixture encoder must start")
        }

        fn output(&self) -> PathBuf {
            self.directory.path().join(self.output_name)
        }
    }

    fn fixture_executable() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ffmpeg")
    }
}
