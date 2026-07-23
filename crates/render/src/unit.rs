//! Composition of solved partitions, frozen assets, and browser presentation.
//!
//! A `RenderUnit` joins solved facts to local byte sources. Its worker request
//! is the portable projection; an `ExecutableUnit` additionally owns the private
//! verified root required by local or worker execution.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use onmark_core::model::{
    AudioChannelLayout, AudioGain, AudioSampleConversionOverflow, AudioSampleCount, FrameInterval,
    FrameRate, FrozenAsset, FrozenAssetId, Rounding, VideoColorProfile, VideoDimensions,
};
use onmark_core::protocol::{BrowserPlan, BundleManifest, InvalidBrowserPlan};
use onmark_core::render_graph::RenderPartition;
use onmark_core::timeline::{TimelineAudio, TimelineIr};

use crate::VisualExecutionPlan;
use crate::{
    AdmittedVideo, CaptureEnvironmentId, RenderProfile, UnsupportedVideo, WorkerCaptureRequest,
};

pub(crate) const MAX_AUDIO_TRACKS: usize = 32;

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
        BundleManifest::asset_path(self.id())
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

/// One materializable local unit containing facts and local requirements.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderUnit {
    browser_plan: BrowserPlan,
    bundle_manifest: BundleManifest,
    profile: RenderProfile,
    videos: BTreeMap<FrozenAssetId, RenderVideo>,
    visual_execution: VisualExecutionPlan,
    audio: AudioPlan,
}

/// One materialized video with its already-proven browser timing capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderVideo {
    asset: MaterializedAsset,
    source_frame_rate: FrameRate,
    dimensions: VideoDimensions,
    color_profile: Option<VideoColorProfile>,
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

    /// Returns the frozen source-pixel dimensions.
    #[must_use]
    pub const fn dimensions(&self) -> VideoDimensions {
        self.dimensions
    }

    /// Returns the complete admitted source-color tuple, when known.
    #[must_use]
    pub const fn color_profile(&self) -> Option<VideoColorProfile> {
        self.color_profile
    }
}

/// Render-owned audio facts for one local execution.
///
/// Audio remains outside [`BrowserPlan`]: Chromium renders resolved pixels,
/// while the executor gives this plan to `FFmpeg` after frame capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioPlan {
    tracks: Vec<RenderAudio>,
}

impl AudioPlan {
    pub(crate) fn empty() -> Self {
        Self { tracks: Vec::new() }
    }

    /// Returns tracks in canonical mix order.
    #[must_use]
    pub fn tracks(&self) -> impl ExactSizeIterator<Item = &RenderAudio> {
        self.tracks.iter()
    }
}

/// One frozen audio artifact placed on the absolute Timeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderAudio {
    mix_order: usize,
    asset: MaterializedAsset,
    interval: FrameInterval,
    gain: AudioGain,
    samples: AudioSampleCount,
    channel_layout: AudioChannelLayout,
}

impl RenderAudio {
    pub(crate) const fn mix_order(&self) -> usize {
        self.mix_order
    }

    /// Returns the verified bytes mixed for this placement.
    #[must_use]
    pub const fn asset(&self) -> &MaterializedAsset {
        &self.asset
    }

    /// Returns the exact half-open Timeline placement.
    #[must_use]
    pub const fn interval(&self) -> FrameInterval {
        self.interval
    }

    /// Returns the exact linear amplitude applied at the media boundary.
    #[must_use]
    pub const fn gain(&self) -> AudioGain {
        self.gain
    }

    /// Returns how many decoded source samples belong to this placement.
    #[must_use]
    pub const fn samples(&self) -> AudioSampleCount {
        self.samples
    }

    /// Returns the normalized source channel layout.
    #[must_use]
    pub const fn channel_layout(&self) -> AudioChannelLayout {
        self.channel_layout
    }
}

