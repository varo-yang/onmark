mod error;
mod output;

use std::path::{Path, PathBuf};

use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, BrowserResponse, RequestId,
    WireFrame,
};

use self::output::StagedOutput;
use crate::{BrowserLimits, BrowserSession, EncodedVideo, Ffmpeg, FfmpegSession};

pub use error::{RenderError, RenderErrorKind};

const LOAD_REQUEST: RequestId = RequestId::new(1);
const PREPARE_REQUEST: RequestId = RequestId::new(2);
const FIRST_FRAME_REQUEST: u32 = 3;

/// Single-process Gate-one renderer composed from Chromium and `FFmpeg`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderExecutor {
    browser_executable: PathBuf,
    browser_limits: BrowserLimits,
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
            browser_executable: browser_executable.into(),
            browser_limits,
            ffmpeg,
        }
    }

    /// Renders every output frame into one H.264 MP4 artifact.
    ///
    /// Frame capture and encoder input are sequential: at most one encoded PNG
    /// is owned between Chromium and `FFmpeg` at any time.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when the plan exceeds configured limits, the
    /// browser protocol deviates from its expected phase, or either process
    /// boundary fails. Chromium shutdown is still attempted after render work
    /// fails.
    pub async fn render(
        &self,
        plan: BrowserPlan,
        bundle_url: &str,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        self.validate_plan(plan, output)?;
        let staging = StagedOutput::new(output)?;
        let browser = BrowserSession::launch(&self.browser_executable, self.browser_limits)
            .await
            .map_err(|source| RenderError::browser(output, source))?;

        let render_result = self
            .render_session(&browser, plan, bundle_url, staging.path(), output)
            .await;
        let shutdown_result = browser
            .shutdown()
            .await
            .map_err(|source| RenderError::browser(output, source));

        let video = render_result?;
        shutdown_result?;
        staging.publish(video, output)
    }

    fn validate_plan(&self, plan: BrowserPlan, output: &Path) -> Result<(), RenderError> {
        let frame_count = output_frame_count(plan).ok_or_else(|| {
            RenderError::new(
                RenderErrorKind::InvalidPlan,
                output,
                "browser output interval is reversed",
            )
        })?;
        let request_limit = u64::from(u32::MAX - FIRST_FRAME_REQUEST);
        let max_frames = self.ffmpeg.max_frames().min(request_limit);

        if frame_count == 0 {
            return Err(RenderError::new(
                RenderErrorKind::InvalidPlan,
                output,
                "browser output interval is empty",
            ));
        }
        if frame_count > max_frames {
            return Err(RenderError::new(
                RenderErrorKind::PlanTooLarge,
                output,
                "browser output interval exceeds the configured frame limit",
            ));
        }
        Ok(())
    }

    async fn render_session(
        &self,
        browser: &BrowserSession,
        plan: BrowserPlan,
        bundle_url: &str,
        staging: &Path,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        browser
            .navigate(bundle_url)
            .await
            .map_err(|source| RenderError::browser(output, source))?;
        dispatch_expected(
            browser,
            BrowserRequest::new(LOAD_REQUEST, BrowserCommand::Load { plan }),
            BrowserEvent::Loaded,
            output,
        )
        .await?;
        dispatch_expected(
            browser,
            BrowserRequest::new(
                PREPARE_REQUEST,
                BrowserCommand::Prepare {
                    evaluation_start: plan.evaluation().start(),
                },
            ),
            BrowserEvent::Prepared {
                evaluation_start: plan.evaluation().start(),
            },
            output,
        )
        .await?;

        let mut encoder = self
            .ffmpeg
            .start(staging, plan.frame_rate())
            .map_err(|source| RenderError::encoder(output, source))?;
        render_frames(browser, &mut encoder, plan, output).await?;
        dispatch_expected(
            browser,
            BrowserRequest::new(next_request_id(plan), BrowserCommand::Dispose),
            BrowserEvent::Disposed,
            output,
        )
        .await?;

        encoder
            .finish()
            .await
            .map_err(|source| RenderError::encoder(output, source))
    }
}

async fn render_frames(
    browser: &BrowserSession,
    encoder: &mut FfmpegSession,
    plan: BrowserPlan,
    output: &Path,
) -> Result<(), RenderError> {
    let start = plan.output().start().get();
    let end = plan.output().end().get();

    for (offset, index) in (start..end).enumerate() {
        let frame = WireFrame::new(index).map_err(|_| {
            RenderError::new(
                RenderErrorKind::InvalidPlan,
                output,
                "browser output frame exceeds the wire domain",
            )
        })?;
        let request_id = frame_request_id(offset, output)?;
        dispatch_expected(
            browser,
            BrowserRequest::new(request_id, BrowserCommand::Seek { frame }),
            BrowserEvent::FrameReady { frame },
            output,
        )
        .await?;
        let captured = browser
            .capture_png()
            .await
            .map_err(|source| RenderError::browser(output, source))?;
        encoder
            .write_frame(&captured)
            .await
            .map_err(|source| RenderError::encoder(output, source))?;
    }
    Ok(())
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
    let message = match response.event() {
        BrowserEvent::Failed(failure) => {
            format!("browser runtime failed: {}", failure.message())
        }
        _ => "browser response does not match the requested phase".to_owned(),
    };
    RenderError::protocol(output, message)
}

fn output_frame_count(plan: BrowserPlan) -> Option<u64> {
    plan.output()
        .end()
        .get()
        .checked_sub(plan.output().start().get())
}

fn frame_request_id(offset: usize, output: &Path) -> Result<RequestId, RenderError> {
    let offset = u32::try_from(offset).map_err(|_| {
        RenderError::new(
            RenderErrorKind::PlanTooLarge,
            output,
            "frame request identity exceeds the protocol domain",
        )
    })?;
    let request_id = FIRST_FRAME_REQUEST.checked_add(offset).ok_or_else(|| {
        RenderError::new(
            RenderErrorKind::PlanTooLarge,
            output,
            "frame request identity exceeds the protocol domain",
        )
    })?;
    Ok(RequestId::new(request_id))
}

fn next_request_id(plan: BrowserPlan) -> RequestId {
    let count = output_frame_count(plan).expect("a validated browser plan has an ordered interval");
    let count = u32::try_from(count).expect("a validated browser plan fits the request domain");
    RequestId::new(
        FIRST_FRAME_REQUEST
            .checked_add(count)
            .expect("a validated browser plan leaves one disposal request identity"),
    )
}
