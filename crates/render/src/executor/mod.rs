//! Native execution of one validated render unit.
//!
//! Rust owns request ordering, absolute frame identity, capture, and encoding;
//! the browser only applies the already-solved plan. Request IDs are allocated
//! once here so protocol sequencing cannot drift across execution paths.

mod capture;
mod error;
mod output;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use onmark_core::model::FrameIndex;
use onmark_core::protocol::{WireFrameRate, WireInterval};
use onmark_core::render_graph::PartitionPlan;

use self::capture::{
    CaptureSurface, CaptureTask, FrameSink, RequestSequence, render_session, validate_plan,
    write_canonical_artifact,
};
use self::output::StagedOutput;
use crate::encoder::{AudioInput, LayeredCompletion, LayeredJob, LayeredMediaInput, LayeredOutput};
use crate::unit::MAX_AUDIO_TRACKS;
use crate::{
    BrowserLaunchPolicy, BrowserLimits, BrowserSession, CaptureEnvironmentId, EncodedVideo,
    ExecutableUnit, Ffmpeg, FfmpegSession, FrameArtifact, FrameArtifactErrorKind,
    FrameArtifactLimits,
};

pub use error::{RenderError, RenderErrorKind};

/// Aggregate wall-time attribution for one browser capture session.
///
/// These measurements explain executor cost; frame identity and scheduling
/// remain derived exclusively from the render plan.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FrameCaptureMetrics {
    frames: u64,
    launch: Duration,
    runtime_setup: Duration,
    seek: Duration,
    readback: Duration,
    fingerprint: Duration,
    confirm: Duration,
    write: Duration,
    shutdown: Duration,
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
        browser_limits: BrowserLimits,
        ffmpeg: Ffmpeg,
    ) -> Self {
        Self {
            browser_executable: browser_executable.into(),
            launch_policy,
            browser_limits,
            ffmpeg,
        }
    }

    /// Captures one independently executable unit into a verified worker artifact.
    ///
    /// The artifact contains ordered PNG frames rather than an independently
    /// encoded MP4. A later assembler can therefore retain Gate two's one
    /// continuous visual encoder and one final audio mix across workers.
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
        let mut metrics = self
            .capture_unit(unit, &mut frames, requests, output)
            .await?;
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

    async fn capture_unit(
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
                entry_url: unit.entry_url().as_str(),
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

        render_result?;
        shutdown_result?;
        Ok(metrics)
    }
}

/// Local renderer composed from [`FrameCaptureExecutor`] and `FFmpeg`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderExecutor {
    capture: FrameCaptureExecutor,
    ffmpeg: Ffmpeg,
}

impl RenderExecutor {
    /// Creates the local composition root from explicit process boundaries.
    #[must_use]
    pub fn new(
        browser_executable: impl Into<PathBuf>,
        browser_limits: BrowserLimits,
        ffmpeg: Ffmpeg,
    ) -> Self {
        Self {
            capture: FrameCaptureExecutor::new(
                browser_executable,
                BrowserLaunchPolicy::local(),
                browser_limits,
                ffmpeg.clone(),
            ),
            ffmpeg,
        }
    }

    /// Renders one independently executable unit into an H.264 MP4 artifact.
    ///
    /// Frame capture and encoder input are sequential: at most one encoded PNG
    /// is owned between Chromium and `FFmpeg` at any time.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when the selected configuration or plan exceeds
    /// supported limits, the browser protocol deviates from its expected phase,
    /// or either process boundary fails. Chromium shutdown is still attempted
    /// after render work fails.
    pub async fn render(
        &self,
        unit: ExecutableUnit,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        let expected_output = unit.browser_plan().output();
        let output_origin = FrameIndex::new(expected_output.start().get());
        let audio = collect_audio_inputs(std::slice::from_ref(&unit), output_origin, output)?;
        self.render_sequence(vec![unit], expected_output, audio, output)
            .await
    }

    /// Renders contiguous independent units into one complete MP4 artifact.
    ///
    /// Every unit keeps its own verified browser root and browser session. The
    /// encoder instead receives their output frames in order as one continuous
    /// stream, then mixes all absolute Timeline audio placements once.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when units do not form one contiguous film, do
    /// not share a bundle, profile, and frame rate, or an execution boundary
    /// rejects the resulting render.
    pub async fn render_partitioned(
        &self,
        partitions: &PartitionPlan,
        units: Vec<ExecutableUnit>,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        Self::validate_partition_units(partitions, &units, output)?;
        let expected_output = wire_interval(partitions.interval(), output)?;
        let output_origin = partitions.interval().start();
        let audio = collect_audio_inputs(&units, output_origin, output)?;
        self.render_sequence(units, expected_output, audio, output)
            .await
    }

