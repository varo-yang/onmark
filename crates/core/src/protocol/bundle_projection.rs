//! Versioned Rust-to-bundler projection of render-region shot ownership.
//!
//! The render graph owns dependency selection. The Node bundler consumes this
//! checked wire value only to project authored HTML; it never infers regions.

use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::timeline::TimelineShotIndex;

const MAX_BUNDLE_REGIONS: usize = 10_000;
const MAX_REGION_SHOTS: usize = 10_000;

/// Current immutable projection-plan version.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[cfg_attr(feature = "schema", schemars(extend("const" = 1)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct BundleProjectionVersion(u16);

impl BundleProjectionVersion {
    /// Only projection-plan version accepted by this build.
    pub const CURRENT: Self = Self(1);
}

impl<'de> Deserialize<'de> for BundleProjectionVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let version = u16::deserialize(deserializer)?;
        if version == Self::CURRENT.0 {
            return Ok(Self::CURRENT);
        }
        Err(D::Error::custom("unsupported bundle projection version"))
    }
}

/// Exact shot dependencies for every render region in output order.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BundleProjection {
    version: BundleProjectionVersion,
    #[cfg_attr(
        feature = "schema",
        schemars(length(min = 1, max = MAX_BUNDLE_REGIONS))
    )]
    regions: Vec<BundleProjectionRegion>,
}

impl BundleProjection {
    /// Creates one checked projection from render-graph region ownership.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBundleProjection`] when the plan is empty, exceeds its
    /// bounded region count, or contains an invalid region.
    pub fn new(
        regions: impl IntoIterator<Item = BundleProjectionRegion>,
    ) -> Result<Self, InvalidBundleProjection> {
        let regions = regions.into_iter().collect::<Vec<_>>();
        validate_regions(&regions)?;
        Ok(Self {
            version: BundleProjectionVersion::CURRENT,
            regions,
        })
    }

    /// Returns render regions in deterministic output order.
    #[must_use]
    pub fn regions(&self) -> &[BundleProjectionRegion] {
        &self.regions
    }
}

impl<'de> Deserialize<'de> for BundleProjection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BundleProjectionWire::deserialize(deserializer)?;
        validate_regions(&wire.regions).map_err(D::Error::custom)?;
        Ok(Self {
            version: wire.version,
            regions: wire.regions,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct BundleProjectionWire {
    version: BundleProjectionVersion,
    regions: Vec<BundleProjectionRegion>,
}

/// Dense screenplay-order shots retained by one render region.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BundleProjectionRegion {
    #[cfg_attr(
        feature = "schema",
        schemars(length(min = 1, max = MAX_REGION_SHOTS))
    )]
    shot_indices: Vec<u32>,
}

impl BundleProjectionRegion {
    /// Creates one nonempty canonical shot set.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidBundleProjection`] when the region is empty, too large,
    /// duplicated, outside the wire index domain, or not ordered by screenplay
    /// position.
    pub fn new(
        shots: impl IntoIterator<Item = TimelineShotIndex>,
    ) -> Result<Self, InvalidBundleProjection> {
        let shot_indices = shots
            .into_iter()
            .map(|shot| {
                u32::try_from(shot.get()).map_err(|_| InvalidBundleProjection::ShotIndexOutOfRange)
            })
            .collect::<Result<Vec<_>, _>>()?;
        validate_shots(&shot_indices)?;
        Ok(Self { shot_indices })
    }

    /// Returns dense Timeline shot identities in canonical order.
    #[must_use]
    pub fn shot_indices(&self) -> &[u32] {
        &self.shot_indices
    }
}

impl<'de> Deserialize<'de> for BundleProjectionRegion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        struct Wire {
            shot_indices: Vec<u32>,
        }

        let wire = Wire::deserialize(deserializer)?;
        validate_shots(&wire.shot_indices).map_err(D::Error::custom)?;
        Ok(Self {
            shot_indices: wire.shot_indices,
        })
    }
}

/// Reason a render graph cannot cross the bundler process boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidBundleProjection {
    /// A plan contains no render region.
    EmptyPlan,
    /// A plan exceeds its bounded region count.
    TooManyRegions,
    /// A region contains no shot dependency.
    EmptyRegion,
    /// A region exceeds its bounded shot count.
    TooManyShots,
    /// A process-local shot identity cannot cross the `u32` wire domain.
    ShotIndexOutOfRange,
    /// Shot identities are duplicated or not strictly increasing.
    NonCanonicalShots,
}

impl fmt::Display for InvalidBundleProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyPlan => "bundle projection contains no render region",
            Self::TooManyRegions => "bundle projection exceeds its region limit",
            Self::EmptyRegion => "bundle projection region contains no shot",
            Self::TooManyShots => "bundle projection region exceeds its shot limit",
            Self::ShotIndexOutOfRange => {
                "bundle projection shot identity exceeds the wire index domain"
            }
            Self::NonCanonicalShots => {
                "bundle projection shot identities are not strictly increasing"
            }
        })
    }
}

impl Error for InvalidBundleProjection {}

fn validate_regions(regions: &[BundleProjectionRegion]) -> Result<(), InvalidBundleProjection> {
    if regions.is_empty() {
        return Err(InvalidBundleProjection::EmptyPlan);
    }
    if regions.len() > MAX_BUNDLE_REGIONS {
        return Err(InvalidBundleProjection::TooManyRegions);
    }
    for region in regions {
        validate_shots(&region.shot_indices)?;
    }
    Ok(())
}

fn validate_shots(shots: &[u32]) -> Result<(), InvalidBundleProjection> {
    if shots.is_empty() {
        return Err(InvalidBundleProjection::EmptyRegion);
    }
    if shots.len() > MAX_REGION_SHOTS {
        return Err(InvalidBundleProjection::TooManyShots);
    }
    if shots.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err(InvalidBundleProjection::NonCanonicalShots);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BundleProjection, BundleProjectionRegion, InvalidBundleProjection};
    use crate::timeline::TimelineShotIndex;

    #[test]
    fn rejects_noncanonical_region_ownership() {
        let error = serde_json::from_str::<BundleProjection>(
            r#"{"version":1,"regions":[{"shotIndices":[1,1]}]}"#,
        )
        .expect_err("duplicate shot identities must be rejected");
        assert!(error.to_string().contains("strictly increasing"));

        assert_eq!(
            BundleProjectionRegion::new([]),
            Err(InvalidBundleProjection::EmptyRegion),
        );
        assert_eq!(
            BundleProjectionRegion::new([TimelineShotIndex::new(usize::MAX)]),
            Err(InvalidBundleProjection::ShotIndexOutOfRange),
        );
    }

    #[test]
    fn round_trips_one_checked_projection() {
        let region =
            BundleProjectionRegion::new([TimelineShotIndex::new(2), TimelineShotIndex::new(3)])
                .expect("the fixture region is canonical");
        let projection = BundleProjection::new([region]).expect("the fixture plan is bounded");
        let encoded = serde_json::to_string(&projection).expect("projection serialization works");
        let decoded =
            serde_json::from_str::<BundleProjection>(&encoded).expect("projection is canonical");

        assert_eq!(decoded, projection);
    }
}
