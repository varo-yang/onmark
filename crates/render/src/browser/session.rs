//! Owned Chromium/CDP session with bounded startup, command, and shutdown work.
//!
//! This module contains vendor-specific control flow so the executor can speak
//! only in Onmark protocol values and typed browser failures.

use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chromiumoxide::browser::Browser;
use chromiumoxide::cdp::browser_protocol::dom::Rgba;
use chromiumoxide::cdp::browser_protocol::emulation::SetDefaultBackgroundColorOverrideParams;
use chromiumoxide::cdp::browser_protocol::headless_experimental::{
    BeginFrameParams, BeginFrameReturns,
};
use chromiumoxide::cdp::browser_protocol::page::{
    BringToFrontParams, CaptureScreenshotFormat, CaptureScreenshotParams,
};
use chromiumoxide::cdp::browser_protocol::system_info::GetInfoParams;
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
use url::Url;

use super::compositor::{CompositorClock, CompositorTransaction};
use super::error::{BrowserError, BrowserErrorKind};
use super::frame::{CapturedFrame, EncodedPng};
use super::limits::BrowserLimits;
use super::process::{
    BrowserCaptureMode, BrowserDiagnostics, BrowserGraphicsBackend, BrowserLaunchPolicy,
    ChromiumProcess,
};
use super::resource::ResourceGuard;
use crate::RenderProfile;

const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Immutable controls for one browser process and render surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserSessionOptions {
    /// Process isolation and sandbox policy owned by the execution host.
    pub launch_policy: BrowserLaunchPolicy,
    /// Graphics implementation admitted for the capture environment.
    pub graphics_backend: BrowserGraphicsBackend,
    /// Chromium surface-commit mechanism selected for the executable.
    pub capture_mode: BrowserCaptureMode,
    /// Exact viewport dimensions owned by the render plan.
    pub render_profile: RenderProfile,
    /// Bounded process deadline and retained capture bytes.
    pub limits: BrowserLimits,
}

/// One owned headless browser process and its single render page.
#[derive(Debug)]
pub struct BrowserSession {
    browser: Browser,
    page: Page,
    handler: JoinHandle<Result<(), CdpError>>,
    process: ChromiumProcess,
    diagnostics: BrowserDiagnostics,
    resources: Option<ResourceGuard>,
    // Headless shell omits screenshotData when a frame has no visual damage.
    // The capture phase owns this cache through `&mut self`; it is not shared
    // across requests or tasks.
    last_capture: Option<EncodedPng>,
    capture_commands: u64,
    capture_mode: BrowserCaptureMode,
    compositor: CompositorClock,
    limits: BrowserLimits,
    render_profile: RenderProfile,
    // Retained so the browser's private profile outlives the process.
    _profile: TempDir,
}

impl BrowserSession {
    pub(crate) const fn render_profile(&self) -> RenderProfile {
        self.render_profile
    }

    pub(crate) const fn capture_commands(&self) -> u64 {
        self.capture_commands
    }

