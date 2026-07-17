//! Owned headless-shell process with bounded, continuously drained diagnostics.
//!
//! Chromium writes its `DevTools` endpoint and later crash diagnostics to the
//! same stderr stream. Onmark therefore keeps that stream open for the entire
//! process lifetime instead of letting the CDP transport consume only startup.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt as _};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::error::{BrowserError, BrowserErrorKind};
use crate::RenderProfile;

const STDERR_LIMIT: usize = 64 * 1024;
const STDERR_READ_BUFFER: usize = 8 * 1024;
const DEVTOOLS_MARKER: &str = "DevTools listening on ";
const STANDARD_ARGUMENTS: &[&str] = &[
    "--allow-file-access-from-files",
    "--disable-background-networking",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-breakpad",
    "--disable-component-extensions-with-background-pages",
    "--disable-default-apps",
    "--disable-extensions",
    "--disable-features=TranslateUI",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-renderer-backgrounding",
    "--disable-sync",
    "--enable-automation",
    "--force-color-profile=srgb",
    "--hide-scrollbars",
    "--lang=en_US",
    "--metrics-recording-only",
    "--mute-audio",
    "--no-first-run",
    "--password-store=basic",
    "--remote-debugging-port=0",
    "--run-all-compositor-stages-before-draw",
    "--use-mock-keychain",
];
const DISABLED_SANDBOX_ARGUMENTS: &[&str] = &["--no-sandbox", "--disable-setuid-sandbox"];
const CAPTURE_BACKEND_ARGUMENTS: &[&str] = &[
    "--ignore-gpu-blocklist",
    "--use-gl=angle",
    "--use-angle=swiftshader",
    "--enable-unsafe-swiftshader",
];
const SINGLE_PROCESS_ARGUMENTS: &[&str] = &[
    "--disable-dev-shm-usage",
    "--single-process",
    "--no-zygote",
    "--in-process-gpu",
];

/// Environment-owned Chromium launch policy.
///
/// The policy keeps process isolation and process topology together as one
/// reviewed choice. Launch failures never weaken it automatically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserLaunchPolicy {
    sandbox: ChromiumSandbox,
    process_model: ChromiumProcessModel,
}

impl BrowserLaunchPolicy {
    /// Retains Chromium's sandbox and normal multi-process topology.
    #[must_use]
    pub const fn local() -> Self {
        Self {
            sandbox: ChromiumSandbox::Enabled,
            process_model: ChromiumProcessModel::Standard,
        }
    }

    /// Uses an audited outer sandbox and a constrained single-process topology.
    ///
    /// This policy is suitable for isolated function workers that cannot host
    /// Chromium's zygote or GPU subprocess sandboxes. It keeps `SwiftShader` in
    /// process instead of disabling the graphics stack.
    #[must_use]
    pub const fn isolated_worker() -> Self {
        Self {
            sandbox: ChromiumSandbox::Disabled,
            process_model: ChromiumProcessModel::SingleProcess,
        }
    }

    const fn disables_chromium_sandbox(self) -> bool {
        matches!(self.sandbox, ChromiumSandbox::Disabled)
    }

