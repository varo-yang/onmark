//! Checked browser projection of Timeline IR.
//!
//! Conversion establishes JavaScript-safe integer and collection bounds before
//! values cross the Rust/TypeScript boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{
    CaptionLanguage, CaptionTrackId, Duration, ElementKind, FrameInterval, FrameRate,
    FrozenAssetId, MediaSource, MediaSourceInterval, MediaTimebase, NodeId, PlayCount,
    PlaybackRate, Rounding, Timebase, VariantFieldKind, VariantFieldName, VariantValue,
    VideoFrameMap, VideoTiming,
};
#[cfg(feature = "schema")]
use crate::model::{MAX_EXACT_VARIANT_INTEGER, MAX_VARIANT_TEXT_BYTES};
use crate::timeline::{TimelineIr, TimelineShotIndex, TimelineVariantField, TimelineVersion};

use super::frame::{
    InvalidWireFrame, WireFrame, WireFrameRate, WireInterval, WireMediaTimebase, WirePlaybackRate,
};
use super::projection::ProjectionBuilder;

pub(super) const MAX_BROWSER_VIDEOS: usize = 10_000;
const MAX_BROWSER_VIDEO_FRAME_BOUNDARIES: usize = 100_000;
pub(super) const MAX_BROWSER_OVERLAYS: usize = 10_000;
pub(super) const MAX_BROWSER_SCENES: usize = 10_000;
pub(super) const MAX_BROWSER_SHOTS: usize = 10_000;
pub(super) const MAX_BROWSER_TRANSITIONS: usize = 10_000;
pub(super) const MAX_BROWSER_VARIANT_FIELDS: usize = 256;
const MAX_BROWSER_OVERLAY_TEXT_CHARACTERS: usize = 65_536;
pub(super) const MAX_BROWSER_TEXT_BYTES: usize = 1 << 20;

