//! Discovery and validation of host executables required by local rendering.
//!
//! Environment conventions stop here; renderer APIs receive explicit paths and
//! never inspect process-global configuration.

use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::arguments::RenderArgs;

const HEADLESS_SHELL: &str = "chrome-headless-shell";

#[cfg(target_os = "macos")]
const MACOS_BROWSERS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
];

/// Validated browser, `FFmpeg`, ffprobe, and bundler paths for one command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Executables {
    pub(super) browser: PathBuf,
    pub(super) bundler: PathBuf,
    pub(super) ffmpeg: PathBuf,
    pub(super) ffprobe: PathBuf,
}

impl Executables {
    pub(super) fn discover(args: &RenderArgs) -> Result<Self, EnvironmentError> {
        let browser = browser(args.browser.as_deref())?;
        let bundler = locate("presentation bundler", &args.bundler)?;
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

fn browser(requested: Option<&Path>) -> Result<PathBuf, EnvironmentError> {
    if let Some(requested) = requested {
        return locate("browser", requested);
    }

    default_browser().ok_or_else(|| EnvironmentError {
        role: "browser",
        requested: PathBuf::from(default_browser_name()),
    })
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EnvironmentError {
    role: &'static str,
    requested: PathBuf,
}

impl fmt::Display for EnvironmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} executable {} was not found",
            self.role,
            self.requested.display()
        )?;
        if self.role == "browser" {
            formatter.write_str("; pass --browser <path>")?;
        }
        Ok(())
    }
}

impl Error for EnvironmentError {}

fn locate(role: &'static str, requested: &Path) -> Result<PathBuf, EnvironmentError> {
    executable_path(requested).ok_or_else(|| EnvironmentError {
        role,
        requested: requested.to_owned(),
    })
}

fn executable_path(requested: &Path) -> Option<PathBuf> {
    if requested.components().count() > 1 {
        return is_executable_file(requested).then(|| requested.to_owned());
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(requested))
        .find(|candidate| is_executable_file(candidate))
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

    use super::{default_browser_candidates, executable_path};

    #[test]
    fn accepts_an_explicit_existing_file() {
        let current_executable = std::env::current_exe().expect("the test executable has a path");
        assert_eq!(
            executable_path(&current_executable),
            Some(current_executable),
        );
        assert!(executable_path(Path::new("definitely-not-an-onmark-tool")).is_none());
    }

    #[test]
    fn local_browser_discovery_has_a_platform_default() {
        assert!(!default_browser_candidates().is_empty());
    }
}