    const fn uses_single_process(self) -> bool {
        matches!(self.process_model, ChromiumProcessModel::SingleProcess)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChromiumSandbox {
    Enabled,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChromiumProcessModel {
    Standard,
    SingleProcess,
}

/// One headless-shell child and the task that drains its diagnostics.
#[derive(Debug)]
pub(super) struct ChromiumProcess {
    child: Child,
    stderr: JoinHandle<std::io::Result<()>>,
    diagnostics: BrowserDiagnostics,
}

impl ChromiumProcess {
    pub(super) async fn launch(
        executable: &Path,
        launch_policy: BrowserLaunchPolicy,
        profile: &Path,
        render_profile: RenderProfile,
        deadline: Duration,
    ) -> Result<(Self, String), BrowserError> {
        let executable = tokio::fs::canonicalize(executable)
            .await
            .map_err(|source| BrowserError::io(BrowserErrorKind::Launch, source))?;
        let mut child = spawn_headless_shell(&executable, launch_policy, profile, render_profile)?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BrowserError::without_source(BrowserErrorKind::Launch))?;
        let diagnostics = BrowserDiagnostics::default();
        let (endpoint_sender, endpoint_receiver) = oneshot::channel();
        let stderr_task = tokio::spawn(drain_stderr(stderr, diagnostics.clone(), endpoint_sender));
        let mut process = Self {
            child,
            stderr: stderr_task,
            diagnostics,
        };

        let endpoint = match timeout(deadline, endpoint_receiver).await {
            Ok(Ok(endpoint)) => endpoint,
            Ok(Err(_)) => {
                process.abort(deadline).await;
                return Err(process.failure(
                    BrowserErrorKind::Launch,
                    "headless shell closed stderr before publishing its DevTools endpoint",
                ));
            }
            Err(_) => {
                process.abort(deadline).await;
                return Err(process.failure(
                    BrowserErrorKind::Launch,
                    "DevTools endpoint missed its launch deadline",
                ));
            }
        };

        Ok((process, endpoint))
    }

    pub(super) fn diagnostics(&self) -> BrowserDiagnostics {
        self.diagnostics.clone()
    }

    pub(super) async fn shutdown(mut self, deadline: Duration) -> Result<(), BrowserError> {
        let status = match timeout(deadline, self.child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(source)) => {
                self.force_stop(deadline).await;
                let _ = self.finish_stderr(deadline).await;
                return Err(BrowserError::io(BrowserErrorKind::Shutdown, source));
            }
            Err(_) => {
                self.force_stop(deadline).await;
                let _ = self.finish_stderr(deadline).await;
                return Err(self.failure(
                    BrowserErrorKind::Shutdown,
                    "headless shell missed its shutdown deadline",
                ));
            }
        };
        self.finish_stderr(deadline).await?;
        if status.success() {
            return Ok(());
        }
        Err(self.exit_error(BrowserErrorKind::Shutdown, status))
    }

    pub(super) async fn force_stop(&mut self, deadline: Duration) {
        let _ = self.child.kill().await;
        let _ = timeout(deadline, self.child.wait()).await;
    }

    pub(super) fn request_stop(&mut self) {
        let _ = self.child.start_kill();
    }

    pub(super) async fn abort(&mut self, deadline: Duration) {
        self.force_stop(deadline).await;
        if timeout(deadline, &mut self.stderr).await.is_err() {
            self.stderr.abort();
            let _ = (&mut self.stderr).await;
        }
    }

    async fn finish_stderr(&mut self, deadline: Duration) -> Result<(), BrowserError> {
        let Ok(joined) = timeout(deadline, &mut self.stderr).await else {
            self.stderr.abort();
            let _ = (&mut self.stderr).await;
            return Err(self.failure(
                BrowserErrorKind::Shutdown,
                "browser diagnostics missed their shutdown deadline",
            ));
        };
        let drained =
            joined.map_err(|source| BrowserError::join(BrowserErrorKind::Shutdown, source))?;
        drained.map_err(|source| BrowserError::io(BrowserErrorKind::Shutdown, source))
    }

    fn exit_error(&self, kind: BrowserErrorKind, status: ExitStatus) -> BrowserError {
        self.failure(kind, format!("headless shell exited with {status}"))
    }

    fn failure(&self, kind: BrowserErrorKind, message: impl Into<String>) -> BrowserError {
        BrowserError::process(kind, message, self.diagnostics.snapshot())
    }
}

/// Shared bounded stderr tail available while CDP commands are still active.
#[derive(Clone, Debug, Default)]
pub(super) struct BrowserDiagnostics(Arc<Mutex<VecDeque<u8>>>);

impl BrowserDiagnostics {
    pub(super) fn snapshot(&self) -> Option<Box<str>> {
        let mut retained = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if retained.is_empty() {
            return None;
        }
        Some(
            String::from_utf8_lossy(retained.make_contiguous())
                .into_owned()
                .into_boxed_str(),
        )
    }

