//! Browser protocol sequencing and bounded destinations for captured frames.
//!
//! This module is the sole owner of per-session request identities and the
//! `Seek`/capture/`Confirm` order. The parent executor owns process composition
//! and final assembly; this boundary owns exactly one browser frame at a time.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, BrowserResponse, RequestId,
    WireFrame, WireFrameRate,
};
use url::Url;

use super::{FrameCaptureMetrics, RenderError, RenderErrorKind, invalid_plan};
use crate::encoder::{CanonicalFrame, LayeredSession};
use crate::frame_artifact::FrameArtifactWriter;
use crate::{
    BrowserCaptureCadence, BrowserError, BrowserSession, CapturedFrame, DecodedRgba, EncodedPng,
    FfmpegSession, RenderProfile,
};

const LOAD_REQUEST: RequestId = RequestId::new(1);
const PREPARE_REQUEST: RequestId = RequestId::new(2);
const FIRST_FRAME_REQUEST: u32 = 3;

/// Sole authority for frame and disposal request identities in one session.
#[derive(Clone, Copy)]
pub(super) struct RequestSequence {
    frames: u32,
}

#[derive(Clone, Copy)]
struct FrameRequests {
    seek: RequestId,
    confirm: RequestId,
}

#[derive(Clone, Copy)]
struct FreshFrameCapture {
    requests: FrameRequests,
    frame_rate: WireFrameRate,
    frame: WireFrame,
    reason: CaptureReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FrameCapture {
    Reuse(EncodedPng),
    Capture(CaptureReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaptureReason {
    Initial,
    Stable,
    Placement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceChange {
    Stable,
    Placement,
}

impl RequestSequence {
    fn new(frame_count: u64, output: &Path) -> Result<Self, RenderError> {
        let frames = u32::try_from(frame_count).map_err(|_| request_identity_overflow(output))?;
        let frame_requests = frames
            .checked_mul(2)
            .ok_or_else(|| request_identity_overflow(output))?;
        FIRST_FRAME_REQUEST
            .checked_add(frame_requests)
            .ok_or_else(|| request_identity_overflow(output))?;

        Ok(Self { frames })
    }

    pub(super) fn frame_count(self) -> u64 {
        u64::from(self.frames)
    }

    fn frame_requests(self) -> impl Iterator<Item = FrameRequests> {
        (0..self.frames).map(|offset| {
            let seek = FIRST_FRAME_REQUEST + offset * 2;
            FrameRequests {
                seek: RequestId::new(seek),
                confirm: RequestId::new(seek + 1),
            }
        })
    }

    const fn disposal(self) -> RequestId {
        RequestId::new(FIRST_FRAME_REQUEST + self.frames * 2)
    }
}

pub(super) fn validate_plan(
    plan: &BrowserPlan,
    configured_max_frames: u64,
    output: &Path,
) -> Result<RequestSequence, RenderError> {
    let Some(frame_count) = output_frame_count(plan) else {
        return Err(invalid_plan(output, "browser output interval is reversed"));
    };
    if frame_count == 0 {
        return Err(invalid_plan(output, "browser output interval is empty"));
    }
    if frame_count > configured_max_frames {
        return Err(RenderError::new(
            RenderErrorKind::PlanTooLarge,
            output,
            "browser output interval exceeds the configured frame limit",
        ));
    }
    RequestSequence::new(frame_count, output)
}

pub(super) struct CaptureTask<'a> {
    pub(super) plan: &'a BrowserPlan,
    pub(super) requests: RequestSequence,
    pub(super) entry_url: &'a Url,
    pub(super) resource_root: &'a Path,
    pub(super) surface: CaptureSurface,
    pub(super) cadence: BrowserCaptureCadence,
    pub(super) output: &'a Path,
}

pub(super) async fn render_session(
    browser: &mut BrowserSession,
    frames: &mut FrameSink<'_>,
    metrics: &mut FrameCaptureMetrics,
    task: CaptureTask<'_>,
) -> Result<(), RenderError> {
    let CaptureTask {
        plan,
        requests,
        entry_url,
        resource_root,
        surface,
        cadence,
        output,
    } = task;
    let capture_commands_before = browser.capture_commands();
    let setup_started = Instant::now();
    if surface == CaptureSurface::Transparent {
        browser
            .use_transparent_capture_surface()
            .await
            .map_err(|source| RenderError::browser(output, source))?;
    }
    browser
        .navigate(entry_url, resource_root)
        .await
        .map_err(|source| RenderError::browser(output, source))?;
    let execution = async {
        load_runtime(browser, plan, output).await?;
        prepare_runtime(browser, plan, output).await?;
        browser
            .initialize_capture_surface(plan.frame_rate())
            .await
            .map_err(|source| RenderError::browser(output, source))?;
        metrics.runtime_setup += setup_started.elapsed();
        let rendered =
            render_frames(browser, frames, plan, requests, cadence, metrics, output).await;
        metrics.browser_capture_commands += browser.capture_commands() - capture_commands_before;
        rendered
    }
    .await;
    let disposal = dispose_runtime(browser, requests.disposal(), output).await;

    finish_runtime_session(execution, disposal)
}

fn finish_runtime_session(
    execution: Result<(), RenderError>,
    disposal: Result<(), RenderError>,
) -> Result<(), RenderError> {
    match (execution, disposal) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(execution), Err(disposal)) => {
            Err(execution.with_cleanup_failure("browser runtime disposal", disposal))
        }
    }
}

/// Root-surface ownership established by visual admission before navigation.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum CaptureSurface {
    Opaque,
    Transparent,
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
    browser: &mut BrowserSession,
    frames: &mut FrameSink<'_>,
    plan: &BrowserPlan,
    requests: RequestSequence,
    cadence: BrowserCaptureCadence,
    metrics: &mut FrameCaptureMetrics,
    output: &Path,
) -> Result<(), RenderError> {
    let frame_rate = plan.frame_rate();
    let placement_boundaries: BTreeSet<_> = plan.placement_boundaries().collect();
    let output_frames = plan.output().start().get()..plan.output().end().get();
    let mut previous_png = None;

    for (index, request_ids) in output_frames.zip(requests.frame_requests()) {
        let frame = WireFrame::new(index)
            .map_err(|_| invalid_plan(output, "browser output frame exceeds the wire domain"))?;
        let surface_change = if placement_boundaries.contains(&frame) {
            SurfaceChange::Placement
        } else {
            SurfaceChange::Stable
        };
        let capture = plan_frame_capture(cadence, surface_change, previous_png.as_ref());
        let png = match capture {
            FrameCapture::Reuse(png) => png,
            FrameCapture::Capture(reason) => {
                capture_frame(
                    browser,
                    FreshFrameCapture {
                        requests: request_ids,
                        frame_rate,
                        frame,
                        reason,
                    },
                    metrics,
                    output,
                )
                .await?
            }
        };
        frames
            .write(png.clone(), browser.render_profile(), metrics, output)
            .await?;
        previous_png = Some(png);
    }
    Ok(())
}

