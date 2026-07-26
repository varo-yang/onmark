//! Discovery and validation of host executables required by local rendering.
//!
//! Environment conventions stop here; renderer APIs receive explicit paths and
//! never inspect process-global configuration.

use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use onmark_render::BrowserCaptureMode;

use crate::arguments::{DoctorArgs, RenderArgs, ValidationArgs};
use crate::browser_install::{self, BrowserInstallError};
use crate::bundler::BundlerProcess;

const HEADLESS_SHELL: &str = "chrome-headless-shell";
const BROWSER_PROVISIONER: &str = "ONMARK_BROWSER_PROVISIONER";
const BROWSER_PROVISIONER_ENTRY: &str = "ONMARK_BROWSER_PROVISIONER_ENTRY";
const BUNDLER_ENTRY: &str = "ONMARK_BUNDLER_ENTRY";

#[cfg(target_os = "macos")]
const MACOS_BROWSERS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
];

/// Validated browser, `FFmpeg`, ffprobe, and bundler paths for one command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Executables {
    pub(super) browser: BrowserExecutable,
    pub(super) bundler: BundlerProcess,
    pub(super) ffmpeg: PathBuf,
    pub(super) ffprobe: PathBuf,
}

impl Executables {
    pub(super) async fn discover(args: &RenderArgs) -> Result<Self, EnvironmentError> {
        let browser = browser(args.browser.as_deref()).await?;
        let bundler = bundler(&args.bundler)?;
        let ffmpeg = locate("FFmpeg", &args.ffmpeg)?;
        let ffprobe = locate("ffprobe", &args.ffprobe)?;

        Ok(Self {
            browser,
            bundler,
            ffmpeg,
            ffprobe,
        })
    }
}

/// Validated tools needed to compile and plan without browser execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CheckExecutables {
    pub(super) bundler: BundlerProcess,
    pub(super) ffprobe: PathBuf,
}

impl CheckExecutables {
    pub(super) fn discover(args: &ValidationArgs) -> Result<Self, EnvironmentError> {
        Ok(Self {
            bundler: bundler(&args.bundler)?,
            ffprobe: locate("ffprobe", &args.ffprobe)?,
        })
    }
}

/// Admitted local toolchain reported by `onmark doctor`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DoctorExecutables {
    pub(super) browser: BrowserExecutable,
    pub(super) bundler: BundlerProcess,
    pub(super) ffmpeg: PathBuf,
    pub(super) ffprobe: PathBuf,
}

impl DoctorExecutables {
    pub(super) async fn discover(args: &DoctorArgs) -> Result<Self, EnvironmentError> {
        Ok(Self {
            browser: browser(args.browser.as_deref()).await?,
            bundler: bundler(&args.bundler)?,
            ffmpeg: locate("FFmpeg", &args.ffmpeg)?,
            ffprobe: locate("ffprobe", &args.ffprobe)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BrowserExecutable {
    pub(super) capture_mode: BrowserCaptureMode,
    pub(super) path: PathBuf,
}

impl BrowserExecutable {
    fn explicit(path: PathBuf) -> Self {
        Self {
            capture_mode: BrowserCaptureMode::Screenshot,
            path,
        }
    }

    fn managed(path: PathBuf) -> Self {
        Self {
            capture_mode: managed_capture_mode(),
            path,
        }
    }
}

async fn browser(requested: Option<&Path>) -> Result<BrowserExecutable, EnvironmentError> {
    if let Some(requested) = requested {
        return locate("browser", requested).map(BrowserExecutable::explicit);
    }

    match (
        env::var_os(BROWSER_PROVISIONER),
        env::var_os(BROWSER_PROVISIONER_ENTRY),
    ) {
        (Some(provisioner), Some(entry)) => {
            let provisioner = locate("browser provisioner", Path::new(&provisioner))?;
            let entry = locate_file("browser provisioner entry", Path::new(&entry))?;
            let browser_path = browser_install::provision(&provisioner, &entry)
                .await
                .map_err(EnvironmentError::Provision)?;
            return locate("browser", &browser_path).map(BrowserExecutable::managed);
        }
        (None, None) => {}
        _ => return Err(EnvironmentError::IncompleteBrowserProvisioner),
    }

    default_browser()
        .map(BrowserExecutable::managed)
        .ok_or_else(|| EnvironmentError::Missing {
            role: "browser",
            requested: PathBuf::from(default_browser_name()),
        })
}

const fn managed_capture_mode() -> BrowserCaptureMode {
    if cfg!(target_os = "linux") {
        BrowserCaptureMode::BeginFrame
    } else {
        BrowserCaptureMode::Screenshot
    }
}

fn bundler(requested: &Path) -> Result<BundlerProcess, EnvironmentError> {
    let executable = locate("presentation bundler", requested)?;
    match env::var_os(BUNDLER_ENTRY) {
        Some(entry) => Ok(BundlerProcess::Node {
            executable,
            entry: locate_file("presentation bundler entry", Path::new(&entry))?,
        }),
        None => Ok(BundlerProcess::Direct(executable)),
    }
}

fn default_browser() -> Option<PathBuf> {
    default_browser_candidates()
        .into_iter()
        .find_map(|candidate| executable_path(&candidate))
}

#[cfg(target_os = "linux")]
fn default_browser_candidates() -> Vec<PathBuf> {
    vec![PathBuf::from(HEADLESS_SHELL)]
}

#[cfg(target_os = "macos")]
fn default_browser_candidates() -> Vec<PathBuf> {
    MACOS_BROWSERS.iter().map(PathBuf::from).collect()
}

#[cfg(target_os = "windows")]
fn default_browser_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for root in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
        if let Some(root) = env::var_os(root) {
            candidates.push(
                PathBuf::from(root)
                    .join("Google")
                    .join("Chrome")
                    .join("Application")
                    .join("chrome.exe"),
            );
        }
    }
    candidates.push(PathBuf::from("chrome.exe"));
    candidates
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn default_browser_candidates() -> Vec<PathBuf> {
    vec![PathBuf::from(HEADLESS_SHELL)]
}

const fn default_browser_name() -> &'static str {
    if cfg!(target_os = "linux") {
        HEADLESS_SHELL
    } else {
        "Chrome or Chromium"
    }
}

pub(super) fn worker_browser(requested: &Path) -> Result<PathBuf, EnvironmentError> {
    locate("browser", requested)
}

#[derive(Debug)]
pub(super) enum EnvironmentError {
    Missing {
        role: &'static str,
        requested: PathBuf,
    },
    MissingFile {
        role: &'static str,
        requested: PathBuf,
    },
    Provision(BrowserInstallError),
    IncompleteBrowserProvisioner,
}

impl fmt::Display for EnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { role, requested } => {
                write!(
                    formatter,
                    "{role} executable {} was not found",
                    requested.display()
                )?;
                if *role == "browser" {
                    formatter.write_str("; pass --browser <path>")?;
                }
                Ok(())
            }
            Self::MissingFile { role, requested } => {
                write!(
                    formatter,
                    "{role} file {} was not found",
                    requested.display()
                )
            }
            Self::Provision(source) => source.fmt(formatter),
            Self::IncompleteBrowserProvisioner => formatter.write_str(
                "browser provisioner executable and entry module must be configured together",
            ),
        }
    }
}

