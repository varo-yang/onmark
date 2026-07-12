use std::path::Path;
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::error::CdpError;
use chromiumoxide::handler::viewport::Viewport;
use chromiumoxide::page::{Page, ScreenshotParams};
use futures::StreamExt as _;
use onmark_core::protocol::{BrowserRequest, BrowserResponse};
use tempfile::TempDir;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep, timeout, timeout_at};

use super::error::{BrowserError, BrowserErrorKind};
use super::limits::BrowserLimits;

const RUNTIME_HOST: &str = "__ONMARK_RUNTIME__";
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// One PNG frame returned by Chromium within the configured byte budget.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodedPng(Vec<u8>);

impl EncodedPng {
    /// Returns the encoded PNG bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Transfers ownership of the encoded PNG bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

/// One owned Chromium process and its single Gate-one page.
#[derive(Debug)]
pub struct BrowserSession {
    browser: Browser,
    page: Page,
    handler: JoinHandle<Result<(), CdpError>>,
    limits: BrowserLimits,
    // Retained so Chrome's private profile outlives the process.
    _profile: TempDir,
}

impl BrowserSession {
    /// Launches a bounded headless Chromium session using an explicit executable.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when configuration, process launch, CDP handler
    /// startup, or initial page creation fails.
    pub async fn launch(
        executable: impl AsRef<Path>,
        limits: BrowserLimits,
    ) -> Result<Self, BrowserError> {
        let profile = browser_profile()?;
        let config = browser_config(executable.as_ref(), profile.path(), limits)?;
        let (mut browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|source| BrowserError::cdp(BrowserErrorKind::Launch, source))?;
        let handler = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                event?;
            }
            Ok(())
        });

        let page = match browser.new_page("about:blank").await {
            Ok(page) => page,
            Err(source) => {
                cleanup_failed_launch(&mut browser, handler, limits.deadline()).await;
                return Err(BrowserError::cdp(BrowserErrorKind::PageCreation, source));
            }
        };

        Ok(Self {
            browser,
            page,
            handler,
            limits,
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
            .map_err(|source| BrowserError::cdp(BrowserErrorKind::Navigation, source))?;
        timeout(self.limits.deadline(), self.page.wait_for_navigation())
            .await
            .map_err(|_| BrowserError::without_source(BrowserErrorKind::Navigation))?
            .map_err(|source| BrowserError::cdp(BrowserErrorKind::Navigation, source))?;
        self.wait_for_runtime_host().await
    }

    /// Dispatches one typed request through the installed browser runtime host.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when serialization, JavaScript evaluation, or
    /// response decoding fails.
    pub async fn dispatch(
        &self,
        request: &BrowserRequest,
    ) -> Result<BrowserResponse, BrowserError> {
        let request = serde_json::to_string(request)
            .map_err(|source| BrowserError::json(BrowserErrorKind::Protocol, source))?;
        let expression = format!("globalThis.{RUNTIME_HOST}.dispatch({request})");
        let result = self
            .page
            .evaluate_expression(expression)
            .await
            .map_err(|source| BrowserError::cdp(BrowserErrorKind::Protocol, source))?;
        result
            .into_value()
            .map_err(|source| BrowserError::json(BrowserErrorKind::Protocol, source))
    }

    /// Captures the current viewport as PNG without writing it to disk.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserError`] when capture fails or exceeds the configured
    /// retained-byte budget.
    pub async fn capture_png(&self) -> Result<EncodedPng, BrowserError> {
        let parameters = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .from_surface(true)
            .capture_beyond_viewport(false)
            .full_page(false)
            .build();
        let bytes = self
            .page
            .screenshot(parameters)
            .await
            .map_err(|source| BrowserError::cdp(BrowserErrorKind::Capture, source))?;

        if bytes.len() > self.limits.max_capture_bytes() {
            return Err(BrowserError::without_source(
                BrowserErrorKind::CaptureTooLarge,
            ));
        }
        Ok(EncodedPng(bytes))
    }

    /// Closes Chromium and waits for both the process and CDP handler to exit.
    ///
    /// # Errors
    ///
    /// Returns the first observed shutdown failure after all cleanup attempts
    /// have completed.
    pub async fn shutdown(mut self) -> Result<(), BrowserError> {
        let deadline = self.limits.deadline();
        let browser_result = shutdown_browser(&mut self.browser, deadline).await;
        let handler_result = shutdown_handler(self.handler, deadline).await;

        browser_result?;
        handler_result
    }

    async fn wait_for_runtime_host(&self) -> Result<(), BrowserError> {
        let deadline = Instant::now() + self.limits.deadline();
        let expression = format!("typeof globalThis.{RUNTIME_HOST}?.dispatch === \"function\"");

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
        let result = timeout_at(deadline, evaluation)
            .await
            .map_err(|_| BrowserError::without_source(BrowserErrorKind::RuntimeHost))?
            .map_err(|source| BrowserError::cdp(BrowserErrorKind::RuntimeHost, source))?;
        result
            .into_value()
            .map_err(|source| BrowserError::json(BrowserErrorKind::RuntimeHost, source))
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

fn browser_config(
    executable: &Path,
    profile: &Path,
    limits: BrowserLimits,
) -> Result<BrowserConfig, BrowserError> {
    BrowserConfig::builder()
        .chrome_executable(executable)
        .user_data_dir(profile)
        .new_headless_mode()
        .window_size(limits.width(), limits.height())
        .viewport(Viewport {
            width: limits.width(),
            height: limits.height(),
            device_scale_factor: Some(1.0),
            emulating_mobile: false,
            is_landscape: limits.width() >= limits.height(),
            has_touch: false,
        })
        .launch_timeout(limits.deadline())
        .request_timeout(limits.deadline())
        .disable_cache()
        .surface_invalid_messages()
        // Chromiumoxide adds the `--` prefix to argument keys.
        .arg("allow-file-access-from-files")
        .build()
        .map_err(BrowserError::configuration)
}

async fn cleanup_failed_launch(
    browser: &mut Browser,
    handler: JoinHandle<Result<(), CdpError>>,
    deadline: Duration,
) {
    let _ = shutdown_browser(browser, deadline).await;
    handler.abort();
    let _ = handler.await;
}

async fn shutdown_browser(browser: &mut Browser, deadline: Duration) -> Result<(), BrowserError> {
    match timeout(deadline, browser.close()).await {
        Ok(Ok(_)) => finish_graceful_shutdown(browser, deadline).await,
        Ok(Err(source)) => {
            force_stop_browser(browser, deadline).await;
            Err(BrowserError::cdp(BrowserErrorKind::Shutdown, source))
        }
        Err(_) => {
            force_stop_browser(browser, deadline).await;
            Err(BrowserError::without_source(BrowserErrorKind::Shutdown))
        }
    }
}

async fn finish_graceful_shutdown(
    browser: &mut Browser,
    deadline: Duration,
) -> Result<(), BrowserError> {
    let result = wait_for_browser(browser, deadline).await;
    if result.is_err() {
        force_stop_browser(browser, deadline).await;
    }
    result
}

async fn wait_for_browser(browser: &mut Browser, deadline: Duration) -> Result<(), BrowserError> {
    timeout(deadline, browser.wait())
        .await
        .map_err(|_| BrowserError::without_source(BrowserErrorKind::Shutdown))?
        .map(|_| ())
        .map_err(|source| BrowserError::io(BrowserErrorKind::Shutdown, source))
}

async fn force_stop_browser(browser: &mut Browser, deadline: Duration) {
    let _ = timeout(deadline, browser.kill()).await;
    let _ = timeout(deadline, browser.wait()).await;
}

async fn shutdown_handler(
    mut handler: JoinHandle<Result<(), CdpError>>,
    deadline: Duration,
) -> Result<(), BrowserError> {
    match timeout(deadline, &mut handler).await {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(source))) => Err(BrowserError::cdp(BrowserErrorKind::Handler, source)),
        Ok(Err(source)) => Err(BrowserError::join(BrowserErrorKind::Handler, source)),
        Err(_) => {
            handler.abort();
            let _ = handler.await;
            Err(BrowserError::without_source(
                BrowserErrorKind::HandlerTimeout,
            ))
        }
    }
}
