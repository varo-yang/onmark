//! Versioned frame facts produced by the pure compiler.
//!
//! Timeline values depend only on foundational model types. They contain no
//! browser, codec, partition, or execution-plan decisions.

use std::collections::BTreeMap;

use crate::model::{
    AudioGain, CueId, ElementKind, EventRef, FrameIndex, FrameInterval, FrozenAssetId, NodeId,
    SourceSpan, Timebase,
};

/// Version of the Timeline IR contract.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TimelineVersion(u16);

impl TimelineVersion {
    /// First Timeline IR version implemented by Gate one.
    pub const V1: Self = Self(1);

    /// Returns the stable integer representation.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Exact frame facts for one compiled film.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineIr {
    version: TimelineVersion,
    timebase: Timebase,
    element: TimelineElement,
    interval: FrameInterval,
    events: BTreeMap<CueId, TimelineEvent>,
    scenes: Vec<TimelineScene>,
    general_audio: Vec<TimelineAudio>,
    captions: Vec<TimelineCaption>,
}

impl TimelineIr {
    pub(crate) const fn new(
        timebase: Timebase,
        element: TimelineElement,
        interval: FrameInterval,
        events: BTreeMap<CueId, TimelineEvent>,
        scenes: Vec<TimelineScene>,
        general_audio: Vec<TimelineAudio>,
        captions: Vec<TimelineCaption>,
    ) -> Self {
        Self {
            version: TimelineVersion::V1,
            timebase,
            element,
            interval,
            events,
            scenes,
            general_audio,
            captions,
        }
    }

    /// Returns the versioned Timeline IR contract identifier.
    #[must_use]
    pub const fn version(&self) -> TimelineVersion {
        self.version
    }

    /// Returns the frame grid used by every interval in this timeline.
    #[must_use]
    pub const fn timebase(&self) -> Timebase {
        self.timebase
    }

    /// Returns the film identity and source facts.
    #[must_use]
    pub const fn element(&self) -> &TimelineElement {
        &self.element
    }

    /// Returns the half-open interval occupied by the film.
    #[must_use]
    pub const fn interval(&self) -> FrameInterval {
        self.interval
    }

    /// Returns named events in deterministic cue-ID order.
    #[must_use]
    pub fn events(&self) -> impl ExactSizeIterator<Item = (&CueId, &TimelineEvent)> {
        self.events.iter()
    }

    /// Returns sequential scenes in authored order.
    #[must_use]
    pub fn scenes(&self) -> &[TimelineScene] {
        &self.scenes
    }

    /// Returns shots in screenplay order without exposing the scene walk.
    pub fn shots(&self) -> impl Iterator<Item = &TimelineShot> {
        self.scenes.iter().flat_map(|scene| &scene.shots)
    }

    /// Returns primary videos in screenplay order without exposing tree walks.
    pub fn videos(&self) -> impl Iterator<Item = &TimelineVideo> {
        self.contents().filter_map(TimelineContent::as_video)
    }

    /// Returns voice-over tracks in screenplay order.
    pub fn voice_overs(&self) -> impl Iterator<Item = &TimelineVoiceOver> {
        self.contents().filter_map(TimelineContent::as_voice_over)
    }

    /// Returns every executable audio placement in canonical mix order.
    ///
    /// Narrative tracks retain screenplay order. Film music follows in authored
    /// order, then shot effects follow in screenplay order. This fixed grouping
    /// keeps floating-point mixing order deterministic without erasing their
    /// distinct semantics. Empty placements remain available through their
    /// narrative nodes but require neither a render dependency nor an `FFmpeg`
    /// input.
    pub fn audio(&self) -> impl Iterator<Item = &TimelineAudio> {
        self.voice_overs()
            .map(TimelineVoiceOver::audio)
            .chain(&self.general_audio)
            .filter(|audio| !audio.timing().interval().is_empty())
    }

    /// Returns film music and shot-local effects in canonical mix order.
    #[must_use]
    pub fn general_audio(&self) -> &[TimelineAudio] {
        &self.general_audio
    }

    /// Returns title and call-to-action overlays in screenplay order.
    pub fn overlays(&self) -> impl Iterator<Item = &TimelineOverlay> {
        self.contents().filter_map(TimelineContent::as_overlay)
    }

