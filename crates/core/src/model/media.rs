//! Exact source-local media selection and playback values.
//!
//! These values describe authored media intent without assigning film frames.

use std::error::Error;
use std::fmt;

use super::{Duration, InvalidDuration};

const RATE_DENOMINATORS: [u128; 7] = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000];

/// Exact number of complete passes through one selected source interval.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PlayCount(u32);

impl PlayCount {
    /// One natural pass through the selected source.
    pub const ONE: Self = Self(1);

    /// Parses a positive decimal integer.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPlayCount`] when the spelling is not a positive `u32`.
    pub fn parse(value: &str) -> Result<Self, InvalidPlayCount> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(InvalidPlayCount::Malformed);
        }
        let count = value
            .parse::<u32>()
            .map_err(|_| InvalidPlayCount::OutOfRange)?;
        Self::new(count)
    }

    /// Creates a positive play count.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPlayCount::Zero`] when `count` contains no play.
    pub const fn new(count: u32) -> Result<Self, InvalidPlayCount> {
        if count == 0 {
            return Err(InvalidPlayCount::Zero);
        }
        Ok(Self(count))
    }

    /// Returns the exact number of complete source passes.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Reason authored play-count text is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidPlayCount {
    /// The value is not an unsigned decimal integer.
    Malformed,
    /// Zero contains no source pass.
    Zero,
    /// The integer exceeds the play-count domain.
    OutOfRange,
}

impl fmt::Display for InvalidPlayCount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Malformed => "plays must be an unsigned decimal integer",
            Self::Zero => "plays must be greater than zero",
            Self::OutOfRange => "plays exceeds the supported integer range",
        };
        formatter.write_str(message)
    }
}

impl Error for InvalidPlayCount {}

/// Authored half-open source interval with an optional natural end.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaTrim {
    start: Duration,
    end: Option<Duration>,
}

impl MediaTrim {
    /// Parses `start..end`, allowing exactly one omitted bound.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidMediaTrim`] when the spelling or interval is invalid.
    pub fn parse(value: &str) -> Result<Self, InvalidMediaTrim> {
        let Some((start, end)) = value.split_once("..") else {
            return Err(InvalidMediaTrim::MissingSeparator);
        };
        if end.contains("..") {
            return Err(InvalidMediaTrim::MultipleSeparators);
        }
        if start.is_empty() && end.is_empty() {
            return Err(InvalidMediaTrim::MissingBounds);
        }

        let start = parse_trim_start(start)?;
        let end = parse_trim_end(end)?;
        if end.is_some_and(|end| end <= start) {
            return Err(InvalidMediaTrim::Reversed);
        }

        Ok(Self { start, end })
    }

    /// Returns the selected source start, defaulting to source zero.
    #[must_use]
    pub const fn start(self) -> Duration {
        self.start
    }

    /// Returns the exclusive source end, or `None` for the natural source end.
    #[must_use]
    pub const fn end(self) -> Option<Duration> {
        self.end
    }
}

impl fmt::Display for MediaTrim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}..", self.start)?;
        if let Some(end) = self.end {
            write!(formatter, "{end}")?;
        }
        Ok(())
    }
}

/// Reason authored source selection cannot become a media trim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidMediaTrim {
    /// The interval separator is absent.
    MissingSeparator,
    /// More than one interval separator was authored.
    MultipleSeparators,
    /// Both interval bounds were omitted.
    MissingBounds,
    /// The source start is not an exact duration.
    InvalidStart(InvalidDuration),
    /// The source end is not an exact duration.
    InvalidEnd(InvalidDuration),
    /// The exclusive end is not after the start.
    Reversed,
}

impl fmt::Display for InvalidMediaTrim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSeparator => formatter.write_str("trim must contain one .. separator"),
            Self::MultipleSeparators => {
                formatter.write_str("trim cannot contain more than one .. separator")
            }
            Self::MissingBounds => formatter.write_str("trim must include at least one bound"),
            Self::InvalidStart(reason) => write!(formatter, "trim start is invalid: {reason}"),
            Self::InvalidEnd(reason) => write!(formatter, "trim end is invalid: {reason}"),
            Self::Reversed => formatter.write_str("trim end must be after its start"),
        }
    }
}