/// Timeline facts consumed by the browser clock and presentation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserPlan {
    #[cfg_attr(
        feature = "schema",
        schemars(extend("const" = TimelineVersion::CURRENT.get()))
    )]
    timeline_version: u16,
    frame_rate: WireFrameRate,
    timeline: WireInterval,
    evaluation: WireInterval,
    output: WireInterval,
    #[cfg_attr(
        feature = "schema",
        schemars(length(max = MAX_BROWSER_VARIANT_FIELDS))
    )]
    variant_fields: Vec<BrowserVariantField>,
    film: BrowserNode,
    #[cfg_attr(feature = "schema", schemars(length(max = MAX_BROWSER_SCENES)))]
    scenes: Vec<BrowserScene>,
    #[cfg_attr(feature = "schema", schemars(length(max = MAX_BROWSER_SHOTS)))]
    shots: Vec<BrowserShot>,
    #[cfg_attr(
        feature = "schema",
        schemars(length(max = MAX_BROWSER_TRANSITIONS))
    )]
    transitions: Vec<BrowserTransition>,
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
        source_timings: &BTreeMap<FrozenAssetId, VideoTiming>,
    ) -> Result<Self, InvalidBrowserPlan> {
        let interval = timeline.interval();
        Self::project_unit(timeline, source_timings, interval, interval, None, None)
    }

    /// Projects one evaluated and published unit from solved Timeline IR.
    ///
    /// Browser facts retain their complete Timeline intervals while
    /// `evaluation` selects the frames executed by this unit. Primary video
    /// must still lie wholly inside `evaluation`: clipping it would change its
    /// source-frame mapping.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserPlan`] when unit bounds are inconsistent, a
    /// placement crosses the evaluation boundary, a placement exceeds its
    /// resource budget, a video has no admitted source rate, an overlay is
    /// malformed, or a frame lies outside JavaScript's exact integer domain.
    pub fn from_timeline_for_unit(
        timeline: &TimelineIr,
        source_timings: &BTreeMap<FrozenAssetId, VideoTiming>,
        evaluation: FrameInterval,
        output: FrameInterval,
    ) -> Result<Self, InvalidBrowserPlan> {
        Self::project_unit(timeline, source_timings, evaluation, output, None, None)
    }

    /// Projects one render-graph region with its exact shot dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserPlan`] under the same conditions as
    /// [`Self::from_timeline_for_unit`], or when the selected shot set is
    /// inconsistent with the unit evaluation interval.
    pub fn from_timeline_for_region(
        timeline: &TimelineIr,
        source_timings: &BTreeMap<FrozenAssetId, VideoTiming>,
        evaluation: FrameInterval,
        output: FrameInterval,
        shots: &BTreeSet<TimelineShotIndex>,
        variant_fields: &BTreeSet<VariantFieldName>,
    ) -> Result<Self, InvalidBrowserPlan> {
        Self::project_unit(
            timeline,
            source_timings,
            evaluation,
            output,
            Some(shots),
            Some(variant_fields),
        )
    }

    fn project_unit(
        timeline: &TimelineIr,
        source_timings: &BTreeMap<FrozenAssetId, VideoTiming>,
        evaluation: FrameInterval,
        output: FrameInterval,
        shots: Option<&BTreeSet<TimelineShotIndex>>,
        selected_variant_fields: Option<&BTreeSet<VariantFieldName>>,
    ) -> Result<Self, InvalidBrowserPlan> {
        if !timeline.interval().contains_interval(evaluation) {
            return Err(InvalidBrowserPlan::EvaluationOutsideTimeline);
        }
        if !evaluation.contains_interval(output) {
            return Err(InvalidBrowserPlan::OutputOutsideEvaluation);
        }

        let evaluation_wire = WireInterval::try_from(evaluation)?;
        let output_wire = WireInterval::try_from(output)?;
        let projection =
            ProjectionBuilder::new(evaluation, source_timings, shots).project(timeline)?;
        Self::checked(BrowserPlanWire {
            timeline_version: timeline.version().get(),
            frame_rate: timeline.timebase().frame_rate().into(),
            timeline: WireInterval::try_from(timeline.interval())?,
            evaluation: evaluation_wire,
            output: output_wire,
            variant_fields: browser_variant_fields(timeline, selected_variant_fields)?,
            film: projection.film,
            scenes: projection.scenes,
            shots: projection.shots,
            transitions: projection.transitions,
            videos: projection.videos,
            overlays: projection.overlays,
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

    /// Returns the complete solved film interval.
    #[must_use]
    pub const fn timeline(&self) -> WireInterval {
        self.timeline
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

    /// Returns canonical typed values required by this render region.
    #[must_use]
    pub fn variant_fields(&self) -> &[BrowserVariantField] {
        &self.variant_fields
    }

    /// Narrows this already-projected unit to a nonempty published interval.
    ///
    /// Evaluation and every projected placement remain unchanged. This is the
    /// exact-output operation used by authoring feedback: it cannot widen a
    /// unit or choose different render dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBrowserPlan`] when the interval is empty, falls outside
    /// the existing output, or cannot cross the JavaScript wire boundary.
    pub fn into_output(mut self, output: FrameInterval) -> Result<Self, InvalidBrowserPlan> {
        let output = WireInterval::try_from(output)?;
        if output.is_empty() {
            return Err(InvalidBrowserPlan::EmptyOutput);
        }
        if !self.output.contains_interval(output) {
            return Err(InvalidBrowserPlan::OutputOutsideOriginalOutput);
        }

        self.output = output;
        Ok(self)
    }

    /// Returns the semantic film root retained from Timeline IR.
    #[must_use]
    pub const fn film(&self) -> &BrowserNode {
        &self.film
    }

    /// Returns scene containers in screenplay order.
    #[must_use]
    pub fn scenes(&self) -> &[BrowserScene] {
        &self.scenes
    }

    /// Returns shot containers in screenplay order.
    #[must_use]
    pub fn shots(&self) -> &[BrowserShot] {
        &self.shots
    }

    /// Returns transition relationships in screenplay order.
    #[must_use]
    pub fn transitions(&self) -> &[BrowserTransition] {
        &self.transitions
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

    /// Projects the same solved unit without browser-owned primary media.
    ///
    /// Render execution may use this only after an independent visual
    /// capability proves that Chromium owns a transparent foreground. The
    /// full plan remains the artifact-identity source.
    #[must_use]
    pub fn foreground_only(&self) -> Self {
        let mut foreground = self.clone();
        foreground.videos.clear();
        foreground.compact_node_ids();
        foreground
    }

    /// Returns the start and end frame of every browser placement.
    ///
    /// Placement visibility is constant between these boundaries. Native
    /// execution may index them once without reconstructing timeline facts or
    /// scanning every placement for every output frame.
    pub fn placement_boundaries(&self) -> impl Iterator<Item = WireFrame> + '_ {
        self.scenes
            .iter()
            .flat_map(|scene| interval_boundaries(scene.interval()))
            .chain(
                self.shots
                    .iter()
                    .flat_map(|shot| interval_boundaries(shot.interval())),
            )
            .chain(
                self.transitions
                    .iter()
                    .flat_map(|transition| interval_boundaries(transition.interval())),
            )
            .chain(
                self.videos
                    .iter()
                    .flat_map(|video| interval_boundaries(video.interval())),
            )
            .chain(
                self.overlays
                    .iter()
                    .flat_map(|overlay| interval_boundaries(overlay.interval())),
            )
    }

    fn checked(wire: BrowserPlanWire) -> Result<Self, InvalidBrowserPlan> {
        if wire.timeline_version != TimelineVersion::CURRENT.get() {
            return Err(InvalidBrowserPlan::UnsupportedTimelineVersion);
        }
        if wire.videos.len() > MAX_BROWSER_VIDEOS {
            return Err(InvalidBrowserPlan::TooManyVideos);
        }
        if source_frame_boundary_count(&wire.videos)? > MAX_BROWSER_VIDEO_FRAME_BOUNDARIES {
            return Err(InvalidBrowserPlan::SourceTimingBudget);
        }
        if wire.overlays.len() > MAX_BROWSER_OVERLAYS {
            return Err(InvalidBrowserPlan::TooManyOverlays);
        }
        if wire.scenes.len() > MAX_BROWSER_SCENES {
            return Err(InvalidBrowserPlan::TooManyScenes);
        }
        if wire.shots.len() > MAX_BROWSER_SHOTS {
            return Err(InvalidBrowserPlan::TooManyShots);
        }
        if wire.transitions.len() > MAX_BROWSER_TRANSITIONS {
            return Err(InvalidBrowserPlan::TooManyTransitions);
        }
        let variant_text_bytes = validate_variant_fields(&wire.variant_fields)?;
        if !wire.timeline.contains_interval(wire.evaluation) {
            return Err(InvalidBrowserPlan::EvaluationOutsideTimeline);
        }
        validate_structure(&wire)?;
        let text_bytes = overlay_text_bytes(&wire.overlays)
            .checked_add(variant_text_bytes)
            .ok_or(InvalidBrowserPlan::BrowserTextBudget)?;
        if text_bytes > MAX_BROWSER_TEXT_BYTES {
            return Err(InvalidBrowserPlan::BrowserTextBudget);
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
            .videos
            .iter()
            .any(|video| !wire.evaluation.contains_interval(video.interval()))
        {
            return Err(InvalidBrowserPlan::VideoCrossesEvaluation);
        }
        validate_video_durations(&wire.videos, wire.frame_rate)?;
        validate_source_timing_durations(&wire.videos)?;
        Ok(Self {
            timeline_version: wire.timeline_version,
            frame_rate: wire.frame_rate,
            timeline: wire.timeline,
            evaluation: wire.evaluation,
            output: wire.output,
            variant_fields: wire.variant_fields,
            film: wire.film,
            scenes: wire.scenes,
            shots: wire.shots,
            transitions: wire.transitions,
            videos: wire.videos,
            overlays: wire.overlays,
        })
    }

    fn compact_node_ids(&mut self) {
        let retained = std::iter::once(self.film.node_id)
            .chain(self.scenes.iter().map(|scene| scene.node.node_id))
            .chain(self.shots.iter().map(|shot| shot.node.node_id))
            .chain(
                self.transitions
                    .iter()
                    .map(|transition| transition.node.node_id),
            )
            .chain(self.overlays.iter().map(|overlay| overlay.node.node_id))
            .collect::<BTreeSet<_>>();
        let replacements = retained
            .into_iter()
            .enumerate()
            .map(|(next, original)| {
                let next = u32::try_from(next)
                    .expect("browser collection limits fit the node-identity domain");
                (original, BrowserNodeId::new(next))
            })
            .collect::<BTreeMap<_, _>>();

        self.film.node_id = replacements[&self.film.node_id];
        for scene in &mut self.scenes {
            scene.node.node_id = replacements[&scene.node.node_id];
        }
        for shot in &mut self.shots {
            shot.node.node_id = replacements[&shot.node.node_id];
            shot.scene_id = replacements[&shot.scene_id];
        }
        for transition in &mut self.transitions {
            transition.node.node_id = replacements[&transition.node.node_id];
            transition.outgoing_shot_id = replacements[&transition.outgoing_shot_id];
            transition.incoming_shot_id = replacements[&transition.incoming_shot_id];
        }
        for overlay in &mut self.overlays {
            overlay.node.node_id = replacements[&overlay.node.node_id];
            overlay.shot_id = overlay.shot_id.map(|id| replacements[&id]);
        }
    }
}

fn validate_structure(wire: &BrowserPlanWire) -> Result<(), InvalidBrowserPlan> {
    validate_node_order(wire.scenes.iter().map(|scene| scene.node().id()))?;
    validate_node_order(wire.shots.iter().map(|shot| shot.node().id()))?;
    validate_node_order(
        wire.transitions
            .iter()
            .map(|transition| transition.node().id()),
    )?;
    validate_node_order(wire.videos.iter().map(|video| video.node().id()))?;
    validate_node_order(wire.overlays.iter().map(|overlay| overlay.node().id()))?;

    let mut claims = NodeClaims::new();
    claims.claim(&wire.film)?;

    let mut scene_intervals = BTreeMap::new();
    for scene in &wire.scenes {
        claims.claim(scene.node())?;
        validate_structural_interval(scene.interval(), wire.timeline, wire.evaluation)?;
        scene_intervals.insert(scene.node().id(), scene.interval());
    }

    let mut shot_intervals = BTreeMap::new();
    for shot in &wire.shots {
        claims.claim(shot.node())?;
        validate_structural_interval(shot.interval(), wire.timeline, wire.evaluation)?;
        let parent = scene_intervals
            .get(&shot.scene_id())
            .ok_or(InvalidBrowserPlan::UnknownParentNode)?;
        validate_child_interval(shot.interval(), *parent)?;
        shot_intervals.insert(shot.node().id(), shot.interval());
    }

    validate_transitions(wire, &shot_intervals, &mut claims)?;

    for video in &wire.videos {
        claims.claim(video.node())?;
        let parent = shot_intervals
            .get(&video.shot_id())
            .ok_or(InvalidBrowserPlan::UnknownParentNode)?;
        validate_child_interval(video.interval(), *parent)?;
    }

    for overlay in &wire.overlays {
        claims.claim(overlay.node())?;
        validate_overlay_interval(overlay.interval(), wire.timeline, wire.evaluation)?;
        validate_caption_track(overlay.kind(), overlay.shot_id(), overlay.caption_track())
            .map_err(|_| InvalidBrowserPlan::InvalidCaptionTrack)?;
        match (overlay.kind(), overlay.shot_id()) {
            (BrowserOverlayKind::Caption, None) => {}
            (BrowserOverlayKind::Title | BrowserOverlayKind::CallToAction, Some(shot_id)) => {
                let parent = shot_intervals
                    .get(&shot_id)
                    .ok_or(InvalidBrowserPlan::UnknownParentNode)?;
                validate_child_interval(overlay.interval(), *parent)?;
            }
            _ => return Err(InvalidBrowserPlan::UnknownParentNode),
        }
    }
    claims.validate_dense_identity()?;
    Ok(())
}

fn validate_transitions<'a>(
    wire: &'a BrowserPlanWire,
    shot_intervals: &BTreeMap<BrowserNodeId, WireInterval>,
    claims: &mut NodeClaims<'a>,
) -> Result<(), InvalidBrowserPlan> {
    let shot_order = wire
        .shots
        .iter()
        .enumerate()
        .map(|(index, shot)| (shot.node().id(), index))
        .collect::<BTreeMap<_, _>>();

    for transition in &wire.transitions {
        claims.claim(transition.node())?;
        validate_structural_interval(transition.interval(), wire.timeline, wire.evaluation)?;
        let outgoing = shot_order
            .get(&transition.outgoing_shot_id())
            .ok_or(InvalidBrowserPlan::UnknownParentNode)?;
        let incoming = shot_order
            .get(&transition.incoming_shot_id())
            .ok_or(InvalidBrowserPlan::UnknownParentNode)?;
        if outgoing.checked_add(1) != Some(*incoming) {
            return Err(InvalidBrowserPlan::InvalidTransitionRelation);
        }
        if wire.shots[*outgoing].scene_id() != wire.shots[*incoming].scene_id() {
            return Err(InvalidBrowserPlan::InvalidTransitionRelation);
        }
        let outgoing_interval = shot_intervals[&transition.outgoing_shot_id()];
        let incoming_interval = shot_intervals[&transition.incoming_shot_id()];
        if !outgoing_interval.contains_interval(transition.interval())
            || !incoming_interval.contains_interval(transition.interval())
            || transition.interval().start() != incoming_interval.start()
            || transition.interval().end() != outgoing_interval.end()
        {
            return Err(InvalidBrowserPlan::InvalidTransitionRelation);
        }
    }

    Ok(())
}

struct NodeClaims<'a> {
    node_ids: BTreeSet<BrowserNodeId>,
    authored_ids: BTreeSet<&'a str>,
}

impl<'a> NodeClaims<'a> {
    const fn new() -> Self {
        Self {
            node_ids: BTreeSet::new(),
            authored_ids: BTreeSet::new(),
        }
    }

    fn claim(&mut self, node: &'a BrowserNode) -> Result<(), InvalidBrowserPlan> {
        if !self.node_ids.insert(node.id()) {
            return Err(InvalidBrowserPlan::DuplicateNodeId);
        }
        let Some(authored_id) = node.authored_id() else {
            return Ok(());
        };
        if NodeId::parse(authored_id).is_err() {
            return Err(InvalidBrowserPlan::InvalidAuthoredId);
        }
        if !self.authored_ids.insert(authored_id) {
            return Err(InvalidBrowserPlan::DuplicateAuthoredId);
        }
        Ok(())
    }

    fn validate_dense_identity(&self) -> Result<(), InvalidBrowserPlan> {
        for (expected, actual) in self.node_ids.iter().enumerate() {
            if usize::try_from(actual.get()) != Ok(expected) {
                return Err(InvalidBrowserPlan::NonDenseNodeIdentity);
            }
        }
        Ok(())
    }
}

fn validate_node_order(
    nodes: impl IntoIterator<Item = BrowserNodeId>,
) -> Result<(), InvalidBrowserPlan> {
    let mut previous = None;
    for node in nodes {
        if previous.is_some_and(|previous| previous >= node) {
            return Err(InvalidBrowserPlan::NonCanonicalNodeOrder);
        }
        previous = Some(node);
    }
    Ok(())
}

fn validate_structural_interval(
    interval: WireInterval,
    timeline: WireInterval,
    evaluation: WireInterval,
) -> Result<(), InvalidBrowserPlan> {
    if interval.is_empty() {
        return Err(InvalidBrowserPlan::EmptyStructure);
    }
    if !timeline.contains_interval(interval) {
        return Err(InvalidBrowserPlan::StructureOutsideTimeline);
    }
    if !intervals_intersect(interval, evaluation) {
        return Err(InvalidBrowserPlan::StructureOutsideEvaluation);
    }
    Ok(())
}

fn validate_overlay_interval(
    interval: WireInterval,
    timeline: WireInterval,
    evaluation: WireInterval,
) -> Result<(), InvalidBrowserPlan> {
    if interval.is_empty() {
        return Err(InvalidBrowserPlan::EmptyOverlay);
    }
    if !timeline.contains_interval(interval) {
        return Err(InvalidBrowserPlan::OverlayOutsideTimeline);
    }
    if !intervals_intersect(interval, evaluation) {
        return Err(InvalidBrowserPlan::OverlayOutsideEvaluation);
    }
    Ok(())
}

fn intervals_intersect(left: WireInterval, right: WireInterval) -> bool {
    left.start() < right.end() && right.start() < left.end()
}

fn validate_child_interval(
    interval: WireInterval,
    parent: WireInterval,
) -> Result<(), InvalidBrowserPlan> {
    if !parent.contains_interval(interval) {
        return Err(InvalidBrowserPlan::ChildCrossesParent);
    }
    Ok(())
}

fn validate_video_durations(
    videos: &[BrowserVideo],
    output_rate: WireFrameRate,
) -> Result<(), InvalidBrowserPlan> {
    let output_rate = FrameRate::new(output_rate.numerator(), output_rate.denominator())
        .expect("wire frame rates are canonical");
    let timebase = Timebase::new(output_rate);

    for video in videos {
        let source = video.source().media_source();
        let expected = timebase
            .frames_for_media(source, Rounding::Ceil)
            .map_err(|_| InvalidBrowserPlan::VideoSourceDurationMismatch)?;
        let interval = video.interval();
        let actual = interval.end().get() - interval.start().get();
        if actual != expected.get() {
            return Err(InvalidBrowserPlan::VideoSourceDurationMismatch);
        }
    }
    Ok(())
}

fn source_frame_boundary_count(videos: &[BrowserVideo]) -> Result<usize, InvalidBrowserPlan> {
    videos.iter().try_fold(0_usize, |count, video| {
        count
            .checked_add(video.source_timing().boundary_count())
            .ok_or(InvalidBrowserPlan::SourceTimingBudget)
    })
}

fn validate_source_timing_durations(videos: &[BrowserVideo]) -> Result<(), InvalidBrowserPlan> {
    for video in videos {
        let Some(duration) = video.source_timing().variable_duration() else {
            continue;
        };
        if duration != video.source().media_source().natural_duration() {
            return Err(InvalidBrowserPlan::SourceTimingDurationMismatch);
        }
    }
    Ok(())
}

fn interval_boundaries(interval: WireInterval) -> [WireFrame; 2] {
    [interval.start(), interval.end()]
}

fn browser_variant_fields(
    timeline: &TimelineIr,
    selected: Option<&BTreeSet<VariantFieldName>>,
) -> Result<Vec<BrowserVariantField>, InvalidBrowserPlan> {
    let fields = timeline
        .variants()
        .iter()
        .filter(|field| !field.scopes().is_empty())
        .map(|field| (field.name(), field))
        .collect::<BTreeMap<_, _>>();
    let selected = selected.map_or_else(
        || fields.keys().copied().collect::<Vec<_>>(),
        |selected| selected.iter().collect(),
    );
    let mut projected = Vec::with_capacity(selected.len());

    for name in selected {
        let Some(field) = fields.get(name) else {
            return Err(InvalidBrowserPlan::InvalidVariantSelection);
        };
        projected.push(BrowserVariantField::from_timeline(field));
    }
    Ok(projected)
}

fn validate_variant_fields(fields: &[BrowserVariantField]) -> Result<usize, InvalidBrowserPlan> {
    if fields.len() > MAX_BROWSER_VARIANT_FIELDS {
        return Err(InvalidBrowserPlan::TooManyVariantFields);
    }
    let mut previous = None;
    let mut text_bytes = 0_usize;

    for field in fields {
        field.validate()?;
        if previous.is_some_and(|previous| previous >= field.name()) {
            return Err(InvalidBrowserPlan::NonCanonicalVariantFields);
        }
        previous = Some(field.name());
        text_bytes = text_bytes
            .checked_add(field.text_bytes())
            .ok_or(InvalidBrowserPlan::BrowserTextBudget)?;
    }
    Ok(text_bytes)
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
    timeline: WireInterval,
    evaluation: WireInterval,
    output: WireInterval,
    variant_fields: Vec<BrowserVariantField>,
    film: BrowserNode,
    scenes: Vec<BrowserScene>,
    shots: Vec<BrowserShot>,
    transitions: Vec<BrowserTransition>,
    videos: Vec<BrowserVideo>,
    overlays: Vec<BrowserOverlay>,
}

/// One canonical typed presentation input carried by this render region.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserVariantField {
    name: Box<str>,
    value: BrowserVariantValue,
}

impl BrowserVariantField {
    fn from_timeline(field: &TimelineVariantField) -> Self {
        Self {
            name: field.name().as_str().into(),
            value: BrowserVariantValue::from_model(field.value()),
        }
    }

    /// Returns the canonical field name.
    #[must_use]
    pub const fn name(&self) -> &str {
        &self.name
    }

    /// Returns the canonical typed value.
    #[must_use]
    pub const fn value(&self) -> &BrowserVariantValue {
        &self.value
    }

    fn validate(&self) -> Result<(), InvalidBrowserPlan> {
        VariantFieldName::parse(&self.name).map_err(|_| InvalidBrowserPlan::InvalidVariantField)?;
        self.value.validate()
    }

    const fn text_bytes(&self) -> usize {
        self.value.text_bytes()
    }
}

/// Closed wire values accepted by the browser's literal binding layer.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "kind")]
pub enum BrowserVariantValue {
    /// Decoded Unicode text.
    Text {
        /// Exact UTF-8 value.
        #[cfg_attr(
            feature = "schema",
            schemars(length(max = MAX_VARIANT_TEXT_BYTES))
        )]
        value: Box<str>,
    },
    /// Exact JavaScript-safe signed integer.
    Integer {
        /// Exact integer value.
        #[cfg_attr(
            feature = "schema",
            schemars(range(
                min = -MAX_EXACT_VARIANT_INTEGER,
                max = MAX_EXACT_VARIANT_INTEGER
            ))
        )]
        value: i64,
    },
    /// Binary presentation choice.
    Boolean {
        /// Exact boolean value.
        value: bool,
    },
    /// Lowercase six- or eight-digit sRGB hexadecimal.
    Color {
        /// Canonical color value.
        value: Box<str>,
    },
}

