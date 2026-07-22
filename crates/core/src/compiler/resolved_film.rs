//! Typed authored intent between resolution and exact timeline solving.
//!
//! Syntax attributes no longer cross this boundary; values retain only the
//! spans needed for later timing diagnostics.

use std::collections::BTreeMap;

use crate::model::{
    AssetRef, AudioGain, CueId, Duration, ElementKind, EventRef, GeneralAudioKind, NodeId,
    SourceSpan,
};

/// One typed value together with the authored bytes that produced it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Authored<T> {
    value: T,
    span: SourceSpan,
}

impl<T> Authored<T> {
    pub(super) const fn new(value: T, span: SourceSpan) -> Self {
        Self { value, span }
    }

    /// Returns the typed value produced from the authored bytes.
    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    /// Returns the source span of the authored value.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(super) fn into_parts(self) -> (T, SourceSpan) {
        (self.value, self.span)
    }
}

/// Shared facts retained after every authored attribute is resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedElement {
    kind: ElementKind,
    id: Option<NodeId>,
    span: SourceSpan,
}

impl ResolvedElement {
    pub(super) const fn new(kind: ElementKind, id: Option<NodeId>, span: SourceSpan) -> Self {
        Self { kind, id, span }
    }

    /// Returns the recognized screenplay element kind.
    #[must_use]
    pub const fn kind(&self) -> ElementKind {
        self.kind
    }

    /// Returns the valid film-wide ID when one was authored.
    #[must_use]
    pub const fn id(&self) -> Option<&NodeId> {
        self.id.as_ref()
    }

    /// Returns the complete authored element span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(super) fn into_parts(self) -> (ElementKind, Option<NodeId>, SourceSpan) {
        (self.kind, self.id, self.span)
    }
}

/// One entry in the resolved film-wide ID index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedNode {
    kind: ElementKind,
    declared_at: SourceSpan,
}

impl ResolvedNode {
    pub(super) const fn new(kind: ElementKind, declared_at: SourceSpan) -> Self {
        Self { kind, declared_at }
    }

    /// Returns the kind of the indexed element.
    #[must_use]
    pub const fn kind(&self) -> ElementKind {
        self.kind
    }

    /// Returns the source span that declares the indexed ID.
    #[must_use]
    pub const fn declared_at(&self) -> SourceSpan {
        self.declared_at
    }
}

/// A film whose attributes and references are typed compiler facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedFilm {
    element: ResolvedElement,
    cues: Option<ResolvedCues>,
    music: Vec<ResolvedAudio>,
    scenes: Vec<ResolvedScene>,
    ids: BTreeMap<NodeId, ResolvedNode>,
}

impl ResolvedFilm {
    pub(super) fn new(
        element: ResolvedElement,
        cues: Option<ResolvedCues>,
        music: Vec<ResolvedAudio>,
        scenes: Vec<ResolvedScene>,
        ids: BTreeMap<NodeId, ResolvedNode>,
    ) -> Self {
        Self {
            element,
            cues,
            music,
            scenes,
            ids,
        }
    }

    /// Returns the resolved film element.
    #[must_use]
    pub const fn element(&self) -> &ResolvedElement {
        &self.element
    }

    /// Returns the optional singleton cue container.
    #[must_use]
    pub const fn cues(&self) -> Option<&ResolvedCues> {
        self.cues.as_ref()
    }

    /// Returns resolved film-wide music in authored order.
    #[must_use]
    pub fn music(&self) -> &[ResolvedAudio] {
        &self.music
    }

    /// Returns sequential scenes in authored order.
    #[must_use]
    pub fn scenes(&self) -> &[ResolvedScene] {
        &self.scenes
    }

    /// Returns the deterministic film-wide ID index.
    #[must_use]
    pub fn ids(&self) -> impl ExactSizeIterator<Item = (&NodeId, &ResolvedNode)> {
        self.ids.iter()
    }

    pub(super) fn into_parts(self) -> ResolvedFilmParts {
        ResolvedFilmParts {
            element: self.element,
            cues: self.cues,
            music: self.music,
            scenes: self.scenes,
        }
    }
}

/// Consuming handoff from attribute resolution into timeline solving.
pub(super) struct ResolvedFilmParts {
    pub(super) element: ResolvedElement,
    pub(super) cues: Option<ResolvedCues>,
    pub(super) music: Vec<ResolvedAudio>,
    pub(super) scenes: Vec<ResolvedScene>,
}

/// The optional singleton cue container after cue resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCues {
    element: ResolvedElement,
    cues: Vec<ResolvedCue>,
}

impl ResolvedCues {
    pub(super) const fn new(element: ResolvedElement, cues: Vec<ResolvedCue>) -> Self {
        Self { element, cues }
    }

    /// Returns the resolved cue-container element.
    #[must_use]
    pub const fn element(&self) -> &ResolvedElement {
        &self.element
    }

    /// Returns resolved cue declarations in authored order.
    #[must_use]
    pub fn cues(&self) -> &[ResolvedCue] {
        &self.cues
    }

