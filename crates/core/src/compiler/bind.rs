//! Structural binding from recovered syntax into the closed Onmark vocabulary.
//!
//! This phase owns containment, cardinality, and film-wide ID rules. It keeps
//! walking after independent authored mistakes, but never lets unknown syntax
//! enter the linked representation.

use std::collections::BTreeMap;

use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::{ElementKind, GeneralAudioKind, InvalidNodeId, NodeId, SourceSpan};
use crate::syntax::{Attribute, Element, Node, SourceDocument, TextNode};

use super::diagnostic::author_diagnostic;
use super::linked_film::{
    LinkedAudio, LinkedCue, LinkedCues, LinkedElement, LinkedFilm, LinkedId, LinkedNode,
    LinkedOverlay, LinkedScene, LinkedShot, LinkedShotContent, LinkedVideo, LinkedVoiceOver,
};

/// Optional structurally linked output and every recoverable binding diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BindReport {
    film: Option<LinkedFilm>,
    diagnostics: Diagnostics,
}

impl BindReport {
    /// Returns the linked film when binding produced no error diagnostic.
    #[must_use]
    pub const fn film(&self) -> Option<&LinkedFilm> {
        self.film.as_ref()
    }

    /// Returns all authored diagnostics produced during structural binding.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Returns the optional linked film and its authored diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (Option<LinkedFilm>, Diagnostics) {
        (self.film, self.diagnostics)
    }
}

/// Binds one parsed document into typed screenplay structure.
///
/// Error diagnostics withhold the linked film from later compiler phases.
#[must_use]
pub fn bind(document: SourceDocument) -> BindReport {
    let mut diagnostics = Diagnostics::new();
    let mut films = Vec::new();
    let (nodes, document_span) = document.into_parts();

    for node in nodes {
        let element = match node {
            Node::Text(text) if text.text().trim().is_empty() => continue,
            Node::Text(text) => {
                diagnostics.push(unexpected_top_level_text(&text));
                continue;
            }
            Node::Element(element) => element,
        };
        let Some(kind) = element_kind(&element) else {
            diagnostics.push(unknown_element(&element));
            continue;
        };
        if kind != ElementKind::Film {
            diagnostics.push(misplaced_element(&element, kind, None));
            continue;
        }

        films.push(element);
    }

    let mut films = films.into_iter();
    let Some(first) = films.next() else {
        diagnostics.push(missing_film_root(document_span));
        return BindReport {
            film: None,
            diagnostics,
        };
    };
    let Some(duplicate) = films.next() else {
        let (candidate, diagnostics) = Binder::new(diagnostics).bind_film(first);
        let film = (!diagnostics.has_errors()).then_some(candidate);
        return BindReport { film, diagnostics };
    };

    // No root is canonical once cardinality fails. Choosing the first would
    // make an invalid authoring order change the linked film.
    diagnostics.push(multiple_film_roots(&first, &duplicate));
    for duplicate in films {
        diagnostics.push(multiple_film_roots(&first, &duplicate));
    }

    BindReport {
        film: None,
        diagnostics,
    }
}

/// Single owner of film-wide names and recoverable structural diagnostics.
struct Binder {
    ids: BTreeMap<NodeId, LinkedNode>,
    diagnostics: Diagnostics,
}

impl Binder {
    const fn new(diagnostics: Diagnostics) -> Self {
        Self {
            ids: BTreeMap::new(),
            diagnostics,
        }
    }

    fn bind_film(mut self, element: Element) -> (LinkedFilm, Diagnostics) {
        let (_, attributes, children, span) = element.into_parts();
        let linked = self.bind_element(attributes, ElementKind::Film, span);
        let mut cues = None;
        let mut music = Vec::new();
        let mut scenes = Vec::new();

        for node in children {
            let Some(child) = self.structural_child(node, ElementKind::Film) else {
                continue;
            };

            match self.recognize_or_report(&child) {
                Some(ElementKind::Cues) => self.bind_cues_container(child, &mut cues),
                Some(ElementKind::Music) => {
                    music.push(self.bind_audio(child, GeneralAudioKind::Music));
                }
                Some(ElementKind::Scene) => scenes.push(self.bind_scene(child)),
                Some(kind) => self.reject_misplaced(&child, kind, ElementKind::Film),
                None => {}
            }
        }

        let cues = cues.map(|(_, cues)| cues);
        let film = LinkedFilm::new(linked, cues, music, scenes, self.ids);
        (film, self.diagnostics)
    }

