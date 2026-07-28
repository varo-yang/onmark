//! Capture of one executable unit into a reusable frame artifact.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use onmark_core::protocol::BrowserMediaMode;

use super::capture::{
    CaptureSurface, CaptureTask, FrameSink, RequestSequence, preflight_media_layout,
    render_session, validate_plan, write_canonical_artifact,
};
use super::{RenderError, RenderErrorKind, backdrop_job, invalid_plan, layered_job};
use crate::encoder::{LayeredCompletion, LayeredOutput};
use crate::frame_artifact::FrameArtifactWriter;
use crate::{
    AlphaMode, BrowserCaptureMode, BrowserGraphicsBackend, BrowserLaunchPolicy, BrowserLimits,
    BrowserSession, BrowserSessionOptions, CaptureEnvironmentId, ExecutableUnit, Ffmpeg,
    FrameArtifact, FrameArtifactErrorKind, FrameArtifactLimits,
};

struct PendingArtifact<'a> {
    unit: &'a ExecutableUnit,
    output: &'a Path,
    requests: RequestSequence,
    writer: FrameArtifactWriter,
}

/// Aggregate wall-time attribution for one browser capture session.
///
/// These measurements explain executor cost; frame identity and scheduling
/// remain derived exclusively from the render plan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameCaptureMetrics {
    pub(super) frames: u64,
    pub(super) browser_captures: u64,
    pub(super) browser_capture_commands: u64,
    pub(super) launch: Duration,
    pub(super) runtime_setup: Duration,
    pub(super) seek: Duration,
    pub(super) readback: Duration,
    pub(super) pixel_processing: Duration,
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

    /// Returns how many authored output frames entered browser capture.
    ///
    /// Bounded retry and reconciliation readbacks contribute to
    /// [`Self::readback`] rather than appearing as additional authored frames.
    #[must_use]
    pub const fn browser_captures(self) -> u64 {
        self.browser_captures
    }

    /// Returns the number of pixel-capture commands sent to Chromium.
    ///
    /// Unlike [`Self::browser_captures`], this includes bounded retries and
    /// placement reconciliation.
    #[must_use]
    pub const fn browser_capture_commands(self) -> u64 {
        self.browser_capture_commands
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

    /// Returns aggregate browser-PNG decoding and raw-RGBA hashing time.
    #[must_use]
    pub const fn pixel_processing(self) -> Duration {
        self.pixel_processing
    }

    /// Returns aggregate decoded-media confirmation time.
    #[must_use]
    pub const fn confirm(self) -> Duration {
        self.confirm
    }

    /// Returns aggregate native composition, canonicalization, and sink-write time.
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
    graphics_backend: BrowserGraphicsBackend,
    launch_policy: BrowserLaunchPolicy,
    browser_limits: BrowserLimits,
    ffmpeg: Ffmpeg,
}

