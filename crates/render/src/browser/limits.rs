use std::error::Error;
use std::fmt;
use std::time::Duration;

const MAX_VIEWPORT_EDGE: u32 = 8_192;
const MAX_CAPTURE_BYTES: usize = 512 * 1024 * 1024;

/// Explicit limits applied to one Chromium capture session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserLimits {
    width: u32,
    height: u32,
    deadline: Duration,
    max_capture_bytes: usize,
}

impl BrowserLimits {
    /// Creates bounded browser-session limits.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserLimits`] when a dimension or bound is zero, or
    /// when a viewport or retained capture exceeds the Gate-one safety ceiling.
    pub const fn new(
        width: u32,
        height: u32,
        deadline: Duration,
        max_capture_bytes: usize,
    ) -> Result<Self, InvalidBrowserLimits> {
        if width == 0 || height == 0 {
            return Err(InvalidBrowserLimits::EmptyViewport);
        }
        if width > MAX_VIEWPORT_EDGE || height > MAX_VIEWPORT_EDGE {
            return Err(InvalidBrowserLimits::ViewportTooLarge);
        }
        if deadline.is_zero() {
            return Err(InvalidBrowserLimits::ZeroDeadline);
        }
        if max_capture_bytes == 0 {
            return Err(InvalidBrowserLimits::EmptyCaptureBudget);
        }
        if max_capture_bytes > MAX_CAPTURE_BYTES {
            return Err(InvalidBrowserLimits::CaptureBudgetTooLarge);
        }

        Ok(Self {
            width,
            height,
            deadline,
            max_capture_bytes,
        })
    }

    /// Returns the viewport width in CSS pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Returns the viewport height in CSS pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Returns the deadline shared by launch and individual CDP requests.
    #[must_use]
    pub const fn deadline(self) -> Duration {
        self.deadline
    }

    /// Returns the maximum encoded capture bytes retained in memory.
    #[must_use]
    pub const fn max_capture_bytes(self) -> usize {
        self.max_capture_bytes
    }
}

/// Reason browser-session limits exceed the supported safety envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidBrowserLimits {
    /// At least one viewport dimension is zero.
    EmptyViewport,
    /// At least one viewport dimension exceeds the supported edge length.
    ViewportTooLarge,
    /// Browser operations have no deadline.
    ZeroDeadline,
    /// No capture bytes may be retained.
    EmptyCaptureBudget,
    /// The capture budget exceeds the supported in-memory ceiling.
    CaptureBudgetTooLarge,
}

impl fmt::Display for InvalidBrowserLimits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyViewport => "browser viewport dimensions must be positive",
            Self::ViewportTooLarge => "browser viewport exceeds the supported edge length",
            Self::ZeroDeadline => "browser operations require a positive deadline",
            Self::EmptyCaptureBudget => "browser capture budget must be positive",
            Self::CaptureBudgetTooLarge => "browser capture budget exceeds the safety ceiling",
        })
    }
}

impl Error for InvalidBrowserLimits {}

#[cfg(test)]
mod tests {
    use super::{BrowserLimits, InvalidBrowserLimits, MAX_CAPTURE_BYTES, MAX_VIEWPORT_EDGE};
    use std::time::Duration;

    #[test]
    fn accepts_explicit_bounded_limits() {
        let limits = BrowserLimits::new(1920, 1080, Duration::from_secs(10), 32 * 1024 * 1024)
            .expect("the fixture limits are bounded");

        assert_eq!(limits.width(), 1920);
        assert_eq!(limits.height(), 1080);
        assert_eq!(limits.deadline(), Duration::from_secs(10));
        assert_eq!(limits.max_capture_bytes(), 32 * 1024 * 1024);
    }

    #[test]
    fn rejects_unbounded_or_empty_limits() {
        assert_eq!(
            BrowserLimits::new(0, 1080, Duration::from_secs(1), 1),
            Err(InvalidBrowserLimits::EmptyViewport),
        );
        assert_eq!(
            BrowserLimits::new(MAX_VIEWPORT_EDGE + 1, 1080, Duration::from_secs(1), 1,),
            Err(InvalidBrowserLimits::ViewportTooLarge),
        );
        assert_eq!(
            BrowserLimits::new(1920, 1080, Duration::ZERO, 1),
            Err(InvalidBrowserLimits::ZeroDeadline),
        );
        assert_eq!(
            BrowserLimits::new(1920, 1080, Duration::from_secs(1), 0),
            Err(InvalidBrowserLimits::EmptyCaptureBudget),
        );
        assert_eq!(
            BrowserLimits::new(1920, 1080, Duration::from_secs(1), MAX_CAPTURE_BYTES + 1,),
            Err(InvalidBrowserLimits::CaptureBudgetTooLarge),
        );
    }
}
