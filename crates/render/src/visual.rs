//! Admission and portable facts for browser/native visual composition.
//!
//! A bundle capability is only a promise. Admission joins it to solved
//! placements and frozen media facts, then carries the resulting execution
//! proof unchanged into local and worker materialization.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use onmark_core::model::{
    FrameRate, FrozenAssetId, PresentationFrameBehavior, PresentationVisualCapability, Timebase,
    VideoColorProfile, VideoDimensions, VideoTiming,
};
use onmark_core::protocol::{
    BROWSER_OBJECT_POSITION_SCALE, BrowserMediaLayout, BrowserMediaPlacement, BrowserNodeId,
    BrowserObjectFit, BrowserPlan, BrowserVideo, MAX_BROWSER_MEDIA_LAYOUTS, WireFrameRate,
};
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{RenderProfile, RenderVideo};

const BROWSER_COMPOSITE: &str = "browserComposite";
const SEPARABLE_BACKDROP: &str = "separableBackdrop";
const SEPARABLE_OVERLAY: &str = "separableOverlay";
const EVERY_FRAME: &str = "everyFrame";
const PLACEMENT_BOUNDED: &str = "placementBounded";
const BT709_LIMITED: &str = "bt709Limited";

/// Checked visual path carried by local and remote execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VisualExecutionPlan {
    composition: VisualComposition,
    capture_cadence: BrowserCaptureCadence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum VisualComposition {
    BrowserComposite,
    SeparableBackdrop(BackdropMediaPlan),
    SeparableOverlay(LayeredMediaPlan),
}

/// Planned cadence at which Chromium must return browser-owned pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserCaptureCadence {
    /// Browser-owned pixels may differ at every authored frame.
    EveryFrame,
    /// One exact capture is reusable until the next placement boundary.
    PlacementBounded,
}

impl VisualExecutionPlan {
    pub(crate) fn select<'a>(
        capability: PresentationVisualCapability,
        frame_behavior: PresentationFrameBehavior,
        plan: &BrowserPlan,
        profile: RenderProfile,
        videos: impl ExactSizeIterator<Item = &'a RenderVideo>,
    ) -> Result<Self, UnsupportedVisualComposition> {
        let composition = select_composition(capability, plan, profile, videos)?;
        Ok(Self::new(composition, frame_behavior, plan))
    }

    pub(crate) fn browser_composite(
        frame_behavior: PresentationFrameBehavior,
        plan: &BrowserPlan,
    ) -> Self {
        Self::new(VisualComposition::BrowserComposite, frame_behavior, plan)
    }

    pub(crate) fn validate(
        &self,
        capability: PresentationVisualCapability,
        frame_behavior: PresentationFrameBehavior,
        plan: &BrowserPlan,
        profile: RenderProfile,
    ) -> Result<(), UnsupportedVisualComposition> {
        match (capability, &self.composition) {
            (
                PresentationVisualCapability::BrowserComposite
                | PresentationVisualCapability::SeparableOverlay,
                VisualComposition::BrowserComposite,
            ) => {}
            (
                PresentationVisualCapability::SeparableOverlay,
                VisualComposition::SeparableOverlay(media),
            ) => validate_layered_plan(media, plan, profile)?,
            (
                PresentationVisualCapability::SeparableBackdrop,
                VisualComposition::SeparableBackdrop(media),
            ) => validate_backdrop_plan(media, plan)?,
            _ => return Err(UnsupportedVisualComposition::CapabilityMismatch),
        }

        if self.capture_cadence == capture_cadence(frame_behavior, &self.composition, plan) {
            return Ok(());
        }
        Err(UnsupportedVisualComposition::CaptureCadenceMismatch)
    }

    /// Returns the presentation capability proved by this execution plan.
    #[must_use]
    pub const fn capability(&self) -> PresentationVisualCapability {
        match &self.composition {
            VisualComposition::BrowserComposite => PresentationVisualCapability::BrowserComposite,
            VisualComposition::SeparableBackdrop(_) => {
                PresentationVisualCapability::SeparableBackdrop
            }
            VisualComposition::SeparableOverlay(_) => {
                PresentationVisualCapability::SeparableOverlay
            }
        }
    }

    /// Returns native media facts when Chromium owns only the foreground.
    #[must_use]
    pub const fn layered_media(&self) -> Option<&LayeredMediaPlan> {
        match &self.composition {
            VisualComposition::BrowserComposite | VisualComposition::SeparableBackdrop(_) => None,
            VisualComposition::SeparableOverlay(media) => Some(media),
        }
    }

    /// Returns browser-backdrop media facts when native pixels sit above it.
    #[must_use]
    pub const fn backdrop_media(&self) -> Option<&BackdropMediaPlan> {
        match &self.composition {
            VisualComposition::SeparableBackdrop(media) if !media.media.is_empty() => Some(media),
            _ => None,
        }
    }

    /// Returns whether native execution owns any media pixels.
    #[must_use]
    pub const fn uses_native_media(&self) -> bool {
        match &self.composition {
            VisualComposition::BrowserComposite => false,
            VisualComposition::SeparableBackdrop(media) => !media.media.is_empty(),
            VisualComposition::SeparableOverlay(_) => true,
        }
    }

    /// Returns the number of native media placements owned by this plan.
    #[must_use]
    pub fn native_media_count(&self) -> usize {
        match &self.composition {
            VisualComposition::BrowserComposite => 0,
            VisualComposition::SeparableBackdrop(media) => media.media().len(),
            VisualComposition::SeparableOverlay(_) => 1,
        }
    }

    pub(crate) fn resolve_backdrop_layout(
        &self,
        evidence: &BrowserMediaLayout,
        profile: RenderProfile,
        plan: &BrowserPlan,
    ) -> Result<BackdropLayoutPlan, UnsupportedVisualComposition> {
        let VisualComposition::SeparableBackdrop(media) = &self.composition else {
            return Err(UnsupportedVisualComposition::CapabilityMismatch);
        };
        resolve_backdrop_layout(media, evidence, profile, plan)
    }

    /// Returns how often Chromium must produce browser-owned pixels.
    #[must_use]
    pub const fn capture_cadence(&self) -> BrowserCaptureCadence {
        self.capture_cadence
    }

    fn new(
        composition: VisualComposition,
        frame_behavior: PresentationFrameBehavior,
        plan: &BrowserPlan,
    ) -> Self {
        let capture_cadence = capture_cadence(frame_behavior, &composition, plan);
        Self {
            composition,
            capture_cadence,
        }
    }
}