    /// Captures one independently executable unit into a verified worker artifact.
    ///
    /// The artifact contains ordered PNG frames rather than an independently
    /// encoded MP4. A later assembler can therefore retain Gate two's one
    /// continuous visual encoder and one final audio mix across workers.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when the unit, browser, or artifact boundary
    /// fails. A failed capture never publishes a partial artifact.
    pub async fn capture_frame_artifact(
        &self,
        unit: &ExecutableUnit,
        capture_environment: CaptureEnvironmentId,
        artifact: &Path,
        limits: FrameArtifactLimits,
    ) -> Result<FrameArtifact, RenderError> {
        self.capture
            .capture_frame_artifact(unit, capture_environment, artifact, limits)
            .await
    }

    /// Assembles independently captured worker artifacts into one MP4.
    ///
    /// The supplied units may be newly materialized on this assembler. They
    /// provide the expected unit identities and the verified local audio bytes;
    /// the browser never launches during assembly.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when artifacts do not match the partition plan
    /// and capture environment, fail verification while streaming, or final
    /// encoding and audio mixing fail.
    pub async fn assemble_frame_artifacts(
        &self,
        partitions: &PartitionPlan,
        units: &[ExecutableUnit],
        artifacts: &[FrameArtifact],
        capture_environment: CaptureEnvironmentId,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        Self::validate_partition_units(partitions, units, output)?;
        Self::validate_frame_artifacts(units, artifacts, capture_environment, output)?;
        let expected_output = wire_interval(partitions.interval(), output)?;
        let output_origin = partitions.interval().start();
        let audio = collect_audio_inputs(units, output_origin, output)?;
        let sequence = self.validate_sequence(units, expected_output, output)?;
        let frame_rate = sequence.frame_rate;

        let staging = StagedOutput::new(output)?;
        let mut encoder = self
            .ffmpeg
            .start(staging.visual_path(), frame_rate)
            .map_err(|source| RenderError::encoder(output, source))?;
        for artifact in artifacts {
            stream_artifact(artifact, &mut encoder, output).await?;
        }

        self.finish_sequence(encoder, staging, audio, frame_rate, output)
            .await
    }

    async fn render_sequence(
        &self,
        units: Vec<ExecutableUnit>,
        expected_output: WireInterval,
        audio: Vec<AudioInput>,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        let ValidatedSequence {
            frame_rate,
            requests,
        } = self.validate_sequence(&units, expected_output, output)?;
        if units[0].visual_execution().layered_media().is_some() {
            return self
                .render_layered_sequence(&units, requests, audio, frame_rate, output)
                .await;
        }
        self.render_browser_sequence(&units, requests, audio, frame_rate, output)
            .await
    }

    async fn render_browser_sequence(
        &self,
        units: &[ExecutableUnit],
        requests: Vec<RequestSequence>,
        audio: Vec<AudioInput>,
        frame_rate: WireFrameRate,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        let staging = StagedOutput::new(output)?;
        let mut encoder = self
            .ffmpeg
            .start(staging.visual_path(), frame_rate)
            .map_err(|source| RenderError::encoder(output, source))?;
        for (unit, requests) in units.iter().zip(requests) {
            let mut frames = FrameSink::Encoder(&mut encoder);
            self.capture
                .capture_unit(unit, &mut frames, requests, output)
                .await?;
        }
        self.finish_sequence(encoder, staging, audio, frame_rate, output)
            .await
    }

    async fn render_layered_sequence(
        &self,
        units: &[ExecutableUnit],
        requests: Vec<RequestSequence>,
        audio: Vec<AudioInput>,
        frame_rate: WireFrameRate,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        let staging = StagedOutput::new(output)?;
        let job = layered_job(
            units,
            LayeredOutput::Video(staging.visual_path().to_owned()),
            output,
        )?;
        let mut compositor = self
            .ffmpeg
            .start_layered(job)
            .map_err(|source| RenderError::encoder(output, source))?;
        let mut frames = FrameSink::LayeredVideo(&mut compositor);
        for (unit, requests) in units.iter().zip(requests) {
            self.capture
                .capture_unit(unit, &mut frames, requests, output)
                .await?;
        }
        let completion = compositor
            .finish()
            .await
            .map_err(|source| RenderError::encoder(output, source))?;
        let LayeredCompletion::Video(visual) = completion else {
            return Err(invalid_plan(
                output,
                "layered local composition did not produce encoded video",
            ));
        };
        let video = self
            .ffmpeg
            .mix_audio(visual, audio, frame_rate, staging.mixed_path())
            .await
            .map_err(|source| RenderError::encoder(output, source))?;
        staging.publish(video, output)
    }

