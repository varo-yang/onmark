//! Checked browser projection of Timeline IR.
//!
//! Conversion establishes JavaScript-safe integer and collection bounds before
//! values cross the Rust/TypeScript boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{ElementKind, FrameIndex, FrameInterval, FrameRate, FrozenAssetId};
use crate::timeline::{
    TimelineCaption, TimelineIr, TimelineOverlay, TimelineText, TimelineVersion, TimelineVideo,
};

/// Largest integer represented exactly by every JavaScript implementation.
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_BROWSER_VIDEOS: usize = 10_000;
const MAX_BROWSER_OVERLAYS: usize = 10_000;
const MAX_BROWSER_OVERLAY_TEXT_CHARACTERS: usize = 65_536;
const MAX_BROWSER_OVERLAY_TEXT_BYTES: usize = 1 << 20;

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
    #[cfg_attr(
        feature = "schema",
        schemars(length(max = MAX_BROWSER_VIDEOS))
    )]
    videos: Vec<BrowserVideo>,
    #[cfg_attr(
        feature = "schema",
        schemars(length(max = MAX_BROWSER_OVERLAYS))
    )]
    overlays: Vec<BrowserOverlay>,
}

impl BrowserPlan {
    /// Projects one whole-film timeline with admitted source video rates.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserPlan`] when a placement exceeds its resource
    /// budget, a video has no admitted source rate, an overlay is malformed,
    /// or a frame lies outside JavaScript's exact integer domain.
    pub fn from_timeline(
        timeline: &TimelineIr,
        source_frame_rates: &BTreeMap<FrozenAssetId, FrameRate>,
    ) -> Result<Self, InvalidBrowserPlan> {
        let interval = timeline.interval();
        Self::from_timeline_for_unit(timeline, source_frame_rates, interval, interval)
    }

