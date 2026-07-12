use std::error::Error;
use std::fmt;
use std::time::Duration;

const MAX_DEADLINE: Duration = Duration::from_hours(24);
const MAX_FRAMES: u64 = 10_000_000;
const MAX_INPUT_BYTES: u64 = 1 << 40;
const MAX_STDERR_BYTES: usize = 1 << 20;

/// Explicit resource limits for one `FFmpeg` encoding process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncodeLimits {
    deadline: Duration,
    max_frames: u64,
    max_input_bytes: u64,
    max_stderr_bytes: usize,
}

impl EncodeLimits {
    /// Creates one bounded encoding policy.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFfmpeg`] when a bound is zero or exceeds the fixed
    /// local-render safety envelope.
    pub fn new(
        deadline: Duration,
        max_frames: u64,
        max_input_bytes: u64,
        max_stderr_bytes: usize,
    ) -> Result<Self, InvalidFfmpeg> {
        if deadline.is_zero() {
            return Err(InvalidFfmpeg::ZeroDeadline);
        }
        if deadline > MAX_DEADLINE {
            return Err(InvalidFfmpeg::DeadlineTooLong);
        }
        if max_frames == 0 {
            return Err(InvalidFfmpeg::EmptyFrameLimit);
        }
        if max_frames > MAX_FRAMES {
            return Err(InvalidFfmpeg::FrameLimitTooLarge);
        }
        if max_input_bytes == 0 {
            return Err(InvalidFfmpeg::EmptyInputBudget);
        }
        if max_input_bytes > MAX_INPUT_BYTES {
            return Err(InvalidFfmpeg::InputBudgetTooLarge);
        }
        if max_stderr_bytes == 0 {
            return Err(InvalidFfmpeg::EmptyStderrBudget);
        }
        if max_stderr_bytes > MAX_STDERR_BYTES {
            return Err(InvalidFfmpeg::StderrBudgetTooLarge);
        }

        Ok(Self {
            deadline,
            max_frames,
            max_input_bytes,
            max_stderr_bytes,
        })
    }

    #[must_use]
    pub(super) const fn deadline(self) -> Duration {
        self.deadline
    }

    #[must_use]
    pub(super) const fn max_frames(self) -> u64 {
        self.max_frames
    }

    #[must_use]
    pub(super) const fn max_input_bytes(self) -> u64 {
        self.max_input_bytes
    }

    #[must_use]
    pub(super) const fn max_stderr_bytes(self) -> usize {
        self.max_stderr_bytes
    }
}

/// Reason an `FFmpeg` boundary cannot be configured safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidFfmpeg {
    /// No executable path was supplied.
    EmptyExecutable,
    /// A zero deadline cannot bound process lifetime.
    ZeroDeadline,
    /// The requested deadline exceeds one day.
    DeadlineTooLong,
    /// No frames may be written.
    EmptyFrameLimit,
    /// The frame count exceeds the fixed local-render ceiling.
    FrameLimitTooLarge,
    /// No encoded input bytes may be written.
    EmptyInputBudget,
    /// The input budget exceeds one tebibyte.
    InputBudgetTooLarge,
    /// No `FFmpeg` diagnostic bytes may be retained.
    EmptyStderrBudget,
    /// The stderr budget exceeds one mebibyte.
    StderrBudgetTooLarge,
}

impl fmt::Display for InvalidFfmpeg {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyExecutable => "FFmpeg executable cannot be empty",
            Self::ZeroDeadline => "FFmpeg requires a positive deadline",
            Self::DeadlineTooLong => "FFmpeg deadline cannot exceed one day",
            Self::EmptyFrameLimit => "FFmpeg frame limit must be positive",
            Self::FrameLimitTooLarge => "FFmpeg frame limit exceeds the safety ceiling",
            Self::EmptyInputBudget => "FFmpeg input budget must be positive",
            Self::InputBudgetTooLarge => "FFmpeg input budget exceeds the safety ceiling",
            Self::EmptyStderrBudget => "FFmpeg stderr budget must be positive",
            Self::StderrBudgetTooLarge => "FFmpeg stderr budget exceeds the safety ceiling",
        })
    }
}

impl Error for InvalidFfmpeg {}

#[cfg(test)]
mod tests {
    use super::{
        EncodeLimits, InvalidFfmpeg, MAX_DEADLINE, MAX_FRAMES, MAX_INPUT_BYTES, MAX_STDERR_BYTES,
    };
    use std::time::Duration;

    #[test]
    fn accepts_explicit_bounded_limits() {
        let limits = EncodeLimits::new(Duration::from_mins(1), 300, 64 << 20, 64 << 10)
            .expect("the fixture limits are bounded");

        assert_eq!(limits.deadline(), Duration::from_mins(1));
        assert_eq!(limits.max_frames(), 300);
        assert_eq!(limits.max_input_bytes(), 64 << 20);
        assert_eq!(limits.max_stderr_bytes(), 64 << 10);
    }

    #[test]
    fn rejects_empty_or_unbounded_limits() {
        let valid = || EncodeLimits::new(Duration::from_secs(1), 1, 1, 1);
        assert!(valid().is_ok());
        assert_eq!(
            EncodeLimits::new(Duration::ZERO, 1, 1, 1),
            Err(InvalidFfmpeg::ZeroDeadline),
        );
        assert_eq!(
            EncodeLimits::new(MAX_DEADLINE + Duration::from_nanos(1), 1, 1, 1),
            Err(InvalidFfmpeg::DeadlineTooLong),
        );
        assert_eq!(
            EncodeLimits::new(Duration::from_secs(1), 0, 1, 1),
            Err(InvalidFfmpeg::EmptyFrameLimit),
        );
        assert_eq!(
            EncodeLimits::new(Duration::from_secs(1), MAX_FRAMES + 1, 1, 1),
            Err(InvalidFfmpeg::FrameLimitTooLarge),
        );
        assert_eq!(
            EncodeLimits::new(Duration::from_secs(1), 1, 0, 1),
            Err(InvalidFfmpeg::EmptyInputBudget),
        );
        assert_eq!(
            EncodeLimits::new(Duration::from_secs(1), 1, MAX_INPUT_BYTES + 1, 1),
            Err(InvalidFfmpeg::InputBudgetTooLarge),
        );
        assert_eq!(
            EncodeLimits::new(Duration::from_secs(1), 1, 1, 0),
            Err(InvalidFfmpeg::EmptyStderrBudget),
        );
        assert_eq!(
            EncodeLimits::new(Duration::from_secs(1), 1, 1, MAX_STDERR_BYTES + 1),
            Err(InvalidFfmpeg::StderrBudgetTooLarge),
        );
    }
}