    pub(super) fn into_parts(self) -> (ResolvedElement, Vec<ResolvedCue>) {
        (self.element, self.cues)
    }
}

/// One named absolute film-time event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedCue {
    id: Authored<CueId>,
    time: Authored<Duration>,
    span: SourceSpan,
}

impl ResolvedCue {
    pub(super) const fn new(
        id: Authored<CueId>,
        time: Authored<Duration>,
        span: SourceSpan,
    ) -> Self {
        Self { id, time, span }
    }

    /// Returns the typed cue ID and its authored span.
    #[must_use]
    pub const fn id(&self) -> &Authored<CueId> {
        &self.id
    }

    /// Returns the absolute cue time and its authored span.
    #[must_use]
    pub const fn time(&self) -> &Authored<Duration> {
        &self.time
    }

    /// Returns the complete authored cue span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(super) fn into_parts(self) -> (Authored<CueId>, Authored<Duration>, SourceSpan) {
        (self.id, self.time, self.span)
    }
}

/// One resolved sequential scene.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedScene {
    element: ResolvedElement,
    shots: Vec<ResolvedShot>,
}

impl ResolvedScene {
    pub(super) const fn new(element: ResolvedElement, shots: Vec<ResolvedShot>) -> Self {
        Self { element, shots }
    }

    /// Returns the resolved scene element.
    #[must_use]
    pub const fn element(&self) -> &ResolvedElement {
        &self.element
    }

    /// Returns sequential shots in authored order.
    #[must_use]
    pub fn shots(&self) -> &[ResolvedShot] {
        &self.shots
    }

    pub(super) fn into_parts(self) -> (ResolvedElement, Vec<ResolvedShot>) {
        (self.element, self.shots)
    }
}

/// One resolved sequential shot and its typed content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedShot {
    element: ResolvedElement,
    duration: Option<Authored<Duration>>,
    content: Vec<ResolvedShotContent>,
    sound_effects: Vec<ResolvedAudio>,
}

impl ResolvedShot {
    pub(super) const fn new(
        element: ResolvedElement,
        duration: Option<Authored<Duration>>,
        content: Vec<ResolvedShotContent>,
        sound_effects: Vec<ResolvedAudio>,
    ) -> Self {
        Self {
            element,
            duration,
            content,
            sound_effects,
        }
    }

    /// Returns the resolved shot element.
    #[must_use]
    pub const fn element(&self) -> &ResolvedElement {
        &self.element
    }

    /// Returns the optional authored shot duration.
    #[must_use]
    pub const fn duration(&self) -> Option<&Authored<Duration>> {
        self.duration.as_ref()
    }

    /// Returns shot content in authored order.
    #[must_use]
    pub fn content(&self) -> &[ResolvedShotContent] {
        &self.content
    }

    /// Returns resolved shot-local sound effects in authored order.
    #[must_use]
    pub fn sound_effects(&self) -> &[ResolvedAudio] {
        &self.sound_effects
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        ResolvedElement,
        Option<Authored<Duration>>,
        Vec<ResolvedShotContent>,
        Vec<ResolvedAudio>,
    ) {
        (
            self.element,
            self.duration,
            self.content,
            self.sound_effects,
        )
    }
}

/// Typed music or sound-effect intent before frame placement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedAudio {
    kind: GeneralAudioKind,
    element: ResolvedElement,
    src: Authored<AssetRef>,
    delay: Option<Authored<Duration>>,
    gain: AudioGain,
}

impl ResolvedAudio {
    pub(super) const fn new(
        kind: GeneralAudioKind,
        element: ResolvedElement,
        src: Authored<AssetRef>,
        delay: Option<Authored<Duration>>,
        gain: AudioGain,
    ) -> Self {
        Self {
            kind,
            element,
            src,
            delay,
            gain,
        }
    }

    /// Returns the resolved music or sound-effect element.
    #[must_use]
    pub const fn element(&self) -> &ResolvedElement {
        &self.element
    }

    /// Returns the required authored audio source.
    #[must_use]
    pub const fn src(&self) -> &Authored<AssetRef> {
        &self.src
    }

    /// Returns the optional shot-local delay.
    #[must_use]
    pub const fn delay(&self) -> Option<&Authored<Duration>> {
        self.delay.as_ref()
    }

    /// Returns the exact authored or default linear gain.
    #[must_use]
    pub const fn gain(&self) -> AudioGain {
        self.gain
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        GeneralAudioKind,
        ResolvedElement,
        Authored<AssetRef>,
        Option<Authored<Duration>>,
        AudioGain,
    ) {
        (self.kind, self.element, self.src, self.delay, self.gain)
    }
}

/// Closed narrative and visual content owned by a resolved shot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolvedShotContent {
    /// Primary video content.
    Video(ResolvedVideo),
    /// Voice-over content and inscription.
    VoiceOver(ResolvedVoiceOver),
    /// A title or call-to-action overlay.
    Overlay(ResolvedOverlay),
}