    fn bind_cues_container(&mut self, child: Element, cues: &mut Option<(SourceSpan, LinkedCues)>) {
        let Some((first, _)) = cues else {
            *cues = Some((child.name().span(), self.bind_cues(child)));
            return;
        };

        // The rejected subtree must not contribute IDs or attributes to the
        // canonical linked film.
        self.diagnostics.push(duplicate_cues(&child, *first));
    }

    fn bind_cues(&mut self, element: Element) -> LinkedCues {
        let (_, attributes, children, span) = element.into_parts();
        let linked = self.bind_element(attributes, ElementKind::Cues, span);
        let mut cues = Vec::new();

        for node in children {
            let Some(child) = self.structural_child(node, ElementKind::Cues) else {
                continue;
            };

            match self.recognize_or_report(&child) {
                Some(ElementKind::Cue) => cues.push(self.bind_cue(child)),
                Some(kind) => self.reject_misplaced(&child, kind, ElementKind::Cues),
                None => {}
            }
        }

        LinkedCues::new(linked, cues)
    }

    fn bind_cue(&mut self, element: Element) -> LinkedCue {
        let (_, attributes, children, span) = element.into_parts();
        let linked = self.bind_element(attributes, ElementKind::Cue, span);
        self.reject_child_elements_and_text(children, ElementKind::Cue);
        LinkedCue::new(linked)
    }

    fn bind_scene(&mut self, element: Element) -> LinkedScene {
        let (_, attributes, children, span) = element.into_parts();
        let linked = self.bind_element(attributes, ElementKind::Scene, span);
        let mut shots = Vec::new();

        for node in children {
            let Some(child) = self.structural_child(node, ElementKind::Scene) else {
                continue;
            };

            match self.recognize_or_report(&child) {
                Some(ElementKind::Shot) => shots.push(self.bind_shot(child)),
                Some(kind) => self.reject_misplaced(&child, kind, ElementKind::Scene),
                None => {}
            }
        }

        LinkedScene::new(linked, shots)
    }

    fn bind_shot(&mut self, element: Element) -> LinkedShot {
        let (_, attributes, children, span) = element.into_parts();
        let linked = self.bind_element(attributes, ElementKind::Shot, span);
        let mut content = Vec::new();
        let mut sound_effects = Vec::new();

        for node in children {
            let Some(child) = self.structural_child(node, ElementKind::Shot) else {
                continue;
            };

            match self.recognize_or_report(&child) {
                Some(ElementKind::Video) => {
                    content.push(LinkedShotContent::Video(self.bind_video(child)));
                }
                Some(ElementKind::VoiceOver) => {
                    content.push(LinkedShotContent::VoiceOver(self.bind_voice_over(child)));
                }
                Some(ElementKind::SoundEffect) => {
                    sound_effects.push(self.bind_audio(child, GeneralAudioKind::SoundEffect));
                }
                Some(kind @ (ElementKind::Title | ElementKind::CallToAction)) => {
                    content.push(LinkedShotContent::Overlay(self.bind_overlay(child, kind)));
                }
                Some(kind) => self.reject_misplaced(&child, kind, ElementKind::Shot),
                None => {}
            }
        }

        LinkedShot::new(linked, content, sound_effects)
    }

    fn bind_video(&mut self, element: Element) -> LinkedVideo {
        let (_, attributes, children, span) = element.into_parts();
        let linked = self.bind_element(attributes, ElementKind::Video, span);
        self.reject_child_elements_and_text(children, ElementKind::Video);
        LinkedVideo::new(linked)
    }

    fn bind_audio(&mut self, element: Element, kind: GeneralAudioKind) -> LinkedAudio {
        let (_, attributes, children, span) = element.into_parts();
        let element_kind = kind.element_kind();
        let linked = self.bind_element(attributes, element_kind, span);
        self.reject_child_elements_and_text(children, element_kind);
        LinkedAudio::new(kind, linked)
    }