fn select_composition<'a>(
    capability: PresentationVisualCapability,
    plan: &BrowserPlan,
    profile: RenderProfile,
    videos: impl ExactSizeIterator<Item = &'a RenderVideo>,
) -> Result<VisualComposition, UnsupportedVisualComposition> {
    match capability {
        PresentationVisualCapability::BrowserComposite => Ok(VisualComposition::BrowserComposite),
        PresentationVisualCapability::SeparableBackdrop if !publishes_video(plan) => Ok(
            VisualComposition::SeparableBackdrop(BackdropMediaPlan::empty()),
        ),
        PresentationVisualCapability::SeparableBackdrop => {
            select_backdrop_media_plan(plan, videos).map(VisualComposition::SeparableBackdrop)
        }
        PresentationVisualCapability::SeparableOverlay => {
            Ok(select_layered_media_plan(plan, profile, videos).map_or(
                VisualComposition::BrowserComposite,
                VisualComposition::SeparableOverlay,
            ))
        }
    }
}

fn capture_cadence(
    frame_behavior: PresentationFrameBehavior,
    composition: &VisualComposition,
    plan: &BrowserPlan,
) -> BrowserCaptureCadence {
    let browser_owns_video =
        matches!(composition, VisualComposition::BrowserComposite) && !plan.videos().is_empty();
    if frame_behavior == PresentationFrameBehavior::PlacementBounded && !browser_owns_video {
        BrowserCaptureCadence::PlacementBounded
    } else {
        BrowserCaptureCadence::EveryFrame
    }
}

fn select_backdrop_media_plan<'a>(
    plan: &BrowserPlan,
    videos: impl ExactSizeIterator<Item = &'a RenderVideo>,
) -> Result<BackdropMediaPlan, UnsupportedVisualComposition> {
    if plan.videos().is_empty() || plan.videos().len() > MAX_BROWSER_MEDIA_LAYOUTS {
        return Err(UnsupportedVisualComposition::BackdropMediaCount);
    }
    let available = videos
        .map(|video| (video.asset().id(), video))
        .collect::<BTreeMap<_, _>>();
    let mut media = Vec::with_capacity(plan.videos().len());

    for placement in plan.videos() {
        let video = available
            .get(&placement.asset_identity())
            .ok_or(UnsupportedVisualComposition::BackdropMediaMismatch)?;
        if video.codec() != "h264" {
            return Err(UnsupportedVisualComposition::UnsupportedCodec);
        }
        if video.color_profile() != Some(VideoColorProfile::Bt709Limited) {
            return Err(UnsupportedVisualComposition::UnsupportedColorProfile);
        }
        if !matches!(video.source_timing(), VideoTiming::Constant(_)) {
            return Err(UnsupportedVisualComposition::VariableSourceTiming);
        }
        if native_media_schedule(plan, placement).is_err() {
            return Err(UnsupportedVisualComposition::UnsupportedSourceTreatment);
        }
        media.push(BackdropMedia {
            node_id: placement.node().id(),
            asset_id: placement.asset_id().into(),
            asset_identity: placement.asset_identity(),
            dimensions: video.dimensions(),
        });
    }

    Ok(BackdropMediaPlan { media })
}

fn publishes_video(plan: &BrowserPlan) -> bool {
    plan.videos()
        .iter()
        .any(|video| intervals_overlap(video.interval(), plan.output()))
}

fn intervals_overlap(
    left: onmark_core::protocol::WireInterval,
    right: onmark_core::protocol::WireInterval,
) -> bool {
    left.start().get() < right.end().get() && right.start().get() < left.end().get()
}

