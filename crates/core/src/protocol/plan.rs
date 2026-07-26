//! Checked browser projection of Timeline IR.
//!
//! Conversion establishes JavaScript-safe integer and collection bounds before
//! values cross the Rust/TypeScript boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::model::{ElementKind, FrameInterval, FrameRate, FrozenAssetId, NodeId};
use crate::timeline::{TimelineIr, TimelineVersion};

use super::frame::{InvalidWireFrame, WireFrame, WireFrameRate, WireInterval};
use super::projection::ProjectionBuilder;

pub(super) const MAX_BROWSER_VIDEOS: usize = 10_000;
pub(super) const MAX_BROWSER_OVERLAYS: usize = 10_000;
pub(super) const MAX_BROWSER_SCENES: usize = 10_000;
pub(super) const MAX_BROWSER_SHOTS: usize = 10_000;
const MAX_BROWSER_OVERLAY_TEXT_CHARACTERS: usize = 65_536;
pub(super) const MAX_BROWSER_OVERLAY_TEXT_BYTES: usize = 1 << 20;

/// Timeline facts consumed by the browser clock and presentation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserPlan {
    #[cfg_attr(feature = "schema", schemars(extend("const" = 1)))]
    timeline_version: u16,
    frame_rate: WireFrameRate,
    timeline: WireInterval,
    evaluation: WireInterval,
    output: WireInterval,
    film: BrowserNode,
    #[cfg_attr(feature = "schema", schemars(length(max = MAX_BROWSER_SCENES)))]
    scenes: Vec<BrowserScene>,
    #[cfg_attr(feature = "schema", schemars(length(max = MAX_BROWSER_SHOTS)))]
    shots: Vec<BrowserShot>,
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
        let projection =
            ProjectionBuilder::new(evaluation, source_frame_rates).project(timeline)?;

        Self::checked(BrowserPlanWire {
            timeline_version: timeline.version().get(),
            frame_rate: timeline.timebase().frame_rate().into(),
            timeline: WireInterval::try_from(timeline.interval())?,
            evaluation: evaluation_wire,
            output: output_wire,
            film: projection.film,
            scenes: projection.scenes,
            shots: projection.shots,
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
        if wire.overlays.len() > MAX_BROWSER_OVERLAYS {
            return Err(InvalidBrowserPlan::TooManyOverlays);
        }
        if wire.scenes.len() > MAX_BROWSER_SCENES {
            return Err(InvalidBrowserPlan::TooManyScenes);
        }
        if wire.shots.len() > MAX_BROWSER_SHOTS {
            return Err(InvalidBrowserPlan::TooManyShots);
        }
        if !wire.timeline.contains_interval(wire.evaluation) {
            return Err(InvalidBrowserPlan::EvaluationOutsideTimeline);
        }
        validate_structure(&wire)?;
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
            .videos
            .iter()
            .any(|video| !wire.evaluation.contains_interval(video.interval()))
        {
            return Err(InvalidBrowserPlan::VideoCrossesEvaluation);
        }
        Ok(Self {
            timeline_version: wire.timeline_version,
            frame_rate: wire.frame_rate,
            timeline: wire.timeline,
            evaluation: wire.evaluation,
            output: wire.output,
            film: wire.film,
            scenes: wire.scenes,
            shots: wire.shots,
            videos: wire.videos,
            overlays: wire.overlays,
        })
    }

    fn compact_node_ids(&mut self) {
        let retained = std::iter::once(self.film.node_id)
            .chain(self.scenes.iter().map(|scene| scene.node.node_id))
            .chain(self.shots.iter().map(|shot| shot.node.node_id))
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
        for overlay in &mut self.overlays {
            overlay.node.node_id = replacements[&overlay.node.node_id];
            overlay.shot_id = overlay.shot_id.map(|id| replacements[&id]);
        }
    }
}

