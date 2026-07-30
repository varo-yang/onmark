//! Native execution of one validated render unit.
//!
//! Rust owns request ordering, absolute frame identity, capture, and encoding;
//! the browser only applies the already-solved plan. Request IDs are allocated
//! once here so protocol sequencing cannot drift across execution paths.

mod capture;
mod error;
mod frame_capture;
mod output;

use std::path::{Path, PathBuf};

use onmark_core::model::{FrameIndex, PresentationVisualCapability};
use onmark_core::protocol::{WireFrameRate, WireInterval};
use onmark_core::render_graph::PartitionPlan;

use self::capture::{FrameSink, RequestSequence, validate_plan};
use self::output::StagedOutput;
use crate::encoder::{
    AudioInput, BackdropMediaInput, LayeredCompletion, LayeredInputs, LayeredJob,
    LayeredMediaInput, LayeredOutput, LayeredSession,
};
use crate::unit::MAX_AUDIO_TRACKS;
use crate::visual::{BackdropLayoutPlan, native_media_schedule};
use crate::{
    BrowserCaptureMode, BrowserGraphicsBackend, BrowserLaunchPolicy, BrowserLimits,
    CaptureEnvironmentId, EncodedVideo, ExecutableUnit, Ffmpeg, FfmpegSession, FrameArtifact,
    FrameArtifactLimits,
};

