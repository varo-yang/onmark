//! Composition of solved partitions, frozen assets, and browser presentation.
//!
//! A `RenderUnit` joins solved facts to local byte sources. Its worker request
//! is the portable projection; an `ExecutableUnit` additionally owns the private
//! verified root required by local or worker execution.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use onmark_core::model::{
    AudioChannelLayout, AudioEnvelope, AudioGain, AudioSampleConversionOverflow, AudioSampleCount,
    FrameCount, FrameIndex, FrameInterval, FrameRate, FrozenAsset, FrozenAssetId,
    PresentationDocumentScope, PresentationVisualCapability, Rounding, VideoColorProfile,
    VideoDimensions, VideoTiming,
};
use onmark_core::protocol::{BrowserPlan, BundleManifest, InvalidBrowserPlan};
use onmark_core::render_graph::{PartitionPlan, RenderPartition};
use onmark_core::timeline::{TimelineAudio, TimelineIr, TimelineShotIndex};

use crate::{
    AdmittedVideo, CaptureEnvironmentId, RenderProfile, UnsupportedVideo, WorkerCaptureRequest,
};
use crate::{UnsupportedVisualComposition, VisualExecutionPlan};

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
    bundle_manifest: Arc<BundleManifest>,
    profile: RenderProfile,
    videos: BTreeMap<FrozenAssetId, RenderVideo>,
    visual_execution: VisualExecutionPlan,
    audio: AudioPlan,
}

/// One materialized video with its already-proven browser timing capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderVideo {
    asset: MaterializedAsset,
    source_timing: VideoTiming,
    dimensions: VideoDimensions,
    color_profile: Option<VideoColorProfile>,
}

impl RenderVideo {
    /// Returns the materialized bytes consumed by this video.
    #[must_use]
    pub const fn asset(&self) -> &MaterializedAsset {
        &self.asset
    }

