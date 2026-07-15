//! Discovery and validation of host executables required by local rendering.
//!
//! Environment conventions stop here; renderer APIs receive explicit paths and
//! never inspect process-global configuration.

use std::env;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::arguments::RenderArgs;

const BROWSER_CANDIDATES: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "chrome",
    "msedge",
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
        let browser = match &args.browser {
            Some(browser) => locate("browser", browser)?,
            None => locate_browser()?,
        };
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

fn locate_browser() -> Result<PathBuf, EnvironmentError> {
    for candidate in BROWSER_CANDIDATES {
        if let Some(path) = executable_path(Path::new(candidate)) {
            return Ok(path);
        }
    }
    Err(EnvironmentError {
        role: "browser",
        requested: PathBuf::from("Chromium or Google Chrome"),
    })
}

fn locate(role: &'static str, requested: &Path) -> Result<PathBuf, EnvironmentError> {
    executable_path(requested).ok_or_else(|| EnvironmentError {
        role,
        requested: requested.to_owned(),
    })
}

fn executable_path(requested: &Path) -> Option<PathBuf> {
    if requested.components().count() > 1 {
        return requested.is_file().then(|| requested.to_owned());
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(requested))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::executable_path;

    #[test]
    fn accepts_an_explicit_existing_file() {
        assert_eq!(
            executable_path(Path::new(env!("CARGO_MANIFEST_PATH"))),
            Some(Path::new(env!("CARGO_MANIFEST_PATH")).to_owned()),
        );
        assert!(executable_path(Path::new("definitely-not-an-onmark-tool")).is_none());
    }
}