fn select_layered_media_plan<'a>(
    plan: &BrowserPlan,
    profile: RenderProfile,
    mut videos: impl ExactSizeIterator<Item = &'a RenderVideo>,
) -> Option<LayeredMediaPlan> {
    // A bundle capability permits native layering; it never requires it.
    // Missing proof therefore selects the conservative browser path instead of
    // turning an optimization opportunity into a render failure.
    if videos.len() != 1 || plan.videos().len() != 1 {
        return None;
    }
    let video = videos
        .next()
        .expect("the exact-size check proved one materialized video");
    let placement = &plan.videos()[0];

    if validate_layered_placement(plan, profile, video.asset().id(), video.dimensions()).is_err() {
        return None;
    }
    if !supports_native_video(video) {
        return None;
    }

    Some(LayeredMediaPlan {
        asset_id: placement.asset_id().into(),
        asset_identity: placement.asset_identity(),
        dimensions: video.dimensions(),
    })
}

fn supports_native_video(video: &RenderVideo) -> bool {
    video.codec() == "h264" && video.color_profile() == Some(VideoColorProfile::Bt709Limited)
}

impl Serialize for VisualExecutionPlan {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.composition {
            VisualComposition::BrowserComposite => {
                let mut plan = serializer.serialize_struct("VisualExecutionPlan", 2)?;
                plan.serialize_field("mode", BROWSER_COMPOSITE)?;
                plan.serialize_field("captureCadence", &self.capture_cadence)?;
                plan.end()
            }
            VisualComposition::SeparableBackdrop(media) => {
                let mut plan = serializer.serialize_struct("VisualExecutionPlan", 3)?;
                plan.serialize_field("mode", SEPARABLE_BACKDROP)?;
                plan.serialize_field("captureCadence", &self.capture_cadence)?;
                plan.serialize_field("media", media.media())?;
                plan.end()
            }
            VisualComposition::SeparableOverlay(media) => {
                let mut plan = serializer.serialize_struct("VisualExecutionPlan", 6)?;
                plan.serialize_field("mode", SEPARABLE_OVERLAY)?;
                plan.serialize_field("captureCadence", &self.capture_cadence)?;
                plan.serialize_field("assetId", media.asset_id())?;
                plan.serialize_field("width", &media.dimensions().width())?;
                plan.serialize_field("height", &media.dimensions().height())?;
                plan.serialize_field("colorProfile", BT709_LIMITED)?;
                plan.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for VisualExecutionPlan {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = VisualExecutionPlanWire::deserialize(deserializer)?;
        match wire {
            VisualExecutionPlanWire::BrowserComposite { capture_cadence } => Ok(Self {
                composition: VisualComposition::BrowserComposite,
                capture_cadence,
            }),
            VisualExecutionPlanWire::SeparableBackdrop {
                capture_cadence,
                media,
            } => Ok(Self {
                composition: VisualComposition::SeparableBackdrop(backdrop_media_plan(media)?),
                capture_cadence,
            }),
            VisualExecutionPlanWire::SeparableOverlay {
                capture_cadence,
                asset_id,
                width,
                height,
                color_profile,
            } => Ok(Self {
                composition: VisualComposition::SeparableOverlay(layered_media(
                    asset_id,
                    width,
                    height,
                    &color_profile,
                )?),
                capture_cadence,
            }),
        }
    }
}

impl Serialize for BrowserCaptureCadence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self {
            Self::EveryFrame => EVERY_FRAME,
            Self::PlacementBounded => PLACEMENT_BOUNDED,
        })
    }
}

impl<'de> Deserialize<'de> for BrowserCaptureCadence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match <Box<str>>::deserialize(deserializer)?.as_ref() {
            EVERY_FRAME => Ok(Self::EveryFrame),
            PLACEMENT_BOUNDED => Ok(Self::PlacementBounded),
            _ => Err(serde::de::Error::custom("invalid browser capture cadence")),
        }
    }
}

#[derive(Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "mode"
)]
enum VisualExecutionPlanWire {
    BrowserComposite {
        capture_cadence: BrowserCaptureCadence,
    },
    SeparableBackdrop {
        capture_cadence: BrowserCaptureCadence,
        media: Vec<BackdropMedia>,
    },
    SeparableOverlay {
        capture_cadence: BrowserCaptureCadence,
        asset_id: Box<str>,
        width: u32,
        height: u32,
        color_profile: Box<str>,
    },
}

fn backdrop_media_plan<E>(media: Vec<BackdropMedia>) -> Result<BackdropMediaPlan, E>
where
    E: serde::de::Error,
{
    if media.len() > MAX_BROWSER_MEDIA_LAYOUTS {
        return Err(E::custom(
            "separable backdrop has an unsupported media count",
        ));
    }
    if media
        .windows(2)
        .any(|pair| pair[0].node_id() >= pair[1].node_id())
    {
        return Err(E::custom(
            "separable backdrop media is not in canonical node order",
        ));
    }
    Ok(BackdropMediaPlan { media })
}

