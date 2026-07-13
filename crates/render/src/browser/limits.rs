use std::error::Error;
use std::fmt;
use std::time::Duration;

const MAX_CAPTURE_BYTES: usize = 512 * 1024 * 1024;

/// Explicit limits applied to one Chromium capture session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserLimits {
    deadline: Duration,
    max_capture_bytes: usize,
}

impl BrowserLimits {
    /// Creates bounded browser-session limits.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserLimits`] when a bound is zero or the retained
    /// capture ceiling exceeds the Gate-one safety envelope.
    pub const fn new(
        deadline: Duration,
        max_capture_bytes: usize,
    ) -> Result<Self, InvalidBrowserLimits> {
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
            deadline,
            max_capture_bytes,
        })
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
            Self::ZeroDeadline => "browser operations require a positive deadline",
            Self::EmptyCaptureBudget => "browser capture budget must be positive",
            Self::CaptureBudgetTooLarge => "browser capture budget exceeds the safety ceiling",
        })
    }
}

impl Error for InvalidBrowserLimits {}

#[cfg(test)]
mod tests {
    use super::{BrowserLimits, InvalidBrowserLimits, MAX_CAPTURE_BYTES};
    use std::time::Duration;

    #[test]
    fn accepts_explicit_bounded_limits() {
        let limits = BrowserLimits::new(Duration::from_secs(10), 32 * 1024 * 1024)
            .expect("the fixture limits are bounded");

        assert_eq!(limits.deadline(), Duration::from_secs(10));
        assert_eq!(limits.max_capture_bytes(), 32 * 1024 * 1024);
    }

    #[test]
    fn rejects_unbounded_or_empty_limits() {
        assert_eq!(
            BrowserLimits::new(Duration::ZERO, 1),
            Err(InvalidBrowserLimits::ZeroDeadline),
        );
        assert_eq!(
            BrowserLimits::new(Duration::from_secs(1), 0),
            Err(InvalidBrowserLimits::EmptyCaptureBudget),
        );
        assert_eq!(
            BrowserLimits::new(Duration::from_secs(1), MAX_CAPTURE_BYTES + 1),
            Err(InvalidBrowserLimits::CaptureBudgetTooLarge),
        );
    }
}
