//! Pure render-dependency facts derived from solved Timeline IR.
//!
//! Gate two begins with the production Gate-one presentation contract: its
//! video and overlay adapter has no state that crosses a shot boundary. That
//! proof permits one region per shot today. It is not a general rule that
//! shots are always independently renderable; a later temporal capability
//! must widen or join regions here before partitioning can use it.

use std::collections::BTreeSet;

use crate::model::{FrameInterval, FrozenAssetId};
use crate::timeline::{TimelineContent, TimelineIr, TimelineShot};

/// Render-dependency regions derived from one solved film.
///
/// A region states the frames that must be evaluated and the immutable media
/// bytes that affect them. The graph contains only dependency facts; bundle,
/// profile, local paths, and process configuration remain execution concerns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderGraph {
    interval: FrameInterval,
    regions: Vec<RenderRegion>,
}

impl RenderGraph {
    /// Derives the Gate-two dependency graph from solved Timeline IR.
    #[must_use]
    pub fn from_timeline(timeline: &TimelineIr) -> Self {
        let mut regions = Vec::new();

        for scene in timeline.scenes() {
            for shot in scene.shots() {
                regions.push(RenderRegion::from_shot(shot));
            }
        }

        Self {
            interval: timeline.interval(),
            regions,
        }
    }

    /// Returns the half-open interval occupied by the complete film.
    #[must_use]
    pub const fn interval(&self) -> FrameInterval {
        self.interval
    }

    /// Returns dependency regions in screenplay order.
    #[must_use]
    pub fn regions(&self) -> &[RenderRegion] {
        &self.regions
    }

    /// Produces one local unit candidate for each independently renderable region.
    ///
    /// The initial Gate-two graph contains only regions whose evaluation and
    /// output intervals are equal. Future graph edges may merge regions or
    /// widen evaluation before this operation produces a partition plan.
    #[must_use]
    pub fn into_partition(self) -> PartitionPlan {
        let units = self
            .regions
            .into_iter()
            .map(RenderPartition::from_region)
            .collect();

        PartitionPlan {
            interval: self.interval,
            units,
        }
    }
}

/// One independently evaluable dependency region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRegion {
    evaluation: FrameInterval,
    output: FrameInterval,
    media_assets: BTreeSet<FrozenAssetId>,
}

impl RenderRegion {
    fn from_shot(shot: &TimelineShot) -> Self {
        let mut media_assets = BTreeSet::new();

        for content in shot.content() {
            match content {
                TimelineContent::Video(video) => {
                    media_assets.insert(video.asset_id());
                }
                TimelineContent::VoiceOver(voice_over) => {
                    media_assets.insert(voice_over.asset_id());
                }
                TimelineContent::Overlay(_) => {}
            }
        }

        let interval = shot.timing().interval();
        Self {
            evaluation: interval,
            output: interval,
            media_assets,
        }
    }

    /// Returns frames that must be evaluated for this region.
    #[must_use]
    pub const fn evaluation(&self) -> FrameInterval {
        self.evaluation
    }

    /// Returns frames this region may publish.
    #[must_use]
    pub const fn output(&self) -> FrameInterval {
        self.output
    }

    /// Returns direct frozen-media dependencies in deterministic identity order.
    #[must_use]
    pub fn media_assets(&self) -> impl ExactSizeIterator<Item = &FrozenAssetId> {
        self.media_assets.iter()
    }
}

/// Immutable local-unit candidates produced by graph partitioning.
///
/// This is distinct from `onmark-render`'s materializable `RenderUnit`:
/// partition facts remain pure and do not own paths, browser URLs, processes,
/// or a presentation bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartitionPlan {
    interval: FrameInterval,
    units: Vec<RenderPartition>,
}

impl PartitionPlan {
    /// Returns the half-open interval covered by all planned output.
    #[must_use]
    pub const fn interval(&self) -> FrameInterval {
        self.interval
    }

    /// Returns local unit candidates in deterministic output order.
    #[must_use]
    pub fn units(&self) -> &[RenderPartition] {
        &self.units
    }
}

/// One pure local-unit candidate after dependency partitioning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderPartition {
    evaluation: FrameInterval,
    output: FrameInterval,
    media_assets: BTreeSet<FrozenAssetId>,
}

impl RenderPartition {
    fn from_region(region: RenderRegion) -> Self {
        Self {
            evaluation: region.evaluation,
            output: region.output,
            media_assets: region.media_assets,
        }
    }

    /// Returns frames that this unit must evaluate before publishing output.
    #[must_use]
    pub const fn evaluation(&self) -> FrameInterval {
        self.evaluation
    }

    /// Returns frames this unit publishes to assembly.
    #[must_use]
    pub const fn output(&self) -> FrameInterval {
        self.output
    }

    /// Returns direct frozen-media dependencies in deterministic identity order.
    #[must_use]
    pub fn media_assets(&self) -> impl ExactSizeIterator<Item = &FrozenAssetId> {
        self.media_assets.iter()
    }

    /// Returns whether this partition directly depends on one frozen asset.
    #[must_use]
    pub fn requires_media_asset(&self, asset: FrozenAssetId) -> bool {
        self.media_assets.contains(&asset)
    }
}