fn layered_media<E>(
    asset_id: Box<str>,
    width: u32,
    height: u32,
    color_profile: &str,
) -> Result<LayeredMediaPlan, E>
where
    E: serde::de::Error,
{
    let asset_identity = FrozenAssetId::parse(&asset_id).map_err(E::custom)?;
    let dimensions = VideoDimensions::new(width, height).map_err(E::custom)?;
    if color_profile != BT709_LIMITED {
        return Err(E::custom(
            "layered visual plan has an unsupported color profile",
        ));
    }

    Ok(LayeredMediaPlan {
        asset_id,
        asset_identity,
        dimensions,
    })
}

/// Frozen native-media facts required after worker transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayeredMediaPlan {
    asset_id: Box<str>,
    asset_identity: FrozenAssetId,
    dimensions: VideoDimensions,
}

/// Native media placed above one browser-owned backdrop.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackdropMediaPlan {
    media: Vec<BackdropMedia>,
}

impl BackdropMediaPlan {
    const fn empty() -> Self {
        Self { media: Vec::new() }
    }

    /// Returns media placements in canonical browser-node order.
    #[must_use]
    pub fn media(&self) -> &[BackdropMedia] {
        &self.media
    }
}

/// Checked native geometry derived from one browser layout preflight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackdropLayoutPlan {
    placements: Vec<BackdropPlacementLayout>,
}

impl BackdropLayoutPlan {
    pub(crate) const fn empty() -> Self {
        Self {
            placements: Vec::new(),
        }
    }

    pub(crate) fn placements(&self) -> &[BackdropPlacementLayout] {
        &self.placements
    }
}

/// Exact source and destination rectangles for one native video.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BackdropPlacementLayout {
    node_id: BrowserNodeId,
    source: PixelRegion,
    destination: PixelRegion,
}

impl BackdropPlacementLayout {
    pub(crate) const fn node_id(self) -> BrowserNodeId {
        self.node_id
    }

    pub(crate) const fn source(self) -> PixelRegion {
        self.source
    }

    pub(crate) const fn destination(self) -> PixelRegion {
        self.destination
    }
}

/// One nonempty integer pixel region.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PixelRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl PixelRegion {
    pub(crate) const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        assert!(
            width != 0 && height != 0,
            "pixel regions require positive extents",
        );
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) const fn x(self) -> u32 {
        self.x
    }

    pub(crate) const fn y(self) -> u32 {
        self.y
    }

    pub(crate) const fn width(self) -> u32 {
        self.width
    }

    pub(crate) const fn height(self) -> u32 {
        self.height
    }
}

/// One admitted native placement awaiting browser layout evidence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackdropMedia {
    node_id: BrowserNodeId,
    asset_id: Box<str>,
    asset_identity: FrozenAssetId,
    dimensions: VideoDimensions,
}

impl Serialize for BackdropMedia {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut media = serializer.serialize_struct("BackdropMedia", 5)?;
        media.serialize_field("nodeId", &self.node_id)?;
        media.serialize_field("assetId", self.asset_id())?;
        media.serialize_field("width", &self.dimensions.width())?;
        media.serialize_field("height", &self.dimensions.height())?;
        media.serialize_field("colorProfile", BT709_LIMITED)?;
        media.end()
    }
}