impl RenderUnit {
    /// Composes the single whole-film unit from solved facts and local inputs.
    ///
    /// Extra materialized assets are not retained. Every referenced video and
    /// audio placement must be present; video also passes the browser profile
    /// while audio becomes a separate executor-owned plan.
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
        let interval = timeline.interval();
        Self::compose(
            timeline,
            interval,
            interval,
            bundle_manifest,
            profile,
            assets,
        )
    }

    /// Composes one independently planned partition from solved facts and local inputs.
    ///
    /// The partition remains a pure core fact until this boundary joins it to a
    /// bundle, profile, and worker-local materializations.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRenderUnit`] when an input is missing, duplicated, not
    /// supported by the browser profile, or outside the browser wire domain.
    pub fn from_partition(
        timeline: &TimelineIr,
        partition: &RenderPartition,
        bundle_manifest: BundleManifest,
        profile: RenderProfile,
        assets: impl IntoIterator<Item = MaterializedAsset>,
    ) -> Result<Self, InvalidRenderUnit> {
        Self::compose(
            timeline,
            partition.evaluation(),
            partition.output(),
            bundle_manifest,
            profile,
            assets,
        )
    }

    fn compose(
        timeline: &TimelineIr,
        evaluation: FrameInterval,
        output: FrameInterval,
        bundle_manifest: BundleManifest,
        profile: RenderProfile,
        assets: impl IntoIterator<Item = MaterializedAsset>,
    ) -> Result<Self, InvalidRenderUnit> {
        let available = materialized_catalog(assets)?;
        let videos = render_videos(timeline, evaluation, &available)?;
        let source_frame_rates = videos
            .iter()
            .map(|(id, video)| (*id, video.source_frame_rate()))
            .collect();
        let browser_plan =
            BrowserPlan::from_timeline_for_unit(timeline, &source_frame_rates, evaluation, output)
                .map_err(InvalidRenderUnit::BrowserPlan)?;
        let audio = audio_plan(timeline, output, &available)?;
        let visual_execution = VisualExecutionPlan::admit(
            bundle_manifest.visual_capability(),
            &browser_plan,
            profile,
            videos.values(),
        )
        .map_err(InvalidRenderUnit::VisualComposition)?;

        Ok(Self {
            browser_plan,
            bundle_manifest,
            profile,
            videos,
            visual_execution,
            audio,
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

    /// Projects solved visual facts into one portable worker capture request.
    ///
    /// The caller supplies the deployment-owned identity that makes captured
    /// pixels reusable. Audio intentionally remains outside this request:
    /// worker capture writes only browser frames, while final assembly mixes
    /// every owned audio placement once.
    #[must_use]
    pub fn worker_capture_request(
        &self,
        capture_environment: CaptureEnvironmentId,
    ) -> WorkerCaptureRequest {
        WorkerCaptureRequest::new(
            capture_environment,
            self.bundle_manifest.clone(),
            self.browser_plan.clone(),
            self.profile,
            self.visual_execution.clone(),
        )
    }

    /// Returns required videos in deterministic frozen-identity order.
    #[must_use]
    pub fn videos(&self) -> impl ExactSizeIterator<Item = &RenderVideo> {
        self.videos.values()
    }

    /// Returns audio placements in canonical mix order.
    #[must_use]
    pub fn audio_tracks(&self) -> impl ExactSizeIterator<Item = &RenderAudio> {
        self.audio.tracks()
    }

    /// Returns the admitted browser/native visual path.
    #[must_use]
    pub const fn visual_execution(&self) -> &VisualExecutionPlan {
        &self.visual_execution
    }

    pub(crate) const fn bundle_manifest(&self) -> &BundleManifest {
        &self.bundle_manifest
    }

    pub(crate) fn materialized_assets(&self) -> impl ExactSizeIterator<Item = &MaterializedAsset> {
        let mut assets = BTreeMap::new();
        for video in self.videos.values() {
            assets.insert(video.asset().id(), video.asset());
        }
        for audio in self.audio.tracks() {
            assets.insert(audio.asset().id(), audio.asset());
        }
        assets.into_values()
    }

    pub(crate) fn into_execution_plans(self) -> (BrowserPlan, VisualExecutionPlan, AudioPlan) {
        (self.browser_plan, self.visual_execution, self.audio)
    }
}

/// Reason solved and materialized facts cannot form one render unit.
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
    /// The declared browser/media relationship cannot be executed faithfully.
    VisualComposition(crate::UnsupportedVisualComposition),
    /// The audio plan would exceed the bounded process envelope.
    AudioTrackLimit,
    /// An audio placement escapes the solved film interval.
    AudioOutsideTimeline(FrozenAssetId),
    /// Materialized bytes do not contain the audio stream solved by core.
    MissingAudioStream(FrozenAssetId),
    /// A solved placement cannot be projected onto the source sample grid.
    AudioSampleConversion {
        /// Identity of the rejected audio artifact.
        id: FrozenAssetId,
        /// Exact conversion failure.
        source: AudioSampleConversionOverflow,
    },
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
            Self::VisualComposition(source) => source.fmt(formatter),
            Self::AudioTrackLimit => {
                write!(
                    formatter,
                    "audio plan exceeds the {MAX_AUDIO_TRACKS}-track limit"
                )
            }
            Self::AudioOutsideTimeline(id) => {
                write!(
                    formatter,
                    "audio placement {id} falls outside the solved Timeline"
                )
            }
            Self::MissingAudioStream(id) => {
                write!(formatter, "materialized audio {id} has no audio stream")
            }
            Self::AudioSampleConversion { id, source } => {
                write!(
                    formatter,
                    "materialized audio {id} exceeds the sample domain: {source}"
                )
            }
            Self::BrowserPlan(source) => source.fmt(formatter),
        }
    }
}