    /// Returns the complete source-frame timing proved during composition.
    #[must_use]
    pub const fn source_timing(&self) -> &VideoTiming {
        &self.source_timing
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
    envelope: AudioEnvelope,
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

    /// Returns exact placement-relative fade lengths.
    #[must_use]
    pub const fn envelope(&self) -> AudioEnvelope {
        self.envelope
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
        require_document_scope(&bundle_manifest, PresentationDocumentScope::WholeFilm)?;
        let interval = timeline.interval();
        Self::compose(
            timeline,
            interval,
            interval,
            None,
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
        let shots = partition.shots().copied().collect();
        Self::compose(
            timeline,
            partition.evaluation(),
            partition.output(),
            Some(&shots),
            bundle_manifest,
            profile,
            assets,
        )
    }

    /// Composes every partition under one common visual execution path.
    ///
    /// A sequence uses native layering only when every partition proves the
    /// admitted profile. Otherwise all units record browser composition before
    /// materialization, so execution never changes paths after launch.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRenderUnit`] under the same conditions as
    /// [`Self::from_partition`].
    pub fn from_partition_plan(
        timeline: &TimelineIr,
        partitions: &PartitionPlan,
        bundle_manifest: &BundleManifest,
        profile: RenderProfile,
        assets: impl IntoIterator<Item = MaterializedAsset>,
    ) -> Result<Vec<Self>, InvalidRenderUnit> {
        require_document_scope(bundle_manifest, PresentationDocumentScope::WholeFilm)?;
        let available = materialized_catalog(assets)?;
        let bundle_manifest = Arc::new(bundle_manifest.clone());
        let mut units = Vec::with_capacity(partitions.units().len());

        for partition in partitions.units() {
            let shots = partition.shots().copied().collect();
            units.push(Self::compose_from_catalog(
                timeline,
                partition.evaluation(),
                partition.output(),
                Some(&shots),
                Arc::clone(&bundle_manifest),
                profile,
                &available,
            )?);
        }
        normalize_visual_execution(&mut units);
        Ok(units)
    }

    /// Composes every partition with its own shot-scoped presentation bundle.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRenderUnit`] when the bundle count differs from the
    /// partition count or any partition fails the ordinary unit checks.
    pub fn from_partitioned_bundles(
        timeline: &TimelineIr,
        partitions: &PartitionPlan,
        bundle_manifests: Vec<BundleManifest>,
        profile: RenderProfile,
        assets: impl IntoIterator<Item = MaterializedAsset>,
    ) -> Result<Vec<Self>, InvalidRenderUnit> {
        if bundle_manifests.len() != partitions.units().len() {
            return Err(InvalidRenderUnit::BundleCount);
        }
        for manifest in &bundle_manifests {
            require_document_scope(manifest, PresentationDocumentScope::RenderRegion)?;
        }
        let available = materialized_catalog(assets)?;
        let mut units = Vec::with_capacity(partitions.units().len());

        for (partition, manifest) in partitions.units().iter().zip(bundle_manifests) {
            let shots = partition.shots().copied().collect();
            units.push(Self::compose_from_catalog(
                timeline,
                partition.evaluation(),
                partition.output(),
                Some(&shots),
                Arc::new(manifest),
                profile,
                &available,
            )?);
        }
        normalize_visual_execution(&mut units);
        Ok(units)
    }

    fn compose(
        timeline: &TimelineIr,
        evaluation: FrameInterval,
        output: FrameInterval,
        shots: Option<&BTreeSet<TimelineShotIndex>>,
        bundle_manifest: BundleManifest,
        profile: RenderProfile,
        assets: impl IntoIterator<Item = MaterializedAsset>,
    ) -> Result<Self, InvalidRenderUnit> {
        let available = materialized_catalog(assets)?;
        Self::compose_from_catalog(
            timeline,
            evaluation,
            output,
            shots,
            Arc::new(bundle_manifest),
            profile,
            &available,
        )
    }

    fn compose_from_catalog(
        timeline: &TimelineIr,
        evaluation: FrameInterval,
        output: FrameInterval,
        shots: Option<&BTreeSet<TimelineShotIndex>>,
        bundle_manifest: Arc<BundleManifest>,
        profile: RenderProfile,
        available: &BTreeMap<FrozenAssetId, MaterializedAsset>,
    ) -> Result<Self, InvalidRenderUnit> {
        let videos = render_videos(timeline, evaluation, shots, available)?;
        let source_timings = videos
            .iter()
            .map(|(id, video)| (*id, video.source_timing().clone()))
            .collect();
        let browser_plan = match shots {
            Some(shots) => BrowserPlan::from_timeline_for_region(
                timeline,
                &source_timings,
                evaluation,
                output,
                shots,
            ),
            None => {
                BrowserPlan::from_timeline_for_unit(timeline, &source_timings, evaluation, output)
            }
        }
        .map_err(InvalidRenderUnit::BrowserPlan)?;
        let audio = audio_plan(timeline, output, available)?;
        let visual_execution = VisualExecutionPlan::select(
            bundle_manifest.visual_capability(),
            bundle_manifest.frame_behavior(),
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
            self.bundle_manifest.as_ref().clone(),
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

    /// Returns the immutable presentation identity used by capture artifacts.
    #[must_use]
    pub fn bundle_id(&self) -> &str {
        self.bundle_manifest.bundle_id()
    }

    pub(crate) fn bundle_manifest(&self) -> &BundleManifest {
        self.bundle_manifest.as_ref()
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

    /// Narrows this normalized unit to one exact published frame.
    ///
    /// The operation consumes an already-composed unit so visual-path
    /// normalization, evaluation dependencies, presentation bytes, and media
    /// admission remain identical to the production sequence. Audio is omitted
    /// because a PNG snapshot has no audio output.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidRenderUnit::FrameOutsideOutput`] when `frame` is not
    /// published by this unit.
    pub fn into_frame(mut self, frame: FrameIndex) -> Result<Self, InvalidRenderUnit> {
        let output = self.browser_plan.output();
        if frame.get() < output.start().get() || frame.get() >= output.end().get() {
            return Err(InvalidRenderUnit::FrameOutsideOutput(frame));
        }
        let end = frame
            .checked_advance(FrameCount::new(1))
            .ok_or(InvalidRenderUnit::FrameOutsideOutput(frame))?;
        let interval = FrameInterval::new(frame, end)
            .map_err(|_| InvalidRenderUnit::FrameOutsideOutput(frame))?;

        self.browser_plan = self
            .browser_plan
            .into_output(interval)
            .map_err(InvalidRenderUnit::BrowserPlan)?;
        self.audio = AudioPlan::empty();
        Ok(self)
    }
}

fn normalize_visual_execution(units: &mut [RenderUnit]) {
    let Some(first) = units.first() else {
        return;
    };
    let capability = first.visual_execution.capability();
    let shares_native_path = capability != PresentationVisualCapability::BrowserComposite
        && units
            .iter()
            .all(|unit| unit.visual_execution.capability() == capability);
    if shares_native_path {
        return;
    }
    for unit in units {
        unit.visual_execution = VisualExecutionPlan::browser_composite(
            unit.bundle_manifest.frame_behavior(),
            &unit.browser_plan,
        );
    }
}

/// Reason solved and materialized facts cannot form one render unit.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum InvalidRenderUnit {
    /// Shot-scoped presentation bundles do not cover the partition plan.
    BundleCount,
    /// A browser artifact contains the wrong semantic DOM extent.
    DocumentScope {
        /// Scope required by the selected composition path.
        expected: PresentationDocumentScope,
        /// Scope declared by the immutable presentation artifact.
        actual: PresentationDocumentScope,
    },
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
    /// The audio plan would exceed the bounded process envelope.
    AudioTrackLimit,
    /// An audio placement escapes the solved film interval.
    AudioOutsideTimeline(FrozenAssetId),
    /// Materialized bytes do not contain the audio stream solved by core.
    MissingAudioStream(FrozenAssetId),
    /// A requested snapshot frame is not published by this render unit.
    FrameOutsideOutput(FrameIndex),
    /// A solved placement cannot be projected onto the source sample grid.
    AudioSampleConversion {
        /// Identity of the rejected audio artifact.
        id: FrozenAssetId,
        /// Exact conversion failure.
        source: AudioSampleConversionOverflow,
    },
    /// A timeline frame cannot cross the JavaScript wire boundary exactly.
    BrowserPlan(InvalidBrowserPlan),
    /// A declared native visual capability lacks the required frozen proof.
    VisualComposition(UnsupportedVisualComposition),
}

impl fmt::Display for InvalidRenderUnit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BundleCount => {
                formatter.write_str("presentation bundles do not match the partition plan")
            }
            Self::DocumentScope { expected, actual } => {
                write!(
                    formatter,
                    "presentation document scope is {actual}; expected {expected}"
                )
            }
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
            Self::AudioOutsideTimeline(id) => {
                write!(
                    formatter,
                    "audio placement {id} falls outside the solved Timeline"
                )
            }
            Self::MissingAudioStream(id) => {
                write!(formatter, "materialized audio {id} has no audio stream")
            }
            Self::FrameOutsideOutput(frame) => {
                write!(
                    formatter,
                    "frame {} lies outside this render unit's output",
                    frame.get()
                )
            }
            Self::AudioSampleConversion { id, source } => {
                write!(
                    formatter,
                    "materialized audio {id} exceeds the sample domain: {source}"
                )
            }
            Self::BrowserPlan(source) => source.fmt(formatter),
            Self::VisualComposition(source) => source.fmt(formatter),
        }
    }
}

impl Error for InvalidRenderUnit {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::UnsupportedVideo { source, .. } => Some(source),
            Self::AudioSampleConversion { source, .. } => Some(source),
            Self::BrowserPlan(source) => Some(source),
            Self::VisualComposition(source) => Some(source),
            Self::BundleCount
            | Self::DocumentScope { .. }
            | Self::DuplicateAsset(_)
            | Self::MissingAsset(_)
            | Self::AudioTrackLimit
            | Self::AudioOutsideTimeline(_)
            | Self::MissingAudioStream(_)
            | Self::FrameOutsideOutput(_) => None,
        }
    }
}