    /// Launches a bounded headless browser session using an explicit executable.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when configuration, process launch, CDP handler
    /// startup, or initial page creation fails.
    pub async fn launch(
        executable: impl AsRef<Path>,
        options: BrowserSessionOptions,
    ) -> Result<Self, BrowserError> {
        let BrowserSessionOptions {
            launch_policy,
            graphics_backend,
            capture_mode,
            render_profile,
            limits,
        } = options;
        let target = render_target(capture_mode).map_err(BrowserError::configuration)?;
        let profile = browser_profile()?;
        let (mut process, endpoint) = ChromiumProcess::launch(
            executable.as_ref(),
            launch_policy,
            graphics_backend,
            capture_mode,
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
        if let Err(error) =
            validate_graphics_backend(&browser, graphics_backend, diagnostics.clone()).await
        {
            cleanup_failed_launch(&mut browser, handler, &mut process, limits.deadline()).await;
            return Err(error);
        }

        Ok(Self {
            browser,
            page,
            handler,
            process,
            diagnostics,
            resources: None,
            last_capture: None,
            capture_commands: 0,
            capture_mode,
            compositor: CompositorClock::new(),
            limits,
            render_profile,
            _profile: profile,
        })
    }

    /// Restricts the page to one private resource root, navigates it, and waits
    /// for the runtime host to become ready.
    ///
    /// A local sequence may reuse the Chromium process across independent
    /// units. Each navigation first retires the preceding root policy and
    /// capture cache; bytes from one unit therefore cannot satisfy another.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when the resource policy cannot be installed,
    /// Chrome rejects navigation, the load event misses its deadline, or the
    /// bundle never installs its runtime host.
    pub async fn navigate(&mut self, url: &Url, resource_root: &Path) -> Result<(), BrowserError> {
        if let Some(resources) = self.resources.take() {
            resources.stop().await?;
        }
        self.last_capture = None;
        self.resources = Some(ResourceGuard::install(&self.page, resource_root).await?);
        self.page
            .goto(url.as_str())
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
    /// The first navigation uses a fixed pre-baseline timestamp. Reused targets
    /// take a later tick from the same session clock, so navigation never
    /// rewinds Chromium's compositor time.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when the compositor frame cannot complete.
    pub async fn initialize_capture_surface(
        &mut self,
        frame_rate: WireFrameRate,
    ) -> Result<(), BrowserError> {
        match self.capture_mode {
            BrowserCaptureMode::BeginFrame => {
                let parameters = self.compositor.initialize(frame_rate);
                self.initialize_begin_frame_surface(parameters).await
            }
            BrowserCaptureMode::Screenshot => self.activate_portable_surface().await,
        }
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
    /// The runtime has already positioned authored time. This method advances
    /// only the session-owned compositor transaction clock, then asks headless
    /// shell to commit and capture in one CDP command.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when capture fails or exceeds the configured
    /// retained-byte budget.
    pub async fn capture_png(
        &mut self,
        frame: WireFrame,
        frame_rate: WireFrameRate,
    ) -> Result<EncodedPng, BrowserError> {
        if self.capture_mode == BrowserCaptureMode::Screenshot {
            return self.capture_screenshot().await;
        }
        let transaction = self.compositor.begin(frame, frame_rate);
        self.capture_begin_frame(transaction, MissingScreenshot::ReusePrevious)
            .await
    }

    pub(crate) async fn capture_png_after_surface_change(
        &mut self,
        frame: WireFrame,
        frame_rate: WireFrameRate,
    ) -> Result<EncodedPng, BrowserError> {
        if self.capture_mode == BrowserCaptureMode::Screenshot {
            return self.capture_screenshot().await;
        }
        let transaction = self.compositor.begin(frame, frame_rate);
        // Runtime staging may introduce a layer that was absent from the
        // compositor. Commit it immediately before this capture transaction so
        // the new surface is visible without changing authored time.
        self.page
            .execute(transaction.surface_commit_parameters())
            .await
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))?;
        self.capture_begin_frame(transaction, MissingScreenshot::RetryOnce)
            .await
    }

    /// Reconciles a freshly staged surface after media confirmation.
    ///
    /// The exact capture is retained when Chromium reports no further damage.
    /// If confirmation allowed a pending layer to settle, the next bounded
    /// compositor tick replaces it with the now-confirmed output.
    pub(crate) async fn recapture_png_after_confirmation(
        &mut self,
        frame: WireFrame,
    ) -> Result<EncodedPng, BrowserError> {
        if self.capture_mode == BrowserCaptureMode::Screenshot {
            return self.capture_screenshot().await;
        }
        let transaction = self.compositor.active_for(frame).ok_or_else(|| {
            BrowserError::capture_pixels(
                "confirmed frame does not match the active compositor transaction",
            )
        })?;
        let response = self
            .capture(transaction.reconciliation_parameters())
            .await?;
        if let Some(screenshot) = response.screenshot_data {
            return self.decode_and_remember(screenshot);
        }
        self.last_capture.clone().ok_or_else(|| {
            BrowserError::capture_pixels("headless shell lost the confirmed boundary capture")
        })
    }

    async fn capture_begin_frame(
        &mut self,
        transaction: CompositorTransaction,
        missing: MissingScreenshot,
    ) -> Result<EncodedPng, BrowserError> {
        let response = self.capture(transaction.capture_parameters()).await?;
        if let Some(screenshot) = response.screenshot_data {
            return self.decode_and_remember(screenshot);
        }
        let previous = match missing {
            MissingScreenshot::ReusePrevious => self.last_capture.clone(),
            MissingScreenshot::RetryOnce => None,
        };
        if let Some(previous) = previous {
            return Ok(previous);
        }

        let retry = self.capture(transaction.retry_parameters()).await?;
        let screenshot = retry.screenshot_data.ok_or_else(|| {
            BrowserError::capture_pixels("headless shell did not return the required screenshot")
        })?;
        self.decode_and_remember(screenshot)
    }

    async fn capture(
        &mut self,
        parameters: BeginFrameParams,
    ) -> Result<BeginFrameReturns, BrowserError> {
        self.record_capture_command();
        self.page
            .execute(parameters)
            .await
            .map(|response| response.result)
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))
    }

    async fn initialize_begin_frame_surface(
        &self,
        parameters: BeginFrameParams,
    ) -> Result<(), BrowserError> {
        self.page
            .execute(parameters)
            .await
            .map(|_| ())
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))
    }

    async fn activate_portable_surface(&self) -> Result<(), BrowserError> {
        self.page
            .execute(BringToFrontParams::default())
            .await
            .map(|_| ())
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))
    }

    async fn capture_screenshot(&mut self) -> Result<EncodedPng, BrowserError> {
        self.record_capture_command();
        let response = self
            .page
            .execute(portable_screenshot_parameters())
            .await
            .map_err(|source| self.cdp_error(BrowserErrorKind::Capture, source))?;
        self.decode_and_remember(response.result.data)
    }

    fn decode_and_remember(
        &mut self,
        screenshot: impl AsRef<str>,
    ) -> Result<EncodedPng, BrowserError> {
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
        self.last_capture = Some(capture.clone());
        Ok(capture)
    }

    fn record_capture_command(&mut self) {
        // Telemetry must never become a rendering failure after the validated
        // frame limit has already bounded useful work.
        self.capture_commands = self.capture_commands.saturating_add(1);
    }

    /// Captures one encoder PNG together with canonical raw-RGBA evidence.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when Chromium cannot capture the viewport, the
    /// retained PNG exceeds its bound, or the decoded pixels do not match the
    /// configured render profile.
    pub async fn capture_frame(
        &mut self,
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
        let resource_result = match self.resources.take() {
            Some(resources) => resources.stop().await,
            None => Ok(()),
        };
        let browser_result = close_browser(&mut self.browser, deadline, &self.diagnostics).await;
        if browser_result.is_err() {
            self.process.request_stop();
        }
        let process_result = self.process.shutdown(deadline).await;
        let handler_result = shutdown_handler(self.handler, deadline, &self.diagnostics).await;

        browser_result?;
        resource_result?;
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

async fn validate_graphics_backend(
    browser: &Browser,
    requested: BrowserGraphicsBackend,
    diagnostics: BrowserDiagnostics,
) -> Result<(), BrowserError> {
    let response = browser
        .execute(GetInfoParams::default())
        .await
        .map_err(|source| {
            BrowserError::cdp_with_diagnostics(
                BrowserErrorKind::GraphicsBackend,
                source,
                diagnostics.snapshot(),
            )
        })?
        .result;
    let renderer = response
        .gpu
        .aux_attributes
        .as_ref()
        .and_then(|attributes| attributes.get("glRenderer"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            BrowserError::process(
                BrowserErrorKind::GraphicsBackend,
                "Chromium did not report its active GL renderer",
                diagnostics.snapshot(),
            )
        })?;
    if requested.accepts_renderer(renderer) {
        return Ok(());
    }
    Err(BrowserError::process(
        BrowserErrorKind::GraphicsBackend,
        format!(
            "requested {}, but Chromium activated {renderer}",
            requested.label()
        ),
        diagnostics.snapshot(),
    ))
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
            "browser protocol handler exited unexpectedly",
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

fn render_target(capture_mode: BrowserCaptureMode) -> Result<CreateTargetParams, String> {
    let mut target = CreateTargetParams::builder().url("about:blank");
    if capture_mode.uses_begin_frame() {
        target = target.enable_begin_frame_control(true);
    }
    target.build()
}

fn portable_screenshot_parameters() -> CaptureScreenshotParams {
    CaptureScreenshotParams::builder()
        .format(CaptureScreenshotFormat::Png)
        .from_surface(true)
        .capture_beyond_viewport(false)
        .optimize_for_speed(true)
        .build()
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

    use super::{
        handler_exit_error, maximum_base64_length, portable_screenshot_parameters, render_target,
    };
    use crate::{BrowserCaptureMode, BrowserErrorKind};

    #[test]
    fn render_target_enables_headless_shell_frame_control() {
        let target = render_target(BrowserCaptureMode::BeginFrame)
            .expect("the fixed render target must be valid");

        assert_eq!(target.url, "about:blank");
        assert_eq!(target.enable_begin_frame_control, Some(true));
    }

    #[test]
    fn portable_target_uses_page_screenshot_without_begin_frame_control() {
        let target = render_target(BrowserCaptureMode::Screenshot)
            .expect("the portable render target must be valid");
        let screenshot = portable_screenshot_parameters();

        assert_eq!(target.url, "about:blank");
        assert_eq!(target.enable_begin_frame_control, None);
        assert_eq!(screenshot.format, Some(super::CaptureScreenshotFormat::Png));
        assert_eq!(screenshot.from_surface, Some(true));
        assert_eq!(screenshot.capture_beyond_viewport, Some(false));
        assert_eq!(screenshot.optimize_for_speed, Some(true));
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