fn plan_frame_capture(
    cadence: BrowserCaptureCadence,
    surface_change: SurfaceChange,
    previous: Option<&EncodedPng>,
) -> FrameCapture {
    match (cadence, surface_change, previous) {
        (BrowserCaptureCadence::PlacementBounded, SurfaceChange::Stable, Some(previous)) => {
            FrameCapture::Reuse(previous.clone())
        }
        (_, SurfaceChange::Placement, _) => FrameCapture::Capture(CaptureReason::Placement),
        (_, SurfaceChange::Stable, None) => FrameCapture::Capture(CaptureReason::Initial),
        (_, SurfaceChange::Stable, Some(_)) => FrameCapture::Capture(CaptureReason::Stable),
    }
}

async fn capture_frame(
    browser: &mut BrowserSession,
    capture: FreshFrameCapture,
    metrics: &mut FrameCaptureMetrics,
    output: &Path,
) -> Result<EncodedPng, RenderError> {
    let started = Instant::now();
    stage_frame(browser, capture.requests.seek, capture.frame, output).await?;
    metrics.seek += started.elapsed();

    let started = Instant::now();
    let mut png = capture_staged_png(browser, capture)
        .await
        .map_err(|source| RenderError::browser(output, source))?;
    metrics.readback += started.elapsed();

    let started = Instant::now();
    confirm_frame(browser, capture.requests.confirm, capture.frame, output).await?;
    metrics.confirm += started.elapsed();

    match capture.reason {
        CaptureReason::Stable => {}
        CaptureReason::Initial | CaptureReason::Placement => {
            let started = Instant::now();
            png = browser
                .recapture_png_after_confirmation(capture.frame)
                .await
                .map_err(|source| RenderError::browser(output, source))?;
            metrics.readback += started.elapsed();
        }
    }

    metrics.browser_captures += 1;
    Ok(png)
}

