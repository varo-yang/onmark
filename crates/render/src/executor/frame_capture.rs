//! Capture of one executable unit into a reusable frame artifact.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::capture::{
    CaptureSurface, CaptureTask, FrameSink, RequestSequence, render_session, validate_plan,
    write_canonical_artifact,
};
use super::{RenderError, invalid_plan, layered_job};
use crate::encoder::{LayeredCompletion, LayeredOutput};
use crate::{
    BrowserCaptureMode, BrowserLaunchPolicy, BrowserLimits, BrowserSession, CaptureEnvironmentId,
    ExecutableUnit, Ffmpeg, FrameArtifact, FrameArtifactErrorKind, FrameArtifactLimits,
};

/// Aggregate wall-time attribution for one browser capture session.
///
/// These measurements explain executor cost; frame identity and scheduling
/// remain derived exclusively from the render plan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameCaptureMetrics {
    pub(super) frames: u64,
    pub(super) launch: Duration,
    pub(super) runtime_setup: Duration,
    pub(super) seek: Duration,
    pub(super) readback: Duration,
    pub(super) fingerprint: Duration,
    pub(super) confirm: Duration,
    pub(super) write: Duration,
    pub(super) shutdown: Duration,
}

impl FrameCaptureMetrics {
    /// Returns the number of frames written by the measured session.
    #[must_use]
    pub const fn frames(self) -> u64 {
        self.frames
    }

    /// Returns Chromium process and CDP connection time.
    #[must_use]
    pub const fn launch(self) -> Duration {
        self.launch
    }

    /// Returns navigation, compositor initialization, load, and prepare time.
    #[must_use]
    pub const fn runtime_setup(self) -> Duration {
        self.runtime_setup
    }

    /// Returns aggregate runtime staging and media-seek time.
    #[must_use]
    pub const fn seek(self) -> Duration {
        self.seek
    }

    /// Returns aggregate `BeginFrame`, screenshot readback, and Base64 decode time.
    #[must_use]
    pub const fn readback(self) -> Duration {
        self.readback
    }

    /// Returns aggregate PNG decode and canonical raw-RGBA hashing time.
    #[must_use]
    pub const fn fingerprint(self) -> Duration {
        self.fingerprint
    }

    /// Returns aggregate decoded-media confirmation time.
    #[must_use]
    pub const fn confirm(self) -> Duration {
        self.confirm
    }

    /// Returns aggregate frame-sink write time.
    #[must_use]
    pub const fn write(self) -> Duration {
        self.write
    }

    /// Returns browser and CDP shutdown time.
    #[must_use]
    pub const fn shutdown(self) -> Duration {
        self.shutdown
    }
}

/// One completed worker artifact together with capture-cost attribution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameCaptureReport {
    artifact: FrameArtifact,
    metrics: Option<FrameCaptureMetrics>,
}

impl FrameCaptureReport {
    /// Returns the completed immutable artifact.
    #[must_use]
    pub const fn artifact(&self) -> &FrameArtifact {
        &self.artifact
    }

    /// Returns aggregate timings when this call performed a capture.
    ///
    /// A reused artifact has no capture session and therefore no timings.
    #[must_use]
    pub const fn metrics(&self) -> Option<FrameCaptureMetrics> {
        self.metrics
    }

    /// Transfers ownership of the completed artifact.
    #[must_use]
    pub fn into_artifact(self) -> FrameArtifact {
        self.artifact
    }
}

/// Bounded Chromium capture boundary shared by local and worker execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameCaptureExecutor {
    browser_executable: PathBuf,
    capture_mode: BrowserCaptureMode,
    launch_policy: BrowserLaunchPolicy,
    browser_limits: BrowserLimits,
    ffmpeg: Ffmpeg,
}

impl FrameCaptureExecutor {
    /// Creates one browser-only capture boundary.
    ///
    /// Local callers retain [`BrowserLaunchPolicy::local`]. A deployment
    /// adapter may select an isolated-worker policy only when its independently
    /// audited outer boundary owns process isolation.
    #[must_use]
    pub fn new(
        browser_executable: impl Into<PathBuf>,
        launch_policy: BrowserLaunchPolicy,
        capture_mode: BrowserCaptureMode,
        browser_limits: BrowserLimits,
        ffmpeg: Ffmpeg,
    ) -> Self {
        Self {
            browser_executable: browser_executable.into(),
            capture_mode,
            launch_policy,
            browser_limits,
            ffmpeg,
        }
    }

    /// Returns the browser surface mechanism selected for this executor.
    #[must_use]
    pub const fn capture_mode(&self) -> BrowserCaptureMode {
        self.capture_mode
    }