pub use error::{RenderError, RenderErrorKind};
pub use frame_capture::{FrameCaptureExecutor, FrameCaptureMetrics, FrameCaptureReport};

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
        capture_mode: BrowserCaptureMode,
        browser_limits: BrowserLimits,
        ffmpeg: Ffmpeg,
    ) -> Self {
        Self {
            capture: FrameCaptureExecutor::new(
                browser_executable.into(),
                BrowserLaunchPolicy::local(),
                capture_mode,
                browser_limits,
                ffmpeg.clone(),
            ),
            ffmpeg,
        }
    }

    /// Selects the immutable graphics implementation for browser capture.
    ///
    /// The default remains [`BrowserGraphicsBackend::SwiftShader`].
    #[must_use]
    pub fn with_graphics_backend(mut self, graphics_backend: BrowserGraphicsBackend) -> Self {
        self.capture = self.capture.with_graphics_backend(graphics_backend);
        self
    }

    /// Returns the browser surface mechanism selected for this executor.
    #[must_use]
    pub const fn capture_mode(&self) -> BrowserCaptureMode {
        self.capture.capture_mode()
    }

    /// Returns the graphics implementation selected for browser capture.
    #[must_use]
    pub const fn graphics_backend(&self) -> BrowserGraphicsBackend {
        self.capture.graphics_backend()
    }

    /// Renders one independently executable unit into the selected video profile.
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

    /// Renders contiguous independent units into one complete video artifact.
    ///
    /// Every unit keeps its own verified browser root and runtime lifecycle.
    /// Local execution reuses one browser process for the validated sequence;
    /// the encoder receives output frames in order as one continuous stream,
    /// then mixes all absolute Timeline audio placements once.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when units do not form one contiguous film, do
    /// not share a profile, frame rate, and visual path, or an execution boundary
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
    /// encoded video. A later assembler can therefore retain one
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

    /// Captures one worker artifact together with bounded phase timings.
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
        self.capture
            .capture_frame_artifact_report(unit, capture_environment, artifact, limits)
            .await
    }

    /// Captures cache misses through one shared Chromium lifetime.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] under the same conditions as
    /// [`Self::capture_frame_artifact`], or when units and destinations differ.
    pub async fn capture_frame_artifacts(
        &self,
        units: &[&ExecutableUnit],
        capture_environment: CaptureEnvironmentId,
        artifacts: &[PathBuf],
        limits: FrameArtifactLimits,
    ) -> Result<Vec<FrameArtifact>, RenderError> {
        self.capture
            .capture_frame_artifacts(units, capture_environment, artifacts, limits)
            .await
    }

    /// Assembles independently captured worker artifacts into one video.
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

        let staging = StagedOutput::new(output, self.ffmpeg.profile())?;
        let mut encoder = self
            .ffmpeg
            .start(staging.visual_path(), frame_rate)
            .map_err(|source| RenderError::encoder(output, source))?;
        for artifact in artifacts {
            if let Err(stream) = stream_artifact(artifact, &mut encoder, output).await {
                return Err(abort_encoder(encoder, stream, output).await);
            }
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
            visual_path,
            requests,
        } = self.validate_sequence(&units, expected_output, output)?;
        match visual_path {
            SequenceVisualPath::Browser => {
                self.render_browser_sequence(&units, requests, audio, frame_rate, output)
                    .await
            }
            SequenceVisualPath::Backdrop => {
                self.render_backdrop_sequence(&units, requests, audio, frame_rate, output)
                    .await
            }
            SequenceVisualPath::Layered => {
                self.render_layered_sequence(&units, requests, audio, frame_rate, output)
                    .await
            }
        }
    }

    async fn render_browser_sequence(
        &self,
        units: &[ExecutableUnit],
        requests: Vec<RequestSequence>,
        audio: Vec<AudioInput>,
        frame_rate: WireFrameRate,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        let staging = StagedOutput::new(output, self.ffmpeg.profile())?;
        let mut encoder = self
            .ffmpeg
            .start(staging.visual_path(), frame_rate)
            .map_err(|source| RenderError::encoder(output, source))?;
        let capture = self
            .capture_units(
                units,
                requests,
                CaptureDestination::Encoder(&mut encoder),
                output,
            )
            .await;
        if let Err(capture) = capture {
            return Err(abort_encoder(encoder, capture, output).await);
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
        let staging = StagedOutput::new(output, self.ffmpeg.profile())?;
        let job = layered_job(
            units,
            LayeredOutput::Video(staging.visual_path().to_owned()),
            output,
        )?;
        let mut compositor = self
            .ffmpeg
            .start_layered(job)
            .map_err(|source| RenderError::encoder(output, source))?;
        let capture = self
            .capture_units(
                units,
                requests,
                CaptureDestination::Layered(&mut compositor),
                output,
            )
            .await;
        if let Err(capture) = capture {
            return Err(abort_compositor(compositor, capture, output).await);
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

    async fn render_backdrop_sequence(
        &self,
        units: &[ExecutableUnit],
        requests: Vec<RequestSequence>,
        audio: Vec<AudioInput>,
        frame_rate: WireFrameRate,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        let Some(first) = units.first() else {
            return Err(invalid_plan(
                output,
                "backdrop render sequence contains no units",
            ));
        };
        let staging = StagedOutput::new(output, self.ffmpeg.profile())?;
        let mut session = self.capture.start_session(first.profile(), output).await?;
        let layouts = match preflight_backdrops(&mut session, units, output).await {
            Ok(layouts) => layouts,
            Err(error) => return Err(session.fail(error, output).await),
        };
        let job = match backdrop_job(
            units,
            &layouts,
            LayeredOutput::Video(staging.visual_path().to_owned()),
            output,
        ) {
            Ok(job) => job,
            Err(error) => return Err(session.fail(error, output).await),
        };
        let mut compositor = match self.ffmpeg.start_layered(job) {
            Ok(compositor) => compositor,
            Err(source) => {
                let error = RenderError::encoder(output, source);
                return Err(session.fail(error, output).await);
            }
        };
        let capture = capture_units_in_session(
            &mut session,
            units,
            requests,
            CaptureDestination::Layered(&mut compositor),
            output,
        )
        .await;
        if let Err(capture) = capture {
            let capture = abort_compositor(compositor, capture, output).await;
            return Err(session.fail(capture, output).await);
        }
        let completion = match compositor.finish().await {
            Ok(completion) => completion,
            Err(source) => {
                let error = RenderError::encoder(output, source);
                return Err(session.fail(error, output).await);
            }
        };
        session.finish(Ok(()), output).await?;
        let LayeredCompletion::Video(visual) = completion else {
            return Err(invalid_plan(
                output,
                "backdrop local composition did not produce encoded video",
            ));
        };
        let video = self
            .ffmpeg
            .mix_audio(visual, audio, frame_rate, staging.mixed_path())
            .await
            .map_err(|source| RenderError::encoder(output, source))?;
        staging.publish(video, output)
    }

    async fn capture_units(
        &self,
        units: &[ExecutableUnit],
        requests: Vec<RequestSequence>,
        destination: CaptureDestination<'_>,
        output: &Path,
    ) -> Result<(), RenderError> {
        let Some(first) = units.first() else {
            return Err(invalid_plan(
                output,
                "browser render sequence contains no units",
            ));
        };
        let mut session = self.capture.start_session(first.profile(), output).await?;
        let capture =
            capture_units_in_session(&mut session, units, requests, destination, output).await;

        session.finish(capture, output).await.map(|_| ())
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
        let visual_path = if units
            .iter()
            .any(|unit| unit.visual_execution().layered_media().is_some())
        {
            SequenceVisualPath::Layered
        } else if units
            .iter()
            .any(|unit| unit.visual_execution().backdrop_media().is_some())
        {
            SequenceVisualPath::Backdrop
        } else {
            SequenceVisualPath::Browser
        };
        let mut expected_start = expected_output.start().get();
        let mut total_frames = 0_u64;
        let mut requests = Vec::with_capacity(units.len());

        for unit in units {
            let plan = unit.browser_plan();
            validate_unit_compatibility(first, unit, output)?;
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
            visual_path,
            requests,
        })
    }
}

async fn abort_encoder(encoder: FfmpegSession, failure: RenderError, output: &Path) -> RenderError {
    match encoder.abort().await {
        Ok(()) => failure,
        Err(source) => {
            failure.with_cleanup_failure("FFmpeg abort", RenderError::encoder(output, source))
        }
    }
}

async fn abort_compositor(
    compositor: LayeredSession,
    failure: RenderError,
    output: &Path,
) -> RenderError {
    match compositor.abort().await {
        Ok(()) => failure,
        Err(source) => failure
            .with_cleanup_failure("layered FFmpeg abort", RenderError::encoder(output, source)),
    }
}

async fn preflight_backdrops(
    session: &mut frame_capture::FrameCaptureSession,
    units: &[ExecutableUnit],
    output: &Path,
) -> Result<Vec<BackdropLayoutPlan>, RenderError> {
    let mut layouts = Vec::with_capacity(units.len());
    for unit in units {
        let layout = if unit.visual_execution().backdrop_media().is_some() {
            session.preflight_backdrop(unit, output).await?
        } else {
            BackdropLayoutPlan::empty()
        };
        layouts.push(layout);
    }
    Ok(layouts)
}

async fn capture_units_in_session(
    session: &mut frame_capture::FrameCaptureSession,
    units: &[ExecutableUnit],
    requests: Vec<RequestSequence>,
    mut destination: CaptureDestination<'_>,
    output: &Path,
) -> Result<(), RenderError> {
    for (unit, requests) in units.iter().zip(requests) {
        let mut frames = destination.frames();
        session.capture(unit, &mut frames, requests, output).await?;
    }
    Ok(())
}

fn collect_audio_inputs(
    units: &[ExecutableUnit],
    origin: FrameIndex,
    output: &Path,
) -> Result<Vec<AudioInput>, RenderError> {
    let mut audio = Vec::new();
    for input in units
        .iter()
        .flat_map(|unit| unit.audio_inputs_rebased_to(origin))
    {
        if audio.len() == MAX_AUDIO_TRACKS {
            return Err(RenderError::new(
                RenderErrorKind::PlanTooLarge,
                output,
                "render sequence exceeds the configured audio-track limit",
            ));
        }
        audio.push(input);
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
    visual_path: SequenceVisualPath,
    requests: Vec<RequestSequence>,
}

enum SequenceVisualPath {
    Browser,
    Backdrop,
    Layered,
}

enum CaptureDestination<'a> {
    Encoder(&'a mut FfmpegSession),
    Layered(&'a mut LayeredSession),
}

impl CaptureDestination<'_> {
    fn frames(&mut self) -> FrameSink<'_> {
        match self {
            Self::Encoder(encoder) => FrameSink::Encoder(encoder),
            Self::Layered(compositor) => FrameSink::LayeredVideo(compositor),
        }
    }
}

fn validate_unit_compatibility(
    expected: &ExecutableUnit,
    actual: &ExecutableUnit,
    output: &Path,
) -> Result<(), RenderError> {
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
        inputs: LayeredInputs::VideoBase(media),
        output_frame_rate: first.browser_plan().frame_rate(),
        frames,
        profile: first.profile(),
        destination,
        diagnostic_path: diagnostic_path.to_owned(),
    })
}