impl BrowserVariantValue {
    fn from_model(value: &VariantValue) -> Self {
        match value {
            VariantValue::Text(value) => Self::Text {
                value: value.clone(),
            },
            VariantValue::Integer(value) => Self::Integer { value: *value },
            VariantValue::Boolean(value) => Self::Boolean { value: *value },
            VariantValue::Color(value) => Self::Color {
                value: value.clone(),
            },
        }
    }

    /// Returns the closed value kind.
    #[must_use]
    pub const fn kind(&self) -> VariantFieldKind {
        match self {
            Self::Text { .. } => VariantFieldKind::Text,
            Self::Integer { .. } => VariantFieldKind::Integer,
            Self::Boolean { .. } => VariantFieldKind::Boolean,
            Self::Color { .. } => VariantFieldKind::Color,
        }
    }

    /// Returns text when this is a text value.
    #[must_use]
    pub const fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { value } => Some(value),
            Self::Integer { .. } | Self::Boolean { .. } | Self::Color { .. } => None,
        }
    }

    /// Returns an integer when this is an integer value.
    #[must_use]
    pub const fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer { value } => Some(*value),
            Self::Text { .. } | Self::Boolean { .. } | Self::Color { .. } => None,
        }
    }

    /// Returns a boolean when this is a boolean value.
    #[must_use]
    pub const fn as_boolean(&self) -> Option<bool> {
        match self {
            Self::Boolean { value } => Some(*value),
            Self::Text { .. } | Self::Integer { .. } | Self::Color { .. } => None,
        }
    }

    /// Returns a color when this is a color value.
    #[must_use]
    pub const fn as_color(&self) -> Option<&str> {
        match self {
            Self::Color { value } => Some(value),
            Self::Text { .. } | Self::Integer { .. } | Self::Boolean { .. } => None,
        }
    }

    fn validate(&self) -> Result<(), InvalidBrowserPlan> {
        let valid = match self {
            Self::Text { value } => VariantValue::text(value).is_ok(),
            Self::Integer { value } => VariantValue::from_integer(*value).is_ok(),
            Self::Boolean { .. } => true,
            Self::Color { value } => VariantValue::parse(VariantFieldKind::Color, value).is_ok(),
        };
        valid
            .then_some(())
            .ok_or(InvalidBrowserPlan::InvalidVariantField)
    }

    const fn text_bytes(&self) -> usize {
        match self {
            Self::Text { value } => value.len(),
            Self::Integer { .. } | Self::Boolean { .. } | Self::Color { .. } => 0,
        }
    }
}

/// Browser identity for one Timeline element or imported caption.
///
/// IDs form dense renderable-semantic preorder within one Browser Plan.
///
/// Authored IDs, rather than this unit-local key, retain cross-projection
/// semantic identity.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BrowserNodeId(#[cfg_attr(feature = "schema", schemars(range(max = u32::MAX)))] u32);

impl BrowserNodeId {
    /// Creates one unit-local wire identity assigned by a projection boundary.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the unit-local wire representation.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Browser-facing identity retained from one Timeline element.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserNode {
    node_id: BrowserNodeId,
    authored_id: Option<Box<str>>,
}

impl BrowserNode {
    pub(super) fn new(node_id: BrowserNodeId, authored_id: Option<&NodeId>) -> Self {
        Self {
            node_id,
            authored_id: authored_id.map(|id| Box::from(id.as_str())),
        }
    }

    /// Returns the compiler-assigned unit-local binding identity.
    #[must_use]
    pub const fn id(&self) -> BrowserNodeId {
        self.node_id
    }

    /// Returns the optional film-wide authored identity.
    #[must_use]
    pub fn authored_id(&self) -> Option<&str> {
        self.authored_id.as_deref()
    }
}

/// One scene container intersecting this unit, with its complete Timeline interval.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserScene {
    node: BrowserNode,
    interval: WireInterval,
}

impl BrowserScene {
    pub(super) const fn new(node: BrowserNode, interval: WireInterval) -> Self {
        Self { node, interval }
    }

    /// Returns the scene identity retained from Timeline IR.
    #[must_use]
    pub const fn node(&self) -> &BrowserNode {
        &self.node
    }

    /// Returns the complete solved scene interval.
    #[must_use]
    pub const fn interval(&self) -> WireInterval {
        self.interval
    }
}

/// One shot container intersecting this unit, with its complete Timeline interval.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserShot {
    node: BrowserNode,
    scene_id: BrowserNodeId,
    interval: WireInterval,
}

impl BrowserShot {
    pub(super) const fn new(
        node: BrowserNode,
        scene_id: BrowserNodeId,
        interval: WireInterval,
    ) -> Self {
        Self {
            node,
            scene_id,
            interval,
        }
    }

