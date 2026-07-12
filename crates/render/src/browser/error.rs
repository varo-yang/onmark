use std::error::Error;
use std::fmt;
use std::io;

use chromiumoxide::error::CdpError;
use tokio::task::JoinError;

/// Stable category for a Chromium execution failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BrowserErrorKind {
    /// The executable or browser configuration is invalid.
    Configuration,
    /// A private Chromium profile could not be created.
    Profile,
    /// Chromium could not launch or expose CDP.
    Launch,
    /// The initial page target could not be created.
    PageCreation,
    /// The page could not navigate to the render bundle.
    Navigation,
    /// The render bundle did not install its runtime host before the deadline.
    RuntimeHost,
    /// The browser runtime handshake failed.
    Protocol,
    /// CDP could not capture the current viewport.
    Capture,
    /// Encoded screenshot bytes exceed the retained-memory budget.
    CaptureTooLarge,
    /// Chromium could not close or be reaped cleanly.
    Shutdown,
    /// The CDP handler exited with an error.
    Handler,
    /// The CDP handler did not stop before the cleanup deadline.
    HandlerTimeout,
}

/// Typed Chromium execution failure with an optional underlying source.
#[derive(Debug)]
pub struct BrowserError {
    kind: BrowserErrorKind,
    message: Option<Box<str>>,
    source: Option<BrowserErrorSource>,
}

impl BrowserError {
    /// Returns the stable failure category.
    #[must_use]
    pub const fn kind(&self) -> BrowserErrorKind {
        self.kind
    }

    pub(super) fn configuration(message: String) -> Self {
        Self {
            kind: BrowserErrorKind::Configuration,
            message: Some(message.into_boxed_str()),
            source: None,
        }
    }

    pub(super) fn without_source(kind: BrowserErrorKind) -> Self {
        Self {
            kind,
            message: None,
            source: None,
        }
    }

    pub(super) fn cdp(kind: BrowserErrorKind, source: CdpError) -> Self {
        Self {
            kind,
            message: None,
            source: Some(BrowserErrorSource::Cdp(source)),
        }
    }

    pub(super) fn io(kind: BrowserErrorKind, source: io::Error) -> Self {
        Self {
            kind,
            message: None,
            source: Some(BrowserErrorSource::Io(source)),
        }
    }

    pub(super) fn join(kind: BrowserErrorKind, source: JoinError) -> Self {
        Self {
            kind,
            message: None,
            source: Some(BrowserErrorSource::Join(source)),
        }
    }

    pub(super) fn json(kind: BrowserErrorKind, source: serde_json::Error) -> Self {
        Self {
            kind,
            message: None,
            source: Some(BrowserErrorSource::Json(source)),
        }
    }
}

impl fmt::Display for BrowserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(message) = &self.message {
            return write!(formatter, "{}: {message}", self.kind);
        }
        write!(formatter, "{}", self.kind)
    }
}

impl Error for BrowserError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_ref().map(|source| source as _)
    }
}

impl fmt::Display for BrowserErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Configuration => "invalid browser configuration",
            Self::Profile => "failed to create a private Chromium profile",
            Self::Launch => "failed to launch Chromium",
            Self::PageCreation => "failed to create the render page",
            Self::Navigation => "failed to navigate the render page",
            Self::RuntimeHost => "browser runtime host missed its readiness deadline",
            Self::Protocol => "browser runtime protocol failed",
            Self::Capture => "failed to capture the browser frame",
            Self::CaptureTooLarge => "captured frame exceeds the byte budget",
            Self::Shutdown => "failed to shut down Chromium",
            Self::Handler => "Chromium protocol handler failed",
            Self::HandlerTimeout => "Chromium protocol handler missed its shutdown deadline",
        })
    }
}

#[derive(Debug)]
enum BrowserErrorSource {
    Cdp(CdpError),
    Io(io::Error),
    Join(JoinError),
    Json(serde_json::Error),
}

impl fmt::Display for BrowserErrorSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cdp(source) => source.fmt(formatter),
            Self::Io(source) => source.fmt(formatter),
            Self::Join(source) => source.fmt(formatter),
            Self::Json(source) => source.fmt(formatter),
        }
    }
}

impl Error for BrowserErrorSource {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Cdp(source) => source.source(),
            Self::Io(source) => source.source(),
            Self::Join(source) => source.source(),
            Self::Json(source) => source.source(),
        }
    }
}