fn backdrop_job(
    units: &[ExecutableUnit],
    layouts: &[BackdropLayoutPlan],
    destination: LayeredOutput,
    diagnostic_path: &Path,
) -> Result<LayeredJob, RenderError> {
    let Some(first) = units.first() else {
        return Err(invalid_plan(
            diagnostic_path,
            "backdrop render sequence contains no units",
        ));
    };
    if units.len() != layouts.len() {
        return Err(invalid_plan(
            diagnostic_path,
            "backdrop layouts do not match render units",
        ));
    }
    let origin = first.browser_plan().output().start().get();
    let end = units
        .last()
        .expect("the nonempty sequence has a final unit")
        .browser_plan()
        .output()
        .end()
        .get();
    let frames = end
        .checked_sub(origin)
        .ok_or_else(|| invalid_plan(diagnostic_path, "backdrop render sequence is reversed"))?;
    let mut media = Vec::new();
    for (unit, layout) in units.iter().zip(layouts) {
        append_backdrop_inputs(&mut media, unit, layout, origin, diagnostic_path)?;
    }
    if media.is_empty() {
        return Err(invalid_plan(
            diagnostic_path,
            "backdrop render sequence contains no published media",
        ));
    }
    Ok(LayeredJob {
        inputs: LayeredInputs::BrowserBase(media),
        output_frame_rate: first.browser_plan().frame_rate(),
        frames,
        profile: first.profile(),
        destination,
        diagnostic_path: diagnostic_path.to_owned(),
    })
}

