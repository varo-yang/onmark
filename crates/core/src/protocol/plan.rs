use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{FrameIndex, FrameInterval, FrameRate, FrozenAssetId};
use crate::timeline::{TimelineIr, TimelineVideo};

/// Largest integer represented exactly by every JavaScript implementation.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Timeline facts consumed by the Gate-one browser clock and presentation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserPlan {
    #[cfg_attr(feature = "schema", schemars(extend("const" = 1)))]
    timeline_version: u16,
    frame_rate: WireFrameRate,
    evaluation: WireInterval,
    output: WireInterval,
    videos: Vec<BrowserVideo>,
}

impl BrowserPlan {
    /// Projects one whole-film timeline with admitted source video rates.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserPlan`] when a video has no admitted source rate
    /// or a frame lies outside JavaScript's exact integer domain.
    pub fn from_timeline(
        timeline: &TimelineIr,
        source_frame_rates: &BTreeMap<FrozenAssetId, FrameRate>,
    ) -> Result<Self, InvalidBrowserPlan> {
        let rate = timeline.timebase().frame_rate();
        let interval = WireInterval::try_from(timeline.interval())?;
        let videos = timeline
            .videos()
            .map(|video| browser_video(video, source_frame_rates))
            .collect::<Result<_, _>>()?;

        Ok(Self {
            timeline_version: timeline.version().get(),
            frame_rate: rate.into(),
            evaluation: interval,
            output: interval,
            videos,
        })
    }

    /// Returns the Timeline IR version that produced this browser plan.
    #[must_use]
    pub const fn timeline_version(&self) -> u16 {
        self.timeline_version
    }

    /// Returns the exact rational browser frame rate.
    #[must_use]
    pub const fn frame_rate(&self) -> WireFrameRate {
        self.frame_rate
    }

    /// Returns frames that must be evaluated by this unit.
    #[must_use]
    pub const fn evaluation(&self) -> WireInterval {
        self.evaluation
    }

    /// Returns frames published by this unit.
    #[must_use]
    pub const fn output(&self) -> WireInterval {
        self.output
    }

    /// Returns primary video placements in screenplay order.
    #[must_use]
    pub fn videos(&self) -> &[BrowserVideo] {
        &self.videos
    }
}

/// One primary video placement consumed by the browser presentation adapter.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserVideo {
    #[cfg_attr(
        feature = "schema",
        schemars(regex(pattern = r"^sha256:[0-9a-f]{64}$"))
    )]
    asset_id: Box<str>,
    interval: WireInterval,
    source_frame_rate: WireFrameRate,
}

impl BrowserVideo {
    /// Returns the immutable asset identity resolved by materialization.
    #[must_use]
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }

    /// Returns the absolute frames during which the video is visible.
    #[must_use]
    pub const fn interval(&self) -> WireInterval {
        self.interval
    }

    /// Returns the exact selected source-stream frame rate.
    #[must_use]
    pub const fn source_frame_rate(&self) -> WireFrameRate {
        self.source_frame_rate
    }
}

fn browser_video(
    video: &TimelineVideo,
    source_frame_rates: &BTreeMap<FrozenAssetId, FrameRate>,
) -> Result<BrowserVideo, InvalidBrowserPlan> {
    let asset_id = video.asset_id();
    let source_frame_rate = source_frame_rates
        .get(&asset_id)
        .copied()
        .ok_or(InvalidBrowserPlan::MissingSourceFrameRate(asset_id))?;

    Ok(BrowserVideo {
        asset_id: asset_id.to_string().into_boxed_str(),
        interval: WireInterval::try_from(video.timing().interval())?,
        source_frame_rate: source_frame_rate.into(),
    })
}

/// Reason Timeline IR cannot form an exact browser-facing plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidBrowserPlan {
    /// One video lacks the source rate proved during render admission.
    MissingSourceFrameRate(FrozenAssetId),
    /// A frame lies outside JavaScript's exact integer range.
    InvalidFrame(InvalidWireFrame),
}

impl fmt::Display for InvalidBrowserPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSourceFrameRate(id) => {
                write!(formatter, "source frame rate is missing for video {id}")
            }
            Self::InvalidFrame(source) => source.fmt(formatter),
        }
    }
}

impl Error for InvalidBrowserPlan {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidFrame(source) => Some(source),
            Self::MissingSourceFrameRate(_) => None,
        }
    }
}

impl From<InvalidWireFrame> for InvalidBrowserPlan {
    fn from(source: InvalidWireFrame) -> Self {
        Self::InvalidFrame(source)
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

impl From<FrameRate> for WireFrameRate {
    fn from(rate: FrameRate) -> Self {
        Self {
            numerator: rate.numerator(),
            denominator: rate.denominator(),
        }
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
