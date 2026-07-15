mod error;
mod output;

use std::path::{Path, PathBuf};

use onmark_core::model::FrameIndex;
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, BrowserResponse, RequestId,
    WireFrame, WireFrameRate, WireInterval,
};
use onmark_core::render_graph::PartitionPlan;

use self::output::StagedOutput;
use crate::encoder::AudioInput;
use crate::frame_artifact::FrameArtifactWriter;
use crate::unit::MAX_AUDIO_TRACKS;
use crate::{
    BrowserLimits, BrowserSession, CaptureEnvironmentId, EncodedVideo, ExecutableUnit, Ffmpeg,
    FfmpegSession, FrameArtifact, FrameArtifactErrorKind, FrameArtifactLimits,
};

pub use error::{RenderError, RenderErrorKind};

const LOAD_REQUEST: RequestId = RequestId::new(1);
const PREPARE_REQUEST: RequestId = RequestId::new(2);
const FIRST_FRAME_REQUEST: u32 = 3;

/// Bounded Chromium capture boundary shared by local and worker execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FrameCaptureExecutor {
    browser_executable: PathBuf,
    browser_limits: BrowserLimits,
}

impl FrameCaptureExecutor {
    /// Creates one browser-only capture boundary.
    #[must_use]
    pub fn new(browser_executable: impl Into<PathBuf>, browser_limits: BrowserLimits) -> Self {
        Self {
            browser_executable: browser_executable.into(),
            browser_limits,
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
        let frame_count = validate_plan(unit.browser_plan(), limits.max_frames(), artifact)?;
        let mut writer =
            match FrameArtifact::writer_for_capture(unit, capture_environment, artifact, limits)
                .await
            {
                Ok(writer) => writer,
                Err(error) if error.kind() == FrameArtifactErrorKind::OutputExists => {
                    return self
                        .reuse_artifact(unit, capture_environment, artifact, limits)
                        .await;
                }
                Err(error) => return Err(RenderError::artifact(artifact, error)),
            };
        let mut frames = FrameSink::Artifact(&mut writer);

        self.capture_unit(unit, &mut frames, frame_count, artifact)
            .await?;
        match writer.finish().await {
            Ok(artifact) => Ok(artifact),
            Err(error) if error.kind() == FrameArtifactErrorKind::OutputExists => {
                self.reuse_artifact(unit, capture_environment, artifact, limits)
                    .await
            }
            Err(error) => Err(RenderError::artifact(artifact, error)),
        }
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
        frame_count: u64,
        output: &Path,
    ) -> Result<(), RenderError> {
        let plan = unit.browser_plan();
        let disposal_request = disposal_request_id(frame_count, output)?;
        let browser = BrowserSession::launch(
            &self.browser_executable,
            unit.profile(),
            self.browser_limits,
        )
        .await
        .map_err(|source| RenderError::browser(output, source))?;

        let render_result = render_session(
            &browser,
            plan,
            disposal_request,
            unit.entry_url().as_str(),
            frames,
            output,
        )
        .await;
        let shutdown_result = browser
            .shutdown()
            .await
            .map_err(|source| RenderError::browser(output, source));

        render_result?;
        shutdown_result
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
            capture: FrameCaptureExecutor::new(browser_executable, browser_limits),
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
        let audio = unit.audio_inputs_rebased_to(output_origin).collect();
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
        let audio: Vec<AudioInput> = units
            .iter()
            .flat_map(|unit| unit.audio_inputs_rebased_to(output_origin))
            .collect();
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
        let audio: Vec<AudioInput> = units
            .iter()
            .flat_map(|unit| unit.audio_inputs_rebased_to(output_origin))
            .collect();
        let frame_rate = self.validate_sequence(units, expected_output, &audio, output)?;

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
        let frame_rate = self.validate_sequence(&units, expected_output, &audio, output)?;
        let staging = StagedOutput::new(output)?;
        let mut encoder = self
            .ffmpeg
            .start(staging.visual_path(), frame_rate)
            .map_err(|source| RenderError::encoder(output, source))?;

        for unit in &units {
            let mut frames = FrameSink::Encoder(&mut encoder);
            let frame_count = output_frame_count(unit.browser_plan())
                .expect("validated render units retain ordered output intervals");
            self.capture
                .capture_unit(unit, &mut frames, frame_count, output)
                .await?;
        }

        self.finish_sequence(encoder, staging, audio, frame_rate, output)
            .await
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
        audio: &[AudioInput],
        output: &Path,
    ) -> Result<WireFrameRate, RenderError> {
        let Some(first) = units.first() else {
            return Err(invalid_plan(output, "render sequence contains no units"));
        };
        let frame_rate = first.browser_plan().frame_rate();
        let profile = first.profile();
        let bundle_id = first.bundle_id();
        let mut expected_start = expected_output.start().get();
        let mut total_frames = 0_u64;

        for unit in units {
            let plan = unit.browser_plan();
            if unit.bundle_id() != bundle_id {
                return Err(invalid_plan(
                    output,
                    "render units do not share one presentation bundle",
                ));
            }
            if unit.profile() != profile {
                return Err(invalid_plan(
                    output,
                    "render units do not share one render profile",
                ));
            }
            if plan.frame_rate() != frame_rate {
                return Err(invalid_plan(
                    output,
                    "render units do not share one frame rate",
                ));
            }
            if plan.output().start().get() != expected_start {
                return Err(invalid_plan(
                    output,
                    "render unit outputs must begin at the planned output start and remain contiguous",
                ));
            }

            let frame_count = validate_plan(plan, self.ffmpeg.max_frames(), output)?;
            total_frames = total_frames.checked_add(frame_count).ok_or_else(|| {
                RenderError::new(
                    RenderErrorKind::PlanTooLarge,
                    output,
                    "render sequence exceeds the configured frame limit",
                )
            })?;
            if total_frames > self.ffmpeg.max_frames() {
                return Err(RenderError::new(
                    RenderErrorKind::PlanTooLarge,
                    output,
                    "render sequence exceeds the configured frame limit",
                ));
            }
            expected_start = plan.output().end().get();
        }

        if expected_start != expected_output.end().get() {
            return Err(invalid_plan(
                output,
                "render unit outputs do not cover the partition plan",
            ));
        }

        if audio.len() > MAX_AUDIO_TRACKS {
            return Err(RenderError::new(
                RenderErrorKind::PlanTooLarge,
                output,
                "render sequence exceeds the configured audio-track limit",
            ));
        }

        Ok(frame_rate)
    }
}

fn validate_plan(
    plan: &BrowserPlan,
    configured_max_frames: u64,
    output: &Path,
) -> Result<u64, RenderError> {
    let Some(frame_count) = output_frame_count(plan) else {
        return Err(invalid_plan(output, "browser output interval is reversed"));
    };
    let request_limit = u64::from(u32::MAX - FIRST_FRAME_REQUEST);
    let max_frames = configured_max_frames.min(request_limit);

    if frame_count == 0 {
        return Err(invalid_plan(output, "browser output interval is empty"));
    }
    if frame_count > max_frames {
        return Err(RenderError::new(
            RenderErrorKind::PlanTooLarge,
            output,
            "browser output interval exceeds the configured frame limit",
        ));
    }
    Ok(frame_count)
}

async fn render_session(
    browser: &BrowserSession,
    plan: &BrowserPlan,
    disposal_request: RequestId,
    entry_url: &str,
    frames: &mut FrameSink<'_>,
    output: &Path,
) -> Result<(), RenderError> {
    browser
        .navigate(entry_url)
        .await
        .map_err(|source| RenderError::browser(output, source))?;
    load_runtime(browser, plan, output).await?;
    prepare_runtime(browser, plan, output).await?;

    render_frames(browser, frames, plan, output).await?;
    dispose_runtime(browser, disposal_request, output).await
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

async fn load_runtime(
    browser: &BrowserSession,
    plan: &BrowserPlan,
    output: &Path,
) -> Result<(), RenderError> {
    let request = BrowserRequest::new(LOAD_REQUEST, BrowserCommand::Load { plan: plan.clone() });
    dispatch_expected(browser, request, BrowserEvent::Loaded, output).await
}

async fn prepare_runtime(
    browser: &BrowserSession,
    plan: &BrowserPlan,
    output: &Path,
) -> Result<(), RenderError> {
    let evaluation_start = plan.evaluation().start();
    let request = BrowserRequest::new(
        PREPARE_REQUEST,
        BrowserCommand::Prepare { evaluation_start },
    );
    let expected = BrowserEvent::Prepared { evaluation_start };

    dispatch_expected(browser, request, expected, output).await
}

async fn dispose_runtime(
    browser: &BrowserSession,
    request_id: RequestId,
    output: &Path,
) -> Result<(), RenderError> {
    let request = BrowserRequest::new(request_id, BrowserCommand::Dispose);
    dispatch_expected(browser, request, BrowserEvent::Disposed, output).await
}

async fn render_frames(
    browser: &BrowserSession,
    frames: &mut FrameSink<'_>,
    plan: &BrowserPlan,
    output: &Path,
) -> Result<(), RenderError> {
    let start = plan.output().start().get();
    let end = plan.output().end().get();

    for (offset, index) in (start..end).enumerate() {
        let frame = WireFrame::new(index)
            .map_err(|_| invalid_plan(output, "browser output frame exceeds the wire domain"))?;
        let request_id = frame_request_id(offset, output)?;
        seek_frame(browser, request_id, frame, output).await?;
        browser
            .wait_for_compositor_commit()
            .await
            .map_err(|source| RenderError::browser(output, source))?;
        frames.capture_and_write(browser, output).await?;
    }
    Ok(())
}

/// The two bounded destinations for a captured browser frame.
///
/// Direct local rendering streams one PNG into `FFmpeg`; Gate three worker
/// capture also records its canonical raw-pixel fingerprint. The enum keeps
/// this closed policy at the capture boundary instead of introducing an
/// unbounded callback or channel abstraction.
enum FrameSink<'a> {
    Encoder(&'a mut FfmpegSession),
    Artifact(&'a mut FrameArtifactWriter),
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

impl FrameSink<'_> {
    async fn capture_and_write(
        &mut self,
        browser: &BrowserSession,
        output: &Path,
    ) -> Result<(), RenderError> {
        match self {
            Self::Encoder(encoder) => {
                let frame = browser
                    .capture_png()
                    .await
                    .map_err(|source| RenderError::browser(output, source))?;
                encoder
                    .write_frame(&frame)
                    .await
                    .map_err(|source| RenderError::encoder(output, source))
            }
            Self::Artifact(writer) => {
                let frame = browser
                    .capture_frame()
                    .await
                    .map_err(|source| RenderError::browser(output, source))?;
                writer
                    .write_frame(&frame)
                    .await
                    .map_err(|source| RenderError::artifact(output, source))
            }
        }
    }
}

async fn seek_frame(
    browser: &BrowserSession,
    request_id: RequestId,
    frame: WireFrame,
    output: &Path,
) -> Result<(), RenderError> {
    let request = BrowserRequest::new(request_id, BrowserCommand::Seek { frame });
    let expected = BrowserEvent::FrameReady { frame };

    dispatch_expected(browser, request, expected, output).await
}

async fn dispatch_expected(
    browser: &BrowserSession,
    request: BrowserRequest,
    expected: BrowserEvent,
    output: &Path,
) -> Result<(), RenderError> {
    let request_id = request.request_id();
    let response = browser
        .dispatch(&request)
        .await
        .map_err(|source| RenderError::browser(output, source))?;

    if response.request_id() != request_id {
        return Err(RenderError::protocol(
            output,
            "browser response has the wrong request identity",
        ));
    }
    if response.event() != &expected {
        return Err(unexpected_event(output, &response));
    }
    Ok(())
}

fn unexpected_event(output: &Path, response: &BrowserResponse) -> RenderError {
    match response.event() {
        BrowserEvent::Failed(failure) => RenderError::runtime_failure(output, failure),
        _ => RenderError::protocol(
            output,
            "browser response does not match the requested phase",
        ),
    }
}

fn output_frame_count(plan: &BrowserPlan) -> Option<u64> {
    plan.output()
        .end()
        .get()
        .checked_sub(plan.output().start().get())
}

fn frame_request_id(offset: usize, output: &Path) -> Result<RequestId, RenderError> {
    let offset = u32::try_from(offset).map_err(|_| request_identity_overflow(output))?;
    let request_id = FIRST_FRAME_REQUEST
        .checked_add(offset)
        .ok_or_else(|| request_identity_overflow(output))?;
    Ok(RequestId::new(request_id))
}

fn disposal_request_id(frame_count: u64, output: &Path) -> Result<RequestId, RenderError> {
    let count = u32::try_from(frame_count).map_err(|_| request_identity_overflow(output))?;
    let request_id = FIRST_FRAME_REQUEST
        .checked_add(count)
        .ok_or_else(|| request_identity_overflow(output))?;

    Ok(RequestId::new(request_id))
}

fn invalid_plan(output: &Path, message: &'static str) -> RenderError {
    RenderError::new(RenderErrorKind::InvalidPlan, output, message)
}

fn request_identity_overflow(output: &Path) -> RenderError {
    RenderError::new(
        RenderErrorKind::PlanTooLarge,
        output,
        "frame request identity exceeds the protocol domain",
    )
}
