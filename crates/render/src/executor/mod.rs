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
    /// Returns [`RenderError`] when the selected configuration or plan exceeds
    /// supported limits, the browser protocol deviates from its expected phase,
    /// or either process boundary fails. Chromium shutdown is still attempted
    /// after render work fails.
    pub async fn render(
        &self,
        plan: BrowserPlan,
        bundle_url: &str,
        output: &Path,
    ) -> Result<EncodedVideo, RenderError> {
        validate_output_dimensions(self.browser_limits, output)?;
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
        let Some(frame_count) = output_frame_count(plan) else {
            return Err(invalid_plan(output, "browser output interval is reversed"));
        };
        let request_limit = u64::from(u32::MAX - FIRST_FRAME_REQUEST);
        let max_frames = self.ffmpeg.max_frames().min(request_limit);

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
        load_runtime(browser, plan, output).await?;
        prepare_runtime(browser, plan, output).await?;

        let mut encoder = self
            .ffmpeg
            .start(staging, plan.frame_rate())
            .map_err(|source| RenderError::encoder(output, source))?;
        render_frames(browser, &mut encoder, plan, output).await?;
        dispose_runtime(browser, plan, output).await?;

        encoder
            .finish()
            .await
            .map_err(|source| RenderError::encoder(output, source))
    }
}

fn validate_output_dimensions(limits: BrowserLimits, output: &Path) -> Result<(), RenderError> {
    if limits.width().is_multiple_of(2) && limits.height().is_multiple_of(2) {
        return Ok(());
    }

    Err(RenderError::new(
        RenderErrorKind::InvalidConfiguration,
        output,
        "H.264 yuv420p output requires even viewport dimensions",
    ))
}

async fn load_runtime(
    browser: &BrowserSession,
    plan: BrowserPlan,
    output: &Path,
) -> Result<(), RenderError> {
    let request = BrowserRequest::new(LOAD_REQUEST, BrowserCommand::Load { plan });
    dispatch_expected(browser, request, BrowserEvent::Loaded, output).await
}

async fn prepare_runtime(
    browser: &BrowserSession,
    plan: BrowserPlan,
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
    plan: BrowserPlan,
    output: &Path,
) -> Result<(), RenderError> {
    let request = BrowserRequest::new(next_request_id(plan), BrowserCommand::Dispose);
    dispatch_expected(browser, request, BrowserEvent::Disposed, output).await
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
        let frame = WireFrame::new(index)
            .map_err(|_| invalid_plan(output, "browser output frame exceeds the wire domain"))?;
        let request_id = frame_request_id(offset, output)?;
        seek_frame(browser, request_id, frame, output).await?;
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
    let offset = u32::try_from(offset).map_err(|_| request_identity_overflow(output))?;
    let request_id = FIRST_FRAME_REQUEST
        .checked_add(offset)
        .ok_or_else(|| request_identity_overflow(output))?;
    Ok(RequestId::new(request_id))
}

fn next_request_id(plan: BrowserPlan) -> RequestId {
    let count = output_frame_count(plan).expect("a validated browser plan has an ordered interval");
    let count = u32::try_from(count).expect("a validated browser plan fits the request domain");
    let request_id = FIRST_FRAME_REQUEST
        .checked_add(count)
        .expect("a validated browser plan leaves one disposal request identity");

    RequestId::new(request_id)
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

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use crate::BrowserLimits;

    use super::{RenderErrorKind, validate_output_dimensions};

    #[test]
    fn rejects_dimensions_that_yuv420p_cannot_encode() {
        let limits = BrowserLimits::new(321, 181, Duration::from_secs(1), 1)
            .expect("odd browser dimensions remain valid for capture");

        let error = validate_output_dimensions(limits, Path::new("video.mp4"))
            .expect_err("the fixed Gate-one encoder requires even dimensions");

        assert_eq!(error.kind(), RenderErrorKind::InvalidConfiguration);
    }
}
