use std::error::Error;
use std::fmt;

const NANOS_PER_SECOND: u64 = 1_000_000_000;
const NANOS_PER_MILLISECOND: u64 = 1_000_000;

/// Exact non-negative authored duration measured in nanoseconds.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Duration(u64);

impl Duration {
    /// A duration containing no time.
    pub const ZERO: Self = Self(0);

    /// Creates a duration from its exact integer representation.
    #[must_use]
    pub const fn from_nanos(nanoseconds: u64) -> Self {
        Self(nanoseconds)
    }

    /// Parses `integer[.fraction]s` or `integer[.fraction]ms` exactly.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDuration`] when the spelling, unit, precision, or
    /// resulting nanosecond value is invalid.
    pub fn parse(value: &str) -> Result<Self, InvalidDuration> {
        if value.is_empty() {
            return Err(InvalidDuration::Empty);
        }

        let (number, unit) = split_number_and_unit(value)?;
        let (scale, precision) = match unit {
            "s" => (NANOS_PER_SECOND, 9),
            "ms" => (NANOS_PER_MILLISECOND, 6),
            _ => return Err(InvalidDuration::UnknownUnit),
        };
        let (integer, fraction) = number.split_once('.').unwrap_or((number, ""));

        if fraction.len() > precision {
            return Err(InvalidDuration::TooPrecise);
        }

        let integer = parse_digits(integer)?;
        let fraction = parse_fraction(fraction, precision)?;
        let nanoseconds = integer
            .checked_mul(scale)
            .and_then(|whole| whole.checked_add(fraction))
            .ok_or(InvalidDuration::OutOfRange)?;

        Ok(Self(nanoseconds))
    }

    /// Returns the exact nanosecond representation.
    #[must_use]
    pub const fn as_nanos(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = self.0 / NANOS_PER_SECOND;
        let remainder = self.0 % NANOS_PER_SECOND;

        if remainder == 0 {
            return write!(formatter, "{seconds}s");
        }

        let fraction = format!("{remainder:09}");
        write!(formatter, "{seconds}.{}s", fraction.trim_end_matches('0'))
    }
}

/// Reason authored duration text cannot become an exact duration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidDuration {
    /// No duration text was authored.
    Empty,
    /// The numeric spelling violates the duration grammar.
    Malformed,
    /// The suffix is not `s` or `ms`.
    UnknownUnit,
    /// The fraction exceeds the precision admitted by its unit.
    TooPrecise,
    /// The exact nanosecond value does not fit in `u64`.
    OutOfRange,
}

impl fmt::Display for InvalidDuration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "duration cannot be empty",
            Self::Malformed => "duration must be an unsigned decimal without whitespace",
            Self::UnknownUnit => "duration unit must be s or ms",
            Self::TooPrecise => "duration exceeds the supported fractional precision",
            Self::OutOfRange => "duration exceeds the supported nanosecond range",
        };
        formatter.write_str(message)
    }
}

impl Error for InvalidDuration {}

fn split_number_and_unit(value: &str) -> Result<(&str, &str), InvalidDuration> {
    let bytes = value.as_bytes();
    let mut cursor = bytes
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();

    if cursor == 0 {
        return Err(InvalidDuration::Malformed);
    }

    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction = bytes[cursor..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if fraction == 0 {
            return Err(InvalidDuration::Malformed);
        }
        cursor += fraction;
    }

    let unit = &value[cursor..];
    if unit.is_empty() || unit.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Ok((&value[..cursor], unit));
    }

    Err(InvalidDuration::Malformed)
}

fn parse_digits(value: &str) -> Result<u64, InvalidDuration> {
    value
        .parse::<u64>()
        .map_err(|_| InvalidDuration::OutOfRange)
}

fn parse_fraction(value: &str, precision: usize) -> Result<u64, InvalidDuration> {
    if value.is_empty() {
        return Ok(0);
    }

    let fraction = parse_digits(value)?;
    let padding = u32::try_from(precision - value.len()).expect("precision bounds the fraction");
    fraction
        .checked_mul(10_u64.pow(padding))
        .ok_or(InvalidDuration::OutOfRange)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Duration, InvalidDuration, NANOS_PER_MILLISECOND};

    #[test]
    fn parses_exact_seconds_and_milliseconds() {
        assert_eq!(
            Duration::parse("3s"),
            Ok(Duration::from_nanos(3_000_000_000))
        );
        assert_eq!(
            Duration::parse("500ms"),
            Ok(Duration::from_nanos(500_000_000))
        );
        assert_eq!(
            Duration::parse("1.5s"),
            Ok(Duration::from_nanos(1_500_000_000))
        );
    }

    #[test]
    fn distinguishes_invalid_duration_reasons() {
        assert_eq!(Duration::parse(""), Err(InvalidDuration::Empty));
        assert_eq!(Duration::parse("-1s"), Err(InvalidDuration::Malformed));
        assert_eq!(Duration::parse("3m"), Err(InvalidDuration::UnknownUnit));
        assert_eq!(
            Duration::parse("1.0000000000s"),
            Err(InvalidDuration::TooPrecise)
        );
        assert_eq!(
            Duration::parse("1.0000001ms"),
            Err(InvalidDuration::TooPrecise)
        );
        assert_eq!(
            Duration::parse("18446744074s"),
            Err(InvalidDuration::OutOfRange)
        );
    }

    proptest! {
        #[test]
        fn display_round_trips_every_duration(nanoseconds in any::<u64>()) {
            let duration = Duration::from_nanos(nanoseconds);
            prop_assert_eq!(Duration::parse(&duration.to_string()), Ok(duration));
        }

        #[test]
        fn milliseconds_equal_the_same_decimal_seconds(
            milliseconds in 0..=u64::MAX / NANOS_PER_MILLISECOND,
        ) {
            let seconds = milliseconds / 1_000;
            let remainder = milliseconds % 1_000;
            let seconds = format!("{seconds}.{remainder:03}s");
            let milliseconds = format!("{milliseconds}ms");

            prop_assert_eq!(Duration::parse(&milliseconds), Duration::parse(&seconds));
        }
    }
}