impl<'de> Deserialize<'de> for BackdropMedia {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BackdropMediaWire::deserialize(deserializer)?;
        if wire.color_profile.as_ref() != BT709_LIMITED {
            return Err(serde::de::Error::custom(
                "backdrop media has an unsupported color profile",
            ));
        }
        Ok(Self {
            node_id: wire.node_id,
            asset_identity: FrozenAssetId::parse(&wire.asset_id)
                .map_err(serde::de::Error::custom)?,
            asset_id: wire.asset_id,
            dimensions: VideoDimensions::new(wire.width, wire.height)
                .map_err(serde::de::Error::custom)?,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BackdropMediaWire {
    node_id: BrowserNodeId,
    asset_id: Box<str>,
    width: u32,
    height: u32,
    color_profile: Box<str>,
}

impl BackdropMedia {
    /// Returns the placement's unit-local browser identity.
    #[must_use]
    pub const fn node_id(&self) -> BrowserNodeId {
        self.node_id
    }

    /// Returns the canonical frozen asset spelling.
    #[must_use]
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }

    /// Returns the parsed frozen identity used for unit-root lookup.
    #[must_use]
    pub const fn asset_identity(&self) -> FrozenAssetId {
        self.asset_identity
    }

    /// Returns the frozen source raster.
    #[must_use]
    pub const fn dimensions(&self) -> VideoDimensions {
        self.dimensions
    }
}

impl LayeredMediaPlan {
    /// Returns the canonical frozen asset spelling.
    #[must_use]
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }

    /// Returns the parsed frozen identity used for unit-root lookup.
    #[must_use]
    pub const fn asset_identity(&self) -> FrozenAssetId {
        self.asset_identity
    }

    /// Returns the admitted source raster.
    #[must_use]
    pub const fn dimensions(&self) -> VideoDimensions {
        self.dimensions
    }
}

fn validate_layered_plan(
    media: &LayeredMediaPlan,
    plan: &BrowserPlan,
    profile: RenderProfile,
) -> Result<(), UnsupportedVisualComposition> {
    if plan.videos().len() != 1 {
        return Err(UnsupportedVisualComposition::PrimaryVideoCount);
    }
    validate_layered_placement(plan, profile, media.asset_identity(), media.dimensions())
}

fn validate_backdrop_plan(
    media: &BackdropMediaPlan,
    plan: &BrowserPlan,
) -> Result<(), UnsupportedVisualComposition> {
    if media.media().is_empty() && !publishes_video(plan) {
        return Ok(());
    }
    if media.media().len() != plan.videos().len()
        || media.media().is_empty()
        || media.media().len() > MAX_BROWSER_MEDIA_LAYOUTS
    {
        return Err(UnsupportedVisualComposition::BackdropMediaCount);
    }
    for (native, placement) in media.media().iter().zip(plan.videos()) {
        if native.node_id() != placement.node().id()
            || native.asset_identity() != placement.asset_identity()
        {
            return Err(UnsupportedVisualComposition::BackdropMediaMismatch);
        }
        if placement.source_timing().constant_frame_rate().is_none() {
            return Err(UnsupportedVisualComposition::VariableSourceTiming);
        }
        if native_media_schedule(plan, placement).is_err() {
            return Err(UnsupportedVisualComposition::UnsupportedSourceTreatment);
        }
    }
    Ok(())
}

fn resolve_backdrop_layout(
    media: &BackdropMediaPlan,
    evidence: &BrowserMediaLayout,
    profile: RenderProfile,
    plan: &BrowserPlan,
) -> Result<BackdropLayoutPlan, UnsupportedVisualComposition> {
    if media.media().len() != evidence.placements().len()
        || media.media().len() != plan.videos().len()
    {
        return Err(UnsupportedVisualComposition::BackdropLayoutCount);
    }

    let placements = media
        .media()
        .iter()
        .zip(evidence.placements())
        .map(|(media, evidence)| resolve_backdrop_placement(media, *evidence, profile))
        .collect::<Result<Vec<_>, _>>()?;
    if placements_overlap(&placements, plan) {
        return Err(UnsupportedVisualComposition::BackdropLayoutOverlap);
    }
    Ok(BackdropLayoutPlan { placements })
}

fn resolve_backdrop_placement(
    media: &BackdropMedia,
    evidence: BrowserMediaPlacement,
    profile: RenderProfile,
) -> Result<BackdropPlacementLayout, UnsupportedVisualComposition> {
    if media.node_id() != evidence.node_id() {
        return Err(UnsupportedVisualComposition::BackdropLayoutMismatch);
    }
    let element = evidence.rectangle();
    let right = element
        .x()
        .checked_add(element.width())
        .ok_or(UnsupportedVisualComposition::BackdropLayoutBounds)?;
    let bottom = element
        .y()
        .checked_add(element.height())
        .ok_or(UnsupportedVisualComposition::BackdropLayoutBounds)?;
    if right > profile.width() || bottom > profile.height() {
        return Err(UnsupportedVisualComposition::BackdropLayoutBounds);
    }

    let source = PixelRegion::new(
        0,
        0,
        media.dimensions().width(),
        media.dimensions().height(),
    );
    let destination = PixelRegion::new(element.x(), element.y(), element.width(), element.height());
    let (source, destination) = match evidence.object_fit() {
        BrowserObjectFit::Fill => (source, destination),
        BrowserObjectFit::Contain => contain_geometry(
            source,
            destination,
            evidence.object_position().x(),
            evidence.object_position().y(),
        )?,
        BrowserObjectFit::Cover => cover_geometry(
            source,
            destination,
            evidence.object_position().x(),
            evidence.object_position().y(),
        )?,
    };

    Ok(BackdropPlacementLayout {
        node_id: media.node_id(),
        source,
        destination,
    })
}

fn contain_geometry(
    source: PixelRegion,
    destination: PixelRegion,
    position_x: u32,
    position_y: u32,
) -> Result<(PixelRegion, PixelRegion), UnsupportedVisualComposition> {
    let source_wider = u64::from(source.width()) * u64::from(destination.height())
        > u64::from(destination.width()) * u64::from(source.height());
    let (width, height) = if source_wider {
        (
            destination.width(),
            exact_scale(source.height(), destination.width(), source.width())?,
        )
    } else {
        (
            exact_scale(source.width(), destination.height(), source.height())?,
            destination.height(),
        )
    };
    let x = positioned_offset(destination.x(), destination.width() - width, position_x)?;
    let y = positioned_offset(destination.y(), destination.height() - height, position_y)?;
    Ok((source, PixelRegion::new(x, y, width, height)))
}

fn cover_geometry(
    source: PixelRegion,
    destination: PixelRegion,
    position_x: u32,
    position_y: u32,
) -> Result<(PixelRegion, PixelRegion), UnsupportedVisualComposition> {
    let source_wider = u64::from(source.width()) * u64::from(destination.height())
        > u64::from(destination.width()) * u64::from(source.height());
    let (width, height) = if source_wider {
        (
            exact_scale(source.height(), destination.width(), destination.height())?,
            source.height(),
        )
    } else {
        (
            source.width(),
            exact_scale(source.width(), destination.height(), destination.width())?,
        )
    };
    let x = positioned_offset(source.x(), source.width() - width, position_x)?;
    let y = positioned_offset(source.y(), source.height() - height, position_y)?;
    Ok((PixelRegion::new(x, y, width, height), destination))
}

fn exact_scale(
    value: u32,
    numerator: u32,
    denominator: u32,
) -> Result<u32, UnsupportedVisualComposition> {
    let product = u64::from(value) * u64::from(numerator);
    if product % u64::from(denominator) != 0 {
        return Err(UnsupportedVisualComposition::FractionalBackdropLayout);
    }
    u32::try_from(product / u64::from(denominator))
        .map_err(|_| UnsupportedVisualComposition::BackdropLayoutBounds)
}

fn positioned_offset(
    origin: u32,
    free_space: u32,
    position: u32,
) -> Result<u32, UnsupportedVisualComposition> {
    let offset = u64::from(free_space) * u64::from(position);
    let scale = u64::from(BROWSER_OBJECT_POSITION_SCALE);
    if offset % scale != 0 {
        return Err(UnsupportedVisualComposition::FractionalBackdropLayout);
    }
    origin
        .checked_add(
            u32::try_from(offset / scale)
                .map_err(|_| UnsupportedVisualComposition::BackdropLayoutBounds)?,
        )
        .ok_or(UnsupportedVisualComposition::BackdropLayoutBounds)
}

fn placements_overlap(placements: &[BackdropPlacementLayout], plan: &BrowserPlan) -> bool {
    placements.iter().enumerate().any(|(index, placement)| {
        placements[index + 1..]
            .iter()
            .enumerate()
            .any(|(offset, other)| {
                let left = plan.videos()[index].interval();
                let right = plan.videos()[index + offset + 1].interval();
                concurrent_regions_overlap(
                    placement.destination(),
                    left,
                    other.destination(),
                    right,
                )
            })
    })
}

fn concurrent_regions_overlap(
    left_region: PixelRegion,
    left_interval: onmark_core::protocol::WireInterval,
    right_region: PixelRegion,
    right_interval: onmark_core::protocol::WireInterval,
) -> bool {
    intervals_overlap(left_interval, right_interval) && regions_overlap(left_region, right_region)
}

fn regions_overlap(left: PixelRegion, right: PixelRegion) -> bool {
    let left_right = u64::from(left.x()) + u64::from(left.width());
    let left_bottom = u64::from(left.y()) + u64::from(left.height());
    let right_right = u64::from(right.x()) + u64::from(right.width());
    let right_bottom = u64::from(right.y()) + u64::from(right.height());

    u64::from(left.x()) < right_right
        && u64::from(right.x()) < left_right
        && u64::from(left.y()) < right_bottom
        && u64::from(right.y()) < left_bottom
}

fn validate_layered_placement(
    plan: &BrowserPlan,
    profile: RenderProfile,
    asset: FrozenAssetId,
    dimensions: VideoDimensions,
) -> Result<(), UnsupportedVisualComposition> {
    let [placement] = plan.videos() else {
        return Err(UnsupportedVisualComposition::PrimaryVideoCount);
    };
    if placement.asset_identity() != asset {
        return Err(UnsupportedVisualComposition::PrimaryVideoMismatch);
    }
    if !placement.interval().contains_interval(plan.output()) {
        return Err(UnsupportedVisualComposition::IncompleteCoverage);
    }
    if placement.source_timing().constant_frame_rate().is_none() {
        return Err(UnsupportedVisualComposition::VariableSourceTiming);
    }
    if native_media_schedule(plan, placement).is_err() {
        return Err(UnsupportedVisualComposition::UnsupportedSourceTreatment);
    }
    if dimensions.width() != profile.width() || dimensions.height() != profile.height() {
        return Err(UnsupportedVisualComposition::DimensionMismatch);
    }
    Ok(())
}

pub(crate) fn native_media_schedule(
    plan: &BrowserPlan,
    placement: &BrowserVideo,
) -> Result<NativeMediaSchedule, UnsupportedVisualComposition> {
    let source = placement.source().media_source();
    if source.plays().get() != 1 {
        return Err(UnsupportedVisualComposition::UnsupportedSourceTreatment);
    }
    let output_rate = frame_rate(plan.frame_rate())?;
    let source_rate = placement
        .source_timing()
        .constant_frame_rate()
        .ok_or(UnsupportedVisualComposition::VariableSourceTiming)?;
    let playback_frames = Timebase::new(output_rate)
        .frames_before_media_hold(source)
        .map_err(|_| UnsupportedVisualComposition::UnsupportedSourceTreatment)?
        .get();
    let output_frames = placement.interval().end().get() - placement.interval().start().get();
    let final_source_frame = final_source_frame(source_rate, source.interval().end().as_nanos())
        .ok_or(UnsupportedVisualComposition::UnsupportedSourceTreatment)?;

    NativeMediaSchedule::new(playback_frames, output_frames, final_source_frame)
        .ok_or(UnsupportedVisualComposition::UnsupportedSourceTreatment)
}

fn frame_rate(rate: WireFrameRate) -> Result<FrameRate, UnsupportedVisualComposition> {
    FrameRate::new(rate.numerator(), rate.denominator())
        .map_err(|_| UnsupportedVisualComposition::UnsupportedSourceTreatment)
}

// The browser selects the frame containing `end - ε`, which is exactly
// `ceil(end * rate) - 1` for a CFR source.
fn final_source_frame(rate: WireFrameRate, end_nanoseconds: u64) -> Option<u64> {
    let numerator = u128::from(end_nanoseconds) * u128::from(rate.numerator());
    let denominator = 1_000_000_000_u128 * u128::from(rate.denominator());
    let exclusive_end = numerator.div_ceil(denominator);

    exclusive_end
        .checked_sub(1)
        .and_then(|frame| u64::try_from(frame).ok())
}

/// Exact one-pass CFR schedule admitted for native final-frame realization.
///
/// `output_frames` includes frame-grid rounding and any authored hold. Repeated
/// playback never constructs this value. The final source index leaves room for
/// the encoder to form its exclusive frame bound.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NativeMediaSchedule {
    playback_frames: u64,
    output_frames: u64,
    final_source_frame: u64,
}

