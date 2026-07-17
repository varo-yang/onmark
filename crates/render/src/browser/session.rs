//! Owned Chromium/CDP session with bounded startup, command, and shutdown work.
//!
//! This module contains vendor-specific control flow so the executor can speak
//! only in Onmark protocol values and typed browser failures.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chromiumoxide::browser::Browser;
use chromiumoxide::cdp::browser_protocol::dom::Rgba;
use chromiumoxide::cdp::browser_protocol::emulation::SetDefaultBackgroundColorOverrideParams;
use chromiumoxide::cdp::browser_protocol::headless_experimental::{
    BeginFrameParams, BeginFrameReturns, ScreenshotParams, ScreenshotParamsFormat,
};
use chromiumoxide::cdp::browser_protocol::target::CreateTargetParams;
use chromiumoxide::error::CdpError;
use chromiumoxide::handler::viewport::Viewport;
use chromiumoxide::handler::{Handler, HandlerConfig};
use chromiumoxide::page::Page;
use futures::StreamExt as _;
use onmark_core::protocol::{
    BrowserRequest, BrowserResponse, RUNTIME_HOST_NAME, WireFrame, WireFrameRate,
};
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout, timeout_at};

use super::error::{BrowserError, BrowserErrorKind};
use super::frame::{CapturedFrame, EncodedPng};
use super::limits::BrowserLimits;
use super::process::{BrowserDiagnostics, BrowserLaunchPolicy, ChromiumProcess};
use crate::RenderProfile;

const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const SURFACE_INITIALIZATION_TIME_MILLIS: f64 = 1.0;
const COMPOSITOR_BASE_TIME_MILLIS: f64 = 1_000.0;
const MAX_COMPOSITOR_OFFSET_MILLIS: f64 = 0.001;

/// One owned headless-shell process and its single render page.
#[derive(Debug)]
pub struct BrowserSession {
    browser: Browser,
    page: Page,
    handler: JoinHandle<Result<(), CdpError>>,
    process: ChromiumProcess,
    diagnostics: BrowserDiagnostics,
    // Headless shell omits screenshotData when a frame has no visual damage.
    last_capture: Mutex<Option<EncodedPng>>,
    limits: BrowserLimits,
    render_profile: RenderProfile,
    // Retained so headless shell's private profile outlives the process.
    _profile: TempDir,
}

impl BrowserSession {
    pub(crate) const fn render_profile(&self) -> RenderProfile {
        self.render_profile
    }

    /// Launches a bounded headless-shell session using an explicit executable.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when configuration, process launch, CDP handler
    /// startup, or initial page creation fails.
    pub async fn launch(
        executable: impl AsRef<Path>,
        launch_policy: BrowserLaunchPolicy,
        render_profile: RenderProfile,
        limits: BrowserLimits,
    ) -> Result<Self, BrowserError> {
        let target = render_target().map_err(BrowserError::configuration)?;
        let profile = browser_profile()?;
        let (mut process, endpoint) = ChromiumProcess::launch(
            executable.as_ref(),
            launch_policy,
            profile.path(),
            render_profile,
            limits.deadline(),
        )
        .await?;
        let diagnostics = process.diagnostics();
        let connection =
            Browser::connect_with_config(endpoint, handler_config(render_profile, limits)).await;
        let (mut browser, handler) = match connection {
            Ok(connection) => connection,
            Err(source) => {
                let error = BrowserError::cdp_with_diagnostics(
                    BrowserErrorKind::Launch,
                    source,
                    diagnostics.snapshot(),
                );
                process.abort(limits.deadline()).await;
                return Err(error);
            }
        };
        let mut handler = Box::pin(drive_handler(handler));
        let page = tokio::select! {
            biased;
            result = &mut handler => {
                process.abort(limits.deadline()).await;
                return Err(handler_exit_error(result, diagnostics.snapshot()));
            }
            result = browser.new_page(target) => result,
        };
        let handler = tokio::spawn(handler);
        let page = match page {
            Ok(page) => page,
            Err(source) => {
                let error = BrowserError::cdp_with_diagnostics(
                    BrowserErrorKind::PageCreation,
                    source,
                    diagnostics.snapshot(),
                );
                cleanup_failed_launch(&mut browser, handler, &mut process, limits.deadline()).await;
                return Err(error);
            }
        };

        Ok(Self {
            browser,
            page,
            handler,
            process,
            diagnostics,
            last_capture: Mutex::new(None),
            limits,
            render_profile,
            _profile: profile,
        })
    }

