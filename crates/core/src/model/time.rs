use std::error::Error;
use std::fmt;

/// Zero-based position of one frame on a timeline.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameIndex(u64);

impl FrameIndex {
    /// The first frame on every timeline.
    pub const ZERO: Self = Self(0);

    /// Creates a frame index from its exact integer representation.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the exact integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advances by a frame count without implicit overflow behavior.
    #[must_use]
    pub const fn checked_advance(self, count: FrameCount) -> Option<Self> {
        match self.0.checked_add(count.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }
}

/// Number of frames in a duration or interval.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameCount(u64);

impl FrameCount {
    /// A duration containing no frames.
    pub const ZERO: Self = Self(0);

    /// Creates a frame count from its exact integer representation.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the exact integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Exact rational frame rate measured in frames per second.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameRate {
    numerator: u32,
    denominator: u32,
}

impl FrameRate {
    /// Creates a canonical rational frame rate.
    ///
    /// `30 / 1` represents 30 fps and `30_000 / 1_001` represents the common
    /// NTSC-derived rate.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFrameRate`] when either part is zero.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, InvalidFrameRate> {
        if numerator == 0 {
            return Err(InvalidFrameRate::ZeroNumerator);
        }

        if denominator == 0 {
            return Err(InvalidFrameRate::ZeroDenominator);
        }

        let divisor = greatest_common_divisor(numerator, denominator);

        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    /// Returns the canonical numerator.
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator
    }

    /// Returns the canonical denominator.
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator
    }
}

/// Reason a rational frame rate is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidFrameRate {
    /// Zero frames per second has no timeline meaning.
    ZeroNumerator,
    /// A rational value cannot have a zero denominator.
    ZeroDenominator,
}

impl fmt::Display for InvalidFrameRate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ZeroNumerator => "frame-rate numerator cannot be zero",
            Self::ZeroDenominator => "frame-rate denominator cannot be zero",
        };

        formatter.write_str(message)
    }
}

impl Error for InvalidFrameRate {}

/// Half-open frame interval `[start, end)`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FrameInterval {
    start: FrameIndex,
    end: FrameIndex,
}

impl FrameInterval {
    /// Creates an interval whose end is not before its start.
    ///
    /// Empty intervals are valid model values. Language semantics decide
    /// whether a particular authored element may have zero duration.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidFrameInterval`] when `end` is before `start`.
    pub const fn new(start: FrameIndex, end: FrameIndex) -> Result<Self, InvalidFrameInterval> {
        if end.0 < start.0 {
            return Err(InvalidFrameInterval { start, end });
        }

        Ok(Self { start, end })
    }

    /// Returns the inclusive start frame.
    #[must_use]
    pub const fn start(self) -> FrameIndex {
        self.start
    }

    /// Returns the exclusive end frame.
    #[must_use]
    pub const fn end(self) -> FrameIndex {
        self.end
    }

    /// Returns the exact number of frames in the interval.
    #[must_use]
    pub const fn len(self) -> FrameCount {
        FrameCount(self.end.0 - self.start.0)
    }

    /// Returns whether the interval contains no frames.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start.0 == self.end.0
    }
}

/// A frame interval whose end precedes its start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidFrameInterval {
    start: FrameIndex,
    end: FrameIndex,
}

impl InvalidFrameInterval {
    /// Returns the rejected start frame.
    #[must_use]
    pub const fn start(self) -> FrameIndex {
        self.start
    }

    /// Returns the rejected end frame.
    #[must_use]
    pub const fn end(self) -> FrameIndex {
        self.end
    }
}

impl fmt::Display for InvalidFrameInterval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "frame interval ends at {} before it starts at {}",
            self.end.get(),
            self.start.get(),
        )
    }
}

impl Error for InvalidFrameInterval {}

const fn greatest_common_divisor(mut left: u32, mut right: u32) -> u32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }

    left
}

#[cfg(test)]
mod tests {
    use super::{
        FrameCount, FrameIndex, FrameInterval, FrameRate, InvalidFrameInterval, InvalidFrameRate,
    };

    #[test]
    fn advances_without_hiding_overflow() {
        assert_eq!(
            FrameIndex::new(90).checked_advance(FrameCount::new(60)),
            Some(FrameIndex::new(150)),
        );
        assert_eq!(
            FrameIndex::new(u64::MAX).checked_advance(FrameCount::new(1)),
            None,
        );
    }

    #[test]
    fn canonicalizes_equivalent_frame_rates() {
        assert_eq!(
            FrameRate::new(60, 2).expect("the fixture is a valid frame rate"),
            FrameRate::new(30, 1).expect("the fixture is a valid frame rate"),
        );
    }

    #[test]
    fn rejects_zero_frame_rate_parts() {
        assert_eq!(FrameRate::new(0, 1), Err(InvalidFrameRate::ZeroNumerator),);
        assert_eq!(
            FrameRate::new(30, 0),
            Err(InvalidFrameRate::ZeroDenominator),
        );
    }

    #[test]
    fn measures_a_half_open_interval() {
        let interval = FrameInterval::new(FrameIndex::new(90), FrameIndex::new(150))
            .expect("the fixture has ordered bounds");

        assert_eq!(interval.len(), FrameCount::new(60));
        assert!(!interval.is_empty());
    }

    #[test]
    fn allows_an_empty_interval() {
        let interval = FrameInterval::new(FrameIndex::new(90), FrameIndex::new(90))
            .expect("equal bounds form an empty interval");

        assert_eq!(interval.len(), FrameCount::ZERO);
        assert!(interval.is_empty());
    }

    #[test]
    fn rejects_reversed_interval_bounds() {
        assert_eq!(
            FrameInterval::new(FrameIndex::new(150), FrameIndex::new(90)),
            Err(InvalidFrameInterval {
                start: FrameIndex::new(150),
                end: FrameIndex::new(90),
            }),
        );
    }
}