    async fn finish_sequence(
        &self,
        encoder: FfmpegSession,
        staging: StagedOutput,
        audio: Vec<AudioInput>,
        frame_rate: WireFrameRate,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        let visual = encoder
            .finish()
            .await
            .map_err(|source| RenderError::encoder(output, source))?;
        let video = self
            .ffmpeg
            .mix_audio(visual, audio, frame_rate, staging.mixed_path())
            .await
            .map_err(|source| RenderError::encoder(output, source))?;
        staging.publish(video, output)
    }

    fn validate_partition_units(
        partitions: &PartitionPlan,
        units: &[ExecutableUnit],
        output: &Path,
    ) -> Result<(), RenderError> {
        if partitions.units().len() != units.len() {
            return Err(invalid_plan(
                output,
                "render units do not match the partition plan",
            ));
        }

        for (partition, unit) in partitions.units().iter().zip(units) {
            let evaluation = wire_interval(partition.evaluation(), output)?;
            let published = wire_interval(partition.output(), output)?;
            let plan = unit.browser_plan();
            if plan.evaluation() != evaluation || plan.output() != published {
                return Err(invalid_plan(
                    output,
                    "render units do not match the partition plan",
                ));
            }
        }

        Ok(())
    }

    fn validate_frame_artifacts(
        units: &[ExecutableUnit],
        artifacts: &[FrameArtifact],
        capture_environment: CaptureEnvironmentId,
        output: &Path,
    ) -> Result<(), RenderError> {
        if units.len() != artifacts.len() {
            return Err(invalid_plan(
                output,
                "worker frame artifacts do not match the partition plan",
            ));
        }

        for (unit, artifact) in units.iter().zip(artifacts) {
            if !artifact.matches_capture(unit, capture_environment) {
                return Err(RenderError::artifact(
                    output,
                    FrameArtifact::identity_mismatch(artifact.path()),
                ));
            }
        }

        Ok(())
    }

    fn validate_sequence(
        &self,
        units: &[ExecutableUnit],
        expected_output: WireInterval,
        output: &Path,
    ) -> Result<ValidatedSequence, RenderError> {
        let Some(first) = units.first() else {
            return Err(invalid_plan(output, "render sequence contains no units"));
        };
        let frame_rate = first.browser_plan().frame_rate();
        let mut expected_start = expected_output.start().get();
        let mut total_frames = 0_u64;
        let mut requests = Vec::with_capacity(units.len());

        for unit in units {
            let plan = unit.browser_plan();
            validate_unit_identity(first, unit, output)?;
            if plan.output().start().get() != expected_start {
                return Err(invalid_plan(
                    output,
                    "render unit outputs must begin at the planned output start and remain contiguous",
                ));
            }

            let unit_requests = validate_plan(plan, self.ffmpeg.max_frames(), output)?;
            total_frames = extend_frame_budget(
                total_frames,
                unit_requests.frame_count(),
                self.ffmpeg.max_frames(),
                output,
            )?;
            expected_start = plan.output().end().get();
            requests.push(unit_requests);
        }

        if expected_start != expected_output.end().get() {
            return Err(invalid_plan(
                output,
                "render unit outputs do not cover the partition plan",
            ));
        }

        Ok(ValidatedSequence {
            frame_rate,
            requests,
        })
    }
}

fn collect_audio_inputs(
    units: &[ExecutableUnit],
    origin: FrameIndex,
    output: &Path,
) -> Result<Vec<AudioInput>, RenderError> {
    let mut audio = Vec::new();
    for unit in units {
        for input in unit.audio_inputs_rebased_to(origin) {
            if audio.len() == MAX_AUDIO_TRACKS {
                return Err(RenderError::new(
                    RenderErrorKind::PlanTooLarge,
                    output,
                    "render sequence exceeds the configured audio-track limit",
                ));
            }
            audio.push(input);
        }
    }
    audio.sort_by_key(AudioInput::mix_order);
    if audio
        .windows(2)
        .any(|pair| pair[0].mix_order() == pair[1].mix_order())
    {
        return Err(invalid_plan(
            output,
            "render sequence contains duplicate canonical audio positions",
        ));
    }
    Ok(audio)
}