    /// Navigates the owned page and waits for the runtime host to become ready.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when Chrome rejects navigation, the load event
    /// misses its deadline, or the bundle never installs its runtime host.
    pub async fn navigate(&self, url: &str) -> Result<(), BrowserError> {
        self.page
            .goto(url)
            .await
            .map_err(|source| self.cdp_error(BrowserErrorKind::Navigation, source))?;
        let navigation_result = timeout(self.limits.deadline(), self.page.wait_for_navigation())
            .await
            .map_err(|_| BrowserError::without_source(BrowserErrorKind::Navigation))?;
        navigation_result.map_err(|source| self.cdp_error(BrowserErrorKind::Navigation, source))?;
        self.wait_for_runtime_host().await
    }

    /// Dispatches one typed request through the installed browser runtime host.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when serialization, JavaScript evaluation,
    /// response decoding, or the configured request deadline fails.
    pub async fn dispatch(
        &self,
        request: &BrowserRequest,
    ) -> Result<BrowserResponse, BrowserError> {
        let request = serde_json::to_string(request)
            .map_err(|source| BrowserError::json(BrowserErrorKind::Protocol, source))?;
        let expression = format!("globalThis.{RUNTIME_HOST_NAME}.dispatch({request})");
        let evaluation = self.page.evaluate_expression(expression);
        let result = timeout(self.limits.deadline(), evaluation)
            .await
            .map_err(|_| BrowserError::without_source(BrowserErrorKind::Protocol))?
            .map_err(|source| self.cdp_error(BrowserErrorKind::Protocol, source))?;
        result
            .into_value()
            .map_err(|source| BrowserError::json(BrowserErrorKind::Protocol, source))
    }

