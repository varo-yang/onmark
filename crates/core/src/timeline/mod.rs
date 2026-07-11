//! Versioned frame facts produced by the pure compiler.
//!
//! Timeline values depend only on foundational model types. They contain no
//! browser, codec, partition, or execution-plan decisions.

use std::collections::BTreeMap;

use crate::model::{
    AssetRef, CueId, ElementKind, EventRef, FrameIndex, FrameInterval, NodeId, SourceSpan, Timebase,
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
}

impl TimelineIr {
    pub(crate) const fn new(
        timebase: Timebase,
        element: TimelineElement,
        interval: FrameInterval,
        events: BTreeMap<CueId, TimelineEvent>,
        scenes: Vec<TimelineScene>,
    ) -> Self {
        Self {
            version: TimelineVersion::V1,
            timebase,
            element,
            interval,
            events,
            scenes,
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

/// Closed Gate-one content after all frame bounds are solved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimelineContent {
    /// Primary video content.
    Video(TimelineVideo),
    /// Voice-over audio and its authored inscription.
    VoiceOver(TimelineVoiceOver),
    /// A title or call-to-action overlay.
    Overlay(TimelineOverlay),
}

/// Solved primary video content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineVideo {
    element: TimelineElement,
    timing: TimelineTiming,
    asset: AssetRef,
}

impl TimelineVideo {
    pub(crate) const fn new(
        element: TimelineElement,
        timing: TimelineTiming,
        asset: AssetRef,
    ) -> Self {
        Self {
            element,
            timing,
            asset,
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

    /// Returns the frozen media artifact.
    #[must_use]
    pub const fn asset(&self) -> &AssetRef {
        &self.asset
    }
}

/// Solved voice-over content and inscription.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineVoiceOver {
    element: TimelineElement,
    timing: TimelineTiming,
    asset: AssetRef,
    text: Vec<TimelineText>,
}

impl TimelineVoiceOver {
    pub(crate) const fn new(
        element: TimelineElement,
        timing: TimelineTiming,
        asset: AssetRef,
        text: Vec<TimelineText>,
    ) -> Self {
        Self {
            element,
            timing,
            asset,
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
        &self.timing
    }

    /// Returns the frozen voice-over artifact.
    #[must_use]
    pub const fn asset(&self) -> &AssetRef {
        &self.asset
    }

    /// Returns authored inscription runs in source order.
    #[must_use]
    pub fn text(&self) -> &[TimelineText] {
        &self.text
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
}
