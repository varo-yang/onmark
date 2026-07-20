//! Structurally valid screenplay form between binding and attribute resolution.
//!
//! Raw attributes and text remain attached to source spans, while unknown or
//! misplaced elements and invalid IDs have already been excluded.

use std::collections::BTreeMap;

use crate::model::{ElementKind, NodeId, SourceSpan};
use crate::syntax::{Attribute, TextNode};

/// Shared authored facts retained by every structurally bound element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedElement {
    kind: ElementKind,
    id: LinkedId,
    attributes: Vec<Attribute>,
    span: SourceSpan,
}

impl LinkedElement {
    pub(super) const fn new(
        kind: ElementKind,
        id: LinkedId,
        attributes: Vec<Attribute>,
        span: SourceSpan,
    ) -> Self {
        Self {
            kind,
            id,
            attributes,
            span,
        }
    }

    /// Returns the recognized screenplay element kind.
    #[must_use]
    pub const fn kind(&self) -> ElementKind {
        self.kind
    }

    /// Returns the valid, film-unique ID when one was authored.
    #[must_use]
    pub const fn id(&self) -> Option<&NodeId> {
        self.id.as_node_id()
    }

    /// Returns the complete authored element span.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }

    pub(super) fn into_parts(self) -> (ElementKind, LinkedId, Vec<Attribute>, SourceSpan) {
        (self.kind, self.id, self.attributes, self.span)
    }
}

/// Outcome of binding an optional authored ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LinkedId {
    /// No ID attribute was authored.
    Missing,
    /// An authored ID was invalid or duplicated and already diagnosed.
    Rejected,
    /// The ID is valid and unique within the film.
    Valid(NodeId),
}

impl LinkedId {
    const fn as_node_id(&self) -> Option<&NodeId> {
        match self {
            Self::Valid(id) => Some(id),
            Self::Missing | Self::Rejected => None,
        }
    }

    pub(super) fn into_node_id(self) -> Option<NodeId> {
        match self {
            Self::Valid(id) => Some(id),
            Self::Missing | Self::Rejected => None,
        }
    }
}

/// A structurally bound screenplay with one film root and valid unique IDs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedFilm {
    element: LinkedElement,
    cues: Option<LinkedCues>,
    music: Vec<LinkedAudio>,
    scenes: Vec<LinkedScene>,
    ids: BTreeMap<NodeId, LinkedNode>,
}

impl LinkedFilm {
    pub(super) fn new(
        element: LinkedElement,
        cues: Option<LinkedCues>,
        music: Vec<LinkedAudio>,
        scenes: Vec<LinkedScene>,
        ids: BTreeMap<NodeId, LinkedNode>,
    ) -> Self {
        Self {
            element,
            cues,
            music,
            scenes,
            ids,
        }
    }

    /// Returns the structurally bound film element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    /// Returns the optional singleton cue container.
    #[must_use]
    pub const fn cues(&self) -> Option<&LinkedCues> {
        self.cues.as_ref()
    }

    /// Returns film-wide music in authored order.
    #[must_use]
    pub fn music(&self) -> &[LinkedAudio] {
        &self.music
    }

    /// Returns sequential scenes in authored order.
    #[must_use]
    pub fn scenes(&self) -> &[LinkedScene] {
        &self.scenes
    }

    /// Returns the deterministic film-wide ID index.
    #[must_use]
    pub fn ids(&self) -> impl ExactSizeIterator<Item = (&NodeId, &LinkedNode)> {
        self.ids.iter()
    }

    pub(super) fn into_parts(self) -> LinkedFilmParts {
        LinkedFilmParts {
            element: self.element,
            cues: self.cues,
            music: self.music,
            scenes: self.scenes,
            ids: self.ids,
        }
    }
}

/// Consuming handoff from structural binding into attribute resolution.
pub(super) struct LinkedFilmParts {
    pub(super) element: LinkedElement,
    pub(super) cues: Option<LinkedCues>,
    pub(super) music: Vec<LinkedAudio>,
    pub(super) scenes: Vec<LinkedScene>,
    pub(super) ids: BTreeMap<NodeId, LinkedNode>,
}

/// One declaration in the film-wide ID index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkedNode {
    kind: ElementKind,
    declared_at: SourceSpan,
}

impl LinkedNode {
    pub(super) const fn new(kind: ElementKind, declared_at: SourceSpan) -> Self {
        Self { kind, declared_at }
    }

    /// Returns the kind of the indexed element.
    #[must_use]
    pub const fn kind(&self) -> ElementKind {
        self.kind
    }

    /// Returns the span of the authored ID value.
    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.declared_at
    }
}

/// The optional singleton cue container owned by a film.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedCues {
    element: LinkedElement,
    cues: Vec<LinkedCue>,
}

