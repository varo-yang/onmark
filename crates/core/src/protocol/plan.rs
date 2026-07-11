use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{FrameIndex, FrameInterval};
use crate::timeline::TimelineIr;

/// Largest integer represented exactly by every JavaScript implementation.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Timeline facts needed to initialize the Gate-one browser clock.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserPlan {
    #[cfg_attr(feature = "schema", schemars(extend("const" = 1)))]
    timeline_version: u16,
    frame_rate: WireFrameRate,
    evaluation: WireInterval,
    output: WireInterval,
}

impl BrowserPlan {
    /// Returns the Timeline IR version that produced this browser plan.
    #[must_use]
    pub const fn timeline_version(self) -> u16 {
        self.timeline_version
    }

    /// Returns the exact rational browser frame rate.
    #[must_use]
    pub const fn frame_rate(self) -> WireFrameRate {
        self.frame_rate
    }

    /// Returns frames that must be evaluated by this unit.
    #[must_use]
    pub const fn evaluation(self) -> WireInterval {
        self.evaluation
    }

    /// Returns frames published by this unit.
    #[must_use]
    pub const fn output(self) -> WireInterval {
        self.output
    }
}

impl TryFrom<&TimelineIr> for BrowserPlan {
    type Error = InvalidWireFrame;

    fn try_from(timeline: &TimelineIr) -> Result<Self, Self::Error> {
        let rate = timeline.timebase().frame_rate();
        let interval = WireInterval::try_from(timeline.interval())?;

        Ok(Self {
            timeline_version: timeline.version().get(),
            frame_rate: WireFrameRate {
                numerator: rate.numerator(),
                denominator: rate.denominator(),
            },
            evaluation: interval,
            output: interval,
        })
    }
}

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