impl Error for InvalidMediaTrim {}

/// Exact positive ratio between source time and output time.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PlaybackRate {
    numerator: u32,
    denominator: u32,
}

impl PlaybackRate {
    /// Natural source playback.
    pub const ONE: Self = Self {
        numerator: 1,
        denominator: 1,
    };

    /// Creates a canonical exact positive playback rate.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPlaybackRate`] when either ratio part is zero.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, InvalidPlaybackRate> {
        if numerator == 0 {
            return Err(InvalidPlaybackRate::Zero);
        }
        if denominator == 0 {
            return Err(InvalidPlaybackRate::ZeroDenominator);
        }
        let divisor = greatest_common_divisor_u32(numerator, denominator);
        Ok(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    /// Parses `integer[.fraction]x` into a canonical rational rate.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidPlaybackRate`] when the spelling, precision, or exact
    /// ratio is invalid.
    pub fn parse(value: &str) -> Result<Self, InvalidPlaybackRate> {
        let number = value
            .strip_suffix('x')
            .ok_or(InvalidPlaybackRate::Malformed)?;
        let (integer, fraction) = split_rate(number)?;
        let denominator = RATE_DENOMINATORS
            .get(fraction.len())
            .copied()
            .ok_or(InvalidPlaybackRate::TooPrecise)?;
        let whole = parse_rate_digits(integer)?;
        let fraction = parse_rate_digits(fraction)?;
        let numerator = whole
            .checked_mul(denominator)
            .and_then(|whole| whole.checked_add(fraction))
            .ok_or(InvalidPlaybackRate::OutOfRange)?;
        if numerator == 0 {
            return Err(InvalidPlaybackRate::Zero);
        }

        let divisor = greatest_common_divisor(numerator, denominator);
        Ok(Self {
            numerator: u32::try_from(numerator / divisor)
                .map_err(|_| InvalidPlaybackRate::OutOfRange)?,
            denominator: u32::try_from(denominator / divisor)
                .map_err(|_| InvalidPlaybackRate::OutOfRange)?,
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

/// Reason authored playback-rate text is invalid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidPlaybackRate {
    /// The rate does not follow the decimal-`x` grammar.
    Malformed,
    /// The fraction exceeds the admitted precision.
    TooPrecise,
    /// Zero cannot advance through source media.
    Zero,
    /// A rational rate cannot have a zero denominator.
    ZeroDenominator,
    /// The reduced rational value does not fit in its domain.
    OutOfRange,
}

impl fmt::Display for InvalidPlaybackRate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Malformed => "speed must be an unsigned decimal followed by x",
            Self::TooPrecise => "speed exceeds six fractional digits",
            Self::Zero => "speed must be greater than zero",
            Self::ZeroDenominator => "speed denominator must be greater than zero",
            Self::OutOfRange => "speed exceeds the supported rational range",
        };
        formatter.write_str(message)
    }
}

impl Error for InvalidPlaybackRate {}

/// Concrete source interval selected from one frozen media artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaSourceInterval {
    start: Duration,
    end: Duration,
}

impl MediaSourceInterval {
    /// Creates a non-empty half-open source interval.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidMediaSourceInterval`] unless `start < end`.
    pub const fn new(start: Duration, end: Duration) -> Result<Self, InvalidMediaSourceInterval> {
        if start.as_nanos() >= end.as_nanos() {
            return Err(InvalidMediaSourceInterval { start, end });
        }
        Ok(Self { start, end })
    }

    /// Returns the inclusive source start.
    #[must_use]
    pub const fn start(self) -> Duration {
        self.start
    }

    /// Returns the exclusive source end.
    #[must_use]
    pub const fn end(self) -> Duration {
        self.end
    }

    /// Returns the exact selected source duration.
    #[must_use]
    pub const fn duration(self) -> Duration {
        Duration::from_nanos(self.end.as_nanos() - self.start.as_nanos())
    }
}

/// A source interval whose exclusive end does not follow its start.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidMediaSourceInterval {
    start: Duration,
    end: Duration,
}