    /// Returns the shot identity retained from Timeline IR.
    #[must_use]
    pub const fn node(&self) -> &BrowserNode {
        &self.node
    }

    /// Returns the owning scene identity.
    #[must_use]
    pub const fn scene_id(&self) -> BrowserNodeId {
        self.scene_id
    }

    /// Returns the complete solved shot interval.
    #[must_use]
    pub const fn interval(&self) -> WireInterval {
        self.interval
    }
}

/// One solved transition between two adjacent browser shots.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserTransition {
    node: BrowserNode,
    outgoing_shot_id: BrowserNodeId,
    incoming_shot_id: BrowserNodeId,
    interval: WireInterval,
}

impl BrowserTransition {
    pub(super) const fn new(
        node: BrowserNode,
        outgoing_shot_id: BrowserNodeId,
        incoming_shot_id: BrowserNodeId,
        interval: WireInterval,
    ) -> Self {
        Self {
            node,
            outgoing_shot_id,
            incoming_shot_id,
            interval,
        }
    }

    /// Returns the transition identity retained from Timeline IR.
    #[must_use]
    pub const fn node(&self) -> &BrowserNode {
        &self.node
    }

    /// Returns the outgoing shot identity.
    #[must_use]
    pub const fn outgoing_shot_id(&self) -> BrowserNodeId {
        self.outgoing_shot_id
    }

    /// Returns the incoming shot identity.
    #[must_use]
    pub const fn incoming_shot_id(&self) -> BrowserNodeId {
        self.incoming_shot_id
    }

    /// Returns the exact overlap interval.
    #[must_use]
    pub const fn interval(&self) -> WireInterval {
        self.interval
    }
}

/// One primary video placement consumed by the browser presentation adapter.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserVideo {
    node: BrowserNode,
    shot_id: BrowserNodeId,
    #[cfg_attr(
        feature = "schema",
        schemars(regex(pattern = r"^sha256:[0-9a-f]{64}$"))
    )]
    asset_id: Box<str>,
    #[serde(skip)]
    #[cfg_attr(feature = "schema", schemars(skip))]
    asset_identity: FrozenAssetId,
    interval: WireInterval,
    source_timing: BrowserVideoTiming,
    source: BrowserVideoSource,
}

impl BrowserVideo {
    pub(super) fn new(
        node: BrowserNode,
        shot_id: BrowserNodeId,
        asset_identity: FrozenAssetId,
        interval: WireInterval,
        source_timing: BrowserVideoTiming,
        source: MediaSource,
    ) -> Self {
        Self {
            node,
            shot_id,
            asset_id: asset_identity.to_string().into_boxed_str(),
            asset_identity,
            interval,
            source_timing,
            source: BrowserVideoSource::new(source),
        }
    }

    /// Returns the video identity retained from Timeline IR.
    #[must_use]
    pub const fn node(&self) -> &BrowserNode {
        &self.node
    }

    /// Returns the owning shot identity.
    #[must_use]
    pub const fn shot_id(&self) -> BrowserNodeId {
        self.shot_id
    }

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

    /// Returns the exact selected source-stream frame timing.
    #[must_use]
    pub const fn source_timing(&self) -> &BrowserVideoTiming {
        &self.source_timing
    }

    /// Returns the exact mapping from output time into source time.
    #[must_use]
    pub const fn source(&self) -> &BrowserVideoSource {
        &self.source
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
            node: wire.node,
            shot_id: wire.shot_id,
            asset_id: wire.asset_id,
            asset_identity,
            interval: wire.interval,
            source_timing: wire.source_timing,
            source: wire.source,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserVideoWire {
    node: BrowserNode,
    shot_id: BrowserNodeId,
    asset_id: Box<str>,
    interval: WireInterval,
    source_timing: BrowserVideoTiming,
    source: BrowserVideoSource,
}

/// Exact source-frame timing projected into the browser runtime.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum BrowserVideoTiming {
    /// Every source frame has one exact rational rate.
    Constant {
        /// Exact source frames per second.
        frame_rate: WireFrameRate,
    },
    /// Every source frame carries one half-open timestamp interval.
    Variable {
        /// Exact seconds represented by one source timestamp tick.
        timebase: WireMediaTimebase,
        /// Canonical decimal ticks from source zero through terminal end.
        #[cfg_attr(
            feature = "schema",
            schemars(length(min = 3, max = MAX_BROWSER_VIDEO_FRAME_BOUNDARIES))
        )]
        boundaries: Vec<Box<str>>,
    },
}

impl BrowserVideoTiming {
    pub(super) fn from_model(timing: &VideoTiming) -> Result<Self, InvalidBrowserPlan> {
        match timing {
            VideoTiming::Constant(frame_rate) => Ok(Self::Constant {
                frame_rate: (*frame_rate).into(),
            }),
            VideoTiming::Variable(frame_map) => {
                if frame_map.boundaries().len() > MAX_BROWSER_VIDEO_FRAME_BOUNDARIES {
                    return Err(InvalidBrowserPlan::SourceTimingBudget);
                }
                Ok(Self::Variable {
                    timebase: frame_map.timebase().into(),
                    boundaries: frame_map
                        .boundaries()
                        .iter()
                        .map(|boundary| boundary.to_string().into_boxed_str())
                        .collect(),
                })
            }
            VideoTiming::Still => Err(InvalidBrowserPlan::UnsupportedSourceTiming),
        }
    }

    /// Returns the exact constant rate, when source intervals are uniform.
    #[must_use]
    pub const fn constant_frame_rate(&self) -> Option<WireFrameRate> {
        match self {
            Self::Constant { frame_rate } => Some(*frame_rate),
            Self::Variable { .. } => None,
        }
    }

    /// Returns the exact source timestamp unit for a variable-rate stream.
    #[must_use]
    pub const fn variable_timebase(&self) -> Option<WireMediaTimebase> {
        match self {
            Self::Constant { .. } => None,
            Self::Variable { timebase, .. } => Some(*timebase),
        }
    }

    /// Returns every canonical source-frame boundary for a variable stream.
    #[must_use]
    pub fn variable_boundaries(&self) -> Option<&[Box<str>]> {
        match self {
            Self::Constant { .. } => None,
            Self::Variable { boundaries, .. } => Some(boundaries),
        }
    }

    fn boundary_count(&self) -> usize {
        match self {
            Self::Constant { .. } => 0,
            Self::Variable { boundaries, .. } => boundaries.len(),
        }
    }

    fn variable_duration(&self) -> Option<Duration> {
        let Self::Variable {
            timebase,
            boundaries,
        } = self
        else {
            return None;
        };
        let terminal = parse_wire_ticks(&boundaries[boundaries.len() - 1])
            .expect("browser video timing validates canonical boundaries");
        let timebase = MediaTimebase::new(timebase.numerator(), timebase.denominator())
            .expect("wire media timebases are canonical");
        Some(
            timebase
                .duration_at(terminal)
                .expect("browser video timing validates its duration domain"),
        )
    }
}

impl<'de> Deserialize<'de> for BrowserVideoTiming {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BrowserVideoTimingWire::deserialize(deserializer)?;
        match wire {
            BrowserVideoTimingWire::Constant { frame_rate } => Ok(Self::Constant { frame_rate }),
            BrowserVideoTimingWire::Variable {
                timebase,
                boundaries,
            } => {
                if boundaries.len() > MAX_BROWSER_VIDEO_FRAME_BOUNDARIES {
                    return Err(D::Error::custom(
                        "source frame map exceeds the browser timing budget",
                    ));
                }
                let parsed = boundaries
                    .iter()
                    .map(|boundary| parse_wire_ticks(boundary))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(D::Error::custom)?;
                let timebase_model =
                    MediaTimebase::new(timebase.numerator(), timebase.denominator())
                        .expect("wire media timebases are canonical");
                VideoFrameMap::new(timebase_model, parsed)
                    .map_err(|source| D::Error::custom(source.to_string()))?;
                Ok(Self::Variable {
                    timebase,
                    boundaries,
                })
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
enum BrowserVideoTimingWire {
    Constant {
        frame_rate: WireFrameRate,
    },
    Variable {
        timebase: WireMediaTimebase,
        boundaries: Vec<Box<str>>,
    },
}

/// Exact source-time mapping for one browser video placement.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserVideoSource {
    #[cfg_attr(
        feature = "schema",
        schemars(length(min = 1, max = 20), regex(pattern = r"^(0|[1-9][0-9]*)$"))
    )]
    start_nanoseconds: Box<str>,
    #[cfg_attr(
        feature = "schema",
        schemars(length(min = 1, max = 20), regex(pattern = r"^(0|[1-9][0-9]*)$"))
    )]
    end_nanoseconds: Box<str>,
    #[cfg_attr(
        feature = "schema",
        schemars(length(min = 1, max = 20), regex(pattern = r"^(0|[1-9][0-9]*)$"))
    )]
    natural_end_nanoseconds: Box<str>,
    playback_rate: WirePlaybackRate,
    #[cfg_attr(feature = "schema", schemars(range(min = 1)))]
    plays: u32,
    #[cfg_attr(
        feature = "schema",
        schemars(length(min = 1, max = 20), regex(pattern = r"^(0|[1-9][0-9]*)$"))
    )]
    hold_last_nanoseconds: Box<str>,
    #[serde(skip)]
    #[cfg_attr(feature = "schema", schemars(skip))]
    source: MediaSource,
}

impl BrowserVideoSource {
    fn new(source: MediaSource) -> Self {
        let interval = source.interval();
        Self {
            start_nanoseconds: interval.start().as_nanos().to_string().into_boxed_str(),
            end_nanoseconds: interval.end().as_nanos().to_string().into_boxed_str(),
            natural_end_nanoseconds: source
                .natural_duration()
                .as_nanos()
                .to_string()
                .into_boxed_str(),
            playback_rate: source.playback_rate().into(),
            plays: source.plays().get(),
            hold_last_nanoseconds: source.hold_last().as_nanos().to_string().into_boxed_str(),
            source,
        }
    }

    /// Returns the inclusive source start as canonical decimal nanoseconds.
    #[must_use]
    pub fn start_nanoseconds(&self) -> &str {
        &self.start_nanoseconds
    }

    /// Returns the exclusive source end as canonical decimal nanoseconds.
    #[must_use]
    pub fn end_nanoseconds(&self) -> &str {
        &self.end_nanoseconds
    }