    /// Returns imported captions in track and authored cue order.
    #[must_use]
    pub fn captions(&self) -> &[TimelineCaption] {
        &self.captions
    }

    pub(crate) fn replace_captions(&mut self, captions: Vec<TimelineCaption>) {
        self.captions = captions;
    }

    fn contents(&self) -> impl Iterator<Item = &TimelineContent> {
        self.shots().flat_map(|shot| &shot.content)
    }
}

/// One imported caption projected onto the solved frame grid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineCaption {
    interval: FrameInterval,
    text: Box<str>,
    timing_span: SourceSpan,
    text_span: SourceSpan,
}

impl TimelineCaption {
    pub(crate) fn new(
        interval: FrameInterval,
        text: impl Into<Box<str>>,
        timing_span: SourceSpan,
        text_span: SourceSpan,
    ) -> Self {
        Self {
            interval,
            text: text.into(),
            timing_span,
            text_span,
        }
    }

    /// Returns the executable half-open frame interval.
    #[must_use]
    pub const fn interval(&self) -> FrameInterval {
        self.interval
    }

    /// Returns normalized authored caption text.
    #[must_use]
    pub const fn text(&self) -> &str {
        &self.text
    }

    /// Returns the external-source range containing the timing expression.
    #[must_use]
    pub const fn timing_span(&self) -> SourceSpan {
        self.timing_span
    }

    /// Returns the external-source range containing the caption payload.
    #[must_use]
    pub const fn text_span(&self) -> SourceSpan {
        self.text_span
    }
}

/// One absolute named event retained for timing provenance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimelineEvent {
    at: FrameIndex,
    authored_at: SourceSpan,
}

impl TimelineEvent {
    pub(crate) const fn new(at: FrameIndex, authored_at: SourceSpan) -> Self {
        Self { at, authored_at }
    }

    /// Returns the absolute frame selected for the event.
    #[must_use]
    pub const fn at(self) -> FrameIndex {
        self.at
    }

    /// Returns the authored value span that produced the event.
    #[must_use]
    pub const fn authored_at(self) -> SourceSpan {
        self.authored_at
    }
}

/// Source identity retained by every Timeline IR element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineElement {
    kind: ElementKind,
    id: Option<NodeId>,
    span: SourceSpan,
}

impl TimelineElement {
    pub(crate) const fn new(kind: ElementKind, id: Option<NodeId>, span: SourceSpan) -> Self {
        Self { kind, id, span }
    }

    /// Returns the closed screenplay element kind.
    #[must_use]
    pub const fn kind(&self) -> ElementKind {
        self.kind
    }

    /// Returns the optional film-wide identity.
    #[must_use]
    pub const fn id(&self) -> Option<&NodeId> {
        self.id.as_ref()
    }

    /// Returns the authored element span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }
}

/// One sequential scene with solved frame bounds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineScene {
    element: TimelineElement,
    timing: TimelineTiming,
    shots: Vec<TimelineShot>,
}

impl TimelineScene {
    pub(crate) const fn new(
        element: TimelineElement,
        timing: TimelineTiming,
        shots: Vec<TimelineShot>,
    ) -> Self {
        Self {
            element,
            timing,
            shots,
        }
    }

    /// Returns the scene identity and source facts.
    #[must_use]
    pub const fn element(&self) -> &TimelineElement {
        &self.element
    }

    /// Returns the solved scene timing.
    #[must_use]
    pub const fn timing(&self) -> &TimelineTiming {
        &self.timing
    }

    /// Returns sequential shots in authored order.
    #[must_use]
    pub fn shots(&self) -> &[TimelineShot] {
        &self.shots
    }
}

/// One sequential shot with solved content bounds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineShot {
    element: TimelineElement,
    timing: TimelineTiming,
    content: Vec<TimelineContent>,
}

impl TimelineShot {
    pub(crate) const fn new(
        element: TimelineElement,
        timing: TimelineTiming,
        content: Vec<TimelineContent>,
    ) -> Self {
        Self {
            element,
            timing,
            content,
        }
    }