impl LinkedCues {
    pub(super) const fn new(element: LinkedElement, cues: Vec<LinkedCue>) -> Self {
        Self { element, cues }
    }

    /// Returns the structurally bound cue-container element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    /// Returns cue declarations in authored order.
    #[must_use]
    pub fn cues(&self) -> &[LinkedCue] {
        &self.cues
    }

    pub(super) fn into_parts(self) -> (LinkedElement, Vec<LinkedCue>) {
        (self.element, self.cues)
    }
}

/// One named event declaration before its time value is parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedCue {
    element: LinkedElement,
}

impl LinkedCue {
    pub(super) const fn new(element: LinkedElement) -> Self {
        Self { element }
    }

    /// Returns the structurally bound cue element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    pub(super) fn into_element(self) -> LinkedElement {
        self.element
    }
}

/// One sequential scene.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedScene {
    element: LinkedElement,
    shots: Vec<LinkedShot>,
}

impl LinkedScene {
    pub(super) const fn new(element: LinkedElement, shots: Vec<LinkedShot>) -> Self {
        Self { element, shots }
    }

    /// Returns the structurally bound scene element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    /// Returns sequential shots in authored order.
    #[must_use]
    pub fn shots(&self) -> &[LinkedShot] {
        &self.shots
    }

    pub(super) fn into_parts(self) -> (LinkedElement, Vec<LinkedShot>) {
        (self.element, self.shots)
    }
}

/// One sequential shot and its authored content order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedShot {
    element: LinkedElement,
    content: Vec<LinkedShotContent>,
    sound_effects: Vec<LinkedAudio>,
}

impl LinkedShot {
    pub(super) const fn new(
        element: LinkedElement,
        content: Vec<LinkedShotContent>,
        sound_effects: Vec<LinkedAudio>,
    ) -> Self {
        Self {
            element,
            content,
            sound_effects,
        }
    }

    /// Returns the structurally bound shot element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    /// Returns recognized shot content in authored order.
    #[must_use]
    pub fn content(&self) -> &[LinkedShotContent] {
        &self.content
    }

    /// Returns shot-local sound effects in authored order.
    #[must_use]
    pub fn sound_effects(&self) -> &[LinkedAudio] {
        &self.sound_effects
    }

    pub(super) fn into_parts(self) -> (LinkedElement, Vec<LinkedShotContent>, Vec<LinkedAudio>) {
        (self.element, self.content, self.sound_effects)
    }
}

/// Closed kinds of narrative and visual content owned by a shot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkedShotContent {
    /// Primary video content.
    Video(LinkedVideo),
    /// Voice-over content and inscription.
    VoiceOver(LinkedVoiceOver),
    /// A title or call-to-action overlay.
    Overlay(LinkedOverlay),
}

impl LinkedShotContent {
    /// Returns the structurally bound element behind this shot-content role.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        match self {
            Self::Video(video) => video.element(),
            Self::VoiceOver(voice_over) => voice_over.element(),
            Self::Overlay(overlay) => overlay.element(),
        }
    }
}

/// The current language's sole visual media element.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedVideo {
    element: LinkedElement,
}

/// General audio before source, gain, and optional delay are resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedAudio {
    element: LinkedElement,
}

impl LinkedAudio {
    pub(super) const fn new(element: LinkedElement) -> Self {
        Self { element }
    }

    /// Returns the structurally bound music or sound-effect element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    pub(super) fn into_element(self) -> LinkedElement {
        self.element
    }
}

impl LinkedVideo {
    pub(super) const fn new(element: LinkedElement) -> Self {
        Self { element }
    }

    /// Returns the structurally bound video element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    pub(super) fn into_element(self) -> LinkedElement {
        self.element
    }
}

/// Authored voice-over text before media attributes are resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedVoiceOver {
    element: LinkedElement,
    text: Vec<TextNode>,
}

impl LinkedVoiceOver {
    pub(super) const fn new(element: LinkedElement, text: Vec<TextNode>) -> Self {
        Self { element, text }
    }

    /// Returns the structurally bound voice-over element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    pub(super) fn into_parts(self) -> (LinkedElement, Vec<TextNode>) {
        (self.element, self.text)
    }
}

/// A title or call-to-action overlay owned by one shot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinkedOverlay {
    element: LinkedElement,
    text: Vec<TextNode>,
}

impl LinkedOverlay {
    pub(super) const fn new(element: LinkedElement, text: Vec<TextNode>) -> Self {
        Self { element, text }
    }

    /// Returns the structurally bound overlay element.
    #[must_use]
    pub const fn element(&self) -> &LinkedElement {
        &self.element
    }

    pub(super) fn into_parts(self) -> (LinkedElement, Vec<TextNode>) {
        (self.element, self.text)
    }
}
