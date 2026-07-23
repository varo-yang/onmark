//! Exact time values admitted across the Rust/browser wire boundary.

use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{FrameIndex, FrameInterval, FrameRate};

/// Largest integer represented exactly by every JavaScript implementation.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Exact rational frame rate represented with browser-safe integers.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WireFrameRate {
    #[cfg_attr(feature = "schema", schemars(range(min = 1, max = u32::MAX)))]
    numerator: u32,
    #[cfg_attr(feature = "schema", schemars(range(min = 1, max = u32::MAX)))]
    denominator: u32,
}

impl WireFrameRate {
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

impl From<FrameRate> for WireFrameRate {
    fn from(rate: FrameRate) -> Self {
        Self {
            numerator: rate.numerator(),
            denominator: rate.denominator(),
        }
    }
}

impl<'de> Deserialize<'de> for WireFrameRate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireFrameRateWire::deserialize(deserializer)?;
        let rate = FrameRate::new(wire.numerator, wire.denominator)
            .map_err(|source| D::Error::custom(source.to_string()))?;
        if rate.numerator() != wire.numerator || rate.denominator() != wire.denominator {
            return Err(D::Error::custom("frame rate is not in canonical form"));
        }

        Ok(Self {
            numerator: wire.numerator,
            denominator: wire.denominator,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFrameRateWire {
    numerator: u32,
    denominator: u32,
}

/// One half-open browser frame interval.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WireInterval {
    start: WireFrame,
    end: WireFrame,
}

impl WireInterval {
    /// Returns the inclusive start frame.
    #[must_use]
    pub const fn start(self) -> WireFrame {
        self.start
    }

    /// Returns the exclusive end frame.
    #[must_use]
    pub const fn end(self) -> WireFrame {
        self.end
    }

    pub(super) const fn contains_interval(self, other: Self) -> bool {
        self.start.get() <= other.start.get() && other.end.get() <= self.end.get()
    }

    pub(super) const fn is_empty(self) -> bool {
        self.start.get() == self.end.get()
    }
}

impl<'de> Deserialize<'de> for WireInterval {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireIntervalWire::deserialize(deserializer)?;
        if wire.end.get() < wire.start.get() {
            return Err(D::Error::custom("frame interval ends before it starts"));
        }

        Ok(Self {
            start: wire.start,
            end: wire.end,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireIntervalWire {
    start: WireFrame,
    end: WireFrame,
}

impl TryFrom<FrameInterval> for WireInterval {
    type Error = InvalidWireFrame;

    fn try_from(interval: FrameInterval) -> Result<Self, Self::Error> {
        Ok(Self {
            start: WireFrame::try_from(interval.start())?,
            end: WireFrame::try_from(interval.end())?,
        })
    }
}

/// Exact frame integer accepted by JavaScript without rounding.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct WireFrame(#[cfg_attr(feature = "schema", schemars(range(max = MAX_SAFE_INTEGER)))] u64);

impl WireFrame {
    /// Creates an exact browser frame integer.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidWireFrame`] when the value exceeds JavaScript's safe
    /// integer range.
    pub const fn new(value: u64) -> Result<Self, InvalidWireFrame> {
        if value > MAX_SAFE_INTEGER {
            return Err(InvalidWireFrame::OutsideSafeIntegerRange);
        }
        Ok(Self(value))
    }

    /// Returns the exact integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<FrameIndex> for WireFrame {
    type Error = InvalidWireFrame;

    fn try_from(frame: FrameIndex) -> Result<Self, Self::Error> {
        Self::new(frame.get())
    }
}

impl<'de> Deserialize<'de> for WireFrame {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let frame = u64::deserialize(deserializer)?;
        Self::new(frame).map_err(D::Error::custom)
    }
}

/// Reason a core frame cannot cross the browser wire boundary exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidWireFrame {
    /// JavaScript would round this integer representation.
    OutsideSafeIntegerRange,
}

impl fmt::Display for InvalidWireFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("frame exceeds JavaScript's exact integer range")
    }
}

impl Error for InvalidWireFrame {}

#[cfg(test)]
mod tests {
    use super::{InvalidWireFrame, MAX_SAFE_INTEGER, WireFrame};

    #[test]
    fn rejects_a_frame_that_javascript_would_round() {
        assert_eq!(
            WireFrame::new(MAX_SAFE_INTEGER + 1),
            Err(InvalidWireFrame::OutsideSafeIntegerRange),
        );
    }

    #[test]
    fn rejects_an_unsafe_deserialized_frame() {
        let encoded = (MAX_SAFE_INTEGER + 1).to_string();
        assert!(serde_json::from_str::<WireFrame>(&encoded).is_err());
    }
}