    /// Returns the frozen artifact's natural end as canonical nanoseconds.
    #[must_use]
    pub fn natural_end_nanoseconds(&self) -> &str {
        &self.natural_end_nanoseconds
    }

    /// Returns the exact source-to-output playback ratio.
    #[must_use]
    pub const fn playback_rate(&self) -> WirePlaybackRate {
        self.playback_rate
    }

    /// Returns the exact number of complete source passes.
    #[must_use]
    pub const fn plays(&self) -> u32 {
        self.plays
    }

    /// Returns the exact final-frame hold as canonical decimal nanoseconds.
    #[must_use]
    pub fn hold_last_nanoseconds(&self) -> &str {
        &self.hold_last_nanoseconds
    }

    /// Returns the validated domain source mapping.
    #[must_use]
    pub const fn media_source(&self) -> MediaSource {
        self.source
    }
}

impl<'de> Deserialize<'de> for BrowserVideoSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BrowserVideoSourceWire::deserialize(deserializer)?;
        let start = parse_wire_nanoseconds(&wire.start_nanoseconds).map_err(D::Error::custom)?;
        let end = parse_wire_nanoseconds(&wire.end_nanoseconds).map_err(D::Error::custom)?;
        let natural_end =
            parse_wire_nanoseconds(&wire.natural_end_nanoseconds).map_err(D::Error::custom)?;
        let hold_last =
            parse_wire_nanoseconds(&wire.hold_last_nanoseconds).map_err(D::Error::custom)?;
        let interval = MediaSourceInterval::new(start, end)
            .map_err(|source| D::Error::custom(source.to_string()))?;
        let playback_rate = PlaybackRate::new(
            wire.playback_rate.numerator(),
            wire.playback_rate.denominator(),
        )
        .map_err(|source| D::Error::custom(source.to_string()))?;
        let plays =
            PlayCount::new(wire.plays).map_err(|source| D::Error::custom(source.to_string()))?;
        let source = MediaSource::new(interval, playback_rate, plays, hold_last, natural_end)
            .map_err(|source| D::Error::custom(source.to_string()))?;

        Ok(Self {
            start_nanoseconds: wire.start_nanoseconds,
            end_nanoseconds: wire.end_nanoseconds,
            natural_end_nanoseconds: wire.natural_end_nanoseconds,
            playback_rate: wire.playback_rate,
            plays: wire.plays,
            hold_last_nanoseconds: wire.hold_last_nanoseconds,
            source,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserVideoSourceWire {
    start_nanoseconds: Box<str>,
    end_nanoseconds: Box<str>,
    natural_end_nanoseconds: Box<str>,
    playback_rate: WirePlaybackRate,
    plays: u32,
    hold_last_nanoseconds: Box<str>,
}

fn parse_wire_nanoseconds(value: &str) -> Result<Duration, &'static str> {
    let nanoseconds = value
        .parse::<u64>()
        .map_err(|_| "source nanoseconds exceed the exact duration domain")?;
    if nanoseconds.to_string() != value {
        return Err("source nanoseconds are not in canonical decimal form");
    }
    Ok(Duration::from_nanos(nanoseconds))
}

fn parse_wire_ticks(value: &str) -> Result<u64, &'static str> {
    let ticks = value
        .parse::<u64>()
        .map_err(|_| "source frame timestamp exceeds its exact integer domain")?;
    if ticks.to_string() != value {
        return Err("source frame timestamp is not in canonical decimal form");
    }
    Ok(ticks)
}

/// Closed overlay roles understood by the browser presentation.
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

/// Stable caption-track metadata projected into browser presentation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserCaptionTrack {
    #[cfg_attr(feature = "schema", schemars(regex(pattern = r"^[^\t\n\f\r ]+$")))]
    id: Box<str>,
    #[cfg_attr(
        feature = "schema",
        schemars(regex(pattern = r"^[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*$"))
    )]
    language: Box<str>,
}

impl BrowserCaptionTrack {
    pub(super) fn new(id: &CaptionTrackId, language: &CaptionLanguage) -> Self {
        Self {
            id: id.as_str().into(),
            language: language.as_str().into(),
        }
    }

    /// Returns the stable authored track identity.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns language metadata for the generated caption element.
    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }
}

/// One solved overlay placement consumed by the browser presentation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserOverlay {
    node: BrowserNode,
    shot_id: Option<BrowserNodeId>,
    kind: BrowserOverlayKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    caption_track: Option<BrowserCaptionTrack>,
    #[cfg_attr(
        feature = "schema",
        schemars(length(max = MAX_BROWSER_OVERLAY_TEXT_CHARACTERS))
    )]
    text: Box<str>,
    interval: WireInterval,
}

impl BrowserOverlay {
    pub(super) const fn new(
        node: BrowserNode,
        shot_id: Option<BrowserNodeId>,
        kind: BrowserOverlayKind,
        caption_track: Option<BrowserCaptionTrack>,
        text: Box<str>,
        interval: WireInterval,
    ) -> Self {
        Self {
            node,
            shot_id,
            kind,
            caption_track,
            text,
            interval,
        }
    }

    /// Returns the overlay identity retained from Timeline IR.
    #[must_use]
    pub const fn node(&self) -> &BrowserNode {
        &self.node
    }

    /// Returns the owning shot, or `None` for a film-level imported caption.
    #[must_use]
    pub const fn shot_id(&self) -> Option<BrowserNodeId> {
        self.shot_id
    }

    /// Returns the presentation role selected by the screenplay element.
    #[must_use]
    pub const fn kind(&self) -> BrowserOverlayKind {
        self.kind
    }

    /// Returns track metadata for a caption overlay.
    #[must_use]
    pub const fn caption_track(&self) -> Option<&BrowserCaptionTrack> {
        self.caption_track.as_ref()
    }

    /// Returns decoded authored text with source runs joined in order.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    pub(super) fn text_bytes(&self) -> usize {
        let track_bytes = self.caption_track.as_ref().map_or(0, |track| {
            track.id.len().saturating_add(track.language.len())
        });
        self.text.len().saturating_add(track_bytes)
    }

    /// Returns the complete solved visibility interval.
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
        validate_caption_track(wire.kind, wire.shot_id, wire.caption_track.as_ref())
            .map_err(D::Error::custom)?;

        Ok(Self {
            node: wire.node,
            shot_id: wire.shot_id,
            kind: wire.kind,
            caption_track: wire.caption_track,
            text: wire.text,
            interval: wire.interval,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BrowserOverlayWire {
    node: BrowserNode,
    shot_id: Option<BrowserNodeId>,
    kind: BrowserOverlayKind,
    caption_track: Option<BrowserCaptionTrack>,
    text: Box<str>,
    interval: WireInterval,
}

fn validate_caption_track(
    kind: BrowserOverlayKind,
    shot_id: Option<BrowserNodeId>,
    track: Option<&BrowserCaptionTrack>,
) -> Result<(), &'static str> {
    match (kind, shot_id, track) {
        (BrowserOverlayKind::Caption, None, Some(track))
            if NodeId::parse(track.id()).is_ok()
                && CaptionLanguage::parse(track.language()).is_ok() =>
        {
            Ok(())
        }
        (BrowserOverlayKind::Title | BrowserOverlayKind::CallToAction, Some(_), None) => Ok(()),
        (BrowserOverlayKind::Caption, _, _) => {
            Err("browser caption must name one valid track and no structural parent")
        }
        (BrowserOverlayKind::Title | BrowserOverlayKind::CallToAction, _, _) => {
            Err("authored browser overlay must name one shot and no caption track")
        }
    }
}

fn overlay_text_bytes(overlays: &[BrowserOverlay]) -> usize {
    overlays
        .iter()
        .map(BrowserOverlay::text_bytes)
        .try_fold(0_usize, usize::checked_add)
        .unwrap_or(usize::MAX)
}

pub(super) fn text_exceeds_limit(text: &str) -> bool {
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
    /// A narrowed published interval lies outside the original output.
    OutputOutsideOriginalOutput,
    /// The published interval contains no frame.
    EmptyOutput,
    /// A render region selects no shot or one outside this Timeline IR.
    InvalidShotSelection,
    /// A video placement contains no frame.
    EmptyVideo,
    /// An overlay placement contains no frame.
    EmptyOverlay,
    /// A projected scene or shot contains no frame.
    EmptyStructure,
    /// A video would need clipping at the unit evaluation boundary.
    VideoCrossesEvaluation,
    /// A video interval disagrees with its exact source-time mapping.
    VideoSourceDurationMismatch,
    /// A variable source map disagrees with the frozen artifact duration.
    SourceTimingDurationMismatch,
    /// A projected overlay lies outside the solved film.
    OverlayOutsideTimeline,
    /// A projected overlay does not intersect this unit.
    OverlayOutsideEvaluation,
    /// A projected scene or shot lies outside the solved film.
    StructureOutsideTimeline,
    /// A projected scene or shot does not intersect this unit.
    StructureOutsideEvaluation,
    /// The plan contains more scene containers than the current contract can carry.
    TooManyScenes,
    /// The plan contains more shot containers than the current contract can carry.
    TooManyShots,
    /// The plan contains more transition relationships than the current contract can carry.
    TooManyTransitions,
    /// The plan contains more video placements than the current contract can carry.
    TooManyVideos,
    /// The plan contains more overlay placements than the current contract can carry.
    TooManyOverlays,
    /// The plan contains more typed fields than the current contract can carry.
    TooManyVariantFields,
    /// The selected region names a field absent from Timeline IR.
    InvalidVariantSelection,
    /// A field name or value is not canonical.
    InvalidVariantField,
    /// Typed fields are not in strictly increasing name order.
    NonCanonicalVariantFields,
    /// Browser node identity overflowed the current wire domain.
    TooManyNodes,
    /// Two projected nodes claim the same unit-local identity.
    DuplicateNodeId,
    /// One projected node carries an invalid authored identity.
    InvalidAuthoredId,
    /// Two projected nodes claim the same authored identity.
    DuplicateAuthoredId,
    /// A browser collection does not retain compiler projection order.
    NonCanonicalNodeOrder,
    /// Browser node identities do not form one dense zero-based domain.
    NonDenseNodeIdentity,
    /// One projected node names an absent or invalid structural parent.
    UnknownParentNode,
    /// One projected node escapes its structural parent interval.
    ChildCrossesParent,
    /// A transition does not connect adjacent projected shots in screenplay order.
    InvalidTransitionRelation,
    /// A Timeline overlay carries a non-overlay element kind.
    InvalidOverlayKind(ElementKind),
    /// One overlay inscription exceeds the current character budget.
    OverlayTextTooLong(ElementKind),
    /// One imported caption exceeds the per-placement text budget.
    CaptionTextTooLong,
    /// Caption-track metadata is missing, malformed, or attached to an authored overlay.
    InvalidCaptionTrack,
    /// Combined overlay and typed-field text exceeds the bounded CDP request budget.
    BrowserTextBudget,
    /// One video lacks the source timing proved during render admission.
    MissingSourceTiming(FrozenAssetId),
    /// The admitted source timing shape cannot be presented as video.
    UnsupportedSourceTiming,
    /// Complete source-frame maps exceed the bounded browser request budget.
    SourceTimingBudget,
    /// A frame lies outside JavaScript's exact integer range.
    InvalidFrame(InvalidWireFrame),
}

impl fmt::Display for InvalidBrowserPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::UnsupportedTimelineVersion => "unsupported browser plan timeline version",
            Self::EvaluationOutsideTimeline => {
                "browser evaluation interval lies outside the solved film"
            }
            Self::OutputOutsideEvaluation => "browser output interval lies outside evaluation",
            Self::OutputOutsideOriginalOutput => {
                "narrowed browser output lies outside the original output"
            }
            Self::EmptyOutput => "browser output interval is empty",
            Self::InvalidShotSelection => "browser region contains an invalid shot selection",
            Self::EmptyVideo => "browser video interval is empty",
            Self::EmptyOverlay => "browser overlay interval is empty",
            Self::EmptyStructure => "browser structural interval is empty",
            Self::VideoCrossesEvaluation => "browser video crosses the evaluation boundary",
            Self::VideoSourceDurationMismatch => {
                "browser video duration disagrees with its source mapping"
            }
            Self::SourceTimingDurationMismatch => {
                "browser source timing disagrees with its natural duration"
            }
            Self::OverlayOutsideTimeline => "browser overlay lies outside the solved film",
            Self::OverlayOutsideEvaluation => "browser overlay does not intersect evaluation",
            Self::StructureOutsideTimeline => "browser structure lies outside the solved film",
            Self::StructureOutsideEvaluation => "browser structure does not intersect evaluation",
            Self::TooManyScenes => "browser plan exceeds the scene-container limit",
            Self::TooManyShots => "browser plan exceeds the shot-container limit",
            Self::TooManyTransitions => "browser plan exceeds the transition limit",
            Self::TooManyVideos => "browser plan exceeds the video-placement limit",
            Self::TooManyOverlays => "browser plan exceeds the overlay-placement limit",
            Self::TooManyVariantFields => "browser plan exceeds the typed-field limit",
            Self::InvalidVariantSelection => "browser region selects an unknown typed field",
            Self::InvalidVariantField => "browser typed field is not canonical",
            Self::NonCanonicalVariantFields => "browser typed fields are not in canonical order",
            Self::TooManyNodes => "browser plan exceeds the node-identity domain",
            Self::DuplicateNodeId => "browser node identity is duplicated",
            Self::InvalidAuthoredId => "browser node carries an invalid authored identity",
            Self::DuplicateAuthoredId => "browser authored identity is duplicated",
            Self::NonCanonicalNodeOrder => "browser nodes are not in canonical order",
            Self::NonDenseNodeIdentity => "browser node identity is not dense",
            Self::UnknownParentNode => "browser node names an unknown structural parent",
            Self::ChildCrossesParent => "browser node crosses its structural parent",
            Self::InvalidTransitionRelation => "browser transition does not connect adjacent shots",
            Self::CaptionTextTooLong => "browser caption text exceeds the character limit",
            Self::InvalidCaptionTrack => "browser caption track metadata is invalid",
            Self::BrowserTextBudget => "browser text exceeds the request byte budget",
            Self::UnsupportedSourceTiming => "source frame timing cannot be presented as video",
            Self::SourceTimingBudget => "browser source timing exceeds the request budget",
            Self::InvalidOverlayKind(kind) => {
                return write!(
                    formatter,
                    "timeline element {kind} is not a browser overlay"
                );
            }
            Self::OverlayTextTooLong(kind) => {
                return write!(formatter, "browser {kind} text exceeds the character limit");
            }
            Self::MissingSourceTiming(id) => {
                return write!(formatter, "source frame timing is missing for video {id}");
            }
            Self::InvalidFrame(source) => return source.fmt(formatter),
        };
        formatter.write_str(message)
    }
}