/// One owned Chromium lifetime that may execute several local partitions.
///
/// Worker capture still creates one of these per artifact. Local assembly may
/// retain it across a validated sequence to amortize process startup while
/// each unit keeps its own runtime disposal and private resource root.
pub(super) struct FrameCaptureSession {
    browser: BrowserSession,
    metrics: FrameCaptureMetrics,
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
            graphics_backend: BrowserGraphicsBackend::SwiftShader,
            launch_policy,
            browser_limits,
            ffmpeg,
        }
    }

    /// Selects the immutable graphics implementation for each browser session.
    ///
    /// This is an execution-host decision, never an automatic fallback.
    #[must_use]
    pub fn with_graphics_backend(mut self, graphics_backend: BrowserGraphicsBackend) -> Self {
        self.graphics_backend = graphics_backend;
        self
    }

    /// Returns the browser surface mechanism selected for this executor.
    #[must_use]
    pub const fn capture_mode(&self) -> BrowserCaptureMode {
        self.capture_mode
    }

    /// Returns the graphics implementation selected for this executor.
    #[must_use]
    pub const fn graphics_backend(&self) -> BrowserGraphicsBackend {
        self.graphics_backend
    }

    /// Captures one independently executable unit into a verified worker artifact.
    ///
    /// The artifact contains ordered PNG frames rather than an independently
    /// encoded video. A later assembler can therefore retain one continuous
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

    /// Captures several independent units through one Chromium lifetime.
    ///
    /// Every destination must be absent. This batch boundary is intended for a
    /// cache owner that already resolved hits and assigned private destinations
    /// to misses.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when inputs disagree, a destination exists, or
    /// any browser, compositor, or artifact operation fails.
    pub async fn capture_frame_artifacts(
        &self,
        units: &[&ExecutableUnit],
        capture_environment: CaptureEnvironmentId,
        artifacts: &[PathBuf],
        limits: FrameArtifactLimits,
    ) -> Result<Vec<FrameArtifact>, RenderError> {
        if units.is_empty() && artifacts.is_empty() {
            return Ok(Vec::new());
        }
        let (profile, output) = validate_capture_batch(units, artifacts)?;
        let mut pending = prepare_artifacts(units, artifacts, capture_environment, limits).await?;
        let mut session = self.start_session(profile, output).await?;
        let capture = self.capture_pending(&mut session, &mut pending).await;
        session.finish(capture, output).await?;
        finish_artifacts(pending).await
    }

    async fn capture_pending(
        &self,
        session: &mut FrameCaptureSession,
        artifacts: &mut [PendingArtifact<'_>],
    ) -> Result<(), RenderError> {
        for artifact in artifacts {
            self.capture_artifact_frames_in_session(
                session,
                artifact.unit,
                &mut artifact.writer,
                artifact.requests,
                artifact.output,
            )
            .await?;
        }
        Ok(())
    }

    async fn capture_artifact_frames(
        &self,
        unit: &ExecutableUnit,
        writer: &mut FrameArtifactWriter,
        requests: RequestSequence,
        output: &Path,
    ) -> Result<FrameCaptureMetrics, RenderError> {
        let mut session = self.start_session(unit.profile(), output).await?;
        let capture = self
            .capture_artifact_frames_in_session(&mut session, unit, writer, requests, output)
            .await;
        session.finish(capture, output).await
    }

    async fn capture_artifact_frames_in_session(
        &self,
        session: &mut FrameCaptureSession,
        unit: &ExecutableUnit,
        writer: &mut crate::frame_artifact::FrameArtifactWriter,
        requests: RequestSequence,
        output: &Path,
    ) -> Result<(), RenderError> {
        if !unit.visual_execution().uses_native_media() {
            let mut frames = FrameSink::Artifact(writer);
            return session.capture(unit, &mut frames, requests, output).await;
        }

        let job = if unit.visual_execution().backdrop_media().is_some() {
            let layout = session.preflight_backdrop(unit, output).await?;
            backdrop_job(
                std::slice::from_ref(unit),
                std::slice::from_ref(&layout),
                LayeredOutput::Frames,
                output,
            )?
        } else {
            layered_job(std::slice::from_ref(unit), LayeredOutput::Frames, output)?
        };
        let mut compositor = self
            .ffmpeg
            .start_layered(job)
            .map_err(|source| RenderError::encoder(output, source))?;
        let mut frames = FrameSink::LayeredArtifact {
            compositor: &mut compositor,
            artifact: writer,
        };
        match session.capture(unit, &mut frames, requests, output).await {
            Ok(()) => {}
            Err(capture) => {
                return Err(super::abort_compositor(compositor, capture, output).await);
            }
        }
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
        let started = Instant::now();
        write_canonical_artifact(writer, unit.profile(), final_frame, output).await?;
        session.metrics.write += started.elapsed();
        Ok(())
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

    pub(super) async fn start_session(
        &self,
        profile: crate::RenderProfile,
        output: &Path,
    ) -> Result<FrameCaptureSession, RenderError> {
        let started = Instant::now();
        let browser = BrowserSession::launch(
            &self.browser_executable,
            BrowserSessionOptions {
                launch_policy: self.launch_policy,
                graphics_backend: self.graphics_backend,
                capture_mode: self.capture_mode,
                render_profile: profile,
                limits: self.browser_limits,
            },
        )
        .await
        .map_err(|source| RenderError::browser(output, source))?;
        let metrics = FrameCaptureMetrics {
            launch: started.elapsed(),
            ..FrameCaptureMetrics::default()
        };

        Ok(FrameCaptureSession { browser, metrics })
    }
}

fn validate_capture_batch<'a>(
    units: &[&ExecutableUnit],
    artifacts: &'a [PathBuf],
) -> Result<(crate::RenderProfile, &'a Path), RenderError> {
    let output = artifacts
        .first()
        .map_or_else(|| Path::new("frame-artifacts"), PathBuf::as_path);
    let Some(first) = units.first() else {
        return Err(invalid_plan(
            output,
            "capture units do not match frame artifact destinations",
        ));
    };
    if units.len() != artifacts.len() {
        return Err(invalid_plan(
            output,
            "capture units do not match frame artifact destinations",
        ));
    }
    let profile = first.profile();
    if units.iter().any(|unit| unit.profile() != profile) {
        return Err(invalid_plan(
            output,
            "one browser capture batch requires one render profile",
        ));
    }
    Ok((profile, output))
}