    /// Initializes the target surface before the first captured frame.
    ///
    /// The fixed pre-baseline timestamp lets Chromium allocate and paint the
    /// page surface without consuming an authored frame.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when the compositor frame cannot complete.
    pub async fn initialize_capture_surface(
        &self,
        frame_rate: WireFrameRate,
    ) -> Result<(), BrowserError> {
        self.page
            .execute(surface_initialization_parameters(frame_rate))
            .await
            .map(|_| ())
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))
    }

    /// Keeps the target's root surface transparent for layered capture.
    ///
    /// CSS transparency alone is insufficient because Chromium normally
    /// composites the root layer over an opaque browser background.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when Chromium rejects the capture-surface
    /// override.
    pub async fn use_transparent_capture_surface(&self) -> Result<(), BrowserError> {
        let color = Rgba {
            r: 0,
            g: 0,
            b: 0,
            a: Some(0.0),
        };
        self.page
            .execute(SetDefaultBackgroundColorOverrideParams { color: Some(color) })
            .await
            .map(|_| ())
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))
    }

    /// Commits and captures one authored frame as PNG without writing it to disk.
    ///
    /// Rust supplies a deterministic compositor timestamp from the exact
    /// authored frame offset. Headless shell commits and captures that frame in
    /// one CDP command, so no wall clock or animation-frame polling enters capture.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when capture fails or exceeds the configured
    /// retained-byte budget.
    pub async fn capture_png(
        &self,
        frame: WireFrame,
        frame_rate: WireFrameRate,
    ) -> Result<EncodedPng, BrowserError> {
        self.capture_png_with_fallback(frame, frame_rate, MissingScreenshot::ReusePrevious)
            .await
    }

    pub(crate) async fn capture_png_after_placement_boundary(
        &self,
        frame: WireFrame,
        frame_rate: WireFrameRate,
    ) -> Result<EncodedPng, BrowserError> {
        // Runtime staging may introduce a layer that was absent from the
        // compositor. Commit it just before the authored timestamp so capture
        // observes the new placement without advancing screenplay time.
        self.page
            .execute(staged_placement_parameters(frame, frame_rate))
            .await
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))?;
        self.capture_png_with_fallback(frame, frame_rate, MissingScreenshot::RetryOnce)
            .await
    }

    async fn capture_png_with_fallback(
        &self,
        frame: WireFrame,
        frame_rate: WireFrameRate,
        missing: MissingScreenshot,
    ) -> Result<EncodedPng, BrowserError> {
        let response = self
            .capture(begin_frame_parameters(frame, frame_rate))
            .await?;
        if let Some(screenshot) = response.screenshot_data {
            return self.decode_and_remember(screenshot);
        }
        let previous = match missing {
            MissingScreenshot::ReusePrevious => self.previous_capture()?,
            MissingScreenshot::RetryOnce => None,
        };
        if let Some(previous) = previous {
            return Ok(previous);
        }

        let retry = self
            .capture(screenshot_retry_parameters(frame, frame_rate))
            .await?;
        let screenshot = retry.screenshot_data.ok_or_else(|| {
            BrowserError::capture_pixels("headless shell did not return the required screenshot")
        })?;
        self.decode_and_remember(screenshot)
    }

    async fn capture(
        &self,
        parameters: BeginFrameParams,
    ) -> Result<BeginFrameReturns, BrowserError> {
        self.page
            .execute(parameters)
            .await
            .map(|response| response.result)
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))
    }

    fn decode_and_remember(&self, screenshot: impl AsRef<str>) -> Result<EncodedPng, BrowserError> {
        let encoded: &str = screenshot.as_ref();
        if encoded.len() > maximum_base64_length(self.limits.max_capture_bytes()) {
            return Err(BrowserError::without_source(
                BrowserErrorKind::CaptureTooLarge,
            ));
        }
        let bytes = BASE64.decode(encoded).map_err(BrowserError::base64)?;

        if bytes.len() > self.limits.max_capture_bytes() {
            return Err(BrowserError::without_source(
                BrowserErrorKind::CaptureTooLarge,
            ));
        }
        let capture = EncodedPng::new(bytes);
        *self.capture_cache()? = Some(capture.clone());
        Ok(capture)
    }

    fn previous_capture(&self) -> Result<Option<EncodedPng>, BrowserError> {
        Ok(self.capture_cache()?.clone())
    }

    fn capture_cache(&self) -> Result<std::sync::MutexGuard<'_, Option<EncodedPng>>, BrowserError> {
        self.last_capture
            .lock()
            .map_err(|_| BrowserError::capture_pixels("browser capture cache is unavailable"))
    }

    /// Captures one encoder PNG together with canonical raw-RGBA evidence.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when Chromium cannot capture the viewport, the
    /// retained PNG exceeds its bound, or the decoded pixels do not match the
    /// configured render profile.
    pub async fn capture_frame(
        &self,
        frame: WireFrame,
        frame_rate: WireFrameRate,
    ) -> Result<CapturedFrame, BrowserError> {
        CapturedFrame::from_png(
            self.capture_png(frame, frame_rate).await?,
            self.render_profile,
        )
    }

    /// Closes Chromium and waits for both the process and CDP handler to exit.
    ///
    /// # Errors
    ///
    /// Returns the first observed shutdown failure after all cleanup attempts
    /// have completed.
    pub async fn shutdown(mut self) -> Result<(), BrowserError> {
        let deadline = self.limits.deadline();
        let browser_result = close_browser(&mut self.browser, deadline, &self.diagnostics).await;
        if browser_result.is_err() {
            self.process.request_stop();
        }
        let process_result = self.process.shutdown(deadline).await;
        let handler_result = shutdown_handler(self.handler, deadline, &self.diagnostics).await;

        browser_result?;
        process_result?;
        handler_result
    }

    async fn wait_for_runtime_host(&self) -> Result<(), BrowserError> {
        let deadline = Instant::now()
            .checked_add(self.limits.deadline())
            .ok_or_else(|| BrowserError::without_source(BrowserErrorKind::RuntimeHost))?;
        let expression =
            format!("typeof globalThis.{RUNTIME_HOST_NAME}?.dispatch === \"function\"");

        loop {
            if self.runtime_host_is_ready(deadline, &expression).await? {
                return Ok(());
            }
            wait_for_next_poll(deadline).await?;
        }
    }

    async fn runtime_host_is_ready(
        &self,
        deadline: Instant,
        expression: &str,
    ) -> Result<bool, BrowserError> {
        let evaluation = self.page.evaluate_expression(expression);
        let evaluation_result = timeout_at(deadline, evaluation)
            .await
            .map_err(|_| BrowserError::without_source(BrowserErrorKind::RuntimeHost))?;
        let remote = evaluation_result
            .map_err(|source| self.cdp_error(BrowserErrorKind::RuntimeHost, source))?;
        remote
            .into_value()
            .map_err(|source| BrowserError::json(BrowserErrorKind::RuntimeHost, source))
    }

    fn cdp_error(&self, kind: BrowserErrorKind, source: CdpError) -> BrowserError {
        BrowserError::cdp_with_diagnostics(kind, source, self.diagnostics.snapshot())
    }
}

