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

use super::{FrameCaptureMetrics, RenderError, RenderErrorKind, invalid_plan};
use crate::frame_artifact::FrameArtifactWriter;
use crate::{
    BrowserError, BrowserSession, CapturedFrame, EncodedPng, FfmpegSession, RenderProfile,
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

pub(super) async fn render_session(
    browser: &mut BrowserSession,
    plan: &BrowserPlan,
    requests: RequestSequence,
    entry_url: &str,
    frames: &mut FrameSink<'_>,
    metrics: &mut FrameCaptureMetrics,
    output: &Path,
) -> Result<(), RenderError> {
    let setup_started = Instant::now();
    browser
        .navigate(entry_url)
        .await
        .map_err(|source| RenderError::browser(output, source))?;
    load_runtime(browser, plan, output).await?;
    prepare_runtime(browser, plan, output).await?;
    browser
        .initialize_capture_surface(plan.frame_rate())
        .await
        .map_err(|source| RenderError::browser(output, source))?;
    metrics.runtime_setup = setup_started.elapsed();

    render_frames(browser, frames, plan, requests, metrics, output).await?;
    dispose_runtime(browser, requests.disposal(), output).await
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
    metrics: &mut FrameCaptureMetrics,
    output: &Path,
) -> Result<(), RenderError> {
    let frame_rate = plan.frame_rate();
    let placement_boundaries = plan.placement_boundaries().collect();
    let output_frames = plan.output().start().get()..plan.output().end().get();

    for (index, request_ids) in output_frames.zip(requests.frame_requests()) {
        let frame = WireFrame::new(index)
            .map_err(|_| invalid_plan(output, "browser output frame exceeds the wire domain"))?;

        let started = Instant::now();
        stage_frame(browser, request_ids.seek, frame, output).await?;
        metrics.seek += started.elapsed();

        let started = Instant::now();
        let png = capture_staged_png(browser, frame_rate, &placement_boundaries, frame)
            .await
            .map_err(|source| RenderError::browser(output, source))?;
        metrics.readback += started.elapsed();

        let started = Instant::now();
        confirm_frame(browser, request_ids.confirm, frame, output).await?;
        metrics.confirm += started.elapsed();

        frames
            .write(png, browser.render_profile(), metrics, output)
            .await?;
    }
    Ok(())
}

/// The two bounded destinations for a captured browser frame.
///
/// Direct local rendering streams one PNG into `FFmpeg`; worker capture also
/// records its canonical raw-pixel fingerprint. This closed policy avoids an
/// unbounded callback or channel at the capture boundary.
pub(super) enum FrameSink<'a> {
    Encoder(&'a mut FfmpegSession),
    Artifact(&'a mut FrameArtifactWriter),
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
        }
        metrics.frames += 1;
        Ok(())
    }
}

async fn write_artifact(
    writer: &mut FrameArtifactWriter,
    profile: RenderProfile,
    png: EncodedPng,
    metrics: &mut FrameCaptureMetrics,
    output: &Path,
) -> Result<(), RenderError> {
    let fingerprint_started = Instant::now();
    let captured = CapturedFrame::from_png(png, profile)
        .map_err(|source| RenderError::browser(output, source))?;
    metrics.fingerprint += fingerprint_started.elapsed();

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
    frame_rate: WireFrameRate,
    placement_boundaries: &BTreeSet<WireFrame>,
    frame: WireFrame,
) -> Result<EncodedPng, BrowserError> {
    if placement_boundaries.contains(&frame) {
        return browser
            .capture_png_after_placement_boundary(frame, frame_rate)
            .await;
    }
    browser.capture_png(frame, frame_rate).await
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