fn append_backdrop_inputs(
    inputs: &mut Vec<BackdropMediaInput>,
    unit: &ExecutableUnit,
    layout: &BackdropLayoutPlan,
    origin: u64,
    output: &Path,
) -> Result<(), RenderError> {
    let Some(media_plan) = unit.visual_execution().backdrop_media() else {
        if layout.placements().is_empty()
            && unit.visual_execution().capability()
                == PresentationVisualCapability::SeparableBackdrop
        {
            return Ok(());
        }
        return Err(invalid_plan(
            output,
            "render unit has no compatible backdrop media",
        ));
    };
    let plan = unit.browser_plan();
    if media_plan.media().len() != plan.videos().len()
        || layout.placements().len() != plan.videos().len()
    {
        return Err(invalid_plan(
            output,
            "backdrop media, layout, and browser placements disagree",
        ));
    }

    for ((media, video), geometry) in media_plan
        .media()
        .iter()
        .zip(plan.videos())
        .zip(layout.placements())
    {
        let start = video
            .interval()
            .start()
            .get()
            .max(plan.output().start().get());
        let end = video.interval().end().get().min(plan.output().end().get());
        if start >= end {
            continue;
        }
        if media.node_id() != video.node().id() || geometry.node_id() != video.node().id() {
            return Err(invalid_plan(
                output,
                "backdrop media identities changed after preflight",
            ));
        }
        let schedule = native_media_schedule(plan, video).map_err(|_| {
            invalid_plan(
                output,
                "backdrop source treatment changed after visual admission",
            )
        })?;
        inputs.push(BackdropMediaInput {
            path: unit.visual_asset_path(media.asset_identity()),
            source_frame_rate: video.source_timing().constant_frame_rate().ok_or_else(|| {
                invalid_plan(
                    output,
                    "backdrop render unit contains variable source timing",
                )
            })?,
            source: video.source().media_source(),
            schedule,
            source_skip: start - video.interval().start().get(),
            output_start: start - origin,
            frames: end - start,
            source_region: geometry.source(),
            destination_region: geometry.destination(),
        });
    }
    Ok(())
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
    let published = unit.browser_plan().output();
    if !video.interval().contains_interval(published) {
        return Err(invalid_plan(
            output,
            "layered media does not cover the published interval",
        ));
    }
    let source_skip = published.start().get() - video.interval().start().get();
    let frames = published.end().get() - published.start().get();
    let schedule = native_media_schedule(unit.browser_plan(), video).map_err(|_| {
        invalid_plan(
            output,
            "layered source treatment changed after visual admission",
        )
    })?;
    Ok(LayeredMediaInput {
        path,
        source_frame_rate: video.source_timing().constant_frame_rate().ok_or_else(|| {
            invalid_plan(
                output,
                "layered render unit contains variable source timing",
            )
        })?,
        source: video.source().media_source(),
        schedule,
        source_skip,
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