    fn retain(&self, bytes: &[u8]) {
        let mut retained = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if bytes.len() >= STDERR_LIMIT {
            retained.clear();
            retained.extend(&bytes[bytes.len() - STDERR_LIMIT..]);
            return;
        }

        let overflow = retained
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(STDERR_LIMIT);
        let retained_len = retained.len();
        retained.drain(..overflow.min(retained_len));
        retained.extend(bytes);
    }

    fn devtools_endpoint(&self) -> Option<String> {
        let mut retained = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let stderr = String::from_utf8_lossy(retained.make_contiguous());
        find_devtools_endpoint(&stderr)
    }
}

fn spawn_headless_shell(
    executable: &Path,
    launch_policy: BrowserLaunchPolicy,
    profile: &Path,
    render_profile: RenderProfile,
) -> Result<Child, BrowserError> {
    let mut command = Command::new(executable);
    command
        .args(browser_arguments(launch_policy, profile, render_profile))
        .envs(sidecar_environment(executable))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| BrowserError::io(BrowserErrorKind::Launch, source))
}

fn sidecar_environment(executable: &Path) -> Vec<(&'static str, PathBuf)> {
    let Some(root) = executable.parent() else {
        return Vec::new();
    };
    let mut environment = Vec::with_capacity(3);
    let libraries = root.join("lib");
    if libraries.is_dir() {
        environment.push(("LD_LIBRARY_PATH", libraries));
    }
    let vulkan = root.join("vk_swiftshader_icd.json");
    if vulkan.is_file() {
        environment.push(("VK_ICD_FILENAMES", vulkan));
    }
    let fonts = root.join("fonts.conf");
    if fonts.is_file() {
        environment.push(("FONTCONFIG_FILE", fonts));
    }
    environment
}

fn browser_arguments(
    launch_policy: BrowserLaunchPolicy,
    profile: &Path,
    render_profile: RenderProfile,
) -> Vec<OsString> {
    let mut arguments: Vec<OsString> = STANDARD_ARGUMENTS.iter().map(OsString::from).collect();
    arguments.extend(CAPTURE_BACKEND_ARGUMENTS.iter().map(OsString::from));
    if launch_policy.disables_chromium_sandbox() {
        arguments.extend(DISABLED_SANDBOX_ARGUMENTS.iter().map(OsString::from));
    }
    if launch_policy.uses_single_process() {
        arguments.extend(SINGLE_PROCESS_ARGUMENTS.iter().map(OsString::from));
    }
    arguments.push(window_size(render_profile));
    arguments.push(format!("--user-data-dir={}", profile.display()).into());
    arguments
}

fn window_size(render_profile: RenderProfile) -> OsString {
    format!(
        "--window-size={},{}",
        render_profile.width(),
        render_profile.height(),
    )
    .into()
}

async fn drain_stderr(
    mut stderr: impl AsyncRead + Unpin,
    diagnostics: BrowserDiagnostics,
    endpoint_sender: oneshot::Sender<String>,
) -> std::io::Result<()> {
    let mut endpoint_sender = Some(endpoint_sender);
    let mut buffer = [0_u8; STDERR_READ_BUFFER];

    loop {
        let count = stderr.read(&mut buffer).await?;
        if count == 0 {
            return Ok(());
        }
        diagnostics.retain(&buffer[..count]);
        report_devtools_endpoint(&diagnostics, &mut endpoint_sender);
    }
}

fn report_devtools_endpoint(
    diagnostics: &BrowserDiagnostics,
    sender: &mut Option<oneshot::Sender<String>>,
) {
    if sender.is_none() {
        return;
    }
    let Some(endpoint) = diagnostics.devtools_endpoint() else {
        return;
    };
    let Some(sender) = sender.take() else {
        return;
    };
    let _ = sender.send(endpoint);
}