impl Error for InvalidBrowserPlan {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidFrame(source) => Some(source),
            Self::UnsupportedTimelineVersion
            | Self::EvaluationOutsideTimeline
            | Self::OutputOutsideEvaluation
            | Self::OutputOutsideOriginalOutput
            | Self::EmptyOutput
            | Self::InvalidShotSelection
            | Self::EmptyVideo
            | Self::EmptyOverlay
            | Self::EmptyStructure
            | Self::VideoCrossesEvaluation
            | Self::VideoSourceDurationMismatch
            | Self::SourceTimingDurationMismatch
            | Self::OverlayOutsideTimeline
            | Self::OverlayOutsideEvaluation
            | Self::StructureOutsideTimeline
            | Self::StructureOutsideEvaluation
            | Self::TooManyScenes
            | Self::TooManyShots
            | Self::TooManyTransitions
            | Self::TooManyVideos
            | Self::TooManyOverlays
            | Self::TooManyVariantFields
            | Self::InvalidVariantSelection
            | Self::InvalidVariantField
            | Self::NonCanonicalVariantFields
            | Self::TooManyNodes
            | Self::DuplicateNodeId
            | Self::InvalidAuthoredId
            | Self::DuplicateAuthoredId
            | Self::NonCanonicalNodeOrder
            | Self::NonDenseNodeIdentity
            | Self::UnknownParentNode
            | Self::ChildCrossesParent
            | Self::InvalidTransitionRelation
            | Self::InvalidOverlayKind(_)
            | Self::OverlayTextTooLong(_)
            | Self::CaptionTextTooLong
            | Self::InvalidCaptionTrack
            | Self::BrowserTextBudget
            | Self::MissingSourceTiming(_)
            | Self::UnsupportedSourceTiming
            | Self::SourceTimingBudget => None,
        }
    }
}

impl From<InvalidWireFrame> for InvalidBrowserPlan {
    fn from(source: InvalidWireFrame) -> Self {
        Self::InvalidFrame(source)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use crate::model::{
        ByteOffset, CaptionLanguage, CaptionTrackId, Duration, ElementKind, FrameIndex,
        FrameInterval, FrameRate, FrozenAssetId, MAX_VARIANT_TEXT_BYTES, MediaSource,
        MediaSourceInterval, NodeId, PlayCount, PlaybackRate, SourceId, SourceSpan, Timebase,
        VariantFieldKind, VariantFieldName, VariantValue, VideoTiming,
    };
    use crate::timeline::{
        TimelineCaption, TimelineContent, TimelineElement, TimelineIr, TimelineOverlay,
        TimelineScene, TimelineShot, TimelineShotIndex, TimelineText, TimelineTiming,
        TimelineTransition, TimelineVariantField, TimelineVariantScope, TimelineVideo,
        TimingReason,
    };

    use super::{
        BrowserNodeId, BrowserOverlayKind, BrowserPlan, InvalidBrowserPlan,
        MAX_BROWSER_OVERLAY_TEXT_CHARACTERS, MAX_BROWSER_OVERLAYS, MAX_BROWSER_TEXT_BYTES,
        MAX_BROWSER_VIDEOS, WireFrame,
    };

    #[test]
    fn parses_only_validated_browser_plan_facts() {
        let plan = r#"{
            "timelineVersion":7,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":1},
            "evaluation":{"start":0,"end":1},
            "output":{"start":0,"end":1},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[],
            "shots":[],
            "transitions":[],
            "variantFields":[],
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
    fn rejects_a_source_selection_outside_its_natural_media() {
        let mut wire =
            serde_json::to_value(one_frame_video_plan()).expect("the browser plan serializes");
        wire["videos"][0]["source"]["endNanoseconds"] =
            serde_json::Value::String(String::from("2000000000"));

        let error = serde_json::from_value::<BrowserPlan>(wire)
            .expect_err("source selection cannot exceed its natural media");

        assert!(error.to_string().contains("artifact ends"));
    }

    #[test]
    fn rejects_a_video_duration_that_disagrees_with_its_source_mapping() {
        let mut wire =
            serde_json::to_value(one_frame_video_plan()).expect("the browser plan serializes");
        wire["videos"][0]["source"]["endNanoseconds"] =
            serde_json::Value::String(String::from("33333334"));
        wire["videos"][0]["source"]["naturalEndNanoseconds"] =
            serde_json::Value::String(String::from("33333334"));

        let error = serde_json::from_value::<BrowserPlan>(wire)
            .expect_err("the source mapping determines two output frames");

        assert!(error.to_string().contains("source mapping"));
    }

    #[test]
    fn rejects_noncanonical_video_source_wire_values() {
        let wire =
            serde_json::to_value(one_frame_video_plan()).expect("the browser plan serializes");
        let mut nanoseconds = wire.clone();
        nanoseconds["videos"][0]["source"]["startNanoseconds"] =
            serde_json::Value::String(String::from("00"));
        let mut playback_rate = wire.clone();
        playback_rate["videos"][0]["source"]["playbackRate"] =
            serde_json::json!({ "numerator": 2, "denominator": 2 });
        let mut plays = wire.clone();
        plays["videos"][0]["source"]["plays"] = serde_json::json!(0);
        let mut hold = wire;
        hold["videos"][0]["source"]["holdLastNanoseconds"] =
            serde_json::Value::String(String::from("00"));

        assert!(serde_json::from_value::<BrowserPlan>(nanoseconds).is_err());
        assert!(serde_json::from_value::<BrowserPlan>(playback_rate).is_err());
        assert!(serde_json::from_value::<BrowserPlan>(plays).is_err());
        assert!(serde_json::from_value::<BrowserPlan>(hold).is_err());
    }

    #[test]
    fn rejects_duplicate_node_identity_at_the_wire_boundary() {
        let plan = r#"{
            "timelineVersion":7,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":1},
            "evaluation":{"start":0,"end":1},
            "output":{"start":0,"end":1},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":0,"end":1}}],
            "shots":[{"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":1}}],
            "transitions":[],
            "variantFields":[],
            "videos":[],
            "overlays":[
                {"node":{"nodeId":7,"authoredId":null},"shotId":2,"kind":"title","text":"A","interval":{"start":0,"end":1}},
                {"node":{"nodeId":7,"authoredId":null},"shotId":2,"kind":"title","text":"B","interval":{"start":0,"end":1}}
            ]
        }"#;

        assert!(serde_json::from_str::<BrowserPlan>(plan).is_err());
    }

