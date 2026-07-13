use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use onmark_core::model::{FrameRate, FrozenAsset, FrozenAssetId};
use onmark_core::protocol::{BrowserPlan, BundleManifest, InvalidBrowserPlan};
use onmark_core::timeline::{TimelineIr, TimelineVideo};

use crate::{AdmittedVideo, RenderProfile, UnsupportedVideo};

/// One frozen artifact at its browser-visible execution location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaterializedAsset {
    frozen: FrozenAsset,
    local_path: PathBuf,
}

impl MaterializedAsset {
    /// Joins frozen facts with the worker-local path holding those exact bytes.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidMaterializedAsset`] when the path is empty. This value
    /// records the claimed join; [`crate::UnitRoot`] verifies the bytes while
    /// copying them into the private execution root.
    pub fn new(
        frozen: FrozenAsset,
        local_path: impl Into<PathBuf>,
    ) -> Result<Self, InvalidMaterializedAsset> {
        let local_path = local_path.into();
        if local_path.as_os_str().is_empty() {
            return Err(InvalidMaterializedAsset::EmptyLocalPath);
        }

        Ok(Self { frozen, local_path })
    }

    /// Returns the immutable identity shared with Timeline IR.
    #[must_use]
    pub const fn id(&self) -> FrozenAssetId {
        self.frozen.id()
    }

    /// Returns normalized facts probed from the materialized bytes.
    #[must_use]
    pub const fn frozen(&self) -> &FrozenAsset {
        &self.frozen
    }

    /// Returns the worker-local location of the verified bytes.
    #[must_use]
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }

    /// Returns the deterministic location beneath a materialized unit root.
    #[must_use]
    pub fn unit_relative_path(&self) -> String {
        let mut path = String::from(BundleManifest::ASSET_DIRECTORY);
        path.push('/');
        for byte in self.id().as_sha256() {
            write!(path, "{byte:02x}").expect("writing into a String cannot fail");
        }
        path
    }
}

/// Reason a materialized artifact cannot be represented safely.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidMaterializedAsset {
    /// No worker-local location was supplied.
    EmptyLocalPath,
}

impl fmt::Display for InvalidMaterializedAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("materialized asset local path cannot be empty")
    }
}

impl Error for InvalidMaterializedAsset {}

/// One whole-film unit containing facts and local materialization requirements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderUnit {
    browser_plan: BrowserPlan,
    bundle_manifest: BundleManifest,
    profile: RenderProfile,
    videos: BTreeMap<FrozenAssetId, RenderVideo>,
}

/// One materialized video with its already-proven browser timing capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderVideo {
    asset: MaterializedAsset,
    source_frame_rate: FrameRate,
}

impl RenderVideo {
    /// Returns the materialized bytes consumed by this video.
    #[must_use]
    pub const fn asset(&self) -> &MaterializedAsset {
        &self.asset
    }

    /// Returns the exact source rate proved during unit composition.
    #[must_use]
    pub const fn source_frame_rate(&self) -> FrameRate {
        self.source_frame_rate
    }
}

impl RenderUnit {
    /// Composes the single Gate-one unit from solved facts and local inputs.
    ///
    /// Extra materialized assets are not retained. Every referenced video must
    /// be present and admissible; voice-over is rejected until the Audio Plan
    /// executes it rather than being silently dropped.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRenderUnit`] when an input is missing, duplicated, not
    /// supported by the browser profile, or outside the browser wire domain.
    pub fn whole_film(
        timeline: &TimelineIr,
        bundle_manifest: BundleManifest,
        profile: RenderProfile,
        assets: impl IntoIterator<Item = MaterializedAsset>,
    ) -> Result<Self, InvalidRenderUnit> {
        reject_unplanned_audio(timeline)?;
        let mut available = materialized_catalog(assets)?;
        let required = required_video_ids(timeline);
        let mut videos = BTreeMap::new();
        for id in required {
            let asset = available
                .remove(&id)
                .ok_or(InvalidRenderUnit::MissingAsset(id))?;
            let source_frame_rate = AdmittedVideo::admit(asset.frozen().metadata())
                .map_err(|source| InvalidRenderUnit::UnsupportedVideo { id, source })?
                .frame_rate();
            let video = RenderVideo {
                asset,
                source_frame_rate,
            };
            videos.insert(id, video);
        }
        let source_frame_rates = videos
            .iter()
            .map(|(id, video)| (*id, video.source_frame_rate()))
            .collect();
        let browser_plan = BrowserPlan::from_timeline(timeline, &source_frame_rates)
            .map_err(InvalidRenderUnit::BrowserPlan)?;

        Ok(Self {
            browser_plan,
            bundle_manifest,
            profile,
            videos,
        })
    }