fn find_devtools_endpoint(stderr: &str) -> Option<String> {
    let start = stderr.rfind(DEVTOOLS_MARKER)? + DEVTOOLS_MARKER.len();
    let line = &stderr[start..];
    let end = line.find(['\r', '\n'])?;
    let endpoint = line[..end].trim();
    endpoint.starts_with("ws://").then(|| endpoint.to_owned())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::{
        BrowserDiagnostics, browser_arguments, find_devtools_endpoint, sidecar_environment,
    };
    use crate::{BrowserLaunchPolicy, RenderProfile};

    #[test]
    fn finds_the_latest_complete_devtools_endpoint() {
        let diagnostics =
            "warning\nDevTools listening on ws://127.0.0.1:9/devtools/browser/id\nmore\n";

        assert_eq!(
            find_devtools_endpoint(diagnostics).as_deref(),
            Some("ws://127.0.0.1:9/devtools/browser/id"),
        );
        assert_eq!(find_devtools_endpoint("DevTools listening on "), None);
        assert_eq!(
            find_devtools_endpoint("DevTools listening on ws://127.0.0.1:9/devtools/brow"),
            None,
        );
    }

    #[test]
    fn retains_only_the_bounded_browser_diagnostic_tail() {
        let diagnostics = BrowserDiagnostics::default();
        diagnostics.retain(&vec![b'a'; super::STDERR_LIMIT]);
        diagnostics.retain(b"fatal");

        let retained = diagnostics
            .snapshot()
            .expect("diagnostics remain available");
        assert_eq!(retained.len(), super::STDERR_LIMIT);
        assert!(retained.ends_with("fatal"));
    }

    #[test]
    fn isolated_worker_arguments_keep_graphics_in_the_browser_process() {
        let profile = RenderProfile::new(320, 180).expect("the fixture profile is valid");
        let arguments = browser_arguments(
            BrowserLaunchPolicy::isolated_worker(),
            Path::new("/tmp/p"),
            profile,
        );

        assert!(has_argument(&arguments, "--remote-debugging-port=0"));
        assert!(has_argument(
            &arguments,
            "--run-all-compositor-stages-before-draw"
        ));
        assert!(has_argument(&arguments, "--no-sandbox"));
        assert!(has_argument(&arguments, "--single-process"));
        assert!(has_argument(&arguments, "--in-process-gpu"));
        assert!(has_argument(&arguments, "--use-angle=swiftshader"));
        assert!(has_argument(&arguments, "--disable-dev-shm-usage"));
    }

    #[test]
    fn local_arguments_lock_software_rendering_without_changing_process_model() {
        let profile = RenderProfile::new(320, 180).expect("the fixture profile is valid");
        let arguments =
            browser_arguments(BrowserLaunchPolicy::local(), Path::new("/tmp/p"), profile);

        assert!(has_argument(&arguments, "--use-angle=swiftshader"));
        assert!(has_argument(&arguments, "--enable-unsafe-swiftshader"));
        assert!(!has_argument(&arguments, "--no-sandbox"));
        assert!(!has_argument(&arguments, "--single-process"));
        assert!(!has_argument(&arguments, "--in-process-gpu"));
        assert!(!has_argument(&arguments, "--disable-dev-shm-usage"));
    }

    #[test]
    fn scopes_sidecar_paths_to_the_browser_child() {
        let root = TempDir::new().expect("the fixture root is writable");
        fs::create_dir(root.path().join("lib")).expect("the sidecar library root is writable");
        fs::write(root.path().join("vk_swiftshader_icd.json"), b"{}")
            .expect("the sidecar Vulkan manifest is writable");
        fs::write(root.path().join("fonts.conf"), b"<fontconfig/>")
            .expect("the sidecar font configuration is writable");

        let environment = sidecar_environment(&root.path().join("chrome-headless-shell"));

        assert_eq!(
            environment,
            vec![
                ("LD_LIBRARY_PATH", root.path().join("lib")),
                (
                    "VK_ICD_FILENAMES",
                    root.path().join("vk_swiftshader_icd.json"),
                ),
                ("FONTCONFIG_FILE", root.path().join("fonts.conf")),
            ],
        );
    }

    fn has_argument(arguments: &[OsString], expected: &str) -> bool {
        arguments.iter().any(|argument| argument == expected)
    }
}