impl Error for InvalidRenderUnit {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnsupportedVideo { source, .. } => Some(source),
            Self::VisualComposition(source) => Some(source),
            Self::AudioSampleConversion { source, .. } => Some(source),
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

fn render_videos(
    timeline: &TimelineIr,
    evaluation: FrameInterval,
    available: &BTreeMap<FrozenAssetId, MaterializedAsset>,
) -> Result<BTreeMap<FrozenAssetId, RenderVideo>, InvalidRenderUnit> {
    let mut videos = BTreeMap::new();

    for timeline_video in timeline.videos() {
        if !timeline_video.timing().interval().intersects(evaluation) {
            continue;
        }

        let id = timeline_video.asset_id();
        if videos.contains_key(&id) {
            continue;
        }
        let asset = available
            .get(&id)
            .cloned()
            .ok_or(InvalidRenderUnit::MissingAsset(id))?;
        let admitted = AdmittedVideo::admit(asset.frozen().metadata())
            .map_err(|source| InvalidRenderUnit::UnsupportedVideo { id, source })?;
        let source_frame_rate = admitted.frame_rate();
        let dimensions = admitted.metadata().dimensions();
        let color_profile = admitted.metadata().color_profile();
        videos.insert(
            id,
            RenderVideo {
                asset,
                source_frame_rate,
                dimensions,
                color_profile,
            },
        );
    }

    Ok(videos)
}

fn audio_plan(
    timeline: &TimelineIr,
    output: FrameInterval,
    available: &BTreeMap<FrozenAssetId, MaterializedAsset>,
) -> Result<AudioPlan, InvalidRenderUnit> {
    let mut tracks = Vec::new();

    for (mix_order, audio) in timeline.audio().enumerate() {
        if !owns_audio_start(output, audio) {
            continue;
        }
        if tracks.len() == MAX_AUDIO_TRACKS {
            return Err(InvalidRenderUnit::AudioTrackLimit);
        }
        tracks.push(render_audio(
            mix_order,
            audio,
            timeline.interval(),
            timeline.timebase().frame_rate(),
            available,
        )?);
    }
    Ok(AudioPlan { tracks })
}

fn render_audio(
    mix_order: usize,
    audio: &TimelineAudio,
    timeline: FrameInterval,
    frame_rate: FrameRate,
    available: &BTreeMap<FrozenAssetId, MaterializedAsset>,
) -> Result<RenderAudio, InvalidRenderUnit> {
    let id = audio.asset_id();
    let asset = available
        .get(&id)
        .cloned()
        .ok_or(InvalidRenderUnit::MissingAsset(id))?;
    let interval = audio.timing().interval();
    if !timeline.contains_interval(interval) {
        return Err(InvalidRenderUnit::AudioOutsideTimeline(id));
    }
    let metadata = asset
        .frozen()
        .metadata()
        .audio_metadata()
        .ok_or(InvalidRenderUnit::MissingAudioStream(id))?;
    let samples = metadata
        .sample_rate()
        .samples_for(interval.len(), frame_rate, Rounding::Ceil)
        .map_err(|source| InvalidRenderUnit::AudioSampleConversion { id, source })?;
    let channel_layout = metadata.channel_layout();

    Ok(RenderAudio {
        mix_order,
        asset,
        interval,
        gain: audio.gain(),
        samples,
        channel_layout,
    })
}

fn owns_audio_start(output: FrameInterval, audio: &TimelineAudio) -> bool {
    let start = audio.timing().interval().start();
    output.start() <= start && start < output.end()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use onmark_core::compiler;
    use onmark_core::model::{
        AssetMetadata, AssetRef, AudioChannelLayout, AudioGain, AudioSampleRate, Duration,
        FrameRate, FrozenAsset, FrozenAssetId, PresentationTemporalCapability,
        PresentationVisualCapability, SourceId, Timebase, VideoColorProfile, VideoDimensions,
        VideoMetadata, VideoTiming,
    };
    use onmark_core::protocol::BundleFile;
    use onmark_core::render_graph::RenderGraph;
    use onmark_core::timeline::TimelineIr;

    use super::{
        BundleManifest, CaptureEnvironmentId, InvalidRenderUnit, MAX_AUDIO_TRACKS,
        MaterializedAsset, RenderProfile, RenderUnit, WorkerCaptureRequest,
    };
    use crate::UnsupportedVisualComposition;

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
    fn projects_a_render_unit_into_a_portable_worker_capture_request() {
        let frozen = video_asset(VideoTiming::Constant(frame_rate()));
        let identity = frozen.id();
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest(),
            render_profile(),
            [materialized],
        )
        .expect("the fixture forms one render unit");

        let environment = CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH]);
        let request = unit.worker_capture_request(environment);
        let encoded =
            serde_json::to_string(&request).expect("the portable worker request serializes");
        let repeated = serde_json::to_string(&unit.worker_capture_request(environment))
            .expect("the same portable worker request serializes again");
        let wire: serde_json::Value =
            serde_json::from_str(&encoded).expect("the portable worker request is JSON");
        let decoded: WorkerCaptureRequest =
            serde_json::from_str(&encoded).expect("the portable worker request parses once");

        assert_eq!(wire["version"], 1);
        assert_eq!(wire["captureEnvironment"], environment.to_string());
        assert_eq!(encoded, repeated);
        assert_eq!(decoded, request);
        assert_eq!(decoded.browser_plan().videos().len(), 1);
        assert_eq!(
            decoded.browser_plan().videos()[0].asset_identity(),
            identity
        );
        assert_eq!(decoded.profile(), render_profile());
        assert_eq!(decoded.capture_environment(), environment);
        assert_eq!(decoded.artifact_id(), request.artifact_id());
        assert_ne!(
            request.artifact_id(),
            unit.worker_capture_request(CaptureEnvironmentId::from_sha256(
                [8; CaptureEnvironmentId::BYTE_LENGTH]
            ))
            .artifact_id()
        );
    }

    #[test]
    fn composes_a_partition_into_its_own_browser_interval() {
        let frozen = video_asset(VideoTiming::Constant(frame_rate()));
        let timeline = solve(
            r#"<film><scene><shot duration="1s"><title>Opening</title></shot><shot duration="2s"><title>Closing</title></shot></scene></film>"#,
            "unused.mp4",
            frozen,
        );
        let partitions =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .expect("the solved fixture has complete render ownership")
                .into_partition();
        let partition = partitions
            .units()
            .get(1)
            .expect("the fixture has a second partition");
        let unit = RenderUnit::from_partition(
            &timeline,
            partition,
            bundle_manifest(),
            render_profile(),
            [],
        )
        .expect("a static second shot forms a browser unit");

        assert_eq!(unit.browser_plan().evaluation().start().get(), 30);
        assert_eq!(unit.browser_plan().evaluation().end().get(), 90);
        assert_eq!(
            unit.browser_plan().output(),
            unit.browser_plan().evaluation()
        );
        assert_eq!(unit.browser_plan().overlays().len(), 1);
        assert_eq!(unit.browser_plan().overlays()[0].text(), "Closing");
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
    fn admits_only_a_complete_pixel_aligned_separable_overlay() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");

        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        )
        .expect("the frozen facts prove the narrow layered profile");

        let environment = CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH]);
        let request = unit.worker_capture_request(environment);
        let encoded = serde_json::to_string(&request).expect("the layered request serializes");
        let wire: serde_json::Value =
            serde_json::from_str(&encoded).expect("the layered request is JSON");
        let decoded: WorkerCaptureRequest =
            serde_json::from_str(&encoded).expect("the layered request validates once");

        assert_eq!(wire["visualExecution"]["mode"], "separableOverlay");
        assert_eq!(wire["visualExecution"]["width"], 320);
        assert_eq!(decoded, request);
        assert_eq!(
            decoded.visual_execution().capability(),
            PresentationVisualCapability::SeparableOverlay,
        );
        let mut invalid = wire;
        invalid["visualExecution"]["width"] = serde_json::Value::from(322);
        assert!(serde_json::from_value::<WorkerCaptureRequest>(invalid).is_err());
    }

    #[test]
    fn rejects_separable_overlay_without_one_complete_primary_video() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = solve(
            r#"<film><scene><shot><video src="opening.mp4" /></shot><shot duration="1s" /></scene></film>"#,
            "opening.mp4",
            frozen.clone(),
        );
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let result = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        );

        assert_eq!(
            result,
            Err(InvalidRenderUnit::VisualComposition(
                UnsupportedVisualComposition::IncompleteCoverage,
            )),
        );
    }

    #[test]
    fn rejects_separable_overlay_without_a_primary_video() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = solve(
            r#"<film><scene><shot duration="1s"><title>Static</title></shot></scene></film>"#,
            "unused.mp4",
            frozen,
        );
        let result = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [],
        );

        assert_eq!(
            result,
            Err(InvalidRenderUnit::VisualComposition(
                UnsupportedVisualComposition::PrimaryVideoCount,
            )),
        );
    }

    #[test]
    fn rejects_separable_overlay_without_native_pixel_facts() {
        let mismatched = layered_video_asset(
            VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive"),
            true,
        );
        let missing_color = layered_video_asset(video_dimensions(), false);

        assert_separable_rejection(mismatched, UnsupportedVisualComposition::DimensionMismatch);
        assert_separable_rejection(
            missing_color,
            UnsupportedVisualComposition::UnsupportedColorProfile,
        );
    }

    #[test]
    fn composes_voice_over_into_the_audio_plan() {
        let id = FrozenAssetId::from_sha256([1; 32]);
        let voice = FrozenAsset::new(
            id,
            AssetMetadata::audio(
                Duration::from_nanos(1_000_000_000),
                audio_sample_rate(),
                AudioChannelLayout::Mono,
            ),
        );
        let timeline = solve(
            r#"<film><scene><shot><vo src="voice.mp3" delay="500ms">Read me</vo></shot></scene></film>"#,
            "voice.mp3",
            voice.clone(),
        );
        let materialized =
            MaterializedAsset::new(voice, "/tmp/voice.mp3").expect("the fixture path is present");
        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest(),
            render_profile(),
            [materialized],
        )
        .expect("voice-over forms one whole-film audio plan");

        assert_eq!(unit.audio_tracks().len(), 1);
        let audio = unit
            .audio_tracks()
            .next()
            .expect("the unit contains one voice-over track");
        assert_eq!(audio.asset().id(), id);
        assert_eq!(audio.interval().start().get(), 15);
        assert_eq!(audio.interval().end().get(), 45);
        assert_eq!(audio.samples().get(), 48_000);
        assert_eq!(audio.gain(), AudioGain::UNITY);
        assert_eq!(unit.materialized_assets().len(), 1);
    }

    #[test]
    fn retains_voice_over_timeline_start_in_a_partition() {
        let id = FrozenAssetId::from_sha256([1; 32]);
        let voice = FrozenAsset::new(
            id,
            AssetMetadata::audio(
                Duration::from_nanos(1_000_000_000),
                audio_sample_rate(),
                AudioChannelLayout::Mono,
            ),
        );
        let timeline = solve(
            r#"<film><scene><shot duration="1s" /><shot><vo src="voice.mp3">Read me</vo></shot></scene></film>"#,
            "voice.mp3",
            voice.clone(),
        );
        let partitions =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .expect("the solved fixture has complete render ownership")
                .into_partition();
        let partition = partitions
            .units()
            .get(1)
            .expect("the fixture has a second partition");
        let materialized =
            MaterializedAsset::new(voice, "/tmp/voice.mp3").expect("the fixture path is present");
        let unit = RenderUnit::from_partition(
            &timeline,
            partition,
            bundle_manifest(),
            render_profile(),
            [materialized],
        )
        .expect("the second shot forms one audio unit");

        let audio = unit
            .audio_tracks()
            .next()
            .expect("the unit contains the second-shot voice-over");
        assert_eq!(audio.asset().id(), id);
        assert_eq!(audio.interval().start().get(), 30);
    }

    #[test]
    fn bounds_the_audio_plan_before_process_composition() {
        let voice = FrozenAsset::new(
            FrozenAssetId::from_sha256([1; 32]),
            AssetMetadata::audio(
                Duration::from_nanos(1_000_000_000),
                audio_sample_rate(),
                AudioChannelLayout::Mono,
            ),
        );
        let source = format!(
            "<film><scene><shot>{}</shot></scene></film>",
            r#"<vo src="voice.mp3" />"#.repeat(MAX_AUDIO_TRACKS + 1)
        );
        let timeline = solve(&source, "voice.mp3", voice.clone());
        let materialized =
            MaterializedAsset::new(voice, "/tmp/voice.mp3").expect("the fixture path is present");

        assert_eq!(
            RenderUnit::whole_film(
                &timeline,
                bundle_manifest(),
                render_profile(),
                [materialized],
            ),
            Err(InvalidRenderUnit::AudioTrackLimit),
        );
    }

    fn video_timeline(frozen: FrozenAsset) -> TimelineIr {
        solve(
            r#"<film><scene><shot><video src="opening.mp4" /></shot></scene></film>"#,
            "opening.mp4",
            frozen,
        )
    }

    fn audio_sample_rate() -> AudioSampleRate {
        AudioSampleRate::new(48_000).expect("48 kHz is valid")
    }

    fn video_asset(timing: VideoTiming) -> FrozenAsset {
        video_asset_with(
            timing,
            VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive"),
            None,
        )
    }

    fn layered_video_asset(dimensions: VideoDimensions, color: bool) -> FrozenAsset {
        let color_profile = color.then_some(VideoColorProfile::Bt709Limited);
        video_asset_with(
            VideoTiming::Constant(frame_rate()),
            dimensions,
            color_profile,
        )
    }

    fn video_asset_with(
        timing: VideoTiming,
        dimensions: VideoDimensions,
        color_profile: Option<VideoColorProfile>,
    ) -> FrozenAsset {
        let duration = Duration::from_nanos(1_000_000_000);
        let metadata = VideoMetadata::new(duration, dimensions, "h264", "yuv420p", timing)
            .expect("the fixture metadata is normalized");
        let metadata = match color_profile {
            Some(profile) => metadata.with_color_profile(profile),
            None => metadata,
        };
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

    fn video_dimensions() -> VideoDimensions {
        VideoDimensions::new(320, 180).expect("fixture dimensions are positive")
    }

    fn bundle_manifest() -> BundleManifest {
        bundle_manifest_with(PresentationVisualCapability::BrowserComposite)
    }

    fn bundle_manifest_with(visual_capability: PresentationVisualCapability) -> BundleManifest {
        const DIGEST: &str =
            "sha256:0101010101010101010101010101010101010101010101010101010101010101";
        let entry = BundleFile::new(BundleManifest::ENTRY_POINT, 1, DIGEST)
            .expect("the fixture entry is valid");
        BundleManifest::new(
            PresentationTemporalCapability::Sequential,
            visual_capability,
            DIGEST,
            vec![entry],
        )
        .expect("the fixture manifest is valid")
    }

    fn assert_separable_rejection(frozen: FrozenAsset, expected: UnsupportedVisualComposition) {
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let result = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        );

        assert_eq!(result, Err(InvalidRenderUnit::VisualComposition(expected)));
    }
}