impl NativeMediaSchedule {
    pub(crate) fn new(
        playback_frames: u64,
        output_frames: u64,
        final_source_frame: u64,
    ) -> Option<Self> {
        if output_frames == 0 || playback_frames > output_frames || final_source_frame == u64::MAX {
            return None;
        }
        Some(Self {
            playback_frames,
            output_frames,
            final_source_frame,
        })
    }

    pub(crate) const fn playback_frames(self) -> u64 {
        self.playback_frames
    }

    pub(crate) const fn output_frames(self) -> u64 {
        self.output_frames
    }

    pub(crate) const fn final_source_frame(self) -> u64 {
        self.final_source_frame
    }
}

/// Reason a declared visual capability cannot enter the production pixel path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedVisualComposition {
    /// The bundle capability and portable execution proof disagree.
    CapabilityMismatch,
    /// The transported capture cadence and admitted bundle proof disagree.
    CaptureCadenceMismatch,
    /// Native backdrop composition requires one bounded nonempty media set.
    BackdropMediaCount,
    /// Native backdrop media differs from the solved browser placements.
    BackdropMediaMismatch,
    /// Browser preflight returned a different number of video layouts.
    BackdropLayoutCount,
    /// Browser preflight returned a layout for the wrong video node.
    BackdropLayoutMismatch,
    /// Browser preflight returned geometry outside the output viewport.
    BackdropLayoutBounds,
    /// Browser preflight requires subpixel native crop or placement.
    FractionalBackdropLayout,
    /// Native video pixels would overlap and require CSS stacking semantics.
    BackdropLayoutOverlap,
    /// The admitted path requires exactly one primary-video placement.
    PrimaryVideoCount,
    /// The portable native-media identity differs from the solved placement.
    PrimaryVideoMismatch,
    /// The primary video does not occupy the complete published interval.
    IncompleteCoverage,
    /// Native selection has not proved variable source-frame timestamps.
    VariableSourceTiming,
    /// Native selection has not proved this source-treatment combination.
    UnsupportedSourceTreatment,
    /// Source pixels cannot be placed without inventing CSS layout semantics.
    DimensionMismatch,
    /// Native decoding requires one complete supported source-color tuple.
    UnsupportedColorProfile,
    /// Native decoding has not admitted the selected source codec.
    UnsupportedCodec,
}

