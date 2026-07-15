//! Exact frame algebra and named duration-to-frame rounding.
//!
//! Indices, counts, rates, and intervals are distinct types even where their
//! storage matches, preventing accidental timeline arithmetic.

use std::error::Error;
use std::fmt;

use super::Duration;
use super::duration::NANOS_PER_SECOND;

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

    /// Adds two frame counts without implicit overflow behavior.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(value) => Some(Self(value)),
            None => None,
        }
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

/// Explicit policy for placing a time value on the discrete frame grid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Rounding {
    /// Select the frame boundary at or before the exact time.
    Floor,
    /// Select the frame boundary at or after the exact time.
    Ceil,
}

/// Exact conversion between nanosecond time and a rational frame grid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Timebase {
    frame_rate: FrameRate,
}

impl Timebase {
    /// Creates a timebase from an already validated rational frame rate.
    #[must_use]
    pub const fn new(frame_rate: FrameRate) -> Self {
        Self { frame_rate }
    }

    /// Returns the rational frame rate that defines this grid.
    #[must_use]
    pub const fn frame_rate(self) -> FrameRate {
        self.frame_rate
    }

    /// Places an absolute non-negative time on the frame grid.
    ///
    /// # Errors
    ///
    /// Returns [`FrameConversionOverflow`] when the resulting index does not
    /// fit in the timeline's `u64` frame domain.
    pub fn frame_at(
        self,
        time: Duration,
        rounding: Rounding,
    ) -> Result<FrameIndex, FrameConversionOverflow> {
        self.frame_number(time, rounding).map(FrameIndex::new)
    }

    /// Converts an elapsed duration into a frame count.
    ///
    /// # Errors
    ///
    /// Returns [`FrameConversionOverflow`] when the resulting count does not
    /// fit in the timeline's `u64` frame domain.
    pub fn frames_for(
        self,
        duration: Duration,
        rounding: Rounding,
    ) -> Result<FrameCount, FrameConversionOverflow> {
        self.frame_number(duration, rounding).map(FrameCount::new)
    }

    fn frame_number(
        self,
        duration: Duration,
        rounding: Rounding,
    ) -> Result<u64, FrameConversionOverflow> {
        let numerator = u128::from(duration.as_nanos()) * u128::from(self.frame_rate.numerator());
        let denominator = u128::from(NANOS_PER_SECOND) * u128::from(self.frame_rate.denominator());
        let quotient = numerator / denominator;
        let remainder = numerator % denominator;
        let frames = match rounding {
            Rounding::Floor => quotient,
            Rounding::Ceil => quotient + u128::from(remainder != 0),
        };

        u64::try_from(frames).map_err(|_| FrameConversionOverflow {
            duration,
            frame_rate: self.frame_rate,
        })
    }
}

/// A time value whose frame-grid representation exceeds the timeline domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameConversionOverflow {
    duration: Duration,
    frame_rate: FrameRate,
}

impl FrameConversionOverflow {
    /// Returns the duration that could not be represented.
    #[must_use]
    pub const fn duration(self) -> Duration {
        self.duration
    }

    /// Returns the frame rate used for the rejected conversion.
    #[must_use]
    pub const fn frame_rate(self) -> FrameRate {
        self.frame_rate
    }
}

impl fmt::Display for FrameConversionOverflow {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "duration {} at {}/{} fps exceeds the frame domain",
            self.duration,
            self.frame_rate.numerator(),
            self.frame_rate.denominator(),
        )
    }
}