    /// Returns the shot identity and source facts.
    #[must_use]
    pub const fn element(&self) -> &TimelineElement {
        &self.element
    }

    /// Returns the solved shot timing.
    #[must_use]
    pub const fn timing(&self) -> &TimelineTiming {
        &self.timing
    }

    /// Returns shot content in authored order.
    #[must_use]
    pub fn content(&self) -> &[TimelineContent] {
        &self.content
    }
}

/// Closed narrative and visual content after all frame bounds are solved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimelineContent {
    /// Primary video content.
    Video(TimelineVideo),
    /// Voice-over audio and its authored inscription.
    VoiceOver(TimelineVoiceOver),
    /// A title or call-to-action overlay.
    Overlay(TimelineOverlay),
}

impl TimelineContent {
    fn as_video(&self) -> Option<&TimelineVideo> {
        match self {
            Self::Video(video) => Some(video),
            Self::VoiceOver(_) | Self::Overlay(_) => None,
        }
    }

    fn as_voice_over(&self) -> Option<&TimelineVoiceOver> {
        match self {
            Self::VoiceOver(voice_over) => Some(voice_over),
            Self::Video(_) | Self::Overlay(_) => None,
        }
    }

    fn as_overlay(&self) -> Option<&TimelineOverlay> {
        match self {
            Self::Overlay(overlay) => Some(overlay),
            Self::Video(_) | Self::VoiceOver(_) => None,
        }
    }
}

/// Solved primary video content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineVideo {
    element: TimelineElement,
    timing: TimelineTiming,
    asset_id: FrozenAssetId,
}

impl TimelineVideo {
    pub(crate) const fn new(
        element: TimelineElement,
        timing: TimelineTiming,
        asset_id: FrozenAssetId,
    ) -> Self {
        Self {
            element,
            timing,
            asset_id,
        }
    }

    /// Returns the media identity and source facts.
    #[must_use]
    pub const fn element(&self) -> &TimelineElement {
        &self.element
    }

    /// Returns the solved media timing.
    #[must_use]
    pub const fn timing(&self) -> &TimelineTiming {
        &self.timing
    }

    /// Returns the frozen media artifact identity.
    #[must_use]
    pub const fn asset_id(&self) -> FrozenAssetId {
        self.asset_id
    }
}

/// Solved voice-over content and inscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineVoiceOver {
    element: TimelineElement,
    audio: TimelineAudio,
    text: Vec<TimelineText>,
}

impl TimelineVoiceOver {
    pub(crate) const fn new(
        element: TimelineElement,
        timing: TimelineTiming,
        asset_id: FrozenAssetId,
        text: Vec<TimelineText>,
    ) -> Self {
        let authored_at = element.span();
        Self {
            element,
            audio: TimelineAudio::new(
                authored_at,
                timing,
                asset_id,
                AudioGain::UNITY,
                TimelineAudioKind::VoiceOver,
            ),
            text,
        }
    }

    /// Returns the voice-over identity and source facts.
    #[must_use]
    pub const fn element(&self) -> &TimelineElement {
        &self.element
    }

    /// Returns the solved voice-over timing.
    #[must_use]
    pub const fn timing(&self) -> &TimelineTiming {
        self.audio.timing()
    }

    /// Returns the frozen voice-over artifact identity.
    #[must_use]
    pub const fn asset_id(&self) -> FrozenAssetId {
        self.audio.asset_id()
    }

    /// Returns the shared executable audio placement.
    #[must_use]
    pub const fn audio(&self) -> &TimelineAudio {
        &self.audio
    }

    /// Returns authored inscription runs in source order.
    #[must_use]
    pub fn text(&self) -> &[TimelineText] {
        &self.text
    }
}

/// Semantic role retained by one solved audio placement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TimelineAudioKind {
    /// Spoken narrative that remains attached to authored voice-over text.
    VoiceOver,
    /// General musical content.
    Music,
    /// General authored sound effect.
    SoundEffect,
}

/// Exact executable facts shared by narrative and general audio.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineAudio {
    authored_at: SourceSpan,
    timing: TimelineTiming,
    asset_id: FrozenAssetId,
    gain: AudioGain,
    kind: TimelineAudioKind,
}