/// The two bounded destinations for a captured browser frame.
///
/// Direct local rendering streams one PNG into `FFmpeg`; worker capture also
/// records its canonical raw-pixel fingerprint. This closed policy avoids an
/// unbounded callback or channel at the capture boundary.
pub(super) enum FrameSink<'a> {
    Encoder(&'a mut FfmpegSession),
    Artifact(&'a mut FrameArtifactWriter),
    LayeredVideo(&'a mut LayeredSession),
    LayeredArtifact {
        compositor: &'a mut LayeredSession,
        artifact: &'a mut FrameArtifactWriter,
    },
}

impl FrameSink<'_> {
    async fn write(
        &mut self,
        png: EncodedPng,
        profile: RenderProfile,
        metrics: &mut FrameCaptureMetrics,
        output: &Path,
    ) -> Result<(), RenderError> {
        match self {
            Self::Encoder(encoder) => {
                let started = Instant::now();
                encoder
                    .write_frame(&png)
                    .await
                    .map_err(|source| RenderError::encoder(output, source))?;
                metrics.write += started.elapsed();
            }
            Self::Artifact(writer) => write_artifact(writer, profile, png, metrics, output).await?,
            Self::LayeredVideo(compositor) => {
                let foreground = decode_foreground(&png, profile, metrics, output)?;
                let started = Instant::now();
                compositor
                    .write_video_frame(&foreground)
                    .await
                    .map_err(|source| RenderError::encoder(output, source))?;
                metrics.write += started.elapsed();
            }
            Self::LayeredArtifact {
                compositor,
                artifact,
            } => {
                write_layered_artifact(compositor, artifact, profile, &png, metrics, output)
                    .await?;
            }
        }
        metrics.frames += 1;
        Ok(())
    }
}

async fn write_layered_artifact(
    compositor: &mut LayeredSession,
    artifact: &mut FrameArtifactWriter,
    profile: RenderProfile,
    foreground: &EncodedPng,
    metrics: &mut FrameCaptureMetrics,
    output: &Path,
) -> Result<(), RenderError> {
    let foreground = decode_foreground(foreground, profile, metrics, output)?;
    let started = Instant::now();
    let frame = compositor
        .write_frame(&foreground)
        .await
        .map_err(|source| RenderError::encoder(output, source))?;
    if let Some(frame) = frame {
        write_canonical_artifact(artifact, profile, frame, output).await?;
    }
    metrics.write += started.elapsed();
    Ok(())
}

fn decode_foreground(
    foreground: &EncodedPng,
    profile: RenderProfile,
    metrics: &mut FrameCaptureMetrics,
    output: &Path,
) -> Result<DecodedRgba, RenderError> {
    let started = Instant::now();
    let foreground = foreground
        .decode_rgba(profile)
        .map_err(|source| RenderError::browser(output, source))?;
    metrics.pixel_processing += started.elapsed();
    Ok(foreground)
}

pub(super) async fn write_canonical_artifact(
    artifact: &mut FrameArtifactWriter,
    profile: RenderProfile,
    frame: CanonicalFrame,
    output: &Path,
) -> Result<(), RenderError> {
    let CanonicalFrame::Pixels { bytes, fingerprint } = frame else {
        return Err(invalid_plan(
            output,
            "layered worker composition did not retain canonical pixels",
        ));
    };
    artifact
        .write_rgba_frame(&bytes, fingerprint, profile)
        .await
        .map_err(|source| RenderError::artifact(output, source))
}

async fn write_artifact(
    writer: &mut FrameArtifactWriter,
    profile: RenderProfile,
    png: EncodedPng,
    metrics: &mut FrameCaptureMetrics,
    output: &Path,
) -> Result<(), RenderError> {
    let pixel_started = Instant::now();
    let captured = CapturedFrame::from_png(png, profile)
        .map_err(|source| RenderError::browser(output, source))?;
    metrics.pixel_processing += pixel_started.elapsed();

    let write_started = Instant::now();
    writer
        .write_frame(&captured)
        .await
        .map_err(|source| RenderError::artifact(output, source))?;
    metrics.write += write_started.elapsed();
    Ok(())
}

async fn capture_staged_png(
    browser: &mut BrowserSession,
    capture: FreshFrameCapture,
) -> Result<EncodedPng, BrowserError> {
    match capture.reason {
        CaptureReason::Stable => browser.capture_png(capture.frame, capture.frame_rate).await,
        CaptureReason::Initial | CaptureReason::Placement => {
            browser
                .capture_png_after_surface_change(capture.frame, capture.frame_rate)
                .await
        }
    }
}

async fn stage_frame(
    browser: &BrowserSession,
    request_id: RequestId,
    frame: WireFrame,
    output: &Path,
) -> Result<(), RenderError> {
    let request = BrowserRequest::new(request_id, BrowserCommand::Seek { frame });
    let expected = BrowserEvent::FrameStaged { frame };
    dispatch_expected(browser, request, expected, output).await
}

async fn confirm_frame(
    browser: &BrowserSession,
    request_id: RequestId,
    frame: WireFrame,
    output: &Path,
) -> Result<(), RenderError> {
    let request = BrowserRequest::new(request_id, BrowserCommand::Confirm { frame });
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

fn request_identity_overflow(output: &Path) -> RenderError {
    RenderError::new(
        RenderErrorKind::PlanTooLarge,
        output,
        "frame request identity exceeds the protocol domain",
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        CaptureReason, FrameCapture, SurfaceChange, finish_runtime_session, plan_frame_capture,
    };
    use crate::executor::{RenderError, RenderErrorKind};
    use crate::{BrowserCaptureCadence, EncodedPng};

    fn preceding_frame() -> EncodedPng {
        EncodedPng::new(Vec::new())
    }

    #[test]
    fn captures_every_authored_frame_when_required() {
        let previous = preceding_frame();
        assert_eq!(
            plan_frame_capture(
                BrowserCaptureCadence::EveryFrame,
                SurfaceChange::Stable,
                Some(&previous),
            ),
            FrameCapture::Capture(CaptureReason::Stable),
        );
        assert_eq!(
            plan_frame_capture(
                BrowserCaptureCadence::EveryFrame,
                SurfaceChange::Placement,
                Some(&previous),
            ),
            FrameCapture::Capture(CaptureReason::Placement),
        );
    }

    #[test]
    fn reuses_a_placement_bounded_frame_between_boundaries() {
        let previous = preceding_frame();
        assert_eq!(
            plan_frame_capture(
                BrowserCaptureCadence::PlacementBounded,
                SurfaceChange::Stable,
                None,
            ),
            FrameCapture::Capture(CaptureReason::Initial),
        );
        assert_eq!(
            plan_frame_capture(
                BrowserCaptureCadence::PlacementBounded,
                SurfaceChange::Stable,
                Some(&previous),
            ),
            FrameCapture::Reuse(previous.clone()),
        );
        assert_eq!(
            plan_frame_capture(
                BrowserCaptureCadence::PlacementBounded,
                SurfaceChange::Placement,
                Some(&previous),
            ),
            FrameCapture::Capture(CaptureReason::Placement),
        );
    }

    #[test]
    fn identifies_the_first_output_without_inventing_a_placement_change() {
        assert_eq!(
            plan_frame_capture(
                BrowserCaptureCadence::EveryFrame,
                SurfaceChange::Stable,
                None,
            ),
            FrameCapture::Capture(CaptureReason::Initial),
        );
    }

    #[test]
    fn retains_the_primary_failure_when_disposal_also_fails() {
        let output = Path::new("render.mp4");
        let execution = RenderError::new(RenderErrorKind::Encoder, output, "frame write failed");
        let disposal = RenderError::new(RenderErrorKind::Protocol, output, "dispose failed");

        let error = finish_runtime_session(Err(execution), Err(disposal))
            .expect_err("both runtime failures must remain observable");

        assert_eq!(error.kind(), RenderErrorKind::Encoder);
        assert_eq!(
            error.to_string(),
            "render.mp4: frame write failed; browser runtime disposal also failed",
        );
        assert!(
            std::error::Error::source(&error)
                .expect("the disposal failure must be retained")
                .to_string()
                .contains("dispose failed"),
        );
    }
}