    /// Projects one evaluated and published unit from solved Timeline IR.
    ///
    /// Every browser placement must lie wholly inside `evaluation`. A later
    /// graph capability that needs a placement across a unit boundary must
    /// widen that region before projection; silently clipping a video would
    /// change its source-frame mapping.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserPlan`] when unit bounds are inconsistent, a
    /// placement crosses the evaluation boundary, a placement exceeds its
    /// resource budget, a video has no admitted source rate, an overlay is
    /// malformed, or a frame lies outside JavaScript's exact integer domain.
    pub fn from_timeline_for_unit(
        timeline: &TimelineIr,
        source_frame_rates: &BTreeMap<FrozenAssetId, FrameRate>,
        evaluation: FrameInterval,
        output: FrameInterval,
    ) -> Result<Self, InvalidBrowserPlan> {
        if !timeline.interval().contains_interval(evaluation) {
            return Err(InvalidBrowserPlan::EvaluationOutsideTimeline);
        }
        if !evaluation.contains_interval(output) {
            return Err(InvalidBrowserPlan::OutputOutsideEvaluation);
        }

        let evaluation_wire = WireInterval::try_from(evaluation)?;
        let output_wire = WireInterval::try_from(output)?;
        let videos = project_videos(timeline, source_frame_rates, evaluation)?;
        let overlays = project_overlays(timeline, evaluation)?;

        Self::checked(BrowserPlanWire {
            timeline_version: timeline.version().get(),
            frame_rate: timeline.timebase().frame_rate().into(),
            evaluation: evaluation_wire,
            output: output_wire,
            videos,
            overlays,
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

    /// Returns overlay placements in screenplay order.
    #[must_use]
    pub fn overlays(&self) -> &[BrowserOverlay] {
        &self.overlays
    }

    /// Returns the start and end frame of every browser placement.
    ///
    /// Placement visibility is constant between these boundaries. Native
    /// execution may index them once without reconstructing timeline facts or
    /// scanning every placement for every output frame.
    pub fn placement_boundaries(&self) -> impl Iterator<Item = WireFrame> + '_ {
        self.videos
            .iter()
            .flat_map(|video| interval_boundaries(video.interval()))
            .chain(
                self.overlays
                    .iter()
                    .flat_map(|overlay| interval_boundaries(overlay.interval())),
            )
    }

    fn checked(wire: BrowserPlanWire) -> Result<Self, InvalidBrowserPlan> {
        if wire.timeline_version != TimelineVersion::V1.get() {
            return Err(InvalidBrowserPlan::UnsupportedTimelineVersion);
        }
        if wire.videos.len() > MAX_BROWSER_VIDEOS {
            return Err(InvalidBrowserPlan::TooManyVideos);
        }
        if wire.overlays.len() > MAX_BROWSER_OVERLAYS {
            return Err(InvalidBrowserPlan::TooManyOverlays);
        }
        let mut component_ids = BTreeSet::new();
        if wire
            .overlays
            .iter()
            .any(|overlay| !component_ids.insert(overlay.component_id))
        {
            return Err(InvalidBrowserPlan::DuplicateComponentId);
        }
        if overlay_text_bytes(&wire.overlays) > MAX_BROWSER_OVERLAY_TEXT_BYTES {
            return Err(InvalidBrowserPlan::OverlayTextBudget);
        }
        if !wire.evaluation.contains_interval(wire.output) {
            return Err(InvalidBrowserPlan::OutputOutsideEvaluation);
        }
        if wire.output.is_empty() {
            return Err(InvalidBrowserPlan::EmptyOutput);
        }
        if wire.videos.iter().any(|video| video.interval().is_empty()) {
            return Err(InvalidBrowserPlan::EmptyVideo);
        }
        if wire
            .overlays
            .iter()
            .any(|overlay| overlay.interval().is_empty())
        {
            return Err(InvalidBrowserPlan::EmptyOverlay);
        }
        if wire
            .videos
            .iter()
            .any(|video| !wire.evaluation.contains_interval(video.interval()))
        {
            return Err(InvalidBrowserPlan::VideoCrossesEvaluation);
        }
        if wire
            .overlays
            .iter()
            .any(|overlay| !wire.evaluation.contains_interval(overlay.interval()))
        {
            return Err(InvalidBrowserPlan::OverlayCrossesEvaluation);
        }

        Ok(Self {
            timeline_version: wire.timeline_version,
            frame_rate: wire.frame_rate,
            evaluation: wire.evaluation,
            output: wire.output,
            videos: wire.videos,
            overlays: wire.overlays,
        })
    }
}

fn project_videos(
    timeline: &TimelineIr,
    source_frame_rates: &BTreeMap<FrozenAssetId, FrameRate>,
    evaluation: FrameInterval,
) -> Result<Vec<BrowserVideo>, InvalidBrowserPlan> {
    let mut videos = Vec::new();

    for video in timeline.videos() {
        let interval = video.timing().interval();
        if !interval.intersects(evaluation) {
            continue;
        }
        if !evaluation.contains_interval(interval) {
            return Err(InvalidBrowserPlan::VideoCrossesEvaluation);
        }
        if videos.len() == MAX_BROWSER_VIDEOS {
            return Err(InvalidBrowserPlan::TooManyVideos);
        }
        videos.push(browser_video(video, source_frame_rates)?);
    }

    Ok(videos)
}

fn project_overlays(
    timeline: &TimelineIr,
    evaluation: FrameInterval,
) -> Result<Vec<BrowserOverlay>, InvalidBrowserPlan> {
    let mut overlays = Vec::new();
    let mut text_bytes = 0_usize;
    let mut next_component_id = 0_u32;

    for overlay in timeline.overlays() {
        let component_id = take_component_id(&mut next_component_id)?;
        let interval = overlay.timing().interval();
        if !interval.intersects(evaluation) {
            continue;
        }
        if !evaluation.contains_interval(interval) {
            return Err(InvalidBrowserPlan::OverlayCrossesEvaluation);
        }
        push_browser_overlay(
            &mut overlays,
            &mut text_bytes,
            browser_overlay(overlay, component_id)?,
        )?;
    }
    for caption in timeline.captions() {
        let component_id = take_component_id(&mut next_component_id)?;
        let Some(interval) = intersection(caption.interval(), evaluation) else {
            continue;
        };
        push_browser_overlay(
            &mut overlays,
            &mut text_bytes,
            browser_caption(caption, component_id, interval)?,
        )?;
    }

    Ok(overlays)
}

fn push_browser_overlay(
    overlays: &mut Vec<BrowserOverlay>,
    text_bytes: &mut usize,
    overlay: BrowserOverlay,
) -> Result<(), InvalidBrowserPlan> {
    if overlays.len() == MAX_BROWSER_OVERLAYS {
        return Err(InvalidBrowserPlan::TooManyOverlays);
    }

    *text_bytes = text_bytes
        .checked_add(overlay.text.len())
        .filter(|bytes| *bytes <= MAX_BROWSER_OVERLAY_TEXT_BYTES)
        .ok_or(InvalidBrowserPlan::OverlayTextBudget)?;
    overlays.push(overlay);
    Ok(())
}

fn intersection(left: FrameInterval, right: FrameInterval) -> Option<FrameInterval> {
    let start = left.start().max(right.start());
    let end = left.end().min(right.end());
    (start < end).then(|| {
        FrameInterval::new(start, end).expect("intersecting ordered intervals remain ordered")
    })
}

fn interval_boundaries(interval: WireInterval) -> [WireFrame; 2] {
    [interval.start(), interval.end()]
}

impl<'de> Deserialize<'de> for BrowserPlan {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::checked(BrowserPlanWire::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserPlanWire {
    timeline_version: u16,
    frame_rate: WireFrameRate,
    evaluation: WireInterval,
    output: WireInterval,
    videos: Vec<BrowserVideo>,
    overlays: Vec<BrowserOverlay>,
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
    #[serde(skip)]
    #[cfg_attr(feature = "schema", schemars(skip))]
    asset_identity: FrozenAssetId,
    interval: WireInterval,
    source_frame_rate: WireFrameRate,
}

impl BrowserVideo {
    /// Returns the immutable asset identity resolved by materialization.
    #[must_use]
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }

    /// Returns the already-validated immutable asset identity.
    #[must_use]
    pub const fn asset_identity(&self) -> FrozenAssetId {
        self.asset_identity
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

impl<'de> Deserialize<'de> for BrowserVideo {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BrowserVideoWire::deserialize(deserializer)?;
        let asset_identity = FrozenAssetId::parse(&wire.asset_id).map_err(D::Error::custom)?;

        Ok(Self {
            asset_id: wire.asset_id,
            asset_identity,
            interval: wire.interval,
            source_frame_rate: wire.source_frame_rate,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserVideoWire {
    asset_id: Box<str>,
    interval: WireInterval,
    source_frame_rate: WireFrameRate,
}

/// Closed overlay roles understood by the Gate-one presentation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserOverlayKind {
    /// Authored title content.
    Title,
    /// Authored call-to-action content.
    CallToAction,
    /// Imported caption text.
    Caption,
}

/// Stable overlay identity retained across whole-film and partition plans.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BrowserComponentId(#[cfg_attr(feature = "schema", schemars(range(max = u32::MAX)))] u32);

impl BrowserComponentId {
    const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the stable integer representation.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// One solved overlay placement consumed by the browser presentation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserOverlay {
    component_id: BrowserComponentId,
    kind: BrowserOverlayKind,
    #[cfg_attr(
        feature = "schema",
        schemars(length(max = MAX_BROWSER_OVERLAY_TEXT_CHARACTERS))
    )]
    text: Box<str>,
    interval: WireInterval,
}

impl BrowserOverlay {
    /// Returns the compiler-owned component identity.
    #[must_use]
    pub const fn component_id(&self) -> BrowserComponentId {
        self.component_id
    }

    /// Returns the presentation role selected by the screenplay element.
    #[must_use]
    pub const fn kind(&self) -> BrowserOverlayKind {
        self.kind
    }

    /// Returns decoded authored text with source runs joined in order.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the absolute frames during which the overlay is visible.
    #[must_use]
    pub const fn interval(&self) -> WireInterval {
        self.interval
    }
}

impl<'de> Deserialize<'de> for BrowserOverlay {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BrowserOverlayWire::deserialize(deserializer)?;
        if text_exceeds_limit(&wire.text) {
            return Err(D::Error::custom(
                "browser overlay text exceeds the character limit",
            ));
        }

        Ok(Self {
            component_id: wire.component_id,
            kind: wire.kind,
            text: wire.text,
            interval: wire.interval,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserOverlayWire {
    component_id: BrowserComponentId,
    kind: BrowserOverlayKind,
    text: Box<str>,
    interval: WireInterval,
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
        asset_identity: asset_id,
        interval: WireInterval::try_from(video.timing().interval())?,
        source_frame_rate: source_frame_rate.into(),
    })
}

fn browser_overlay(
    overlay: &TimelineOverlay,
    component_id: BrowserComponentId,
) -> Result<BrowserOverlay, InvalidBrowserPlan> {
    let element_kind = overlay.element().kind();
    let kind = match element_kind {
        ElementKind::Title => BrowserOverlayKind::Title,
        ElementKind::CallToAction => BrowserOverlayKind::CallToAction,
        _ => return Err(InvalidBrowserPlan::InvalidOverlayKind(element_kind)),
    };
    let text = overlay
        .text()
        .iter()
        .map(TimelineText::text)
        .collect::<String>();
    if text_exceeds_limit(&text) {
        return Err(InvalidBrowserPlan::OverlayTextTooLong(element_kind));
    }

    Ok(BrowserOverlay {
        component_id,
        kind,
        text: text.into_boxed_str(),
        interval: WireInterval::try_from(overlay.timing().interval())?,
    })
}

fn browser_caption(
    caption: &TimelineCaption,
    component_id: BrowserComponentId,
    interval: FrameInterval,
) -> Result<BrowserOverlay, InvalidBrowserPlan> {
    if text_exceeds_limit(caption.text()) {
        return Err(InvalidBrowserPlan::CaptionTextTooLong);
    }

    Ok(BrowserOverlay {
        component_id,
        kind: BrowserOverlayKind::Caption,
        text: caption.text().into(),
        interval: WireInterval::try_from(interval)?,
    })
}

fn take_component_id(next: &mut u32) -> Result<BrowserComponentId, InvalidBrowserPlan> {
    let component_id = BrowserComponentId::new(*next);
    *next = next
        .checked_add(1)
        .ok_or(InvalidBrowserPlan::TooManyOverlays)?;
    Ok(component_id)
}

fn overlay_text_bytes(overlays: &[BrowserOverlay]) -> usize {
    overlays
        .iter()
        .map(|overlay| overlay.text.len())
        .try_fold(0_usize, usize::checked_add)
        .unwrap_or(usize::MAX)
}

fn text_exceeds_limit(text: &str) -> bool {
    text.chars()
        .take(MAX_BROWSER_OVERLAY_TEXT_CHARACTERS + 1)
        .count()
        > MAX_BROWSER_OVERLAY_TEXT_CHARACTERS
}

/// Reason Timeline IR cannot form an exact browser-facing plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidBrowserPlan {
    /// The plan names a Timeline IR version this runtime cannot consume.
    UnsupportedTimelineVersion,
    /// The unit evaluation interval lies outside the solved film.
    EvaluationOutsideTimeline,
    /// The published interval lies outside the unit evaluation interval.
    OutputOutsideEvaluation,
    /// The published interval contains no frame.
    EmptyOutput,
    /// A video placement contains no frame.
    EmptyVideo,
    /// An overlay placement contains no frame.
    EmptyOverlay,
    /// A video would need clipping at the unit evaluation boundary.
    VideoCrossesEvaluation,
    /// An overlay would need clipping at the unit evaluation boundary.
    OverlayCrossesEvaluation,
    /// The plan contains more video placements than V1 can carry.
    TooManyVideos,
    /// The plan contains more overlay placements than V1 can carry.
    TooManyOverlays,
    /// Two overlay placements claim the same component identity.
    DuplicateComponentId,
    /// A Timeline overlay carries a non-overlay element kind.
    InvalidOverlayKind(ElementKind),
    /// One overlay inscription exceeds the V1 character budget.
    OverlayTextTooLong(ElementKind),
    /// One imported caption exceeds the per-placement text budget.
    CaptionTextTooLong,
    /// Combined browser text would exceed the bounded CDP request budget.
    OverlayTextBudget,
    /// One video lacks the source rate proved during render admission.
    MissingSourceFrameRate(FrozenAssetId),
    /// A frame lies outside JavaScript's exact integer range.
    InvalidFrame(InvalidWireFrame),
}

impl fmt::Display for InvalidBrowserPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTimelineVersion => {
                formatter.write_str("unsupported browser plan timeline version")
            }
            Self::EvaluationOutsideTimeline => {
                formatter.write_str("browser evaluation interval lies outside the solved film")
            }
            Self::OutputOutsideEvaluation => {
                formatter.write_str("browser output interval lies outside evaluation")
            }
            Self::EmptyOutput => formatter.write_str("browser output interval is empty"),
            Self::EmptyVideo => formatter.write_str("browser video interval is empty"),
            Self::EmptyOverlay => formatter.write_str("browser overlay interval is empty"),
            Self::VideoCrossesEvaluation => {
                formatter.write_str("browser video crosses the evaluation boundary")
            }
            Self::OverlayCrossesEvaluation => {
                formatter.write_str("browser overlay crosses the evaluation boundary")
            }
            Self::TooManyVideos => {
                formatter.write_str("browser plan exceeds the V1 video-placement limit")
            }
            Self::TooManyOverlays => {
                formatter.write_str("browser plan exceeds the V1 overlay-placement limit")
            }
            Self::DuplicateComponentId => {
                formatter.write_str("browser overlay component identity is duplicated")
            }
            Self::InvalidOverlayKind(kind) => {
                write!(
                    formatter,
                    "timeline element {kind} is not a browser overlay"
                )
            }
            Self::OverlayTextTooLong(kind) => {
                write!(
                    formatter,
                    "browser {kind} text exceeds the V1 character limit"
                )
            }
            Self::CaptionTextTooLong => {
                formatter.write_str("browser caption text exceeds the V1 character limit")
            }
            Self::OverlayTextBudget => {
                formatter.write_str("browser overlay text exceeds the request byte budget")
            }
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
            Self::UnsupportedTimelineVersion
            | Self::EvaluationOutsideTimeline
            | Self::OutputOutsideEvaluation
            | Self::EmptyOutput
            | Self::EmptyVideo
            | Self::EmptyOverlay
            | Self::VideoCrossesEvaluation
            | Self::OverlayCrossesEvaluation
            | Self::TooManyVideos
            | Self::TooManyOverlays
            | Self::DuplicateComponentId
            | Self::InvalidOverlayKind(_)
            | Self::OverlayTextTooLong(_)
            | Self::CaptionTextTooLong
            | Self::OverlayTextBudget
            | Self::MissingSourceFrameRate(_) => None,
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

    const fn contains_interval(self, other: Self) -> bool {
        self.start.get() <= other.start.get() && other.end.get() <= self.end.get()
    }

    const fn is_empty(self) -> bool {
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
    use std::collections::{BTreeMap, BTreeSet};

    use crate::model::{
        ByteOffset, ElementKind, FrameIndex, FrameInterval, FrameRate, FrozenAssetId, SourceId,
        SourceSpan, Timebase,
    };
    use crate::timeline::{
        TimelineCaption, TimelineContent, TimelineElement, TimelineIr, TimelineOverlay,
        TimelineScene, TimelineShot, TimelineText, TimelineTiming, TimelineVideo, TimingReason,
    };

    use super::{
        BrowserOverlayKind, BrowserPlan, InvalidBrowserPlan, InvalidWireFrame,
        MAX_BROWSER_OVERLAY_TEXT_BYTES, MAX_BROWSER_OVERLAY_TEXT_CHARACTERS, MAX_BROWSER_OVERLAYS,
        MAX_BROWSER_VIDEOS, MAX_SAFE_INTEGER, WireFrame,
    };

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

    #[test]
    fn parses_only_validated_browser_plan_facts() {
        let plan = r#"{
            "timelineVersion":1,
            "frameRate":{"numerator":30,"denominator":1},
            "evaluation":{"start":0,"end":1},
            "output":{"start":0,"end":1},
            "videos":[],
            "overlays":[]
        }"#;

        let parsed = serde_json::from_str::<BrowserPlan>(plan)
            .expect("the canonical browser plan fixture is valid");
        assert_eq!(parsed.output().end().get(), 1);

        let noncanonical_rate = plan.replace(
            "\"numerator\":30,\"denominator\":1",
            "\"numerator\":60,\"denominator\":2",
        );
        assert!(serde_json::from_str::<BrowserPlan>(&noncanonical_rate).is_err());

        let empty_output = plan.replace(
            "\"output\":{\"start\":0,\"end\":1}",
            "\"output\":{\"start\":0,\"end\":0}",
        );
        assert!(serde_json::from_str::<BrowserPlan>(&empty_output).is_err());
    }

    #[test]
    fn rejects_duplicate_component_identity_at_the_wire_boundary() {
        let plan = r#"{
            "timelineVersion":1,
            "frameRate":{"numerator":30,"denominator":1},
            "evaluation":{"start":0,"end":1},
            "output":{"start":0,"end":1},
            "videos":[],
            "overlays":[
                {"componentId":7,"kind":"title","text":"A","interval":{"start":0,"end":1}},
                {"componentId":7,"kind":"title","text":"B","interval":{"start":0,"end":1}}
            ]
        }"#;

        assert!(serde_json::from_str::<BrowserPlan>(plan).is_err());
    }

    #[test]
    fn enumerates_video_and_overlay_placement_boundaries() {
        let asset_id = FrozenAssetId::from_sha256([1; 32]);
        let timeline = timeline_with_content_in(
            vec![
                video(asset_id, interval(0, 2)),
                overlay(ElementKind::Title, interval(2, 4), "Opening"),
            ],
            interval(0, 4),
        );
        let source_rates = BTreeMap::from([(
            asset_id,
            FrameRate::new(30, 1).expect("the fixture frame rate is valid"),
        )]);
        let plan = BrowserPlan::from_timeline(&timeline, &source_rates)
            .expect("the fixture forms a valid browser plan");

        let boundaries = plan.placement_boundaries().collect::<BTreeSet<_>>();

        assert_eq!(
            boundaries,
            BTreeSet::from([wire_frame(0), wire_frame(2), wire_frame(4)]),
        );
    }

    #[test]
    fn rejects_a_plan_outside_the_video_budget() {
        let asset_id = FrozenAssetId::from_sha256([1; 32]);
        let timeline = timeline_with_videos(asset_id, MAX_BROWSER_VIDEOS + 1);
        let source_rates = BTreeMap::from([(
            asset_id,
            FrameRate::new(30, 1).expect("the fixture frame rate is valid"),
        )]);

        assert_eq!(
            BrowserPlan::from_timeline(&timeline, &source_rates),
            Err(InvalidBrowserPlan::TooManyVideos),
        );
    }

    #[test]
    fn rejects_a_plan_outside_the_overlay_budget() {
        let timeline = timeline_with_overlays(MAX_BROWSER_OVERLAYS + 1, "Opening");

        assert_eq!(
            BrowserPlan::from_timeline(&timeline, &BTreeMap::new()),
            Err(InvalidBrowserPlan::TooManyOverlays),
        );
    }

    #[test]
    fn rejects_overlay_text_outside_the_character_budget() {
        let text = "片".repeat(MAX_BROWSER_OVERLAY_TEXT_CHARACTERS + 1);
        let timeline = timeline_with_overlays(1, &text);

        assert_eq!(
            BrowserPlan::from_timeline(&timeline, &BTreeMap::new()),
            Err(InvalidBrowserPlan::OverlayTextTooLong(ElementKind::Title)),
        );
    }

    #[test]
    fn clips_caption_overlays_to_each_unit_evaluation() {
        let mut timeline = timeline_with_content_in(Vec::new(), interval(0, 4));
        timeline.replace_captions(vec![caption(interval(1, 3), "Caption")]);

        let plan = BrowserPlan::from_timeline_for_unit(
            &timeline,
            &BTreeMap::new(),
            interval(0, 2),
            interval(0, 2),
        )
        .expect("a crossing caption is clipped without widening evaluation");

        assert_eq!(plan.overlays().len(), 1);
        assert_eq!(plan.overlays()[0].kind(), BrowserOverlayKind::Caption);
        assert_eq!(plan.overlays()[0].interval().start().get(), 1);
        assert_eq!(plan.overlays()[0].interval().end().get(), 2);
    }

    #[test]
    fn retains_component_identity_when_a_partition_omits_earlier_overlays() {
        let timeline = timeline_with_content_in(
            vec![
                overlay(ElementKind::Title, interval(0, 2), "Opening"),
                overlay(ElementKind::CallToAction, interval(2, 4), "Buy now"),
            ],
            interval(0, 4),
        );
        let unit = interval(2, 4);

        let plan = BrowserPlan::from_timeline_for_unit(&timeline, &BTreeMap::new(), unit, unit)
            .expect("the second overlay fits its partition");

        assert_eq!(plan.overlays().len(), 1);
        assert_eq!(plan.overlays()[0].component_id().get(), 1);
    }

    #[test]
    fn rejects_combined_overlay_text_outside_the_cdp_budget() {
        let text = "a".repeat(MAX_BROWSER_OVERLAY_TEXT_BYTES / 17 + 1);
        let mut timeline = timeline_with_content_in(Vec::new(), interval(0, 1));
        timeline.replace_captions((0..17).map(|_| caption(interval(0, 1), &text)).collect());

        assert_eq!(
            BrowserPlan::from_timeline(&timeline, &BTreeMap::new()),
            Err(InvalidBrowserPlan::OverlayTextBudget),
        );
    }

    #[test]
    fn omits_placements_outside_the_unit_evaluation() {
        let asset_id = FrozenAssetId::from_sha256([1; 32]);
        let timeline =
            timeline_with_content_in(vec![video(asset_id, interval(0, 1))], interval(0, 4));
        let unit = interval(2, 4);

        let plan = BrowserPlan::from_timeline_for_unit(&timeline, &BTreeMap::new(), unit, unit)
            .expect("placements outside evaluation do not enter the browser plan");

        assert!(plan.videos().is_empty());
        assert_eq!(plan.evaluation().start().get(), 2);
        assert_eq!(plan.evaluation().end().get(), 4);
    }

    #[test]
    fn rejects_a_video_that_crosses_the_unit_evaluation() {
        let asset_id = FrozenAssetId::from_sha256([1; 32]);
        let timeline =
            timeline_with_content_in(vec![video(asset_id, interval(1, 3))], interval(0, 4));
        let source_rates = BTreeMap::from([(
            asset_id,
            FrameRate::new(30, 1).expect("the fixture frame rate is valid"),
        )]);
        let unit = interval(0, 2);

        assert_eq!(
            BrowserPlan::from_timeline_for_unit(&timeline, &source_rates, unit, unit),
            Err(InvalidBrowserPlan::VideoCrossesEvaluation),
        );
    }

    #[test]
    fn rejects_output_outside_the_unit_evaluation() {
        let timeline = timeline_with_overlays(1, "Opening");

        assert_eq!(
            BrowserPlan::from_timeline_for_unit(
                &timeline,
                &BTreeMap::new(),
                interval(0, 1),
                interval(0, 2),
            ),
            Err(InvalidBrowserPlan::OutputOutsideEvaluation),
        );
    }

    #[test]
    fn rejects_empty_output_from_timeline_projection() {
        let empty = interval(0, 0);
        let timeline = timeline_with_content_in(Vec::new(), empty);

        assert_eq!(
            BrowserPlan::from_timeline_for_unit(&timeline, &BTreeMap::new(), empty, empty,),
            Err(InvalidBrowserPlan::EmptyOutput),
        );
    }

    fn timeline_with_videos(asset_id: FrozenAssetId, count: usize) -> TimelineIr {
        let video = video(asset_id, interval(0, 1));
        timeline_with_content(vec![video; count])
    }

    fn timeline_with_overlays(count: usize, text: &str) -> TimelineIr {
        let overlay = overlay(ElementKind::Title, interval(0, 1), text);
        timeline_with_content(vec![overlay; count])
    }

    fn overlay(kind: ElementKind, interval: FrameInterval, text: &str) -> TimelineContent {
        let span = SourceSpan::new(SourceId::new(0), ByteOffset::ZERO, ByteOffset::ZERO)
            .expect("equal source bounds form a valid span");
        let timing = TimelineTiming::new(interval, TimingReason::ShotStart, TimingReason::ShotEnd);
        TimelineContent::Overlay(TimelineOverlay::new(
            TimelineElement::new(kind, None, span),
            timing,
            vec![TimelineText::new(text.to_owned().into_boxed_str(), span)],
        ))
    }

    fn caption(interval: FrameInterval, text: &str) -> TimelineCaption {
        let span = SourceSpan::new(SourceId::new(1), ByteOffset::ZERO, ByteOffset::ZERO)
            .expect("equal source bounds form a valid span");
        TimelineCaption::new(interval, text, span, span)
    }

    fn timeline_with_content(content: Vec<TimelineContent>) -> TimelineIr {
        timeline_with_content_in(content, interval(0, 1))
    }

    fn timeline_with_content_in(
        content: Vec<TimelineContent>,
        interval: FrameInterval,
    ) -> TimelineIr {
        let span = SourceSpan::new(SourceId::new(0), ByteOffset::ZERO, ByteOffset::ZERO)
            .expect("equal source bounds form a valid span");
        let timing = TimelineTiming::new(interval, TimingReason::ShotStart, TimingReason::ShotEnd);
        let shot = TimelineShot::new(
            TimelineElement::new(ElementKind::Shot, None, span),
            timing.clone(),
            content,
        );
        let scene = TimelineScene::new(
            TimelineElement::new(ElementKind::Scene, None, span),
            timing,
            vec![shot],
        );
        let frame_rate = FrameRate::new(30, 1).expect("the fixture frame rate is valid");

        TimelineIr::new(
            Timebase::new(frame_rate),
            TimelineElement::new(ElementKind::Film, None, span),
            interval,
            BTreeMap::new(),
            vec![scene],
            Vec::new(),
            Vec::new(),
        )
    }

    fn video(asset_id: FrozenAssetId, interval: FrameInterval) -> TimelineContent {
        let span = SourceSpan::new(SourceId::new(0), ByteOffset::ZERO, ByteOffset::ZERO)
            .expect("equal source bounds form a valid span");
        let timing = TimelineTiming::new(interval, TimingReason::ShotStart, TimingReason::ShotEnd);

        TimelineContent::Video(TimelineVideo::new(
            TimelineElement::new(ElementKind::Video, None, span),
            timing,
            asset_id,
        ))
    }

    fn interval(start: u64, end: u64) -> FrameInterval {
        FrameInterval::new(FrameIndex::new(start), FrameIndex::new(end))
            .expect("the fixture interval is ordered")
    }

    fn wire_frame(value: u64) -> WireFrame {
        WireFrame::new(value).expect("the fixture frame is browser-safe")
    }
}
