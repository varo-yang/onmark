//! Admission and portable facts for browser/native visual composition.
//!
//! A bundle capability is only a promise. Admission joins it to solved
//! placements and frozen media facts, then carries the resulting execution
//! proof unchanged into local and worker materialization.

use std::error::Error;
use std::fmt;

use onmark_core::model::{
    FrozenAssetId, PresentationFrameBehavior, PresentationVisualCapability, VideoColorProfile,
    VideoDimensions,
};
use onmark_core::protocol::BrowserPlan;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{RenderProfile, RenderVideo};

const BROWSER_COMPOSITE: &str = "browserComposite";
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
    ) -> Self {
        let composition = select_composition(capability, plan, profile, videos);
        Self::new(composition, frame_behavior, plan)
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
            VisualComposition::SeparableOverlay(_) => {
                PresentationVisualCapability::SeparableOverlay
            }
        }
    }

    /// Returns native media facts when Chromium owns only the foreground.
    #[must_use]
    pub const fn layered_media(&self) -> Option<&LayeredMediaPlan> {
        match &self.composition {
            VisualComposition::BrowserComposite => None,
            VisualComposition::SeparableOverlay(media) => Some(media),
        }
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
) -> VisualComposition {
    if capability == PresentationVisualCapability::BrowserComposite {
        return VisualComposition::BrowserComposite;
    }
    let Some(media) = select_layered_media_plan(plan, profile, videos) else {
        return VisualComposition::BrowserComposite;
    };
    VisualComposition::SeparableOverlay(media)
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
    if video.color_profile() != Some(VideoColorProfile::Bt709Limited) {
        return None;
    }

    Some(LayeredMediaPlan {
        asset_id: placement.asset_id().into(),
        asset_identity: placement.asset_identity(),
        dimensions: video.dimensions(),
    })
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
    SeparableOverlay {
        capture_cadence: BrowserCaptureCadence,
        asset_id: Box<str>,
        width: u32,
        height: u32,
        color_profile: Box<str>,
    },
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
    if placement.interval() != plan.output() {
        return Err(UnsupportedVisualComposition::IncompleteCoverage);
    }
    if dimensions.width() != profile.width() || dimensions.height() != profile.height() {
        return Err(UnsupportedVisualComposition::DimensionMismatch);
    }
    Ok(())
}

/// Reason a declared visual capability cannot enter the production pixel path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedVisualComposition {
    /// The bundle capability and portable execution proof disagree.
    CapabilityMismatch,
    /// The transported capture cadence and admitted bundle proof disagree.
    CaptureCadenceMismatch,
    /// The admitted path requires exactly one primary-video placement.
    PrimaryVideoCount,
    /// The portable native-media identity differs from the solved placement.
    PrimaryVideoMismatch,
    /// The primary video does not occupy the complete published interval.
    IncompleteCoverage,
    /// Source pixels cannot be placed without inventing CSS layout semantics.
    DimensionMismatch,
    /// Native decoding requires one complete supported source-color tuple.
    UnsupportedColorProfile,
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
            Self::PrimaryVideoCount => {
                "separable overlay requires exactly one primary-video placement"
            }
            Self::PrimaryVideoMismatch => {
                "separable overlay media does not match the primary-video placement"
            }
            Self::IncompleteCoverage => {
                "separable overlay requires primary video to cover the complete output"
            }
            Self::DimensionMismatch => {
                "separable overlay requires source and output dimensions to match"
            }
            Self::UnsupportedColorProfile => {
                "separable overlay requires a complete supported source-color profile"
            }
        })
    }
}

impl Error for UnsupportedVisualComposition {}