    fn bind_voice_over(&mut self, element: Element) -> LinkedVoiceOver {
        let (_, attributes, children, span) = element.into_parts();
        let linked = self.bind_element(attributes, ElementKind::VoiceOver, span);
        let text = self.bind_text(children, ElementKind::VoiceOver);
        LinkedVoiceOver::new(linked, text)
    }

    fn bind_overlay(&mut self, element: Element, kind: ElementKind) -> LinkedOverlay {
        let (_, attributes, children, span) = element.into_parts();
        let linked = self.bind_element(attributes, kind, span);
        let text = self.bind_text(children, kind);
        LinkedOverlay::new(linked, text)
    }

    fn bind_text(&mut self, children: Vec<Node>, parent: ElementKind) -> Vec<TextNode> {
        let mut text = Vec::new();

        for node in children {
            match node {
                Node::Text(node) => text.push(node),
                Node::Element(child) => self.reject_child_element(&child, parent),
            }
        }

        text
    }

    fn reject_child_elements_and_text(&mut self, children: Vec<Node>, parent: ElementKind) {
        for node in children {
            match node {
                Node::Text(text) if text.text().trim().is_empty() => {}
                Node::Text(text) => self.diagnostics.push(unexpected_text(&text, parent)),
                Node::Element(child) => self.reject_child_element(&child, parent),
            }
        }
    }

    fn reject_child_element(&mut self, child: &Element, parent: ElementKind) {
        if let Some(kind) = self.recognize_or_report(child) {
            self.diagnostics
                .push(misplaced_element(child, kind, Some(parent)));
        }
    }

    fn structural_child(&mut self, node: Node, parent: ElementKind) -> Option<Element> {
        match node {
            Node::Element(element) => Some(element),
            Node::Text(text) if text.text().trim().is_empty() => None,
            Node::Text(text) => {
                self.diagnostics.push(unexpected_text(&text, parent));
                None
            }
        }
    }

    fn recognize_or_report(&mut self, element: &Element) -> Option<ElementKind> {
        let kind = element_kind(element);
        if kind.is_none() {
            self.diagnostics.push(unknown_element(element));
        }
        kind
    }

    fn reject_misplaced(&mut self, element: &Element, kind: ElementKind, parent: ElementKind) {
        self.diagnostics
            .push(misplaced_element(element, kind, Some(parent)));
    }

    fn bind_element(
        &mut self,
        mut attributes: Vec<Attribute>,
        kind: ElementKind,
        span: SourceSpan,
    ) -> LinkedElement {
        let id = attributes
            .iter()
            .position(is_id_attribute)
            .map(|index| attributes.remove(index));
        // Syntax owns duplicate-attribute diagnostics. Binding consumes the
        // first ID spelling and keeps no duplicate as a semantic attribute.
        attributes.retain(|attribute| !is_id_attribute(attribute));
        let id = self.bind_id(id.as_ref(), kind);

        LinkedElement::new(kind, id, attributes, span)
    }

    fn bind_id(&mut self, attribute: Option<&Attribute>, kind: ElementKind) -> LinkedId {
        let Some(attribute) = attribute else {
            return LinkedId::Missing;
        };
        let id = match NodeId::parse(attribute.value()) {
            Ok(id) => id,
            Err(reason) => {
                self.diagnostics
                    .push(invalid_node_id(attribute, kind, reason));
                return LinkedId::Rejected;
            }
        };

        if let Some(first) = self.ids.get(&id) {
            self.diagnostics
                .push(duplicate_node_id(attribute, kind, &id, first.span()));
            return LinkedId::Rejected;
        }

        // The linked tree and lookup index independently own stable identity;
        // neither structure borrows from the other.
        self.ids
            .insert(id.clone(), LinkedNode::new(kind, attribute.value_span()));
        LinkedId::Valid(id)
    }
}

fn element_kind(element: &Element) -> Option<ElementKind> {
    if element.name().prefix().is_some() {
        return None;
    }

    ElementKind::from_local_name(element.name().local())
}