impl Error for FrameConversionOverflow {}

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

    /// Returns whether `other` is bounded by this interval.
    #[must_use]
    pub const fn contains_interval(self, other: Self) -> bool {
        self.start.0 <= other.start.0 && other.end.0 <= self.end.0
    }

    /// Returns whether this interval and `other` share at least one frame.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        !self.is_empty()
            && !other.is_empty()
            && self.start.0 < other.end.0
            && other.start.0 < self.end.0
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
    use proptest::prelude::*;

    use super::{
        FrameCount, FrameIndex, FrameInterval, FrameRate, InvalidFrameInterval, InvalidFrameRate,
        Rounding, Timebase, greatest_common_divisor,
    };
    use crate::model::Duration;

    proptest! {
        #[test]
        fn canonicalizes_equally_scaled_frame_rates(
            numerator in 1_u32..=65_535,
            denominator in 1_u32..=65_535,
            scale in 1_u32..=65_535,
        ) {
            let original = FrameRate::new(numerator, denominator)
                .expect("positive parts form a valid frame rate");
            let scaled = FrameRate::new(numerator * scale, denominator * scale)
                .expect("the generated products fit in u32 and remain positive");

            prop_assert_eq!(scaled, original);
        }

        #[test]
        fn canonical_frame_rate_parts_are_coprime(
            numerator in 1_u32..=u32::MAX,
            denominator in 1_u32..=u32::MAX,
        ) {
            let rate = FrameRate::new(numerator, denominator)
                .expect("positive parts form a valid frame rate");

            prop_assert_eq!(
                greatest_common_divisor(rate.numerator(), rate.denominator()),
                1,
            );
        }

        #[test]
        fn interval_length_reconstructs_its_end(left in any::<u64>(), right in any::<u64>()) {
            let (start, end) = if left <= right {
                (FrameIndex::new(left), FrameIndex::new(right))
            } else {
                (FrameIndex::new(right), FrameIndex::new(left))
            };
            let interval = FrameInterval::new(start, end)
                .expect("ordered generated bounds form a valid interval");

            prop_assert_eq!(start.checked_advance(interval.len()), Some(end));
        }

        #[test]
        fn rounding_brackets_the_exact_frame_boundary(
            nanoseconds in any::<u64>(),
            numerator in 1_u32..=240,
            denominator in 1_u32..=1_001,
        ) {
            let rate = FrameRate::new(numerator, denominator)
                .expect("positive parts form a valid frame rate");
            let timebase = Timebase::new(rate);
            let duration = Duration::from_nanos(nanoseconds);
            let floor = timebase.frames_for(duration, Rounding::Floor)
                .expect("the generated rate keeps the frame count in range");
            let ceil = timebase.frames_for(duration, Rounding::Ceil)
                .expect("the generated rate keeps the frame count in range");

            prop_assert!(floor <= ceil);
            prop_assert!(ceil.get() - floor.get() <= 1);
            prop_assert_eq!(
                timebase.frame_at(duration, Rounding::Floor).map(FrameIndex::get),
                Ok(floor.get()),
            );
        }
    }

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
    fn adds_frame_counts_without_hiding_overflow() {
        assert_eq!(
            FrameCount::new(90).checked_add(FrameCount::new(60)),
            Some(FrameCount::new(150)),
        );
        assert_eq!(
            FrameCount::new(u64::MAX).checked_add(FrameCount::new(1)),
            None,
        );
    }

    #[test]
    fn converts_exact_and_fractional_frame_boundaries() {
        let timebase = Timebase::new(FrameRate::new(30, 1).expect("30 fps is valid"));

        assert_eq!(
            timebase.frame_at(Duration::from_nanos(2_000_000_000), Rounding::Floor),
            Ok(FrameIndex::new(60)),
        );
        assert_eq!(
            timebase.frames_for(Duration::from_nanos(1), Rounding::Floor),
            Ok(FrameCount::ZERO),
        );
        assert_eq!(
            timebase.frames_for(Duration::from_nanos(1), Rounding::Ceil),
            Ok(FrameCount::new(1)),
        );
    }

    #[test]
    fn converts_ntsc_boundaries_without_floating_point() {
        let rate = FrameRate::new(30_000, 1_001).expect("the NTSC-derived rate is valid");
        let timebase = Timebase::new(rate);
        let second = Duration::from_nanos(1_000_000_000);

        assert_eq!(
            timebase.frames_for(second, Rounding::Floor),
            Ok(FrameCount::new(29)),
        );
        assert_eq!(
            timebase.frames_for(second, Rounding::Ceil),
            Ok(FrameCount::new(30)),
        );
    }

    #[test]
    fn reports_frame_domain_overflow() {
        let rate = FrameRate::new(u32::MAX, 1).expect("the maximum numerator is valid");
        let timebase = Timebase::new(rate);
        let duration = Duration::from_nanos(u64::MAX);
        let error = timebase
            .frames_for(duration, Rounding::Ceil)
            .expect_err("the converted frame count exceeds u64");

        assert_eq!(error.duration(), duration);
        assert_eq!(error.frame_rate(), rate);
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
    fn empty_intervals_do_not_intersect() {
        let empty = FrameInterval::new(FrameIndex::new(5), FrameIndex::new(5))
            .expect("equal bounds form an empty interval");
        let enclosing = FrameInterval::new(FrameIndex::new(4), FrameIndex::new(6))
            .expect("ordered bounds form an interval");

        assert!(!empty.intersects(enclosing));
        assert!(!enclosing.intersects(empty));
    }

    #[test]
    fn adjacent_intervals_do_not_intersect() {
        let left = FrameInterval::new(FrameIndex::new(4), FrameIndex::new(5))
            .expect("ordered bounds form an interval");
        let right = FrameInterval::new(FrameIndex::new(5), FrameIndex::new(6))
            .expect("ordered bounds form an interval");
        let overlap = FrameInterval::new(FrameIndex::new(4), FrameIndex::new(6))
            .expect("ordered bounds form an interval");

        assert!(!left.intersects(right));
        assert!(left.intersects(overlap));
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