fn validate_structure(wire: &BrowserPlanWire) -> Result<(), InvalidBrowserPlan> {
    validate_node_order(wire.scenes.iter().map(|scene| scene.node().id()))?;
    validate_node_order(wire.shots.iter().map(|shot| shot.node().id()))?;
    validate_node_order(wire.videos.iter().map(|video| video.node().id()))?;
    validate_node_order(wire.overlays.iter().map(|overlay| overlay.node().id()))?;

    let mut node_ids = BTreeSet::new();
    let mut authored_ids = BTreeSet::new();
    validate_node(&wire.film, &mut node_ids, &mut authored_ids)?;

    let mut scene_intervals = BTreeMap::new();
    for scene in &wire.scenes {
        validate_node(scene.node(), &mut node_ids, &mut authored_ids)?;
        validate_structural_interval(scene.interval(), wire.timeline, wire.evaluation)?;
        scene_intervals.insert(scene.node().id(), scene.interval());
    }

    let mut shot_intervals = BTreeMap::new();
    for shot in &wire.shots {
        validate_node(shot.node(), &mut node_ids, &mut authored_ids)?;
        validate_structural_interval(shot.interval(), wire.timeline, wire.evaluation)?;
        let parent = scene_intervals
            .get(&shot.scene_id())
            .ok_or(InvalidBrowserPlan::UnknownParentNode)?;
        validate_child_interval(shot.interval(), *parent)?;
        shot_intervals.insert(shot.node().id(), shot.interval());
    }

    for video in &wire.videos {
        validate_node(video.node(), &mut node_ids, &mut authored_ids)?;
        let parent = shot_intervals
            .get(&video.shot_id())
            .ok_or(InvalidBrowserPlan::UnknownParentNode)?;
        validate_child_interval(video.interval(), *parent)?;
    }

    for overlay in &wire.overlays {
        validate_node(overlay.node(), &mut node_ids, &mut authored_ids)?;
        validate_overlay_interval(overlay.interval(), wire.timeline, wire.evaluation)?;
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
    validate_dense_node_ids(&node_ids)?;
    Ok(())
}

fn validate_dense_node_ids(node_ids: &BTreeSet<BrowserNodeId>) -> Result<(), InvalidBrowserPlan> {
    for (expected, actual) in node_ids.iter().enumerate() {
        if usize::try_from(actual.get()) != Ok(expected) {
            return Err(InvalidBrowserPlan::NonDenseNodeIdentity);
        }
    }
    Ok(())
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

fn validate_node<'a>(
    node: &'a BrowserNode,
    node_ids: &mut BTreeSet<BrowserNodeId>,
    authored_ids: &mut BTreeSet<&'a str>,
) -> Result<(), InvalidBrowserPlan> {
    if !node_ids.insert(node.id()) {
        return Err(InvalidBrowserPlan::DuplicateNodeId);
    }
    let Some(authored_id) = node.authored_id() else {
        return Ok(());
    };
    if NodeId::parse(authored_id).is_err() {
        return Err(InvalidBrowserPlan::InvalidAuthoredId);
    }
    if !authored_ids.insert(authored_id) {
        return Err(InvalidBrowserPlan::DuplicateAuthoredId);
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
    timeline: WireInterval,
    evaluation: WireInterval,
    output: WireInterval,
    film: BrowserNode,
    scenes: Vec<BrowserScene>,
    shots: Vec<BrowserShot>,
    videos: Vec<BrowserVideo>,
    overlays: Vec<BrowserOverlay>,
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
    pub(super) const fn new(value: u32) -> Self {
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
    source_frame_rate: WireFrameRate,
}

impl BrowserVideo {
    pub(super) fn new(
        node: BrowserNode,
        shot_id: BrowserNodeId,
        asset_identity: FrozenAssetId,
        interval: WireInterval,
        source_frame_rate: WireFrameRate,
    ) -> Self {
        Self {
            node,
            shot_id,
            asset_id: asset_identity.to_string().into_boxed_str(),
            asset_identity,
            interval,
            source_frame_rate,
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
            node: wire.node,
            shot_id: wire.shot_id,
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
    node: BrowserNode,
    shot_id: BrowserNodeId,
    asset_id: Box<str>,
    interval: WireInterval,
    source_frame_rate: WireFrameRate,
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

/// One solved overlay placement consumed by the browser presentation.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BrowserOverlay {
    node: BrowserNode,
    shot_id: Option<BrowserNodeId>,
    kind: BrowserOverlayKind,
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
        text: Box<str>,
        interval: WireInterval,
    ) -> Self {
        Self {
            node,
            shot_id,
            kind,
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

    /// Returns decoded authored text with source runs joined in order.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
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

        Ok(Self {
            node: wire.node,
            shot_id: wire.shot_id,
            kind: wire.kind,
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
    text: Box<str>,
    interval: WireInterval,
}

fn overlay_text_bytes(overlays: &[BrowserOverlay]) -> usize {
    overlays
        .iter()
        .map(|overlay| overlay.text.len())
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
    /// The published interval contains no frame.
    EmptyOutput,
    /// A video placement contains no frame.
    EmptyVideo,
    /// An overlay placement contains no frame.
    EmptyOverlay,
    /// A projected scene or shot contains no frame.
    EmptyStructure,
    /// A video would need clipping at the unit evaluation boundary.
    VideoCrossesEvaluation,
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
    /// The plan contains more video placements than the current contract can carry.
    TooManyVideos,
    /// The plan contains more overlay placements than the current contract can carry.
    TooManyOverlays,
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
    /// A Timeline overlay carries a non-overlay element kind.
    InvalidOverlayKind(ElementKind),
    /// One overlay inscription exceeds the current character budget.
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
            Self::EmptyStructure => formatter.write_str("browser structural interval is empty"),
            Self::VideoCrossesEvaluation => {
                formatter.write_str("browser video crosses the evaluation boundary")
            }
            Self::OverlayOutsideTimeline => {
                formatter.write_str("browser overlay lies outside the solved film")
            }
            Self::OverlayOutsideEvaluation => {
                formatter.write_str("browser overlay does not intersect evaluation")
            }
            Self::StructureOutsideTimeline => {
                formatter.write_str("browser structure lies outside the solved film")
            }
            Self::StructureOutsideEvaluation => {
                formatter.write_str("browser structure does not intersect evaluation")
            }
            Self::TooManyScenes => {
                formatter.write_str("browser plan exceeds the scene-container limit")
            }
            Self::TooManyShots => {
                formatter.write_str("browser plan exceeds the shot-container limit")
            }
            Self::TooManyVideos => {
                formatter.write_str("browser plan exceeds the video-placement limit")
            }
            Self::TooManyOverlays => {
                formatter.write_str("browser plan exceeds the overlay-placement limit")
            }
            Self::TooManyNodes => {
                formatter.write_str("browser plan exceeds the node-identity domain")
            }
            Self::DuplicateNodeId => formatter.write_str("browser node identity is duplicated"),
            Self::InvalidAuthoredId => {
                formatter.write_str("browser node carries an invalid authored identity")
            }
            Self::DuplicateAuthoredId => {
                formatter.write_str("browser authored identity is duplicated")
            }
            Self::NonCanonicalNodeOrder => {
                formatter.write_str("browser nodes are not in canonical order")
            }
            Self::NonDenseNodeIdentity => formatter.write_str("browser node identity is not dense"),
            Self::UnknownParentNode => {
                formatter.write_str("browser node names an unknown structural parent")
            }
            Self::ChildCrossesParent => {
                formatter.write_str("browser node crosses its structural parent")
            }
            Self::InvalidOverlayKind(kind) => {
                write!(
                    formatter,
                    "timeline element {kind} is not a browser overlay"
                )
            }
            Self::OverlayTextTooLong(kind) => {
                write!(formatter, "browser {kind} text exceeds the character limit")
            }
            Self::CaptionTextTooLong => {
                formatter.write_str("browser caption text exceeds the character limit")
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
            | Self::EmptyStructure
            | Self::VideoCrossesEvaluation
            | Self::OverlayOutsideTimeline
            | Self::OverlayOutsideEvaluation
            | Self::StructureOutsideTimeline
            | Self::StructureOutsideEvaluation
            | Self::TooManyScenes
            | Self::TooManyShots
            | Self::TooManyVideos
            | Self::TooManyOverlays
            | Self::TooManyNodes
            | Self::DuplicateNodeId
            | Self::InvalidAuthoredId
            | Self::DuplicateAuthoredId
            | Self::NonCanonicalNodeOrder
            | Self::NonDenseNodeIdentity
            | Self::UnknownParentNode
            | Self::ChildCrossesParent
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
        BrowserNodeId, BrowserOverlayKind, BrowserPlan, InvalidBrowserPlan,
        MAX_BROWSER_OVERLAY_TEXT_BYTES, MAX_BROWSER_OVERLAY_TEXT_CHARACTERS, MAX_BROWSER_OVERLAYS,
        MAX_BROWSER_VIDEOS, WireFrame,
    };

    #[test]
    fn parses_only_validated_browser_plan_facts() {
        let plan = r#"{
            "timelineVersion":1,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":1},
            "evaluation":{"start":0,"end":1},
            "output":{"start":0,"end":1},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[],
            "shots":[],
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
    fn rejects_duplicate_node_identity_at_the_wire_boundary() {
        let plan = r#"{
            "timelineVersion":1,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":1},
            "evaluation":{"start":0,"end":1},
            "output":{"start":0,"end":1},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":0,"end":1}}],
            "shots":[{"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":1}}],
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
            "timelineVersion":1,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":1},
            "evaluation":{"start":0,"end":1},
            "output":{"start":0,"end":1},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":0,"end":1}}],
            "shots":[{"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":1}}],
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
            "timelineVersion":1,
            "frameRate":{"numerator":30,"denominator":1},
            "timeline":{"start":0,"end":4},
            "evaluation":{"start":0,"end":4},
            "output":{"start":0,"end":4},
            "film":{"nodeId":0,"authoredId":null},
            "scenes":[{"node":{"nodeId":1,"authoredId":null},"interval":{"start":1,"end":3}}],
            "shots":[{"node":{"nodeId":2,"authoredId":null},"sceneId":1,"interval":{"start":0,"end":4}}],
            "videos":[],
            "overlays":[]
        }"#;

        assert!(serde_json::from_str::<BrowserPlan>(plan).is_err());
    }

    #[test]
    fn rejects_noncanonical_browser_node_order() {
        let plan = r#"{
            "timelineVersion":1,
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
            "videos":[],
            "overlays":[]
        }"#;

        let error = serde_json::from_str::<BrowserPlan>(plan)
            .expect_err("browser arrays retain canonical compiler order");

        assert!(error.to_string().contains("canonical order"));
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
            "timelineVersion":1,
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
        timeline_with_shots(vec![shot_with_content(content, interval)])
    }

    fn shot_with_content(content: Vec<TimelineContent>, interval: FrameInterval) -> TimelineShot {
        let span = SourceSpan::new(SourceId::new(0), ByteOffset::ZERO, ByteOffset::ZERO)
            .expect("equal source bounds form a valid span");
        let timing = TimelineTiming::new(interval, TimingReason::ShotStart, TimingReason::ShotEnd);
        TimelineShot::new(
            TimelineElement::new(ElementKind::Shot, None, span),
            timing,
            content,
        )
    }

    fn timeline_with_shots(shots: Vec<TimelineShot>) -> TimelineIr {
        let span = SourceSpan::new(SourceId::new(0), ByteOffset::ZERO, ByteOffset::ZERO)
            .expect("equal source bounds form a valid span");
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

    const fn wire_node_id(value: u32) -> BrowserNodeId {
        BrowserNodeId::new(value)
    }
}