fn is_id_attribute(attribute: &Attribute) -> bool {
    attribute.name().prefix().is_none() && attribute.name().local() == "id"
}

fn unknown_element(element: &Element) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnknownElement,
        element.name().span(),
        format!(
            "element <{}> is not part of the screenplay language",
            element.name()
        ),
        "use a screenplay element from the current language or remove this element",
    )
}

fn missing_film_root(document: SourceSpan) -> Diagnostic {
    let primary = SourceSpan::new(document.source(), document.start(), document.start())
        .expect("a point at the document start has ordered bounds");
    author_diagnostic(
        DiagnosticCode::MissingFilmRoot,
        primary,
        "screenplay must contain one top-level <film> element",
        "wrap the screenplay in one <film> element",
    )
}

fn multiple_film_roots(first: &Element, duplicate: &Element) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::MultipleFilmRoots,
        duplicate.name().span(),
        "screenplay contains more than one top-level <film> element",
        "keep exactly one top-level <film> element",
    )
    .with_related(first.name().span(), "the first <film> element is here")
    .expect("the static related message is non-blank")
}

fn misplaced_element(
    element: &Element,
    kind: ElementKind,
    parent: Option<ElementKind>,
) -> Diagnostic {
    let message = match parent {
        Some(parent) => format!("element <{kind}> is not allowed inside <{parent}>"),
        None => format!("element <{kind}> is not allowed at the top level"),
    };

    author_diagnostic(
        DiagnosticCode::MisplacedElement,
        element.name().span(),
        message,
        format!("move <{kind}> to a valid screenplay container"),
    )
}

fn duplicate_cues(element: &Element, first: SourceSpan) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::DuplicateCues,
        element.name().span(),
        "film contains more than one <cues> container",
        "merge all cue declarations into one <cues> container",
    )
    .with_related(first, "the first <cues> container is here")
    .expect("the static related message is non-blank")
}

fn invalid_node_id(attribute: &Attribute, kind: ElementKind, reason: InvalidNodeId) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidNodeId,
        attribute.value_span(),
        format!("{kind} ID is invalid: {reason}"),
        match reason {
            InvalidNodeId::Empty => "provide a non-empty id value",
            InvalidNodeId::ContainsAsciiWhitespace => {
                "remove ASCII whitespace or replace it with a visible separator"
            }
        },
    )
}

fn duplicate_node_id(
    attribute: &Attribute,
    kind: ElementKind,
    id: &NodeId,
    first: SourceSpan,
) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::DuplicateNodeId,
        attribute.value_span(),
        format!("{kind} ID \"{id}\" is already used in this film"),
        format!("choose a unique id for this {kind}"),
    )
    .with_related(first, format!("ID \"{id}\" is first declared here"))
    .expect("a formatted related message is non-blank")
}

fn unexpected_top_level_text(text: &TextNode) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnexpectedText,
        text.span(),
        "text is not allowed outside the top-level <film> element",
        "move this text into a text-bearing screenplay element or remove it",
    )
}