    /// Returns the browser-facing projection of this unit.
    #[must_use]
    pub const fn browser_plan(&self) -> &BrowserPlan {
        &self.browser_plan
    }

    /// Returns pixel-affecting output facts for this unit.
    #[must_use]
    pub const fn profile(&self) -> RenderProfile {
        self.profile
    }

    /// Returns required videos in deterministic frozen-identity order.
    #[must_use]
    pub fn videos(&self) -> impl ExactSizeIterator<Item = &RenderVideo> {
        self.videos.values()
    }

    pub(crate) const fn bundle_manifest(&self) -> &BundleManifest {
        &self.bundle_manifest
    }

    pub(crate) fn materialized_assets(&self) -> impl ExactSizeIterator<Item = &MaterializedAsset> {
        self.videos.values().map(RenderVideo::asset)
    }

    pub(crate) fn into_browser_plan(self) -> BrowserPlan {
        self.browser_plan
    }
}

/// Reason solved and materialized facts cannot form one Gate-one unit.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidRenderUnit {
    /// Two materialized inputs claim the same frozen identity.
    DuplicateAsset(FrozenAssetId),
    /// Timeline IR references bytes absent from materialization.
    MissingAsset(FrozenAssetId),
    /// A visual stream falls outside the browser media profile.
    UnsupportedVideo {
        /// Identity of the rejected artifact.
        id: FrozenAssetId,
        /// Exact profile rule that rejected it.
        source: UnsupportedVideo,
    },
    /// Gate one has not yet built the Audio Plan required by voice-over.
    VoiceOverNotSupported(FrozenAssetId),
    /// A timeline frame cannot cross the JavaScript wire boundary exactly.
    BrowserPlan(InvalidBrowserPlan),
}

impl fmt::Display for InvalidRenderUnit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateAsset(id) => write!(formatter, "materialized asset {id} is duplicated"),
            Self::MissingAsset(id) => write!(formatter, "materialized asset {id} is missing"),
            Self::UnsupportedVideo { id, source } => {
                write!(
                    formatter,
                    "materialized video {id} is unsupported: {source}"
                )
            }
            Self::VoiceOverNotSupported(id) => {
                write!(formatter, "voice-over asset {id} requires an Audio Plan")
            }
            Self::BrowserPlan(source) => source.fmt(formatter),
        }
    }
}

impl Error for InvalidRenderUnit {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnsupportedVideo { source, .. } => Some(source),
            Self::BrowserPlan(source) => Some(source),
            _ => None,
        }
    }
}

fn materialized_catalog(
    assets: impl IntoIterator<Item = MaterializedAsset>,
) -> Result<BTreeMap<FrozenAssetId, MaterializedAsset>, InvalidRenderUnit> {
    let mut catalog = BTreeMap::new();
    for asset in assets {
        let id = asset.id();
        if catalog.insert(id, asset).is_some() {
            return Err(InvalidRenderUnit::DuplicateAsset(id));
        }
    }
    Ok(catalog)
}

fn reject_unplanned_audio(timeline: &TimelineIr) -> Result<(), InvalidRenderUnit> {
    if let Some(voice_over) = timeline.voice_overs().next() {
        return Err(InvalidRenderUnit::VoiceOverNotSupported(
            voice_over.asset_id(),
        ));
    }
    Ok(())
}