impl fmt::Display for InvalidMediaSourceInterval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "media source interval {}..{} is not increasing",
            self.start, self.end
        )
    }
}

impl Error for InvalidMediaSourceInterval {}

/// Solved mapping from output time into one source artifact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MediaSource {
    interval: MediaSourceInterval,
    playback_rate: PlaybackRate,
    plays: PlayCount,
    hold_last: Duration,
    natural_duration: Duration,
}

impl MediaSource {
    /// Creates a solved source mapping bounded by its frozen artifact.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidMediaSource`] when the selected end exceeds the
    /// artifact's natural duration.
    pub const fn new(
        interval: MediaSourceInterval,
        playback_rate: PlaybackRate,
        plays: PlayCount,
        hold_last: Duration,
        natural_duration: Duration,
    ) -> Result<Self, InvalidMediaSource> {
        if interval.end().as_nanos() > natural_duration.as_nanos() {
            return Err(InvalidMediaSource {
                interval,
                natural_duration,
            });
        }
        Ok(Self {
            interval,
            playback_rate,
            plays,
            hold_last,
            natural_duration,
        })
    }

    /// Returns the concrete half-open source interval.
    #[must_use]
    pub const fn interval(self) -> MediaSourceInterval {
        self.interval
    }

    /// Returns the exact source-to-output playback ratio.
    #[must_use]
    pub const fn playback_rate(self) -> PlaybackRate {
        self.playback_rate
    }

    /// Returns the exact number of complete source passes.
    #[must_use]
    pub const fn plays(self) -> PlayCount {
        self.plays
    }

    /// Returns the exact final-frame hold after all source passes.
    #[must_use]
    pub const fn hold_last(self) -> Duration {
        self.hold_last
    }

    /// Returns the frozen artifact's natural source duration.
    #[must_use]
    pub const fn natural_duration(self) -> Duration {
        self.natural_duration
    }

    /// Returns whether output samples the complete source at its natural rate.
    #[must_use]
    pub const fn is_identity(self) -> bool {
        self.interval.start().as_nanos() == 0
            && self.interval.end().as_nanos() == self.natural_duration.as_nanos()
            && self.playback_rate.numerator() == 1
            && self.playback_rate.denominator() == 1
            && self.plays.get() == 1
            && self.hold_last.as_nanos() == 0
    }
}

/// A solved source selection that escapes its frozen artifact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidMediaSource {
    interval: MediaSourceInterval,
    natural_duration: Duration,
}

impl fmt::Display for InvalidMediaSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "media source selection ends at {} after the artifact ends at {}",
            self.interval.end(),
            self.natural_duration,
        )
    }
}

impl Error for InvalidMediaSource {}

fn parse_trim_start(value: &str) -> Result<Duration, InvalidMediaTrim> {
    if value.is_empty() {
        return Ok(Duration::ZERO);
    }
    Duration::parse(value).map_err(InvalidMediaTrim::InvalidStart)
}

fn parse_trim_end(value: &str) -> Result<Option<Duration>, InvalidMediaTrim> {
    if value.is_empty() {
        return Ok(None);
    }
    Duration::parse(value)
        .map(Some)
        .map_err(InvalidMediaTrim::InvalidEnd)
}

fn split_rate(value: &str) -> Result<(&str, &str), InvalidPlaybackRate> {
    if value.is_empty() {
        return Err(InvalidPlaybackRate::Malformed);
    }

    let Some((integer, fraction)) = value.split_once('.') else {
        return Ok((value, ""));
    };
    if integer.is_empty() || fraction.is_empty() || fraction.contains('.') {
        return Err(InvalidPlaybackRate::Malformed);
    }
    Ok((integer, fraction))
}

fn parse_rate_digits(value: &str) -> Result<u128, InvalidPlaybackRate> {
    if value.is_empty() {
        return Ok(0);
    }
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(InvalidPlaybackRate::Malformed);
    }
    value
        .parse::<u128>()
        .map_err(|_| InvalidPlaybackRate::OutOfRange)
}