async fn prepare_artifacts<'a>(
    units: &[&'a ExecutableUnit],
    artifacts: &'a [PathBuf],
    capture_environment: CaptureEnvironmentId,
    limits: FrameArtifactLimits,
) -> Result<Vec<PendingArtifact<'a>>, RenderError> {
    let mut pending = Vec::with_capacity(units.len());
    for (unit, output) in units.iter().zip(artifacts) {
        let requests = validate_plan(unit.browser_plan(), limits.max_frames(), output)?;
        let writer = FrameArtifact::writer_for_capture(unit, capture_environment, output, limits)
            .await
            .map_err(|source| RenderError::artifact(output, source))?;
        pending.push(PendingArtifact {
            unit,
            output,
            requests,
            writer,
        });
    }
    Ok(pending)
}

async fn finish_artifacts(
    artifacts: Vec<PendingArtifact<'_>>,
) -> Result<Vec<FrameArtifact>, RenderError> {
    let mut completed = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        completed.push(
            artifact
                .writer
                .finish()
                .await
                .map_err(|source| RenderError::artifact(artifact.output, source))?,
        );
    }
    Ok(completed)
}

impl FrameCaptureSession {
    pub(super) async fn capture(
        &mut self,
        unit: &ExecutableUnit,
        frames: &mut FrameSink<'_>,
        requests: RequestSequence,
        output: &Path,
    ) -> Result<(), RenderError> {
        let (surface, media_mode) = if unit.visual_execution().layered_media().is_some() {
            (CaptureSurface::Transparent, BrowserMediaMode::Omitted)
        } else if unit.visual_execution().backdrop_media().is_some() {
            (
                capture_surface(unit.profile().alpha()),
                BrowserMediaMode::Omitted,
            )
        } else {
            (
                capture_surface(unit.profile().alpha()),
                BrowserMediaMode::Decoded,
            )
        };
        render_session(
            &mut self.browser,
            frames,
            &mut self.metrics,
            CaptureTask {
                plan: unit.browser_plan(),
                requests,
                entry_url: unit.entry_url(),
                resource_root: unit.resource_root(),
                surface,
                media_mode,
                cadence: unit.visual_execution().capture_cadence(),
                output,
            },
        )
        .await
    }

    pub(super) async fn preflight_backdrop(
        &mut self,
        unit: &ExecutableUnit,
        output: &Path,
    ) -> Result<crate::visual::BackdropLayoutPlan, RenderError> {
        let evidence = preflight_media_layout(
            &mut self.browser,
            unit.browser_plan(),
            unit.entry_url(),
            unit.resource_root(),
            &mut self.metrics,
            output,
        )
        .await?;
        unit.visual_execution()
            .resolve_backdrop_layout(&evidence, unit.profile(), unit.browser_plan())
            .map_err(|source| {
                RenderError::new(
                    RenderErrorKind::InvalidPlan,
                    output,
                    format!("browser media layout violates its declared capability: {source}"),
                )
            })
    }

    pub(super) async fn finish(
        mut self,
        capture: Result<(), RenderError>,
        output: &Path,
    ) -> Result<FrameCaptureMetrics, RenderError> {
        let started = Instant::now();
        let shutdown = self
            .browser
            .shutdown()
            .await
            .map_err(|source| RenderError::browser(output, source));
        self.metrics.shutdown = started.elapsed();

        match (capture, shutdown) {
            (Ok(()), Ok(())) => Ok(self.metrics),
            (Err(render), Ok(())) => Err(render),
            (Ok(()), Err(shutdown)) => Err(shutdown),
            (Err(render), Err(shutdown)) => {
                Err(render.with_cleanup_failure("browser shutdown", shutdown))
            }
        }
    }

    pub(super) async fn fail(self, error: RenderError, output: &Path) -> RenderError {
        match self.finish(Err(error), output).await {
            Err(error) => error,
            Ok(_) => unreachable!("a failed capture session cannot finish successfully"),
        }
    }
}

const fn capture_surface(alpha: AlphaMode) -> CaptureSurface {
    match alpha {
        AlphaMode::Opaque => CaptureSurface::Opaque,
        AlphaMode::Preserve => CaptureSurface::Transparent,
    }
}