    #[test]
    fn rejects_non_dense_node_identity_at_the_wire_boundary() {
        let plan = r#"{
            "timelineVersion":7,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":1},
            "evaluation":{"start":0,"end":1},
            "output":{"start":0,"end":1},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":0,"end":1}}],
            "shots":[{"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":1}}],
            "transitions":[],
            "variantFields":[],
            "videos":[],
            "overlays":[
                {"node":{"nodeId":4,"authoredId":null},"shotId":2,"kind":"title","text":"A","interval":{"start":0,"end":1}}
            ]
        }"#;

        let error = serde_json::from_str::<BrowserPlan>(plan)
            .expect_err("browser node identity must remain dense");

        assert!(error.to_string().contains("not dense"));
    }

    #[test]
    fn rejects_a_child_interval_outside_its_structural_parent() {
        let plan = r#"{
            "timelineVersion":7,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":4},
            "evaluation":{"start":0,"end":4},
            "output":{"start":0,"end":4},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":1,"end":3}}],
            "shots":[{"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":4}}],
            "transitions":[],
            "variantFields":[],
            "videos":[],
            "overlays":[]
        }"#;

        assert!(serde_json::from_str::<BrowserPlan>(plan).is_err());
    }

    #[test]
    fn rejects_noncanonical_browser_node_order() {
        let plan = r#"{
            "timelineVersion":7,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":4},
            "evaluation":{"start":0,"end":4},
            "output":{"start":0,"end":4},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":0,"end":4}}],
            "shots":[
                {"node":{"nodeId":3,"authoredId":null},"sceneId":1,"interval":{"start":2,"end":4}},
                {"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":2}}
            ],
            "transitions":[],
            "variantFields":[],
            "videos":[],
            "overlays":[]
        }"#;

        let error = serde_json::from_str::<BrowserPlan>(plan)
            .expect_err("browser arrays retain canonical compiler order");

        assert!(error.to_string().contains("canonical order"));
    }

    #[test]
    fn rejects_a_transition_across_scene_ownership() {
        let plan = r#"{
            "timelineVersion":7,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":4},
            "evaluation":{"start":0,"end":4},
            "output":{"start":0,"end":4},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[
                {"node":{"nodeId":1,"authoredId":null},"interval":{"start":0,"end":2}},
                {"node":{"nodeId":3,"authoredId":null},"interval":{"start":1,"end":4}}
            ],
            "shots":[
                {"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":2}},
                {"node":{"nodeId":4,"authoredId":null},"sceneId":3,"interval":{"start":1,"end":4}}
            ],
            "transitions":[{
                "node":{"nodeId":5,"authoredId":null},
                "outgoingShotId":2,
                "incomingShotId":4,
                "interval":{"start":1,"end":2}
            }],
            "variantFields":[],
            "videos":[],
            "overlays":[]
        }"#;

        let error = serde_json::from_str::<BrowserPlan>(plan)
            .expect_err("a transition cannot cross scene ownership");

        assert!(error.to_string().contains("browser transition"));
    }

    #[test]
    fn rejects_a_transition_that_does_not_match_the_shot_boundary() {
        let plan = r#"{
            "timelineVersion":7,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":6},
            "evaluation":{"start":0,"end":6},
            "output":{"start":0,"end":6},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":0,"end":6}}],
            "shots":[
                {"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":4}},
                {"node":{"nodeId":4,"authoredId":null},"sceneId":1,"interval":{"start":2,"end":6}}
            ],
            "transitions":[{
                "node":{"nodeId":3,"authoredId":null},
                "outgoingShotId":2,
                "incomingShotId":4,
                "interval":{"start":3,"end":4}
            }],
            "variantFields":[],
            "videos":[],
            "overlays":[]
        }"#;

        let error = serde_json::from_str::<BrowserPlan>(plan)
            .expect_err("the transition must equal the complete shot overlap");

        assert!(error.to_string().contains("browser transition"));
    }

    #[test]
    fn enumerates_content_placement_boundaries() {
        let asset_id = FrozenAssetId::from_sha256([1; 32]);
        let timeline = timeline_with_content_in(
            vec![
                video(asset_id, interval(0, 2)),
                overlay(ElementKind::Title, interval(2, 4), "Opening"),
            ],
            interval(0, 4),
        );
        let source_timings = BTreeMap::from([(
            asset_id,
            VideoTiming::Constant(FrameRate::new(30, 1).expect("the fixture frame rate is valid")),
        )]);
        let plan = BrowserPlan::from_timeline(&timeline, &source_timings)
            .expect("the fixture forms a valid browser plan");

        let boundaries = plan.placement_boundaries().collect::<BTreeSet<_>>();

        assert_eq!(
            boundaries,
            BTreeSet::from([wire_frame(0), wire_frame(2), wire_frame(4)]),
        );

        let foreground = plan.foreground_only();
        assert!(foreground.videos().is_empty());
        assert_eq!(foreground.output(), plan.output());
        assert_eq!(foreground.overlays()[0].node().id(), wire_node_id(3));
        assert_eq!(foreground.overlays()[0].kind(), plan.overlays()[0].kind());
        assert_eq!(foreground.overlays()[0].text(), plan.overlays()[0].text());
        assert_eq!(
            foreground.overlays()[0].interval(),
            plan.overlays()[0].interval(),
        );
        let encoded = serde_json::to_string(&foreground).expect("the foreground plan serializes");
        serde_json::from_str::<BrowserPlan>(&encoded)
            .expect("a Rust-produced foreground plan satisfies its own wire contract");
    }

    #[test]
    fn enumerates_structural_placement_boundaries_without_content() {
        let plan = r#"{
            "timelineVersion":7,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":4},
            "evaluation":{"start":0,"end":4},
            "output":{"start":0,"end":4},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":0,"end":4}}],
            "shots":[
                {"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":2}},
                {"node":{"nodeId":3,"authoredId":null},"sceneId":1,"interval":{"start":2,"end":4}}
            ],
            "transitions":[],
            "variantFields":[],
            "videos":[],
            "overlays":[]
        }"#;
        let plan = serde_json::from_str::<BrowserPlan>(plan)
            .expect("the structural fixture satisfies the browser contract");

        assert_eq!(
            plan.placement_boundaries().collect::<BTreeSet<_>>(),
            BTreeSet::from([wire_frame(0), wire_frame(2), wire_frame(4)]),
        );
    }

    #[test]
    fn rejects_a_plan_outside_the_video_budget() {
        let asset_id = FrozenAssetId::from_sha256([1; 32]);
        let timeline = timeline_with_videos(asset_id, MAX_BROWSER_VIDEOS + 1);
        let source_timings = BTreeMap::from([(
            asset_id,
            VideoTiming::Constant(FrameRate::new(30, 1).expect("the fixture frame rate is valid")),
        )]);

        assert_eq!(
            BrowserPlan::from_timeline(&timeline, &source_timings),
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
    fn retains_caption_timing_across_unit_evaluations() {
        let mut timeline = timeline_with_content_in(Vec::new(), interval(0, 4));
        timeline.replace_captions(vec![caption(interval(1, 3), "Caption")]);

        let plan = BrowserPlan::from_timeline_for_unit(
            &timeline,
            &BTreeMap::new(),
            interval(0, 2),
            interval(0, 2),
        )
        .expect("a crossing caption remains exactly evaluable inside the unit");

        assert_eq!(plan.overlays().len(), 1);
        assert_eq!(plan.overlays()[0].kind(), BrowserOverlayKind::Caption);
        assert_eq!(plan.overlays()[0].interval().start().get(), 1);
        assert_eq!(plan.overlays()[0].interval().end().get(), 3);
    }

    #[test]
    fn rejects_missing_or_malformed_caption_track_metadata() {
        let mut timeline = timeline_with_content_in(Vec::new(), interval(0, 4));
        timeline.replace_captions(vec![caption(interval(1, 3), "Caption")]);
        let plan = BrowserPlan::from_timeline(&timeline, &BTreeMap::new())
            .expect("the typed caption forms a valid browser plan");
        let wire = serde_json::to_value(plan).expect("the browser plan serializes");

        let mut missing = wire.clone();
        missing["overlays"][0]
            .as_object_mut()
            .expect("the fixture overlay is an object")
            .remove("captionTrack");
        assert!(serde_json::from_value::<BrowserPlan>(missing).is_err());

        let mut malformed = wire;
        malformed["overlays"][0]["captionTrack"]["language"] =
            serde_json::Value::String("en_US".to_owned());
        assert!(serde_json::from_value::<BrowserPlan>(malformed).is_err());
    }

    #[test]
    fn retains_structural_timing_across_unit_evaluations() {
        let timeline = timeline_with_shots(vec![
            shot_with_content(Vec::new(), interval(0, 2)),
            shot_with_content(Vec::new(), interval(2, 4)),
        ]);
        let unit = interval(2, 4);

        let plan = BrowserPlan::from_timeline_for_unit(&timeline, &BTreeMap::new(), unit, unit)
            .expect("the second shot remains exactly evaluable");

        assert_eq!(plan.scenes()[0].interval().start().get(), 0);
        assert_eq!(plan.scenes()[0].interval().end().get(), 4);
        assert_eq!(plan.shots()[0].interval().start().get(), 2);
        assert_eq!(plan.shots()[0].interval().end().get(), 4);
    }

    #[test]
    fn gives_each_unit_a_dense_local_node_identity() {
        let mut timeline = timeline_with_shots(vec![
            shot_with_content(
                vec![overlay(ElementKind::Title, interval(0, 2), "Opening")],
                interval(0, 2),
            ),
            shot_with_content(
                vec![overlay(
                    ElementKind::CallToAction,
                    interval(2, 4),
                    "Buy now",
                )],
                interval(2, 4),
            ),
        ]);
        timeline.replace_captions(vec![
            caption(interval(0, 1), "Earlier"),
            caption(interval(2, 3), "Visible"),
        ]);
        let unit = interval(2, 4);

        let plan = BrowserPlan::from_timeline_for_unit(&timeline, &BTreeMap::new(), unit, unit)
            .expect("the second overlay fits its partition");

        assert_eq!(plan.film().id().get(), 0);
        assert_eq!(plan.scenes()[0].node().id().get(), 1);
        assert_eq!(plan.shots()[0].node().id().get(), 2);
        assert_eq!(plan.overlays().len(), 2);
        assert_eq!(plan.overlays()[0].node().id().get(), 3);
        assert_eq!(plan.overlays()[1].node().id().get(), 4);
    }

    #[test]
    fn rejects_combined_overlay_text_outside_the_browser_plan_budget() {
        let text = "a".repeat(MAX_BROWSER_TEXT_BYTES / 17 + 1);
        let mut timeline = timeline_with_content_in(Vec::new(), interval(0, 1));
        timeline.replace_captions((0..17).map(|_| caption(interval(0, 1), &text)).collect());

        assert_eq!(
            BrowserPlan::from_timeline(&timeline, &BTreeMap::new()),
            Err(InvalidBrowserPlan::BrowserTextBudget),
        );
    }

    #[test]
    fn shares_one_text_budget_between_overlays_and_typed_fields() {
        let text = "a".repeat((MAX_BROWSER_TEXT_BYTES - MAX_VARIANT_TEXT_BYTES) / 17 + 1);
        let mut timeline = timeline_with_content_and_variants(
            Vec::new(),
            vec![variant_text("headline", MAX_VARIANT_TEXT_BYTES)],
        );
        timeline.replace_captions((0..17).map(|_| caption(interval(0, 1), &text)).collect());

        assert_eq!(
            BrowserPlan::from_timeline(&timeline, &BTreeMap::new()),
            Err(InvalidBrowserPlan::BrowserTextBudget),
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
        let source_timings = BTreeMap::from([(
            asset_id,
            VideoTiming::Constant(FrameRate::new(30, 1).expect("the fixture frame rate is valid")),
        )]);
        let unit = interval(0, 2);

        assert_eq!(
            BrowserPlan::from_timeline_for_unit(&timeline, &source_timings, unit, unit),
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
    fn narrows_only_the_existing_published_interval() {
        let timeline = timeline_with_content_in(
            vec![overlay(ElementKind::Title, interval(0, 4), "Opening")],
            interval(0, 4),
        );
        let plan = BrowserPlan::from_timeline(&timeline, &BTreeMap::new())
            .expect("the fixture forms one browser plan");

        let narrowed = plan
            .clone()
            .into_output(interval(2, 3))
            .expect("one existing output frame can be selected");

        assert_eq!(narrowed.output().start().get(), 2);
        assert_eq!(narrowed.output().end().get(), 3);
        assert_eq!(narrowed.evaluation(), plan.evaluation());
        assert_eq!(narrowed.overlays(), plan.overlays());
        assert_eq!(
            plan.clone().into_output(interval(4, 5)),
            Err(InvalidBrowserPlan::OutputOutsideOriginalOutput),
        );
        assert_eq!(
            plan.into_output(interval(2, 2)),
            Err(InvalidBrowserPlan::EmptyOutput),
        );
    }

    #[test]
    fn rejects_invalid_region_shot_selections() {
        let timeline = timeline_with_shots(vec![
            shot_with_content(Vec::new(), interval(0, 2)),
            shot_with_content(Vec::new(), interval(2, 4)),
        ]);
        let unit = timeline.interval();

        for selected in [BTreeSet::new(), BTreeSet::from([TimelineShotIndex::new(2)])] {
            assert_eq!(
                BrowserPlan::from_timeline_for_region(
                    &timeline,
                    &BTreeMap::new(),
                    unit,
                    unit,
                    &selected,
                    &BTreeSet::new(),
                ),
                Err(InvalidBrowserPlan::InvalidShotSelection),
            );
        }
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

    #[test]
    fn preserves_transition_identity_in_authored_dom_order() {
        let span = source_span();
        let transition = TimelineTransition::new(
            TimelineElement::new(ElementKind::Transition, None, span),
            interval(45, 60),
            span,
        );
        let first = shot_with_content(Vec::new(), interval(0, 60));
        let second = TimelineShot::new(
            TimelineElement::new(ElementKind::Shot, None, span),
            TimelineTiming::new(
                interval(45, 105),
                TimingReason::ShotStart,
                TimingReason::ShotEnd,
            ),
            Some(transition),
            Vec::new(),
        );

        let plan =
            BrowserPlan::from_timeline(&timeline_with_shots(vec![first, second]), &BTreeMap::new())
                .expect("the transition fixture forms one browser plan");

        assert_eq!(plan.shots()[0].node().id(), wire_node_id(2));
        assert_eq!(plan.transitions()[0].node().id(), wire_node_id(3));
        assert_eq!(plan.shots()[1].node().id(), wire_node_id(4));
        assert_eq!(plan.transitions()[0].outgoing_shot_id(), wire_node_id(2),);
        assert_eq!(plan.transitions()[0].incoming_shot_id(), wire_node_id(4),);
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
        TimelineCaption::new(
            CaptionTrackId::from(NodeId::parse("en").expect("the track ID is valid")),
            CaptionLanguage::parse("en").expect("the language is valid"),
            interval,
            text,
            span,
            span,
        )
    }

    fn variant_text(name: &str, bytes: usize) -> TimelineVariantField {
        let span = source_span();
        TimelineVariantField::new(
            VariantFieldName::parse(name).expect("the fixture field name is canonical"),
            VariantFieldKind::Text,
            VariantValue::text(&"x".repeat(bytes)).expect("the fixture fits the field bound"),
            span,
            vec![TimelineVariantScope::Film],
        )
    }

    fn timeline_with_content(content: Vec<TimelineContent>) -> TimelineIr {
        timeline_with_content_in(content, interval(0, 1))
    }

    fn timeline_with_content_and_variants(
        content: Vec<TimelineContent>,
        variants: Vec<TimelineVariantField>,
    ) -> TimelineIr {
        timeline_with_shots_and_variants(vec![shot_with_content(content, interval(0, 1))], variants)
    }

    fn timeline_with_content_in(
        content: Vec<TimelineContent>,
        interval: FrameInterval,
    ) -> TimelineIr {
        timeline_with_shots(vec![shot_with_content(content, interval)])
    }

    fn shot_with_content(content: Vec<TimelineContent>, interval: FrameInterval) -> TimelineShot {
        let span = source_span();
        let timing = TimelineTiming::new(interval, TimingReason::ShotStart, TimingReason::ShotEnd);
        TimelineShot::new(
            TimelineElement::new(ElementKind::Shot, None, span),
            timing,
            None,
            content,
        )
    }

    fn timeline_with_shots(shots: Vec<TimelineShot>) -> TimelineIr {
        timeline_with_shots_and_variants(shots, Vec::new())
    }

    fn timeline_with_shots_and_variants(
        shots: Vec<TimelineShot>,
        variants: Vec<TimelineVariantField>,
    ) -> TimelineIr {
        let span = source_span();
        let start = shots
            .first()
            .expect("the fixture owns at least one shot")
            .timing()
            .interval()
            .start();
        let end = shots
            .last()
            .expect("the fixture owns at least one shot")
            .timing()
            .interval()
            .end();
        let interval = FrameInterval::new(start, end).expect("fixture shots remain ordered");
        let timing = TimelineTiming::new(interval, TimingReason::ShotStart, TimingReason::ShotEnd);
        let scene = TimelineScene::new(
            TimelineElement::new(ElementKind::Scene, None, span),
            timing,
            shots,
        );
        let frame_rate = FrameRate::new(30, 1).expect("the fixture frame rate is valid");

        TimelineIr::new(crate::timeline::TimelineFacts {
            timebase: Timebase::new(frame_rate),
            element: TimelineElement::new(ElementKind::Film, None, span),
            interval,
            variants,
            events: BTreeMap::new(),
            scenes: vec![scene],
            general_audio: Vec::new(),
            captions: Vec::new(),
        })
    }

    fn video(asset_id: FrozenAssetId, interval: FrameInterval) -> TimelineContent {
        let span = source_span();
        let timing = TimelineTiming::new(interval, TimingReason::ShotStart, TimingReason::ShotEnd);
        // Use the shortest source duration whose ceiling projection reproduces
        // the requested fixture interval at 30 fps.
        let final_frame = interval
            .len()
            .get()
            .checked_sub(1)
            .expect("fixture videos are non-empty");
        let duration = u64::try_from(u128::from(final_frame) * 1_000_000_000 / 30 + 1)
            .expect("fixture source duration fits its domain");
        let duration = Duration::from_nanos(duration);
        let source = MediaSource::new(
            MediaSourceInterval::new(Duration::ZERO, duration)
                .expect("the fixture source interval is non-empty"),
            PlaybackRate::ONE,
            PlayCount::ONE,
            Duration::ZERO,
            duration,
        )
        .expect("the fixture selection fits its natural source");

        TimelineContent::Video(TimelineVideo::new(
            TimelineElement::new(ElementKind::Video, None, span),
            timing,
            asset_id,
            source,
        ))
    }

    fn one_frame_video_plan() -> BrowserPlan {
        let asset_id = FrozenAssetId::from_sha256([1; 32]);
        let timeline = timeline_with_content(vec![video(asset_id, interval(0, 1))]);
        let source_timings = BTreeMap::from([(
            asset_id,
            VideoTiming::Constant(FrameRate::new(30, 1).expect("the fixture frame rate is valid")),
        )]);

        BrowserPlan::from_timeline(&timeline, &source_timings)
            .expect("the fixture forms a valid browser plan")
    }

    fn interval(start: u64, end: u64) -> FrameInterval {
        FrameInterval::new(FrameIndex::new(start), FrameIndex::new(end))
            .expect("the fixture interval is ordered")
    }

    fn source_span() -> SourceSpan {
        SourceSpan::new(SourceId::new(0), ByteOffset::ZERO, ByteOffset::ZERO)
            .expect("equal source bounds form a valid span")
    }

    fn wire_frame(value: u64) -> WireFrame {
        WireFrame::new(value).expect("the fixture frame is browser-safe")
    }

    const fn wire_node_id(value: u32) -> BrowserNodeId {
        BrowserNodeId::new(value)
    }
}
