//! Composition of solved partitions, frozen assets, and browser presentation.
//!
//! A `RenderUnit` joins solved facts to local byte sources. Its worker request
//! is the portable projection; an `ExecutableUnit` additionally owns the private
//! verified root required by local or worker execution.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use onmark_core::model::{FrameIndex, FrameInterval, FrameRate, FrozenAsset, FrozenAssetId};
use onmark_core::protocol::{BrowserPlan, BundleManifest, InvalidBrowserPlan};
use onmark_core::render_graph::RenderPartition;
use onmark_core::timeline::{TimelineIr, TimelineVoiceOver};

use crate::{
    AdmittedVideo, CaptureEnvironmentId, RenderProfile, UnsupportedVideo, WorkerCaptureRequest,
};

pub(crate) const MAX_AUDIO_TRACKS: usize = 512;

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
    audio: AudioPlan,
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

    /// Returns tracks in screenplay order.
    #[must_use]
    pub fn tracks(&self) -> impl ExactSizeIterator<Item = &RenderAudio> {
        self.tracks.iter()
    }
}

/// One frozen voice-over artifact placed at an absolute Timeline frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderAudio {
    asset: MaterializedAsset,
    start: FrameIndex,
}

impl RenderAudio {
    /// Returns the verified bytes mixed for this voice-over.
    #[must_use]
    pub const fn asset(&self) -> &MaterializedAsset {
        &self.asset
    }

    /// Returns the Timeline frame at which the voice-over starts.
    #[must_use]
    pub const fn start(&self) -> FrameIndex {
        self.start
    }
}

impl RenderUnit {
    /// Composes the single whole-film unit from solved facts and local inputs.
    ///
    /// Extra materialized assets are not retained. Every referenced video and
    /// voice-over must be present; video also passes the browser profile while
    /// voice-over becomes a separate executor-owned audio plan.
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

        Ok(Self {
            browser_plan,
            bundle_manifest,
            profile,
            videos,
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
    /// every voice-over once.
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
        )
    }

    /// Returns required videos in deterministic frozen-identity order.
    #[must_use]
    pub fn videos(&self) -> impl ExactSizeIterator<Item = &RenderVideo> {
        self.videos.values()
    }

    /// Returns voice-over tracks in screenplay order.
    #[must_use]
    pub fn audio_tracks(&self) -> impl ExactSizeIterator<Item = &RenderAudio> {
        self.audio.tracks()
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

    pub(crate) fn into_execution_plans(self) -> (BrowserPlan, AudioPlan) {
        (self.browser_plan, self.audio)
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
    /// The audio plan would exceed the bounded Gate-one process envelope.
    AudioTrackLimit,
    /// A voice-over crosses a unit output boundary and cannot be mixed safely.
    AudioCrossesOutput(FrozenAssetId),
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
            Self::AudioTrackLimit => {
                write!(
                    formatter,
                    "audio plan exceeds the {MAX_AUDIO_TRACKS}-track limit"
                )
            }
            Self::AudioCrossesOutput(id) => {
                write!(
                    formatter,
                    "voice-over {id} crosses the unit output boundary"
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
        let source_frame_rate = AdmittedVideo::admit(asset.frozen().metadata())
            .map_err(|source| InvalidRenderUnit::UnsupportedVideo { id, source })?
            .frame_rate();
        videos.insert(
            id,
            RenderVideo {
                asset,
                source_frame_rate,
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

    for voice_over in timeline.voice_overs() {
        if !voice_over.timing().interval().intersects(output) {
            continue;
        }
        if tracks.len() == MAX_AUDIO_TRACKS {
            return Err(InvalidRenderUnit::AudioTrackLimit);
        }
        tracks.push(render_audio(voice_over, output, available)?);
    }
    Ok(AudioPlan { tracks })
}

fn render_audio(
    voice_over: &TimelineVoiceOver,
    output: FrameInterval,
    available: &BTreeMap<FrozenAssetId, MaterializedAsset>,
) -> Result<RenderAudio, InvalidRenderUnit> {
    let id = voice_over.asset_id();
    let asset = available
        .get(&id)
        .cloned()
        .ok_or(InvalidRenderUnit::MissingAsset(id))?;
    let interval = voice_over.timing().interval();
    if !output.contains_interval(interval) {
        return Err(InvalidRenderUnit::AudioCrossesOutput(id));
    }
    Ok(RenderAudio {
        asset,
        start: interval.start(),
    })
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
    use onmark_core::render_graph::RenderGraph;
    use onmark_core::timeline::TimelineIr;

    use super::{
        BundleManifest, CaptureEnvironmentId, InvalidRenderUnit, MAX_AUDIO_TRACKS,
        MaterializedAsset, RenderProfile, RenderUnit, WorkerCaptureRequest,
    };

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

        assert_eq!(wire["version"], 2);
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
        let partitions = RenderGraph::from_timeline(&timeline).into_partition();
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
    fn composes_voice_over_into_the_audio_plan() {
        let id = FrozenAssetId::from_sha256([1; 32]);
        let voice = FrozenAsset::new(
            id,
            AssetMetadata::audio(Duration::from_nanos(1_000_000_000)),
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
        assert_eq!(audio.start().get(), 15);
        assert_eq!(unit.materialized_assets().len(), 1);
    }

    #[test]
    fn retains_voice_over_timeline_start_in_a_partition() {
        let id = FrozenAssetId::from_sha256([1; 32]);
        let voice = FrozenAsset::new(
            id,
            AssetMetadata::audio(Duration::from_nanos(1_000_000_000)),
        );
        let timeline = solve(
            r#"<film><scene><shot duration="1s" /><shot><vo src="voice.mp3">Read me</vo></shot></scene></film>"#,
            "voice.mp3",
            voice.clone(),
        );
        let partitions = RenderGraph::from_timeline(&timeline).into_partition();
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
        assert_eq!(audio.start().get(), 30);
    }

    #[test]
    fn bounds_the_audio_plan_before_process_composition() {
        let voice = FrozenAsset::new(
            FrozenAssetId::from_sha256([1; 32]),
            AssetMetadata::audio(Duration::from_nanos(1_000_000_000)),
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