impl fmt::Display for UnsupportedVisualComposition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::CapabilityMismatch => {
                "visual execution plan does not match the bundle capability"
            }
            Self::CaptureCadenceMismatch => {
                "visual execution plan does not match the bundle frame behavior"
            }
            Self::BackdropMediaCount => "separable backdrop requires a bounded nonempty media set",
            Self::BackdropMediaMismatch => {
                "separable backdrop media does not match browser placements"
            }
            Self::BackdropLayoutCount => {
                "separable backdrop layout does not cover every media placement"
            }
            Self::BackdropLayoutMismatch => {
                "separable backdrop layout does not match browser media identities"
            }
            Self::BackdropLayoutBounds => {
                "separable backdrop layout lies outside the output viewport"
            }
            Self::FractionalBackdropLayout => {
                "separable backdrop layout requires subpixel native geometry"
            }
            Self::BackdropLayoutOverlap => {
                "separable backdrop media overlaps and requires browser stacking"
            }
            Self::PrimaryVideoCount => {
                "separable overlay requires exactly one primary-video placement"
            }
            Self::PrimaryVideoMismatch => {
                "separable overlay media does not match the primary-video placement"
            }
            Self::IncompleteCoverage => {
                "separable overlay requires primary video to cover the complete output"
            }
            Self::VariableSourceTiming => {
                "native visual composition requires constant source timing"
            }
            Self::UnsupportedSourceTreatment => {
                "native visual composition does not admit this source treatment"
            }
            Self::DimensionMismatch => {
                "separable overlay requires source and output dimensions to match"
            }
            Self::UnsupportedColorProfile => {
                "native visual composition requires a complete supported source-color profile"
            }
            Self::UnsupportedCodec => {
                "native visual composition does not support the selected video codec"
            }
        })
    }
}