impl TimelineAudio {
    pub(crate) const fn new(
        authored_at: SourceSpan,
        timing: TimelineTiming,
        asset_id: FrozenAssetId,
        gain: AudioGain,
        kind: TimelineAudioKind,
    ) -> Self {
        Self {
            authored_at,
            timing,
            asset_id,
            gain,
            kind,
        }
    }

    /// Returns the source span that authored this placement.
    #[must_use]
    pub const fn authored_at(&self) -> SourceSpan {
        self.authored_at
    }

    /// Returns the exact Timeline placement.
    #[must_use]
    pub const fn timing(&self) -> &TimelineTiming {
        &self.timing
    }

    /// Returns the frozen source artifact identity.
    #[must_use]
    pub const fn asset_id(&self) -> FrozenAssetId {
        self.asset_id
    }

    /// Returns the exact linear amplitude.
    #[must_use]
    pub const fn gain(&self) -> AudioGain {
        self.gain
    }

    /// Returns the retained narrative or general-audio role.
    #[must_use]
    pub const fn kind(&self) -> TimelineAudioKind {
        self.kind
    }
}

/// Solved overlay active until its owning shot ends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineOverlay {
    element: TimelineElement,
    timing: TimelineTiming,
    text: Vec<TimelineText>,
}

impl TimelineOverlay {
    pub(crate) const fn new(
        element: TimelineElement,
        timing: TimelineTiming,
        text: Vec<TimelineText>,
    ) -> Self {
        Self {
            element,
            timing,
            text,
        }
    }

    /// Returns the overlay identity and source facts.
    #[must_use]
    pub const fn element(&self) -> &TimelineElement {
        &self.element
    }

    /// Returns the solved overlay timing.
    #[must_use]
    pub const fn timing(&self) -> &TimelineTiming {
        &self.timing
    }

    /// Returns authored overlay text runs in source order.
    #[must_use]
    pub fn text(&self) -> &[TimelineText] {
        &self.text
    }
}

/// One decoded authored text run retained by Timeline IR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineText {
    text: Box<str>,
    span: SourceSpan,
}

impl TimelineText {
    pub(crate) const fn new(text: Box<str>, span: SourceSpan) -> Self {
        Self { text, span }
    }

    /// Returns decoded authored text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the authored text span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }
}

/// One solved interval and the reasons for both boundaries.
///
/// These boundaries are compiler facts. They do not introduce authored
/// `start`, `end`, or `begin` attributes into the screenplay language.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineTiming {
    interval: FrameInterval,
    start_reason: TimingReason,
    end_reason: TimingReason,
}

impl TimelineTiming {
    pub(crate) const fn new(
        interval: FrameInterval,
        start_reason: TimingReason,
        end_reason: TimingReason,
    ) -> Self {
        Self {
            interval,
            start_reason,
            end_reason,
        }
    }

    /// Returns the solved half-open frame interval.
    #[must_use]
    pub const fn interval(&self) -> FrameInterval {
        self.interval
    }

    /// Returns why the interval starts at its selected frame.
    #[must_use]
    pub const fn start_reason(&self) -> &TimingReason {
        &self.start_reason
    }

    /// Returns why the interval ends at its selected frame.
    #[must_use]
    pub const fn end_reason(&self) -> &TimingReason {
        &self.end_reason
    }
}

/// Provenance for one solved frame boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TimingReason {
    /// The film's fixed zero frame.
    FilmStart,
    /// The end of the preceding sequential element.
    Sequential,
    /// The owning shot's start frame.
    ShotStart,
    /// An authored delay from the owning shot.
    AuthoredDelay(SourceSpan),
    /// An authored named event.
    Event {
        /// Resolved event identity.
        event: EventRef,
        /// Span of the authored reference.
        authored_at: SourceSpan,
    },
    /// An authored explicit duration.
    ExplicitDuration(SourceSpan),
    /// Duration measured from a frozen media artifact.
    AssetDuration,
    /// The primary content whose end determines its owning shot.
    LongestContent(SourceSpan),
    /// Bounds derived from sequential children.
    Children,
    /// The owning shot's exclusive end frame.
    ShotEnd,
    /// The solved film's exclusive end frame.
    FilmEnd,
}
