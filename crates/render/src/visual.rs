//! Admission and portable facts for browser/native visual composition.
//!
//! A bundle capability is only a promise. Admission joins it to solved
//! placements and frozen media facts, then carries the resulting execution
//! proof unchanged into local and worker materialization.

use std::error::Error;
use std::fmt;

use onmark_core::model::{
    FrozenAssetId, PresentationVisualCapability, VideoColorProfile, VideoDimensions,
};
use onmark_core::protocol::BrowserPlan;
use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{RenderProfile, RenderVideo};

const BROWSER_COMPOSITE: &str = "browserComposite";
const SEPARABLE_OVERLAY: &str = "separableOverlay";
const BT709_LIMITED: &str = "bt709Limited";

/// Checked visual path carried by local and remote execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VisualExecutionPlan {
    /// Chromium owns the complete frame.
    BrowserComposite,
    /// Chromium owns a transparent foreground over one admitted native video.
    SeparableOverlay(LayeredMediaPlan),
}

impl VisualExecutionPlan {
    pub(crate) fn admit<'a>(
        capability: PresentationVisualCapability,
        plan: &BrowserPlan,
        profile: RenderProfile,
        videos: impl ExactSizeIterator<Item = &'a RenderVideo>,
    ) -> Result<Self, UnsupportedVisualComposition> {
        if capability == PresentationVisualCapability::BrowserComposite {
            return Ok(Self::BrowserComposite);
        }

        let mut videos = videos;
        if videos.len() != 1 || plan.videos().len() != 1 {
            return Err(UnsupportedVisualComposition::PrimaryVideoCount);
        }
        let video = videos
            .next()
            .expect("the exact-size check proved one materialized video");
        let placement = &plan.videos()[0];

        validate_layered_placement(plan, profile, video.asset().id(), video.dimensions())?;
        if video.color_profile() != Some(VideoColorProfile::Bt709Limited) {
            return Err(UnsupportedVisualComposition::UnsupportedColorProfile);
        }

        Ok(Self::SeparableOverlay(LayeredMediaPlan {
            asset_id: placement.asset_id().into(),
            asset_identity: placement.asset_identity(),
            dimensions: video.dimensions(),
        }))
    }

    pub(crate) fn validate(
        &self,
        capability: PresentationVisualCapability,
        plan: &BrowserPlan,
        profile: RenderProfile,
    ) -> Result<(), UnsupportedVisualComposition> {
        match (capability, self) {
            (PresentationVisualCapability::BrowserComposite, Self::BrowserComposite) => Ok(()),
            (PresentationVisualCapability::SeparableOverlay, Self::SeparableOverlay(media)) => {
                validate_layered_plan(media, plan, profile)
            }
            _ => Err(UnsupportedVisualComposition::CapabilityMismatch),
        }
    }

    /// Returns the presentation capability proved by this execution plan.
    #[must_use]
    pub const fn capability(&self) -> PresentationVisualCapability {
        match self {
            Self::BrowserComposite => PresentationVisualCapability::BrowserComposite,
            Self::SeparableOverlay(_) => PresentationVisualCapability::SeparableOverlay,
        }
    }

    /// Returns native media facts when Chromium owns only the foreground.
    #[must_use]
    pub const fn layered_media(&self) -> Option<&LayeredMediaPlan> {
        match self {
            Self::BrowserComposite => None,
            Self::SeparableOverlay(media) => Some(media),
        }
    }
}

impl Serialize for VisualExecutionPlan {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::BrowserComposite => {
                let mut plan = serializer.serialize_struct("VisualExecutionPlan", 1)?;
                plan.serialize_field("mode", BROWSER_COMPOSITE)?;
                plan.end()
            }
            Self::SeparableOverlay(media) => {
                let mut plan = serializer.serialize_struct("VisualExecutionPlan", 5)?;
                plan.serialize_field("mode", SEPARABLE_OVERLAY)?;
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
            VisualExecutionPlanWire::BrowserComposite => Ok(Self::BrowserComposite),
            VisualExecutionPlanWire::SeparableOverlay {
                asset_id,
                width,
                height,
                color_profile,
            } => layered_media(asset_id, width, height, &color_profile).map(Self::SeparableOverlay),
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
    BrowserComposite,
    SeparableOverlay {
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