impl Error for UnsupportedVisualComposition {}

#[cfg(test)]
mod tests {
    use onmark_core::protocol::{BrowserMediaLayout, WireInterval};

    use super::{
        BackdropLayoutPlan, BackdropMedia, BackdropMediaPlan, PixelRegion,
        UnsupportedVisualComposition, concurrent_regions_overlap, resolve_backdrop_placement,
    };
    use crate::RenderProfile;

    #[test]
    fn resolves_exact_contain_and_cover_geometry() {
        let media = media_plan(&[(1, "01", 1_920, 1_080), (2, "02", 1_920, 1_080)]);
        let layout = layout(
            r#"[
                {
                    "nodeId": 1,
                    "rectangle": {"x": 10, "y": 10, "width": 160, "height": 100},
                    "objectFit": "contain",
                    "objectPosition": {"x": 500000, "y": 500000}
                },
                {
                    "nodeId": 2,
                    "rectangle": {"x": 200, "y": 10, "width": 100, "height": 100},
                    "objectFit": "cover",
                    "objectPosition": {"x": 500000, "y": 500000}
                }
            ]"#,
        );

        let resolved = resolve_layout(&media, &layout);

        assert_eq!(
            resolved.placements()[0].destination(),
            PixelRegion::new(10, 15, 160, 90),
        );
        assert_eq!(
            resolved.placements()[1].source(),
            PixelRegion::new(420, 0, 1_080, 1_080),
        );
    }

    #[test]
    fn rejects_fractional_native_geometry() {
        let media = media_plan(&[(1, "01", 1_920, 1_080), (2, "02", 1_920, 1_080)]);
        let fractional = layout(
            r#"[
                {
                    "nodeId": 1,
                    "rectangle": {"x": 0, "y": 0, "width": 100, "height": 100},
                    "objectFit": "contain",
                    "objectPosition": {"x": 500000, "y": 500000}
                },
                {
                    "nodeId": 2,
                    "rectangle": {"x": 200, "y": 0, "width": 100, "height": 100},
                    "objectFit": "fill",
                    "objectPosition": {"x": 500000, "y": 500000}
                }
            ]"#,
        );
        let profile = RenderProfile::new(320, 180).expect("the fixture profile is valid");

        assert_eq!(
            resolve_backdrop_placement(&media.media()[0], fractional.placements()[0], profile),
            Err(UnsupportedVisualComposition::FractionalBackdropLayout),
        );
    }

    #[test]
    fn rejects_only_spatially_and_temporally_overlapping_media() {
        let left = PixelRegion::new(0, 0, 160, 90);
        let right = PixelRegion::new(80, 0, 160, 90);
        let accounting_edge = PixelRegion::new(u32::MAX - 1, 0, 2, 1);

        assert!(concurrent_regions_overlap(
            left,
            interval(0, 30),
            right,
            interval(15, 45),
        ));
        assert!(!concurrent_regions_overlap(
            left,
            interval(0, 30),
            right,
            interval(30, 60),
        ));
        assert!(concurrent_regions_overlap(
            accounting_edge,
            interval(0, 1),
            accounting_edge,
            interval(0, 1),
        ));
    }

    fn resolve_layout(
        media: &BackdropMediaPlan,
        evidence: &BrowserMediaLayout,
    ) -> BackdropLayoutPlan {
        let profile = RenderProfile::new(320, 180).expect("the fixture profile is valid");
        let placements = media
            .media()
            .iter()
            .zip(evidence.placements())
            .map(|(media, evidence)| {
                resolve_backdrop_placement(media, *evidence, profile)
                    .expect("integer CSS geometry has one exact native projection")
            })
            .collect();
        BackdropLayoutPlan { placements }
    }

    fn interval(start: u64, end: u64) -> WireInterval {
        serde_json::from_value(serde_json::json!({
            "start": start,
            "end": end,
        }))
        .expect("the fixture interval is valid")
    }

    fn media_plan(media: &[(u32, &str, u32, u32)]) -> BackdropMediaPlan {
        let media = media
            .iter()
            .map(|(node_id, byte, width, height)| {
                let asset_id = format!("sha256:{}", byte.repeat(32));
                serde_json::from_value::<BackdropMedia>(serde_json::json!({
                    "nodeId": node_id,
                    "assetId": asset_id,
                    "width": width,
                    "height": height,
                    "colorProfile": "bt709Limited",
                }))
                .expect("the fixture media facts are valid")
            })
            .collect();
        BackdropMediaPlan { media }
    }

    fn layout(json: &str) -> BrowserMediaLayout {
        serde_json::from_str(json).expect("the fixture layout evidence is valid")
    }
}