/// Video content with optional artifact and local delay.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedVideo {
    media: ResolvedMedia,
}

impl ResolvedVideo {
    pub(super) const fn new(
        element: ResolvedElement,
        src: Option<Authored<AssetRef>>,
        delay: Option<Authored<Duration>>,
    ) -> Self {
        Self {
            media: ResolvedMedia::new(element, src, delay),
        }
    }

    /// Returns the resolved video element.
    #[must_use]
    pub const fn element(&self) -> &ResolvedElement {
        self.media.element()
    }

    /// Returns the optional authored media reference.
    #[must_use]
    pub const fn src(&self) -> Option<&Authored<AssetRef>> {
        self.media.src()
    }

    /// Returns the optional delay from the owning shot start.
    #[must_use]
    pub const fn delay(&self) -> Option<&Authored<Duration>> {
        self.media.delay()
    }

    pub(super) fn into_media(self) -> ResolvedMedia {
        self.media
    }
}

/// Voice-over content with typed media facts and authored inscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedVoiceOver {
    media: ResolvedMedia,
    text: Vec<ResolvedText>,
}

impl ResolvedVoiceOver {
    pub(super) const fn new(
        element: ResolvedElement,
        src: Option<Authored<AssetRef>>,
        delay: Option<Authored<Duration>>,
        text: Vec<ResolvedText>,
    ) -> Self {
        Self {
            media: ResolvedMedia::new(element, src, delay),
            text,
        }
    }

    /// Returns the resolved voice-over element.
    #[must_use]
    pub const fn element(&self) -> &ResolvedElement {
        self.media.element()
    }

    /// Returns the optional authored audio reference.
    #[must_use]
    pub const fn src(&self) -> Option<&Authored<AssetRef>> {
        self.media.src()
    }

    /// Returns the optional delay from the owning shot start.
    #[must_use]
    pub const fn delay(&self) -> Option<&Authored<Duration>> {
        self.media.delay()
    }

    /// Returns decoded authored inscription in source order.
    #[must_use]
    pub fn text(&self) -> &[ResolvedText] {
        &self.text
    }

    pub(super) fn into_parts(self) -> (ResolvedMedia, Vec<ResolvedText>) {
        (self.media, self.text)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ResolvedMedia {
    element: ResolvedElement,
    src: Option<Authored<AssetRef>>,
    delay: Option<Authored<Duration>>,
}

impl ResolvedMedia {
    const fn new(
        element: ResolvedElement,
        src: Option<Authored<AssetRef>>,
        delay: Option<Authored<Duration>>,
    ) -> Self {
        Self {
            element,
            src,
            delay,
        }
    }

    const fn element(&self) -> &ResolvedElement {
        &self.element
    }

    const fn src(&self) -> Option<&Authored<AssetRef>> {
        self.src.as_ref()
    }

    const fn delay(&self) -> Option<&Authored<Duration>> {
        self.delay.as_ref()
    }

    pub(super) fn into_parts(
        self,
    ) -> (
        ResolvedElement,
        Option<Authored<AssetRef>>,
        Option<Authored<Duration>>,
    ) {
        (self.element, self.src, self.delay)
    }
}

/// A title or call-to-action with one unambiguous start rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedOverlay {
    element: ResolvedElement,
    start: ResolvedStart,
    text: Vec<ResolvedText>,
}

impl ResolvedOverlay {
    pub(super) const fn new(
        element: ResolvedElement,
        start: ResolvedStart,
        text: Vec<ResolvedText>,
    ) -> Self {
        Self {
            element,
            start,
            text,
        }
    }

    /// Returns the resolved overlay element.
    #[must_use]
    pub const fn element(&self) -> &ResolvedElement {
        &self.element
    }

    /// Returns the single resolved overlay start rule.
    #[must_use]
    pub const fn start(&self) -> &ResolvedStart {
        &self.start
    }

    /// Returns decoded authored overlay text in source order.
    #[must_use]
    pub fn text(&self) -> &[ResolvedText] {
        &self.text
    }

    pub(super) fn into_parts(self) -> (ResolvedElement, ResolvedStart, Vec<ResolvedText>) {
        (self.element, self.start, self.text)
    }
}

/// Resolved start rule for an overlay.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum ResolvedStart {
    /// Start with the owning shot.
    #[default]
    ShotStart,
    /// Start after an authored delay from the owning shot.
    Delayed(Authored<Duration>),
    /// Start at an authored named event.
    Cue(Authored<EventRef>),
}

/// One decoded text run without a syntax-layer public dependency.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedText {
    text: Box<str>,
    span: SourceSpan,
}

impl ResolvedText {
    pub(super) const fn new(text: Box<str>, span: SourceSpan) -> Self {
        Self { text, span }
    }

    /// Returns the decoded authored text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the authored text span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(super) fn into_parts(self) -> (Box<str>, SourceSpan) {
        (self.text, self.span)
    }
}