/// Execution facts whose frame count is already representable by request IDs.
struct ValidatedSequence {
    frame_rate: WireFrameRate,
    requests: Vec<RequestSequence>,
}

fn validate_unit_identity(
    expected: &ExecutableUnit,
    actual: &ExecutableUnit,
    output: &Path,
) -> Result<(), RenderError> {
    if actual.bundle_id() != expected.bundle_id() {
        return Err(invalid_plan(
            output,
            "render units do not share one presentation bundle",
        ));
    }
    if actual.profile() != expected.profile() {
        return Err(invalid_plan(
            output,
            "render units do not share one render profile",
        ));
    }
    if actual.browser_plan().frame_rate() != expected.browser_plan().frame_rate() {
        return Err(invalid_plan(
            output,
            "render units do not share one frame rate",
        ));
    }
    if actual.visual_execution().capability() != expected.visual_execution().capability() {
        return Err(invalid_plan(
            output,
            "render units do not share one visual execution path",
        ));
    }
    Ok(())
}

fn layered_job(
    units: &[ExecutableUnit],
    destination: LayeredOutput,
    diagnostic_path: &Path,
) -> Result<LayeredJob, RenderError> {
    let Some(first) = units.first() else {
        return Err(invalid_plan(
            diagnostic_path,
            "layered render sequence contains no units",
        ));
    };
    let media = units
        .iter()
        .map(|unit| layered_media_input(unit, diagnostic_path))
        .collect::<Result<Vec<_>, _>>()?;
    let frames = media
        .iter()
        .try_fold(0_u64, |total, media| total.checked_add(media.frames));
    let Some(frames) = frames else {
        return Err(sequence_too_large(diagnostic_path));
    };
    Ok(LayeredJob {
        media,
        output_frame_rate: first.browser_plan().frame_rate(),
        frames,
        profile: first.profile(),
        destination,
        diagnostic_path: diagnostic_path.to_owned(),
    })
}

fn layered_media_input(
    unit: &ExecutableUnit,
    output: &Path,
) -> Result<LayeredMediaInput, RenderError> {
    let path = unit
        .layered_media_path()
        .ok_or_else(|| invalid_plan(output, "render unit has no layered media"))?;
    let [video] = unit.browser_plan().videos() else {
        return Err(invalid_plan(
            output,
            "layered render unit does not contain one primary video",
        ));
    };
    let frames = unit
        .browser_plan()
        .output()
        .end()
        .get()
        .checked_sub(unit.browser_plan().output().start().get())
        .ok_or_else(|| invalid_plan(output, "layered render unit has a reversed output"))?;
    Ok(LayeredMediaInput {
        path,
        source_frame_rate: video.source_frame_rate(),
        frames,
    })
}

fn extend_frame_budget(
    current: u64,
    additional: u64,
    limit: u64,
    output: &Path,
) -> Result<u64, RenderError> {
    let total = current
        .checked_add(additional)
        .ok_or_else(|| sequence_too_large(output))?;
    if total > limit {
        return Err(sequence_too_large(output));
    }
    Ok(total)
}

fn sequence_too_large(output: &Path) -> RenderError {
    RenderError::new(
        RenderErrorKind::PlanTooLarge,
        output,
        "render sequence exceeds the configured frame limit",
    )
}

fn wire_interval(
    interval: onmark_core::model::FrameInterval,
    output: &Path,
) -> Result<WireInterval, RenderError> {
    WireInterval::try_from(interval).map_err(|_| {
        invalid_plan(
            output,
            "partition interval exceeds the browser frame domain",
        )
    })
}

async fn stream_artifact(
    artifact: &FrameArtifact,
    encoder: &mut FfmpegSession,
    output: &Path,
) -> Result<(), RenderError> {
    let mut frames = artifact
        .reader()
        .await
        .map_err(|source| RenderError::artifact(output, source))?;
    while let Some(frame) = frames
        .next_frame()
        .await
        .map_err(|source| RenderError::artifact(output, source))?
    {
        encoder
            .write_frame(frame.png())
            .await
            .map_err(|source| RenderError::encoder(output, source))?;
    }
    Ok(())
}

fn invalid_plan(output: &Path, message: &'static str) -> RenderError {
    RenderError::new(RenderErrorKind::InvalidPlan, output, message)
}