#[derive(Clone, Copy)]
enum MissingScreenshot {
    ReusePrevious,
    RetryOnce,
}

async fn drive_handler(mut handler: Handler) -> Result<(), CdpError> {
    while let Some(event) = handler.next().await {
        event?;
    }
    Ok(())
}

fn handler_exit_error(result: Result<(), CdpError>, diagnostics: Option<Box<str>>) -> BrowserError {
    match result {
        Ok(()) => BrowserError::process(
            BrowserErrorKind::Handler,
            "headless-shell protocol handler exited unexpectedly",
            diagnostics,
        ),
        Err(source) => {
            BrowserError::cdp_with_diagnostics(BrowserErrorKind::Handler, source, diagnostics)
        }
    }
}

async fn wait_for_next_poll(deadline: Instant) -> Result<(), BrowserError> {
    timeout_at(deadline, sleep(READINESS_POLL_INTERVAL))
        .await
        .map_err(|_| BrowserError::without_source(BrowserErrorKind::RuntimeHost))
}

fn browser_profile() -> Result<TempDir, BrowserError> {
    tempfile::Builder::new()
        .prefix("onmark-chromium-")
        .tempdir()
        .map_err(|source| BrowserError::io(BrowserErrorKind::Profile, source))
}

fn handler_config(render_profile: RenderProfile, limits: BrowserLimits) -> HandlerConfig {
    HandlerConfig {
        ignore_https_errors: true,
        ignore_invalid_messages: false,
        viewport: Some(Viewport {
            width: render_profile.width(),
            height: render_profile.height(),
            device_scale_factor: Some(1.0),
            emulating_mobile: false,
            is_landscape: render_profile.width() >= render_profile.height(),
            has_touch: false,
        }),
        context_ids: Vec::new(),
        request_timeout: limits.deadline(),
        request_intercept: false,
        cache_enabled: false,
    }
}

fn render_target() -> Result<CreateTargetParams, String> {
    CreateTargetParams::builder()
        .url("about:blank")
        .enable_begin_frame_control(true)
        .build()
}

fn begin_frame_parameters(frame: WireFrame, frame_rate: WireFrameRate) -> BeginFrameParams {
    capture_parameters(frame, frame_rate, 0.0)
}

fn screenshot_retry_parameters(frame: WireFrame, frame_rate: WireFrameRate) -> BeginFrameParams {
    capture_parameters(frame, frame_rate, compositor_offset_millis(frame_rate))
}

fn capture_parameters(
    frame: WireFrame,
    frame_rate: WireFrameRate,
    time_offset_millis: f64,
) -> BeginFrameParams {
    let frame_time_ticks = compositor_time_millis(frame, frame_rate) + time_offset_millis;
    let screenshot = ScreenshotParams::builder()
        .format(ScreenshotParamsFormat::Png)
        .optimize_for_speed(true)
        .build();

    BeginFrameParams::builder()
        .frame_time_ticks(frame_time_ticks)
        .interval(frame_interval_millis(frame_rate))
        .screenshot(screenshot)
        .build()
}

fn surface_initialization_parameters(frame_rate: WireFrameRate) -> BeginFrameParams {
    BeginFrameParams::builder()
        .frame_time_ticks(SURFACE_INITIALIZATION_TIME_MILLIS)
        .interval(frame_interval_millis(frame_rate))
        .no_display_updates(false)
        .build()
}