fn required_video_ids(timeline: &TimelineIr) -> BTreeSet<FrozenAssetId> {
    timeline.videos().map(TimelineVideo::asset_id).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use onmark_core::compiler;
    use onmark_core::model::{
        AssetMetadata, AssetRef, Duration, FrameRate, FrozenAsset, FrozenAssetId, SourceId,
        Timebase, VideoMetadata, VideoTiming,
    };
    use onmark_core::protocol::BundleFile;
    use onmark_core::timeline::TimelineIr;

    use super::{BundleManifest, InvalidRenderUnit, MaterializedAsset, RenderProfile, RenderUnit};

    #[test]
    fn composes_only_required_admitted_video_assets() {
        let frozen = video_asset(VideoTiming::Constant(frame_rate()));
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest(),
            render_profile(),
            [materialized],
        )
        .expect("CFR H.264 forms one whole-film unit");

        assert_eq!(unit.browser_plan().videos().len(), 1);
        assert_eq!(unit.videos().len(), 1);
        assert_eq!(unit.profile(), render_profile());
        assert_eq!(
            unit.videos()
                .next()
                .expect("the unit contains one video")
                .asset()
                .unit_relative_path(),
            format!("{}/{}", BundleManifest::ASSET_DIRECTORY, "01".repeat(32)),
        );
        assert_eq!(
            unit.videos()
                .next()
                .expect("the unit contains one video")
                .source_frame_rate(),
            frame_rate(),
        );
    }

    #[test]
    fn rejects_a_missing_materialization() {
        let frozen = video_asset(VideoTiming::Constant(frame_rate()));
        let id = frozen.id();
        let timeline = video_timeline(frozen);

        assert_eq!(
            RenderUnit::whole_film(&timeline, bundle_manifest(), render_profile(), []),
            Err(InvalidRenderUnit::MissingAsset(id)),
        );
    }

    #[test]
    fn rejects_video_outside_the_browser_profile() {
        let frozen = video_asset(VideoTiming::Variable);
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");

        assert!(matches!(
            RenderUnit::whole_film(
                &timeline,
                bundle_manifest(),
                render_profile(),
                [materialized]
            ),
            Err(InvalidRenderUnit::UnsupportedVideo { .. }),
        ));
    }

    #[test]
    fn rejects_voice_over_until_an_audio_plan_executes_it() {
        let id = FrozenAssetId::from_sha256([1; 32]);
        let voice = FrozenAsset::new(
            id,
            AssetMetadata::audio(Duration::from_nanos(1_000_000_000)),
        );
        let timeline = solve(
            r#"<film><scene><shot><vo src="voice.mp3">Read me</vo></shot></scene></film>"#,
            "voice.mp3",
            voice,
        );
        assert_eq!(
            RenderUnit::whole_film(&timeline, bundle_manifest(), render_profile(), []),
            Err(InvalidRenderUnit::VoiceOverNotSupported(id)),
        );
    }

    fn video_timeline(frozen: FrozenAsset) -> TimelineIr {
        solve(
            r#"<film><scene><shot><video src="opening.mp4" /></shot></scene></film>"#,
            "opening.mp4",
            frozen,
        )
    }

    fn video_asset(timing: VideoTiming) -> FrozenAsset {
        let duration = Duration::from_nanos(1_000_000_000);
        let metadata = VideoMetadata::new(duration, "h264", "yuv420p", timing)
            .expect("the fixture metadata is normalized");
        FrozenAsset::new(
            FrozenAssetId::from_sha256([1; 32]),
            AssetMetadata::video(duration, metadata),
        )
    }

    fn solve(source: &str, asset: &str, frozen: FrozenAsset) -> TimelineIr {
        let asset = AssetRef::parse(asset).expect("the fixture asset reference is valid");
        let assets = BTreeMap::from([(asset, frozen)]);
        let (document, diagnostics) = compiler::parse(SourceId::new(0), source).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::bind(document).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
        assert!(diagnostics.is_empty());
        let report = compiler::solve(
            film.expect("the fixture resolves"),
            &assets,
            Timebase::new(frame_rate()),
        )
        .expect("the fixture has frozen metadata");
        let (timeline, diagnostics) = report.into_parts();
        assert!(diagnostics.is_empty());
        timeline.expect("the fixture produces Timeline IR")
    }

    fn frame_rate() -> FrameRate {
        FrameRate::new(30, 1).expect("the fixture frame rate is valid")
    }

    fn render_profile() -> RenderProfile {
        RenderProfile::new(320, 180).expect("the fixture dimensions are valid")
    }

    fn bundle_manifest() -> BundleManifest {
        const DIGEST: &str =
            "sha256:0101010101010101010101010101010101010101010101010101010101010101";
        let entry = BundleFile::new(BundleManifest::ENTRY_POINT, 1, DIGEST)
            .expect("the fixture entry is valid");
        BundleManifest::new(DIGEST, vec![entry]).expect("the fixture manifest is valid")
    }
}