impl Error for EnvironmentError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Missing { .. }
            | Self::MissingFile { .. }
            | Self::IncompleteBrowserProvisioner => None,
            Self::Provision(source) => Some(source),
        }
    }
}

fn locate_file(role: &'static str, requested: &Path) -> Result<PathBuf, EnvironmentError> {
    requested
        .is_file()
        .then(|| requested.to_owned())
        .ok_or_else(|| EnvironmentError::MissingFile {
            role,
            requested: requested.to_owned(),
        })
}

fn locate(role: &'static str, requested: &Path) -> Result<PathBuf, EnvironmentError> {
    executable_path(requested).ok_or_else(|| EnvironmentError::Missing {
        role,
        requested: requested.to_owned(),
    })
}

fn executable_path(requested: &Path) -> Option<PathBuf> {
    if requested.components().count() > 1 {
        return is_executable_file(requested).then(|| requested.to_owned());
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path).find_map(|directory| executable_in_directory(&directory, requested))
}

fn executable_in_directory(directory: &Path, requested: &Path) -> Option<PathBuf> {
    let candidate = directory.join(requested);
    if is_executable_file(&candidate) {
        return Some(candidate);
    }

    #[cfg(target_os = "windows")]
    if requested.extension().is_none() {
        for extension in windows_executable_extensions() {
            let mut candidate = candidate.clone();
            candidate.set_extension(extension);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn windows_executable_extensions() -> Vec<String> {
    env::var("PATHEXT")
        .unwrap_or_else(|_| String::from(".COM;.EXE;.BAT;.CMD"))
        .split(';')
        .filter_map(|extension| {
            let extension = extension.trim().trim_start_matches('.');
            (!extension.is_empty()).then(|| extension.to_owned())
        })
        .collect()
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use onmark_render::BrowserCaptureMode;

    use super::{
        BrowserExecutable, default_browser_candidates, executable_path, managed_capture_mode,
    };

    #[test]
    fn accepts_an_explicit_existing_file() {
        let current_executable = std::env::current_exe().expect("the test executable has a path");
        assert_eq!(
            executable_path(&current_executable),
            Some(current_executable),
        );
        assert!(executable_path(Path::new("definitely-not-an-onmark-tool")).is_none());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn resolves_path_commands_through_windows_executable_extensions() {
        let current_executable = std::env::current_exe().expect("the test executable has a path");
        let directory = current_executable
            .parent()
            .expect("the test executable has a parent directory");
        let command = current_executable
            .file_stem()
            .map(Path::new)
            .expect("the test executable has a file stem");

        assert_eq!(
            super::executable_in_directory(directory, command),
            Some(current_executable),
        );
    }

    #[test]
    fn local_browser_discovery_has_a_platform_default() {
        assert!(!default_browser_candidates().is_empty());
    }

    #[test]
    fn explicit_browser_paths_never_invent_capture_capabilities() {
        let browser =
            BrowserExecutable::explicit(Path::new("/tools/chrome-headless-shell").to_owned());

        assert_eq!(browser.capture_mode, BrowserCaptureMode::Screenshot);
    }

    #[test]
    fn managed_browser_artifacts_own_the_platform_capture_contract() {
        let expected = if cfg!(target_os = "linux") {
            BrowserCaptureMode::BeginFrame
        } else {
            BrowserCaptureMode::Screenshot
        };

        assert_eq!(managed_capture_mode(), expected);
    }
}