fn staged_placement_parameters(frame: WireFrame, frame_rate: WireFrameRate) -> BeginFrameParams {
    let frame_time_ticks =
        compositor_time_millis(frame, frame_rate) - compositor_offset_millis(frame_rate);
    BeginFrameParams::builder()
        .frame_time_ticks(frame_time_ticks)
        .interval(frame_interval_millis(frame_rate))
        .no_display_updates(false)
        .build()
}

#[allow(clippy::cast_precision_loss)]
fn frame_time_millis(frame: WireFrame, frame_rate: WireFrameRate) -> f64 {
    frame.get() as f64 * f64::from(frame_rate.denominator()) * 1_000.0
        / f64::from(frame_rate.numerator())
}

fn frame_interval_millis(frame_rate: WireFrameRate) -> f64 {
    f64::from(frame_rate.denominator()) * 1_000.0 / f64::from(frame_rate.numerator())
}

fn compositor_time_millis(frame: WireFrame, frame_rate: WireFrameRate) -> f64 {
    COMPOSITOR_BASE_TIME_MILLIS + frame_time_millis(frame, frame_rate)
}

fn compositor_offset_millis(frame_rate: WireFrameRate) -> f64 {
    (frame_interval_millis(frame_rate) / 4.0).min(MAX_COMPOSITOR_OFFSET_MILLIS)
}

fn maximum_base64_length(decoded_bytes: usize) -> usize {
    decoded_bytes.div_ceil(3).saturating_mul(4)
}

async fn cleanup_failed_launch(
    browser: &mut Browser,
    handler: JoinHandle<Result<(), CdpError>>,
    process: &mut ChromiumProcess,
    deadline: Duration,
) {
    let _ = timeout(deadline, browser.close()).await;
    process.abort(deadline).await;
    handler.abort();
    let _ = handler.await;
}

async fn close_browser(
    browser: &mut Browser,
    deadline: Duration,
    diagnostics: &BrowserDiagnostics,
) -> Result<(), BrowserError> {
    timeout(deadline, browser.close())
        .await
        .map_err(|_| {
            BrowserError::process(
                BrowserErrorKind::Shutdown,
                "CDP browser close missed its deadline",
                diagnostics.snapshot(),
            )
        })?
        .map(|_| ())
        .map_err(|source| {
            BrowserError::cdp_with_diagnostics(
                BrowserErrorKind::Shutdown,
                source,
                diagnostics.snapshot(),
            )
        })
}

async fn shutdown_handler(
    mut handler: JoinHandle<Result<(), CdpError>>,
    deadline: Duration,
    diagnostics: &BrowserDiagnostics,
) -> Result<(), BrowserError> {
    let Ok(joined) = timeout(deadline, &mut handler).await else {
        handler.abort();
        let _ = handler.await;
        return Err(BrowserError::process(
            BrowserErrorKind::HandlerTimeout,
            "CDP handler missed its cleanup deadline",
            diagnostics.snapshot(),
        ));
    };
    let handler_result =
        joined.map_err(|source| BrowserError::join(BrowserErrorKind::Handler, source))?;
    handler_result.map_err(|source| {
        BrowserError::cdp_with_diagnostics(
            BrowserErrorKind::Handler,
            source,
            diagnostics.snapshot(),
        )
    })
}

#[cfg(test)]
mod tests {
    use chromiumoxide::error::CdpError;
    use onmark_core::model::FrameRate;
    use onmark_core::protocol::{WireFrame, WireFrameRate};

    use super::{
        begin_frame_parameters, handler_exit_error, maximum_base64_length, render_target,
        screenshot_retry_parameters, staged_placement_parameters,
        surface_initialization_parameters,
    };
    use crate::BrowserErrorKind;

    #[test]
    fn render_target_enables_headless_shell_frame_control() {
        let target = render_target().expect("the fixed render target must be valid");

        assert_eq!(target.url, "about:blank");
        assert_eq!(target.enable_begin_frame_control, Some(true));
    }

    #[test]
    fn begin_frame_offsets_authored_time_from_the_capture_baseline() {
        let frame = WireFrame::new(15).expect("the fixture frame is browser-safe");
        let rate = WireFrameRate::from(
            FrameRate::new(30, 1).expect("the fixture rate is canonical and nonzero"),
        );
        let parameters = begin_frame_parameters(frame, rate);
        let screenshot = parameters
            .screenshot
            .expect("every compositor frame must carry its capture");

        assert_eq!(parameters.frame_time_ticks, Some(1_500.0));
        assert_eq!(screenshot.format, Some(super::ScreenshotParamsFormat::Png));
        assert_eq!(screenshot.optimize_for_speed, Some(true));
    }