    /// Captures one independently executable unit into a verified worker artifact.
    ///
    /// The artifact contains ordered PNG frames rather than an independently
    /// encoded MP4. A later assembler can therefore retain one continuous
    /// visual encoder and one final audio mix across workers.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when the unit, browser, or artifact boundary
    /// fails. A failed capture never publishes a partial artifact. If a
    /// matching complete artifact for the same capture environment already
    /// exists, it is checksum-verified and reused without launching Chromium.
    pub async fn capture_frame_artifact(
        &self,
        unit: &ExecutableUnit,
        capture_environment: CaptureEnvironmentId,
        artifact: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<FrameArtifact, RenderError> {
        self.capture_frame_artifact_report(unit, capture_environment, artifact, limits)
            .await
            .map(FrameCaptureReport::into_artifact)
    }

    /// Captures one worker artifact and reports bounded phase timings.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] under the same conditions as
    /// [`Self::capture_frame_artifact`].
    pub async fn capture_frame_artifact_report(
        &self,
        unit: &ExecutableUnit,
        capture_environment: CaptureEnvironmentId,
        artifact: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<FrameCaptureReport, RenderError> {
        let requests = validate_plan(unit.browser_plan(), limits.max_frames(), artifact)?;
        let mut writer =
            match FrameArtifact::writer_for_capture(unit, capture_environment, artifact, limits)
                .await
            {
                Ok(writer) => writer,
                Err(error) if error.kind() == FrameArtifactErrorKind::OutputExists => {
                    let artifact = self
                        .reuse_artifact(unit, capture_environment, artifact, limits)
                        .await?;
                    return Ok(FrameCaptureReport {
                        artifact,
                        metrics: None,
                    });
                }
                Err(error) => return Err(RenderError::artifact(artifact, error)),
            };
        let metrics = self
            .capture_artifact_frames(unit, &mut writer, requests, artifact)
            .await?;
        let artifact = match writer.finish().await {
            Ok(artifact) => artifact,
            Err(error) if error.kind() == FrameArtifactErrorKind::OutputExists => {
                self.reuse_artifact(unit, capture_environment, artifact, limits)
                    .await?
            }
            Err(error) => return Err(RenderError::artifact(artifact, error)),
        };
        Ok(FrameCaptureReport {
            artifact,
            metrics: Some(metrics),
        })
    }

    async fn capture_artifact_frames(
        &self,
        unit: &ExecutableUnit,
        writer: &mut crate::frame_artifact::FrameArtifactWriter,
        requests: RequestSequence,
        output: &Path,
    ) -> Result<FrameCaptureMetrics, RenderError> {
        if unit.visual_execution().layered_media().is_none() {
            let mut frames = FrameSink::Artifact(writer);
            return self.capture_unit(unit, &mut frames, requests, output).await;
        }

        let job = layered_job(std::slice::from_ref(unit), LayeredOutput::Frames, output)?;
        let mut compositor = self
            .ffmpeg
            .start_layered(job)
            .map_err(|source| RenderError::encoder(output, source))?;
        let mut frames = FrameSink::LayeredArtifact {
            compositor: &mut compositor,
            artifact: writer,
        };
        let capture = self.capture_unit(unit, &mut frames, requests, output).await;
        let mut metrics = match capture {
            Ok(metrics) => metrics,
            Err(capture) => {
                return Err(super::abort_compositor(compositor, capture, output).await);
            }
        };
        let started = Instant::now();
        let completion = compositor
            .finish()
            .await
            .map_err(|source| RenderError::encoder(output, source))?;
        let LayeredCompletion::Frames(final_frame) = completion else {
            return Err(invalid_plan(
                output,
                "layered worker composition unexpectedly produced encoded video",
            ));
        };
        write_canonical_artifact(writer, unit.profile(), final_frame, output).await?;
        metrics.write += started.elapsed();
        Ok(metrics)
    }

    async fn reuse_artifact(
        &self,
        unit: &ExecutableUnit,
        capture_environment: CaptureEnvironmentId,
        artifact: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<FrameArtifact, RenderError> {
        FrameArtifact::reuse_for_capture(unit, capture_environment, artifact, limits)
            .await
            .map_err(|source| RenderError::artifact(artifact, source))
    }

    pub(super) async fn capture_unit(
        &self,
        unit: &ExecutableUnit,
        frames: &mut FrameSink<'_>,
        requests: RequestSequence,
        output: &Path,
    ) -> Result<FrameCaptureMetrics, RenderError> {
        let foreground = unit
            .visual_execution()
            .layered_media()
            .is_some()
            .then(|| unit.browser_plan().foreground_only());
        let (plan, surface) = match foreground.as_ref() {
            Some(plan) => (plan, CaptureSurface::Transparent),
            None => (unit.browser_plan(), CaptureSurface::Opaque),
        };
        let launch_started = Instant::now();
        let mut browser = BrowserSession::launch(
            &self.browser_executable,
            self.launch_policy,
            self.capture_mode,
            unit.profile(),
            self.browser_limits,
        )
        .await
        .map_err(|source| RenderError::browser(output, source))?;
        let mut metrics = FrameCaptureMetrics {
            launch: launch_started.elapsed(),
            ..FrameCaptureMetrics::default()
        };

        let render_result = render_session(
            &mut browser,
            frames,
            &mut metrics,
            CaptureTask {
                plan,
                requests,
                entry_url: unit.entry_url(),
                resource_root: unit.resource_root(),
                surface,
                output,
            },
        )
        .await;
        let shutdown_started = Instant::now();
        let shutdown_result = browser
            .shutdown()
            .await
            .map_err(|source| RenderError::browser(output, source));
        metrics.shutdown = shutdown_started.elapsed();

        match (render_result, shutdown_result) {
            (Ok(()), Ok(())) => Ok(metrics),
            (Err(render), Ok(())) => Err(render),
            (Ok(()), Err(shutdown)) => Err(shutdown),
            (Err(render), Err(shutdown)) => {
                Err(render.with_cleanup_failure("browser shutdown", shutdown))
            }
        }
    }
}