const fn greatest_common_divisor(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

const fn greatest_common_divisor_u32(mut left: u32, mut right: u32) -> u32 {
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
        InvalidMediaTrim, InvalidPlayCount, InvalidPlaybackRate, MediaTrim, PlayCount, PlaybackRate,
    };
    use crate::model::{Duration, InvalidDuration};
    use proptest::prelude::*;

    #[test]
    fn parses_open_and_closed_source_intervals() {
        assert_eq!(
            MediaTrim::parse("250ms..2.25s"),
            Ok(MediaTrim {
                start: Duration::from_nanos(250_000_000),
                end: Some(Duration::from_nanos(2_250_000_000)),
            })
        );
        assert_eq!(
            MediaTrim::parse("3s.."),
            Ok(MediaTrim {
                start: Duration::from_nanos(3_000_000_000),
                end: None,
            })
        );
        assert_eq!(
            MediaTrim::parse("..3s"),
            Ok(MediaTrim {
                start: Duration::ZERO,
                end: Some(Duration::from_nanos(3_000_000_000)),
            })
        );
    }

    #[test]
    fn rejects_invalid_source_intervals() {
        assert_eq!(
            MediaTrim::parse("3s"),
            Err(InvalidMediaTrim::MissingSeparator)
        );
        assert_eq!(
            MediaTrim::parse("1s..2s..3s"),
            Err(InvalidMediaTrim::MultipleSeparators)
        );
        assert_eq!(MediaTrim::parse(".."), Err(InvalidMediaTrim::MissingBounds));
        assert_eq!(
            MediaTrim::parse("bad..3s"),
            Err(InvalidMediaTrim::InvalidStart(InvalidDuration::Malformed))
        );
        assert_eq!(
            MediaTrim::parse("3s..bad"),
            Err(InvalidMediaTrim::InvalidEnd(InvalidDuration::Malformed))
        );
        assert_eq!(MediaTrim::parse("3s..2s"), Err(InvalidMediaTrim::Reversed));
    }

    #[test]
    fn canonicalizes_decimal_playback_rates() {
        assert_eq!(PlaybackRate::parse("1x"), Ok(PlaybackRate::ONE));
        assert_eq!(
            PlaybackRate::parse("0.5x"),
            Ok(PlaybackRate {
                numerator: 1,
                denominator: 2,
            })
        );
        assert_eq!(
            PlaybackRate::parse("2.500000x"),
            Ok(PlaybackRate {
                numerator: 5,
                denominator: 2,
            })
        );
    }

    #[test]
    fn parses_positive_play_counts() {
        assert_eq!(PlayCount::parse("1"), Ok(PlayCount::ONE));
        assert_eq!(PlayCount::parse("12"), Ok(PlayCount(12)));
        assert_eq!(PlayCount::parse("0"), Err(InvalidPlayCount::Zero));
        assert_eq!(PlayCount::parse("-1"), Err(InvalidPlayCount::Malformed));
        assert_eq!(
            PlayCount::parse("4294967296"),
            Err(InvalidPlayCount::OutOfRange)
        );
    }

    #[test]
    fn rejects_invalid_playback_rates() {
        assert_eq!(
            PlaybackRate::parse("2"),
            Err(InvalidPlaybackRate::Malformed)
        );
        assert_eq!(
            PlaybackRate::parse(".5x"),
            Err(InvalidPlaybackRate::Malformed)
        );
        assert_eq!(PlaybackRate::parse("0x"), Err(InvalidPlaybackRate::Zero));
        assert_eq!(
            PlaybackRate::parse("1.0000001x"),
            Err(InvalidPlaybackRate::TooPrecise)
        );
        assert_eq!(
            PlaybackRate::parse("4294967296x"),
            Err(InvalidPlaybackRate::OutOfRange)
        );
    }

    proptest! {
        #[test]
        fn canonical_rate_is_invariant_under_scaling(
            numerator in 1_u32..=65_535,
            denominator in 1_u32..=65_535,
            scale in 1_u32..=65_535,
        ) {
            let scaled = PlaybackRate::new(numerator * scale, denominator * scale);
            let canonical = PlaybackRate::new(numerator, denominator);

            prop_assert_eq!(scaled, canonical);
        }
    }
}