    #[test]
    fn surface_initialization_is_visual_and_precedes_the_capture_baseline() {
        let rate = WireFrameRate::from(
            FrameRate::new(30, 1).expect("the fixture rate is canonical and nonzero"),
        );
        let parameters = surface_initialization_parameters(rate);

        assert_eq!(parameters.frame_time_ticks, Some(1.0));
        assert_eq!(parameters.no_display_updates, Some(false));
        assert_eq!(parameters.interval, Some(1_000.0 / 30.0));
        assert_eq!(parameters.screenshot, None);
    }

    #[test]
    fn staged_placement_commit_precedes_its_exact_capture_timestamp() {
        let frame = WireFrame::new(15).expect("the fixture frame is browser-safe");
        let rate = WireFrameRate::from(
            FrameRate::new(30, 1).expect("the fixture rate is canonical and nonzero"),
        );
        let commit = staged_placement_parameters(frame, rate);
        let capture = begin_frame_parameters(frame, rate);

        assert_eq!(commit.frame_time_ticks, Some(1_499.999));
        assert_eq!(commit.no_display_updates, Some(false));
        assert_eq!(commit.screenshot, None);
        assert_eq!(capture.frame_time_ticks, Some(1_500.0));
    }

    #[test]
    fn screenshot_retry_advances_the_compositor_by_a_bounded_epsilon() {
        let frame = WireFrame::new(15).expect("the fixture frame is browser-safe");
        let rate = WireFrameRate::from(
            FrameRate::new(30, 1).expect("the fixture rate is canonical and nonzero"),
        );
        let initial = begin_frame_parameters(frame, rate);
        let retry = screenshot_retry_parameters(frame, rate);

        assert_eq!(initial.frame_time_ticks, Some(1_500.0));
        assert_eq!(retry.frame_time_ticks, Some(1_500.001));
        assert_eq!(retry.interval, initial.interval);
        assert_eq!(retry.screenshot, initial.screenshot);
    }

    #[test]
    fn compositor_offsets_remain_between_adjacent_high_rate_frames() {
        let previous = WireFrame::new(14).expect("the fixture frame is browser-safe");
        let frame = WireFrame::new(15).expect("the fixture frame is browser-safe");
        let next = WireFrame::new(16).expect("the fixture frame is browser-safe");
        let rate = WireFrameRate::from(
            FrameRate::new(u32::MAX, 1).expect("the fixture rate is canonical and nonzero"),
        );
        let previous_capture = compositor_time(&begin_frame_parameters(previous, rate));
        let commit = compositor_time(&staged_placement_parameters(frame, rate));
        let capture = compositor_time(&begin_frame_parameters(frame, rate));
        let retry = compositor_time(&screenshot_retry_parameters(frame, rate));
        let next_capture = compositor_time(&begin_frame_parameters(next, rate));

        assert!(previous_capture < commit);
        assert!(commit < capture);
        assert!(capture < retry);
        assert!(retry < next_capture);
    }

    fn compositor_time(parameters: &super::BeginFrameParams) -> f64 {
        parameters
            .frame_time_ticks
            .expect("frame parameters carry compositor time")
    }

    #[test]
    fn bounds_the_base64_envelope_before_allocating_decoded_bytes() {
        assert_eq!(maximum_base64_length(1), 4);
        assert_eq!(maximum_base64_length(3), 4);
        assert_eq!(maximum_base64_length(4), 8);
    }

    #[test]
    fn reports_a_protocol_handler_failure_during_browser_startup() {
        let error = handler_exit_error(Err(CdpError::NoResponse), None);

        assert_eq!(error.kind(), BrowserErrorKind::Handler);
        assert!(std::error::Error::source(&error).is_some());
    }

    #[test]
    fn reports_an_unexpected_clean_handler_exit_during_browser_startup() {
        let error = handler_exit_error(Ok(()), None);

        assert_eq!(error.kind(), BrowserErrorKind::Handler);
        assert!(std::error::Error::source(&error).is_none());
    }
}