fn unexpected_text(text: &TextNode, parent: ElementKind) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnexpectedText,
        text.span(),
        format!("text is not allowed directly inside <{parent}>"),
        "move this text into a text-bearing element or remove it",
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use proptest::prelude::*;

    use crate::compiler::parse;
    use crate::diagnostics::DiagnosticCode;
    use crate::model::{ElementKind, NodeId, SourceId};

    use super::{
        BindReport, LinkedAudio, LinkedCue, LinkedElement, LinkedFilm, LinkedShotContent, bind,
    };

    fn bind_source(source: SourceId, text: &str) -> BindReport {
        let parsed = parse(source, text);
        let (document, diagnostics) = parsed.into_parts();
        assert!(diagnostics.is_empty());
        bind(document)
    }

    #[test]
    fn locates_a_missing_film_in_an_empty_source() {
        let report = bind_source(SourceId::new(7), "");
        let diagnostic = report
            .diagnostics()
            .iter()
            .next()
            .expect("an empty screenplay has no film root");

        assert_eq!(diagnostic.code(), DiagnosticCode::MissingFilmRoot);
        assert_eq!(diagnostic.primary().source(), SourceId::new(7));
        assert_eq!(diagnostic.primary().start().get(), 0);
        assert_eq!(diagnostic.primary().end().get(), 0);
    }

    #[test]
    fn cue_ids_share_the_film_wide_namespace() {
        let source = "<film><cues><cue id=\"shared\"/></cues><scene id=\"shared\"/></film>";
        let report = bind_source(SourceId::new(0), source);
        let duplicate = report
            .diagnostics()
            .iter()
            .find(|diagnostic| diagnostic.code() == DiagnosticCode::DuplicateNodeId);

        assert!(duplicate.is_some());
        assert!(report.film().is_none());
    }

    proptest! {
        #[test]
        fn linked_ids_and_the_film_index_describe_the_same_nodes(
            ids in proptest::collection::btree_set("[a-z0-9-]{1,8}", 11..=11),
        ) {
            let ids = ids.into_iter().collect::<Vec<_>>();
            let source = screenplay_with_ids(&ids);
            let first = bind_source(SourceId::new(0), &source);
            let second = bind_source(SourceId::new(0), &source);
            let film = first.film().expect("the generated screenplay has one film root");

            prop_assert!(first.diagnostics().is_empty());
            prop_assert_eq!(&first, &second);
            prop_assert_eq!(linked_ids(film), expected_ids(&ids));
            prop_assert_eq!(indexed_ids(film), expected_ids(&ids));
        }
    }

    fn screenplay_with_ids(ids: &[String]) -> String {
        format!(
            concat!(
                "<film id=\"{}\">",
                "<cues id=\"{}\"><cue id=\"{}\"/></cues>",
                "<music id=\"{}\"/>",
                "<scene id=\"{}\"><shot id=\"{}\">",
                "<video id=\"{}\"/>",
                "<vo id=\"{}\">voice</vo>",
                "<sfx id=\"{}\"/>",
                "<title id=\"{}\">title</title>",
                "<cta id=\"{}\">action</cta>",
                "</shot></scene></film>",
            ),
            ids[0], ids[1], ids[2], ids[3], ids[4], ids[5], ids[6], ids[7], ids[8], ids[9], ids[10],
        )
    }

    fn expected_ids(ids: &[String]) -> BTreeMap<NodeId, ElementKind> {
        let kinds = [
            ElementKind::Film,
            ElementKind::Cues,
            ElementKind::Cue,
            ElementKind::Music,
            ElementKind::Scene,
            ElementKind::Shot,
            ElementKind::Video,
            ElementKind::VoiceOver,
            ElementKind::SoundEffect,
            ElementKind::Title,
            ElementKind::CallToAction,
        ];
        let mut expected = BTreeMap::new();

        for (authored, kind) in ids.iter().zip(kinds) {
            if let Ok(id) = NodeId::parse(authored.as_str()) {
                expected.entry(id).or_insert(kind);
            }
        }

        expected
    }

    fn linked_ids(film: &LinkedFilm) -> BTreeMap<NodeId, ElementKind> {
        linked_elements(film)
            .into_iter()
            .filter_map(|element| element.id().cloned().map(|id| (id, element.kind())))
            .collect()
    }

    fn indexed_ids(film: &LinkedFilm) -> BTreeMap<NodeId, ElementKind> {
        film.ids()
            .map(|(id, node)| (id.clone(), node.kind()))
            .collect()
    }

    fn linked_elements(film: &LinkedFilm) -> Vec<&LinkedElement> {
        let mut elements = vec![film.element()];

        if let Some(cues) = film.cues() {
            elements.push(cues.element());
            elements.extend(cues.cues().iter().map(LinkedCue::element));
        }
        elements.extend(film.music().iter().map(LinkedAudio::element));

        for scene in film.scenes() {
            elements.push(scene.element());
            for shot in scene.shots() {
                elements.push(shot.element());
                elements.extend(shot.content().iter().map(LinkedShotContent::element));
                elements.extend(shot.sound_effects().iter().map(LinkedAudio::element));
            }
        }

        elements
    }
}