fn require_document_scope(
    manifest: &BundleManifest,
    expected: PresentationDocumentScope,
) -> Result<(), InvalidRenderUnit> {
    let actual = manifest.document_scope();
    if actual == expected {
        return Ok(());
    }
    Err(InvalidRenderUnit::DocumentScope { expected, actual })
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
    selected_shots: Option<&BTreeSet<TimelineShotIndex>>,
    available: &BTreeMap<FrozenAssetId, MaterializedAsset>,
) -> Result<BTreeMap<FrozenAssetId, RenderVideo>, InvalidRenderUnit> {
    let mut videos = BTreeMap::new();

    for (index, shot) in timeline.indexed_shots() {
        let selected = selected_shots.map_or_else(
            || shot.timing().interval().intersects(evaluation),
            |shots| shots.contains(&index),
        );
        if !selected {
            continue;
        }
        for content in shot.content() {
            let Some(video) = content.as_video() else {
                continue;
            };
            insert_render_video(video, available, &mut videos)?;
        }
    }

    Ok(videos)
}

fn insert_render_video(
    video: &onmark_core::timeline::TimelineVideo,
    available: &BTreeMap<FrozenAssetId, MaterializedAsset>,
    videos: &mut BTreeMap<FrozenAssetId, RenderVideo>,
) -> Result<(), InvalidRenderUnit> {
    let id = video.asset_id();
    if videos.contains_key(&id) {
        return Ok(());
    }
    let asset = available
        .get(&id)
        .cloned()
        .ok_or(InvalidRenderUnit::MissingAsset(id))?;
    let admitted = AdmittedVideo::admit(asset.frozen().metadata())
        .map_err(|source| InvalidRenderUnit::UnsupportedVideo { id, source })?;
    videos.insert(
        id,
        RenderVideo {
            source_timing: admitted.timing().clone(),
            dimensions: admitted.metadata().dimensions(),
            color_profile: admitted.metadata().color_profile(),
            asset,
        },
    );
    Ok(())
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
        envelope: audio.envelope(),
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
    use std::sync::Arc;

    use onmark_core::compiler;
    use onmark_core::model::{
        AssetMetadata, AssetRef, AudioChannelLayout, AudioGain, AudioSampleRate, Duration,
        FrameIndex, FrameRate, FrozenAsset, FrozenAssetId, MediaTimebase,
        PresentationDocumentScope, PresentationFrameBehavior, PresentationTemporalCapability,
        PresentationVisualCapability, SourceId, Timebase, VideoColorProfile, VideoDimensions,
        VideoFrameMap, VideoMetadata, VideoTiming,
    };
    use onmark_core::protocol::BundleFile;
    use onmark_core::render_graph::RenderGraph;
    use onmark_core::timeline::TimelineIr;

    use super::{
        BundleManifest, CaptureEnvironmentId, InvalidRenderUnit, MAX_AUDIO_TRACKS,
        MaterializedAsset, RenderAudio, RenderProfile, RenderUnit, VisualExecutionPlan,
        WorkerCaptureRequest,
    };
    use crate::{AlphaMode, BrowserCaptureCadence, WorkerCaptureVersion};

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
                .source_timing(),
            &VideoTiming::Constant(frame_rate()),
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

        assert_eq!(wire["version"], WorkerCaptureVersion::CURRENT.get());
        assert_eq!(wire["profile"]["alpha"], "opaque");
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
    fn invalidates_partitions_that_observe_complete_film_timing() {
        let before = concat!(
            "<om-film><om-scene>",
            r#"<om-shot duration="1s"><om-title>Opening</om-title></om-shot>"#,
            r#"<om-shot duration="1s"><om-title>Closing</om-title></om-shot>"#,
            "</om-scene></om-film>",
        );
        let after = concat!(
            "<om-film><om-scene>",
            r#"<om-shot duration="1s"><om-title>Opening</om-title></om-shot>"#,
            r#"<om-shot duration="2s"><om-title>Closing</om-title></om-shot>"#,
            "</om-scene></om-film>",
        );
        let before = partition_artifact_ids(before);
        let after = partition_artifact_ids(after);

        assert_ne!(before[0], after[0]);
        assert_ne!(before[1], after[1]);
    }

    #[test]
    fn scopes_media_identity_to_its_random_access_partition() {
        let before = concat!(
            "<om-film><om-scene>",
            r#"<om-shot><video src="opening.mp4"></video></om-shot>"#,
            r#"<om-shot><video src="closing.mp4"></video></om-shot>"#,
            "</om-scene></om-film>",
        );
        let after = concat!(
            "<om-film><om-scene>",
            r#"<om-shot><video src="opening.mp4"></video></om-shot>"#,
            r#"<om-shot><video src="replacement.mp4"></video></om-shot>"#,
            "</om-scene></om-film>",
        );
        let opening = video_asset_with_identity(1);
        let closing = video_asset_with_identity(2);
        let replacement = video_asset_with_identity(3);

        let before = video_partition_artifact_ids(
            before,
            [("opening.mp4", opening.clone()), ("closing.mp4", closing)],
        );
        let after = video_partition_artifact_ids(
            after,
            [("opening.mp4", opening), ("replacement.mp4", replacement)],
        );

        assert_eq!(before[0], after[0]);
        assert_ne!(before[1], after[1]);
    }

    #[test]
    fn scopes_source_edits_to_their_distributed_partition() {
        let before = concat!(
            "<om-film><om-scene>",
            r#"<om-shot><video src="opening.mp4"></video></om-shot>"#,
            r#"<om-shot><video src="closing.mp4" trim="..500ms"></video></om-shot>"#,
            "</om-scene></om-film>",
        );
        let after = concat!(
            "<om-film><om-scene>",
            r#"<om-shot><video src="opening.mp4"></video></om-shot>"#,
            r#"<om-shot><video src="closing.mp4" trim="500ms.."></video></om-shot>"#,
            "</om-scene></om-film>",
        );
        let opening = video_asset_with_identity(1);
        let closing = video_asset_with_identity(2);
        let before = video_partition_artifact_ids(
            before,
            [
                ("opening.mp4", opening.clone()),
                ("closing.mp4", closing.clone()),
            ],
        );
        let after = video_partition_artifact_ids(
            after,
            [("opening.mp4", opening), ("closing.mp4", closing)],
        );

        assert_eq!(before[0], after[0]);
        assert_ne!(before[1], after[1]);
    }

    #[test]
    fn excludes_native_audio_from_visual_artifact_identity() {
        let first = voice_over_unit("first.mp3", audio_asset(1));
        let second = voice_over_unit("other.mp3", audio_asset(2));

        assert_ne!(
            only_audio(&first).asset().id(),
            only_audio(&second).asset().id()
        );
        assert_eq!(artifact_id(&first), artifact_id(&second));
    }

    #[test]
    fn visual_execution_path_participates_in_artifact_identity() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let layered = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        )
        .expect("the fixture admits native layering");
        let mut browser = layered.clone();
        browser.visual_execution = VisualExecutionPlan::browser_composite(
            browser.bundle_manifest.frame_behavior(),
            &browser.browser_plan,
        );

        assert_ne!(artifact_id(&layered), artifact_id(&browser));
    }

    #[test]
    fn alpha_contract_participates_in_artifact_identity() {
        let timeline = solve_with_assets(
            r#"<om-film><om-scene><om-shot duration="1s"></om-shot></om-scene></om-film>"#,
            &BTreeMap::new(),
        );
        let opaque = RenderUnit::whole_film(&timeline, bundle_manifest(), render_profile(), [])
            .expect("the fixture forms one opaque render unit");
        let mut transparent = opaque.clone();
        transparent.profile = transparent.profile.with_alpha(AlphaMode::Preserve);

        assert_ne!(artifact_id(&opaque), artifact_id(&transparent));
    }

    #[test]
    fn composes_a_partition_into_its_own_browser_interval() {
        let frozen = video_asset(VideoTiming::Constant(frame_rate()));
        let timeline = solve(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot duration="1s"><om-title>Opening</om-title></om-shot>"#,
                r#"<om-shot duration="2s"><om-title>Closing</om-title></om-shot>"#,
                "</om-scene></om-film>",
            ),
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
    fn admits_complete_variable_timing_only_to_browser_composition() {
        let frozen = video_asset(variable_timing());
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");

        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest(),
            render_profile(),
            [materialized],
        )
        .expect("complete VFR timing remains browser-presentable");

        assert_eq!(
            unit.visual_execution().capability(),
            PresentationVisualCapability::BrowserComposite,
        );
        assert!(
            unit.browser_plan().videos()[0]
                .source_timing()
                .variable_boundaries()
                .is_some(),
        );
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
        assert_eq!(wire["visualExecution"]["captureCadence"], "everyFrame");
        assert_eq!(wire["visualExecution"]["width"], 320);
        assert_eq!(decoded, request);
        assert_eq!(
            decoded.visual_execution().capability(),
            PresentationVisualCapability::SeparableOverlay,
        );
        let mut invalid_cadence = wire.clone();
        invalid_cadence["visualExecution"]["captureCadence"] =
            serde_json::Value::from("placementBounded");
        assert!(serde_json::from_value::<WorkerCaptureRequest>(invalid_cadence).is_err());

        let mut invalid = wire;
        invalid["visualExecution"]["width"] = serde_json::Value::from(322);
        assert!(serde_json::from_value::<WorkerCaptureRequest>(invalid).is_err());
    }

    #[test]
    fn admits_static_browser_backdrop_layout_to_native_media() {
        let source_dimensions =
            VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive");
        let frozen = layered_video_asset(source_dimensions, true);
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");

        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableBackdrop),
            render_profile(),
            [materialized],
        )
        .expect("the frozen facts admit browser-measured native layout");
        let request = unit.worker_capture_request(capture_environment());
        let wire = serde_json::to_value(&request).expect("the backdrop request serializes");
        let decoded: WorkerCaptureRequest =
            serde_json::from_value(wire.clone()).expect("the backdrop request validates");

        assert_eq!(wire["visualExecution"]["mode"], "separableBackdrop");
        assert_eq!(wire["visualExecution"]["media"][0]["width"], 1_920);
        assert_eq!(decoded, request);
        assert!(unit.visual_execution().backdrop_media().is_some());
    }

    #[test]
    fn rejects_an_unproved_declared_backdrop_without_fallback() {
        let frozen = layered_video_asset(video_dimensions(), false);
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");

        let error = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableBackdrop),
            render_profile(),
            [materialized],
        )
        .expect_err("a strong authored capability cannot fall back after admission");

        assert_eq!(
            error,
            InvalidRenderUnit::VisualComposition(
                crate::UnsupportedVisualComposition::UnsupportedColorProfile,
            ),
        );
    }

    #[test]
    fn admits_placement_bounded_capture_for_layered_foreground() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let layered = RenderUnit::whole_film(
            &timeline,
            placement_bounded_manifest(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        )
        .expect("native video leaves placement-bounded foreground pixels");

        assert_eq!(
            layered.visual_execution().capture_cadence(),
            BrowserCaptureCadence::PlacementBounded,
        );
        let request = layered.worker_capture_request(CaptureEnvironmentId::from_sha256(
            [7; CaptureEnvironmentId::BYTE_LENGTH],
        ));
        let wire = serde_json::to_value(request).expect("the admitted cadence serializes");
        assert_eq!(
            wire["visualExecution"]["captureCadence"],
            "placementBounded",
        );
    }

    #[test]
    fn admits_placement_bounded_capture_for_static_browser_output() {
        let timeline = solve(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot duration="1s"><om-title>Static</om-title></om-shot>"#,
                "</om-scene></om-film>",
            ),
            "unused.mp4",
            video_asset(VideoTiming::Constant(frame_rate())),
        );
        let unit = RenderUnit::whole_film(
            &timeline,
            placement_bounded_manifest(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [],
        )
        .expect("static browser composition is placement-bounded");

        assert_eq!(
            unit.visual_execution().capture_cadence(),
            BrowserCaptureCadence::PlacementBounded,
        );
    }

    #[test]
    fn keeps_browser_video_on_per_frame_capture() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let unit = RenderUnit::whole_film(
            &timeline,
            placement_bounded_manifest(PresentationVisualCapability::BrowserComposite),
            render_profile(),
            [materialized],
        )
        .expect("browser video remains a supported conservative path");

        assert_eq!(
            unit.visual_execution().capture_cadence(),
            BrowserCaptureCadence::EveryFrame,
        );
    }

    #[test]
    fn admits_exact_source_edits_to_native_layering() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = solve(
            concat!(
                "<om-film><om-scene><om-shot>",
                r#"<video src="opening.mp4" trim="250ms..750ms" speed="2x"></video>"#,
                "</om-shot></om-scene></om-film>",
            ),
            "opening.mp4",
            frozen.clone(),
        );
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let unit = RenderUnit::whole_film(
            &timeline,
            placement_bounded_manifest(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        )
        .expect("exact source edits retain native layering");

        assert!(unit.visual_execution().layered_media().is_some());
        assert_eq!(
            unit.visual_execution().capture_cadence(),
            BrowserCaptureCadence::PlacementBounded,
        );
    }

    #[test]
    fn keeps_unproved_source_continuity_in_browser_composition() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = solve(
            concat!(
                "<om-film><om-scene><om-shot>",
                r#"<video src="opening.mp4" plays="2" hold-last="500ms"></video>"#,
                "</om-shot></om-scene></om-film>",
            ),
            "opening.mp4",
            frozen.clone(),
        );
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let unit = RenderUnit::whole_film(
            &timeline,
            placement_bounded_manifest(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        )
        .expect("source continuity retains the conservative browser path");

        assert!(unit.visual_execution().layered_media().is_none());
        assert_eq!(
            unit.visual_execution().capture_cadence(),
            BrowserCaptureCadence::EveryFrame,
        );
    }

    #[test]
    fn keeps_browser_composition_without_one_complete_primary_video() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = solve(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot><video src="opening.mp4"></video></om-shot>"#,
                r#"<om-shot duration="1s"></om-shot>"#,
                "</om-scene></om-film>",
            ),
            "opening.mp4",
            frozen.clone(),
        );
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        )
        .expect("the capable presentation retains its conservative path");

        assert!(unit.visual_execution().layered_media().is_none());
    }

    #[test]
    fn keeps_browser_composition_without_a_primary_video() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = solve(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot duration="1s"><om-title>Static</om-title></om-shot>"#,
                "</om-scene></om-film>",
            ),
            "unused.mp4",
            frozen,
        );
        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [],
        )
        .expect("the capable presentation retains its conservative path");

        assert!(unit.visual_execution().layered_media().is_none());
    }

    #[test]
    fn keeps_browser_composition_without_native_pixel_facts() {
        let mismatched = layered_video_asset(
            VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive"),
            true,
        );
        let missing_color = layered_video_asset(video_dimensions(), false);

        assert_browser_composition(mismatched);
        assert_browser_composition(missing_color);
    }

    #[test]
    fn selects_one_visual_path_for_the_partition_plan() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = solve(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot><video src="opening.mp4"></video></om-shot>"#,
                r#"<om-shot duration="1s"><om-title>Static</om-title></om-shot>"#,
                "</om-scene></om-film>",
            ),
            "opening.mp4",
            frozen.clone(),
        );
        let partitions =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .expect("the fixture has complete render ownership")
                .into_partition();
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");

        let units = RenderUnit::from_partition_plan(
            &timeline,
            &partitions,
            &bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        )
        .expect("the mixed sequence retains one conservative visual path");

        assert_eq!(units.len(), 2);
        assert!(Arc::ptr_eq(
            &units[0].bundle_manifest,
            &units[1].bundle_manifest,
        ));
        assert!(
            units
                .iter()
                .all(|unit| unit.visual_execution().layered_media().is_none())
        );
    }

    #[test]
    fn narrows_a_normalized_unit_without_replanning_its_visual_path() {
        let timeline = solve_with_assets(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot duration="1s"><om-title>Opening</om-title></om-shot>"#,
                "</om-scene></om-film>",
            ),
            &BTreeMap::new(),
        );
        let unit = RenderUnit::whole_film(&timeline, bundle_manifest(), render_profile(), [])
            .expect("the fixture forms one complete unit");
        let expected_visual = unit.visual_execution().clone();

        let frame = unit
            .into_frame(FrameIndex::new(7))
            .expect("the requested frame is published by the unit");

        assert_eq!(frame.browser_plan().output().start().get(), 7);
        assert_eq!(frame.browser_plan().output().end().get(), 8);
        assert_eq!(frame.visual_execution(), &expected_visual);
        assert_eq!(frame.audio_tracks().len(), 0);
    }

    #[test]
    fn rejects_a_frame_outside_the_existing_unit_output() {
        let timeline = solve_with_assets(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot duration="1s"></om-shot>"#,
                "</om-scene></om-film>",
            ),
            &BTreeMap::new(),
        );
        let unit = RenderUnit::whole_film(&timeline, bundle_manifest(), render_profile(), [])
            .expect("the fixture forms one complete unit");

        assert_eq!(
            unit.into_frame(FrameIndex::new(30)),
            Err(InvalidRenderUnit::FrameOutsideOutput(FrameIndex::new(30))),
        );
    }

    #[test]
    fn keeps_native_backdrop_beside_a_browser_only_partition() {
        let frozen = layered_video_asset(video_dimensions(), true);
        let timeline = solve(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot><video src="opening.mp4"></video></om-shot>"#,
                r#"<om-shot duration="1s"><om-title>Static</om-title></om-shot>"#,
                "</om-scene></om-film>",
            ),
            "opening.mp4",
            frozen.clone(),
        );
        let partitions =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .expect("the fixture has complete render ownership")
                .into_partition();
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");

        let units = RenderUnit::from_partition_plan(
            &timeline,
            &partitions,
            &bundle_manifest_with(PresentationVisualCapability::SeparableBackdrop),
            render_profile(),
            [materialized],
        )
        .expect("a browser-only partition does not invalidate native video elsewhere");

        assert_eq!(units.len(), 2);
        assert!(units[0].visual_execution().backdrop_media().is_some());
        assert_eq!(
            units[1].visual_execution().capability(),
            PresentationVisualCapability::SeparableBackdrop,
        );
        let encoded =
            serde_json::to_string(&units[1].worker_capture_request(capture_environment()))
                .expect("the browser-only backdrop request serializes");
        let decoded: WorkerCaptureRequest =
            serde_json::from_str(&encoded).expect("the backdrop capability survives transport");
        assert_eq!(
            decoded.visual_execution().capability(),
            PresentationVisualCapability::SeparableBackdrop,
        );
        assert_eq!(decoded.visual_execution().native_media_count(), 0);
    }

    #[test]
    fn rejects_whole_film_artifacts_at_the_region_bundle_boundary() {
        let timeline = solve_with_assets(
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot duration="1s"><om-title>Opening</om-title></om-shot>"#,
                "</om-scene></om-film>",
            ),
            &BTreeMap::new(),
        );
        let partitions =
            RenderGraph::from_timeline(&timeline, PresentationTemporalCapability::RandomAccess)
                .expect("the fixture has complete render ownership")
                .into_partition();
        let manifest = bundle_manifest_for(
            PresentationTemporalCapability::RandomAccess,
            PresentationVisualCapability::BrowserComposite,
            PresentationFrameBehavior::PerFrame,
        );

        let error = RenderUnit::from_partitioned_bundles(
            &timeline,
            &partitions,
            vec![manifest],
            render_profile(),
            [],
        )
        .expect_err("one whole-film DOM cannot stand in for a shot projection");

        assert_eq!(
            error,
            InvalidRenderUnit::DocumentScope {
                expected: PresentationDocumentScope::RenderRegion,
                actual: PresentationDocumentScope::WholeFilm,
            },
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
            concat!(
                "<om-film><om-scene><om-shot>",
                concat!(
                    r#"<om-vo src="voice.mp3" delay="500ms" "#,
                    r#"fade-in="250ms" fade-out="500ms">Read me</om-vo>"#,
                ),
                "</om-shot></om-scene></om-film>",
            ),
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
        assert_eq!(audio.envelope().fade_in().get(), 8);
        assert_eq!(audio.envelope().fade_out().get(), 15);
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
            concat!(
                "<om-film><om-scene>",
                r#"<om-shot duration="1s"></om-shot>"#,
                r#"<om-shot><om-vo src="voice.mp3">Read me</om-vo></om-shot>"#,
                "</om-scene></om-film>",
            ),
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
            "<om-film><om-scene><om-shot>{}</om-shot></om-scene></om-film>",
            r#"<om-vo src="voice.mp3"></om-vo>"#.repeat(MAX_AUDIO_TRACKS + 1)
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
            concat!(
                "<om-film><om-scene><om-shot>",
                r#"<video src="opening.mp4"></video>"#,
                "</om-shot></om-scene></om-film>",
            ),
            "opening.mp4",
            frozen,
        )
    }

    fn audio_sample_rate() -> AudioSampleRate {
        AudioSampleRate::new(48_000).expect("48 kHz is valid")
    }

    fn audio_asset(identity: u8) -> FrozenAsset {
        FrozenAsset::new(
            FrozenAssetId::from_sha256([identity; 32]),
            AssetMetadata::audio(
                Duration::from_nanos(1_000_000_000),
                audio_sample_rate(),
                AudioChannelLayout::Mono,
            ),
        )
    }

    fn voice_over_unit(source: &str, frozen: FrozenAsset) -> RenderUnit {
        let screenplay = format!(
            r#"<om-film>
  <om-scene>
    <om-shot><om-vo src="{source}">Read me</om-vo></om-shot>
  </om-scene>
</om-film>"#
        );
        let timeline = solve(&screenplay, source, frozen.clone());
        let materialized = MaterializedAsset::new(frozen, format!("/tmp/{source}"))
            .expect("the fixture path is present");
        RenderUnit::whole_film(
            &timeline,
            bundle_manifest(),
            render_profile(),
            [materialized],
        )
        .expect("voice-over forms one visual capture contract")
    }

    fn only_audio(unit: &RenderUnit) -> &RenderAudio {
        let mut audio = unit.audio_tracks();
        let track = audio.next().expect("the fixture owns one audio track");
        assert!(audio.next().is_none());
        track
    }

    fn video_asset(timing: VideoTiming) -> FrozenAsset {
        video_asset_with(
            1,
            timing,
            VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive"),
            None,
        )
    }

    fn variable_timing() -> VideoTiming {
        let timebase =
            MediaTimebase::new(1, 1_000).expect("one millisecond ticks form a valid timebase");
        let frames = VideoFrameMap::new(timebase, [0, 400, 1_000])
            .expect("the fixture has two variable frame intervals");
        VideoTiming::Variable(frames)
    }

    fn video_asset_with_identity(identity: u8) -> FrozenAsset {
        video_asset_with(
            identity,
            VideoTiming::Constant(frame_rate()),
            VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive"),
            None,
        )
    }

    fn layered_video_asset(dimensions: VideoDimensions, color: bool) -> FrozenAsset {
        let color_profile = color.then_some(VideoColorProfile::Bt709Limited);
        video_asset_with(
            1,
            VideoTiming::Constant(frame_rate()),
            dimensions,
            color_profile,
        )
    }

    fn video_asset_with(
        identity: u8,
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
            FrozenAssetId::from_sha256([identity; 32]),
            AssetMetadata::video(duration, metadata),
        )
    }

    fn solve(source: &str, asset: &str, frozen: FrozenAsset) -> TimelineIr {
        let asset = AssetRef::parse(asset).expect("the fixture asset reference is valid");
        let assets = BTreeMap::from([(asset, frozen)]);
        solve_with_assets(source, &assets)
    }

    fn solve_with_assets(source: &str, assets: &BTreeMap<AssetRef, FrozenAsset>) -> TimelineIr {
        let (document, diagnostics) = compiler::parse(SourceId::new(0), source).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::bind(document).into_parts();
        assert!(diagnostics.is_empty());
        let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
        assert!(diagnostics.is_empty());
        let report = compiler::solve(
            film.expect("the fixture resolves"),
            assets,
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
        bundle_manifest_for(
            PresentationTemporalCapability::Sequential,
            visual_capability,
            PresentationFrameBehavior::PerFrame,
        )
    }

    fn placement_bounded_manifest(
        visual_capability: PresentationVisualCapability,
    ) -> BundleManifest {
        bundle_manifest_for(
            PresentationTemporalCapability::RandomAccess,
            visual_capability,
            PresentationFrameBehavior::PlacementBounded,
        )
    }

    fn bundle_manifest_for(
        temporal_capability: PresentationTemporalCapability,
        visual_capability: PresentationVisualCapability,
        frame_behavior: PresentationFrameBehavior,
    ) -> BundleManifest {
        const DIGEST: &str =
            "sha256:0101010101010101010101010101010101010101010101010101010101010101";
        let entry = BundleFile::new(BundleManifest::ENTRY_POINT, 1, DIGEST)
            .expect("the fixture entry is valid");
        BundleManifest::new(
            PresentationDocumentScope::WholeFilm,
            temporal_capability,
            visual_capability,
            frame_behavior,
            DIGEST,
            vec![entry],
        )
        .expect("the fixture manifest is valid")
    }

    fn partition_artifact_ids(source: &str) -> Vec<crate::FrameArtifactId> {
        let timeline = solve_with_assets(source, &BTreeMap::new());
        random_access_artifact_ids(&timeline, [])
    }

    fn video_partition_artifact_ids<const N: usize>(
        source: &str,
        assets: [(&str, FrozenAsset); N],
    ) -> Vec<crate::FrameArtifactId> {
        let catalog: BTreeMap<_, _> = assets
            .iter()
            .map(|(reference, asset)| {
                let reference =
                    AssetRef::parse(*reference).expect("the fixture asset reference is valid");
                (reference, asset.clone())
            })
            .collect();
        let timeline = solve_with_assets(source, &catalog);
        let materialized = assets.map(|(reference, asset)| {
            MaterializedAsset::new(asset, format!("/tmp/{reference}"))
                .expect("the fixture path is present")
        });
        random_access_artifact_ids(&timeline, materialized)
    }

    fn random_access_artifact_ids(
        timeline: &TimelineIr,
        assets: impl IntoIterator<Item = MaterializedAsset>,
    ) -> Vec<crate::FrameArtifactId> {
        let manifest = bundle_manifest_for(
            PresentationTemporalCapability::RandomAccess,
            PresentationVisualCapability::BrowserComposite,
            PresentationFrameBehavior::PerFrame,
        );
        let partitions = RenderGraph::from_timeline(timeline, manifest.temporal_capability())
            .expect("the fixture has complete render ownership")
            .into_partition();

        RenderUnit::from_partition_plan(timeline, &partitions, &manifest, render_profile(), assets)
            .expect("each fixture shot forms one render unit")
            .into_iter()
            .map(|unit| artifact_id(&unit))
            .collect()
    }

    fn artifact_id(unit: &RenderUnit) -> crate::FrameArtifactId {
        unit.worker_capture_request(capture_environment())
            .artifact_id()
    }

    fn capture_environment() -> CaptureEnvironmentId {
        CaptureEnvironmentId::from_sha256([7; CaptureEnvironmentId::BYTE_LENGTH])
    }

    fn assert_browser_composition(frozen: FrozenAsset) {
        let timeline = video_timeline(frozen.clone());
        let materialized = MaterializedAsset::new(frozen, "/tmp/opening.mp4")
            .expect("the fixture path is present");
        let unit = RenderUnit::whole_film(
            &timeline,
            bundle_manifest_with(PresentationVisualCapability::SeparableOverlay),
            render_profile(),
            [materialized],
        )
        .expect("the capable presentation retains its conservative path");

        assert!(unit.visual_execution().layered_media().is_none());
    }
}
