//! Attribute and reference resolution without assigning frame positions.
//!
//! Raw syntax values are consumed here and replaced by typed authored intent.
//! Asset probing and frame arithmetic remain later-phase responsibilities.

use std::collections::BTreeMap;

use crate::diagnostics::{Diagnostic, DiagnosticCode, Diagnostics};
use crate::model::{
    AssetRef, AudioGain, CueId, Duration, ElementKind, EventRef, GeneralAudioKind, InvalidAssetRef,
    InvalidAudioGain, InvalidDuration, InvalidNodeId, NodeId, SourceSpan,
};
use crate::syntax::{Attribute, TextNode};

use super::diagnostic::author_diagnostic;
use super::linked_film::{
    LinkedAudio, LinkedCue, LinkedCues, LinkedElement, LinkedFilm, LinkedFilmParts, LinkedId,
    LinkedNode, LinkedOverlay, LinkedScene, LinkedShot, LinkedShotContent, LinkedVideo,
    LinkedVoiceOver,
};
use super::resolved_film::{
    Authored, ResolvedAudio, ResolvedCue, ResolvedCues, ResolvedElement, ResolvedFilm,
    ResolvedNode, ResolvedOverlay, ResolvedScene, ResolvedShot, ResolvedShotContent, ResolvedStart,
    ResolvedText, ResolvedVideo, ResolvedVoiceOver,
};

/// Optional typed attribute/reference output and its authored diagnostics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolveReport {
    film: Option<ResolvedFilm>,
    diagnostics: Diagnostics,
}

impl ResolveReport {
    /// Returns the resolved film when no error diagnostic was produced.
    #[must_use]
    pub const fn film(&self) -> Option<&ResolvedFilm> {
        self.film.as_ref()
    }

    /// Returns all authored diagnostics produced during resolution.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// Returns the optional resolved film and its authored diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (Option<ResolvedFilm>, Diagnostics) {
        (self.film, self.diagnostics)
    }
}

/// Resolves every remaining authored attribute without performing IO.
///
/// Error diagnostics withhold the resolved film; warnings remain non-blocking.
#[must_use]
pub fn resolve(film: LinkedFilm) -> ResolveReport {
    Resolver::resolve(film)
}

/// Single owner of cue usage and all recoverable attribute diagnostics.
struct Resolver {
    cue_table: BTreeMap<CueId, CueState>,
    ids: BTreeMap<NodeId, LinkedNode>,
    diagnostics: Diagnostics,
}

impl Resolver {
    fn new(ids: BTreeMap<NodeId, LinkedNode>) -> Self {
        Self {
            cue_table: BTreeMap::new(),
            ids,
            diagnostics: Diagnostics::new(),
        }
    }

    fn resolve(film: LinkedFilm) -> ResolveReport {
        let LinkedFilmParts {
            element,
            cues,
            music,
            scenes,
            ids,
        } = film.into_parts();
        let mut resolver = Self::new(ids);
        let element = resolver.resolve_id_only_element(element);
        let cues = cues.map(|cues| resolver.resolve_cues(cues));
        let music = music
            .into_iter()
            .filter_map(|audio| resolver.resolve_audio(audio))
            .collect();
        let mut resolved_scenes = Vec::with_capacity(scenes.len());
        for scene in scenes {
            resolved_scenes.push(resolver.resolve_scene(scene));
        }

        resolver.report_unused_cues();

        let Self {
            ids, diagnostics, ..
        } = resolver;
        let ids = ids
            .into_iter()
            .map(|(id, node)| (id, resolved_node(node)))
            .collect();
        let candidate = ResolvedFilm::new(element, cues, music, resolved_scenes, ids);
        let film = (!diagnostics.has_errors()).then_some(candidate);

        ResolveReport { film, diagnostics }
    }

    fn resolve_cues(&mut self, cues: LinkedCues) -> ResolvedCues {
        let (element, cues) = cues.into_parts();
        let element = self.resolve_id_only_element(element);
        let mut resolved_cues = Vec::with_capacity(cues.len());

        for cue in cues {
            if let Some(cue) = self.resolve_cue(cue) {
                resolved_cues.push(cue);
            }
        }

        ResolvedCues::new(element, resolved_cues)
    }

    fn resolve_cue(&mut self, cue: LinkedCue) -> Option<ResolvedCue> {
        let input = ElementInput::new(cue.into_element());
        let mut attributes = input.attributes;
        let time = attributes.take("time");
        attributes.reject_unknown(input.kind, &mut self.diagnostics);

        let id = match input.id {
            LinkedId::Valid(id) => Some(id),
            LinkedId::Missing => {
                self.diagnostics
                    .push(missing_attribute(input.kind, "id", input.span));
                None
            }
            LinkedId::Rejected => None,
        };
        let time = if let Some(attribute) = time {
            self.resolve_duration(&attribute)
        } else {
            self.diagnostics
                .push(missing_attribute(input.kind, "time", input.span));
            None
        };

        let id = id?;
        let Some(time) = time else {
            self.ids.remove(&id);
            return None;
        };

        let declared_at = self
            .ids
            .get(&id)
            .expect("every bound ID has an index entry")
            .span();
        let id = CueId::from(id);
        // The resolved tree and lookup table independently own stable cue
        // identity; neither borrows from the other phase output.
        self.cue_table.insert(
            id.clone(),
            CueState {
                declared_at,
                used: false,
            },
        );

        Some(ResolvedCue::new(
            Authored::new(id, declared_at),
            time,
            input.span,
        ))
    }

    fn resolve_scene(&mut self, scene: LinkedScene) -> ResolvedScene {
        let (element, shots) = scene.into_parts();
        let element = self.resolve_id_only_element(element);
        let mut resolved_shots = Vec::with_capacity(shots.len());

        for shot in shots {
            resolved_shots.push(self.resolve_shot(shot));
        }

        ResolvedScene::new(element, resolved_shots)
    }

    fn resolve_shot(&mut self, shot: LinkedShot) -> ResolvedShot {
        let (element, content, sound_effects) = shot.into_parts();
        let input = ElementInput::new(element);
        let (element, mut attributes) = input.into_resolved_parts();
        let duration = self.take_positive_duration(&mut attributes, "duration");
        attributes.reject_unknown(element.kind(), &mut self.diagnostics);
        let mut resolved_content = Vec::with_capacity(content.len());

        for content in content {
            resolved_content.push(self.resolve_content(content));
        }

        let sound_effects = sound_effects
            .into_iter()
            .filter_map(|audio| self.resolve_audio(audio))
            .collect();

        ResolvedShot::new(element, duration, resolved_content, sound_effects)
    }

    fn resolve_content(&mut self, content: LinkedShotContent) -> ResolvedShotContent {
        match content {
            LinkedShotContent::Video(video) => {
                ResolvedShotContent::Video(self.resolve_video(video))
            }
            LinkedShotContent::VoiceOver(voice_over) => {
                ResolvedShotContent::VoiceOver(self.resolve_voice_over(voice_over))
            }
            LinkedShotContent::Overlay(overlay) => {
                ResolvedShotContent::Overlay(self.resolve_overlay(overlay))
            }
        }
    }

    fn resolve_video(&mut self, video: LinkedVideo) -> ResolvedVideo {
        let media = self.resolve_media_attributes(video.into_element());
        ResolvedVideo::new(media.element, media.src, media.delay)
    }

    fn resolve_voice_over(&mut self, voice_over: LinkedVoiceOver) -> ResolvedVoiceOver {
        let (element, text) = voice_over.into_parts();
        let media = self.resolve_media_attributes(element);
        ResolvedVoiceOver::new(media.element, media.src, media.delay, resolve_text(text))
    }

    fn resolve_media_attributes(&mut self, element: LinkedElement) -> MediaAttributes {
        let input = ElementInput::new(element);
        let (element, mut attributes) = input.into_resolved_parts();
        let src = self.take_asset(&mut attributes, "src");
        let delay = self.take_duration(&mut attributes, "delay");
        attributes.reject_unknown(element.kind(), &mut self.diagnostics);
        MediaAttributes {
            element,
            src,
            delay,
        }
    }

    fn resolve_audio(&mut self, audio: LinkedAudio) -> Option<ResolvedAudio> {
        let (kind, element) = audio.into_parts();
        let input = ElementInput::new(element);
        let (element, mut attributes) = input.into_resolved_parts();
        let source_attribute = attributes.take("src");
        let delay_attribute = match kind {
            GeneralAudioKind::Music => None,
            GeneralAudioKind::SoundEffect => attributes.take("delay"),
        };
        let gain_attribute = attributes.take("gain");
        attributes.reject_unknown(element.kind(), &mut self.diagnostics);

        let source = self.resolve_required_asset(source_attribute, &element);
        let delay = match delay_attribute.as_ref() {
            Some(attribute) => self.resolve_duration(attribute).map(Some),
            None => Some(None),
        };
        let gain = match gain_attribute {
            Some(attribute) => self.resolve_audio_gain(&attribute),
            None => Some(AudioGain::UNITY),
        };
        let (Some(source), Some(delay), Some(gain)) = (source, delay, gain) else {
            return None;
        };

        Some(ResolvedAudio::new(kind, element, source, delay, gain))
    }

    fn resolve_audio_gain(&mut self, attribute: &Attribute) -> Option<AudioGain> {
        match AudioGain::parse_percentage(attribute.value()) {
            Ok(gain) => Some(gain),
            Err(reason) => {
                self.diagnostics.push(invalid_audio_gain(attribute, reason));
                None
            }
        }
    }

    fn resolve_required_asset(
        &mut self,
        attribute: Option<Attribute>,
        element: &ResolvedElement,
    ) -> Option<Authored<AssetRef>> {
        let Some(attribute) = attribute else {
            self.diagnostics
                .push(missing_attribute(element.kind(), "src", element.span()));
            return None;
        };

        self.resolve_asset(&attribute)
    }

    fn resolve_overlay(&mut self, overlay: LinkedOverlay) -> ResolvedOverlay {
        let (element, text) = overlay.into_parts();
        let input = ElementInput::new(element);
        let (element, mut attributes) = input.into_resolved_parts();
        let cue = attributes.take("cue");
        let delay = attributes.take("delay");
        attributes.reject_unknown(element.kind(), &mut self.diagnostics);
        let start = self.resolve_overlay_start(cue, delay);

        ResolvedOverlay::new(element, start, resolve_text(text))
    }

    fn resolve_overlay_start(
        &mut self,
        cue: Option<Attribute>,
        delay: Option<Attribute>,
    ) -> ResolvedStart {
        // Resolve both spellings before rejecting the competing rule so an
        // independent invalid value is not hidden behind the conflict.
        let cue_value = match cue.as_ref() {
            Some(attribute) => self.resolve_cue_reference(attribute),
            None => None,
        };
        let delay_value = match delay.as_ref() {
            Some(attribute) => self.resolve_duration(attribute),
            None => None,
        };

        match (cue, delay) {
            (Some(cue), Some(delay)) => {
                self.diagnostics.push(conflicting_attributes(&cue, &delay));
                ResolvedStart::ShotStart
            }
            (Some(_), None) => cue_value.map_or(ResolvedStart::ShotStart, ResolvedStart::Cue),
            (None, Some(_)) => delay_value.map_or(ResolvedStart::ShotStart, ResolvedStart::Delayed),
            (None, None) => ResolvedStart::ShotStart,
        }
    }

    fn resolve_id_only_element(&mut self, element: LinkedElement) -> ResolvedElement {
        let input = ElementInput::new(element);
        let (element, attributes) = input.into_resolved_parts();
        attributes.reject_unknown(element.kind(), &mut self.diagnostics);
        element
    }

    fn resolve_duration(&mut self, attribute: &Attribute) -> Option<Authored<Duration>> {
        self.resolve_duration_with(attribute, Duration::parse)
    }

    fn resolve_duration_with(
        &mut self,
        attribute: &Attribute,
        parse: fn(&str) -> Result<Duration, InvalidDuration>,
    ) -> Option<Authored<Duration>> {
        match parse(attribute.value()) {
            Ok(duration) => Some(Authored::new(duration, attribute.value_span())),
            Err(reason) => {
                self.diagnostics.push(invalid_duration(attribute, reason));
                None
            }
        }
    }

    fn take_duration(
        &mut self,
        attributes: &mut Attributes,
        name: &str,
    ) -> Option<Authored<Duration>> {
        let attribute = attributes.take(name)?;
        self.resolve_duration(&attribute)
    }

    fn take_positive_duration(
        &mut self,
        attributes: &mut Attributes,
        name: &str,
    ) -> Option<Authored<Duration>> {
        let attribute = attributes.take(name)?;
        self.resolve_duration_with(&attribute, Duration::parse_positive)
    }

    fn resolve_asset(&mut self, attribute: &Attribute) -> Option<Authored<AssetRef>> {
        match AssetRef::parse(attribute.value()) {
            Ok(asset) => Some(Authored::new(asset, attribute.value_span())),
            Err(reason) => {
                self.diagnostics
                    .push(invalid_asset_reference(attribute, reason));
                None
            }
        }
    }

    fn take_asset(
        &mut self,
        attributes: &mut Attributes,
        name: &str,
    ) -> Option<Authored<AssetRef>> {
        let attribute = attributes.take(name)?;
        self.resolve_asset(&attribute)
    }

    fn resolve_cue_reference(&mut self, attribute: &Attribute) -> Option<Authored<EventRef>> {
        let id = match CueId::parse(attribute.value()) {
            Ok(id) => id,
            Err(reason) => {
                self.diagnostics
                    .push(invalid_cue_reference(attribute, reason));
                return None;
            }
        };
        let Some(state) = self.cue_table.get_mut(&id) else {
            self.diagnostics.push(unknown_cue(attribute));
            return None;
        };

        state.used = true;
        Some(Authored::new(EventRef::Cue(id), attribute.value_span()))
    }

    fn report_unused_cues(&mut self) {
        for (id, state) in &self.cue_table {
            if !state.used {
                self.diagnostics.push(unused_cue(id, state.declared_at));
            }
        }
    }
}

/// Declaration facts retained until unused-cue reporting closes the phase.
struct CueState {
    declared_at: SourceSpan,
    used: bool,
}

/// Attributes shared by media-bearing elements after raw names are consumed.
struct MediaAttributes {
    element: ResolvedElement,
    src: Option<Authored<AssetRef>>,
    delay: Option<Authored<Duration>>,
}

/// Destructured linked element whose attributes still require typed resolution.
struct ElementInput {
    kind: ElementKind,
    id: LinkedId,
    attributes: Attributes,
    span: SourceSpan,
}

impl ElementInput {
    fn new(element: LinkedElement) -> Self {
        let (kind, id, attributes, span) = element.into_parts();
        Self {
            kind,
            id,
            attributes: Attributes(attributes),
            span,
        }
    }

    fn into_resolved_parts(self) -> (ResolvedElement, Attributes) {
        (
            ResolvedElement::new(self.kind, self.id.into_node_id(), self.span),
            self.attributes,
        )
    }
}

/// Consuming attribute bag: recognized names are removed exactly once.
struct Attributes(Vec<Attribute>);

impl Attributes {
    fn take(&mut self, name: &str) -> Option<Attribute> {
        let index = self.0.iter().position(|attribute| {
            attribute.name().prefix().is_none() && attribute.name().local() == name
        })?;
        Some(self.0.remove(index))
    }

    fn reject_unknown(self, kind: ElementKind, diagnostics: &mut Diagnostics) {
        for attribute in self.0 {
            diagnostics.push(unknown_attribute(&attribute, kind));
        }
    }
}

fn resolve_text(text: Vec<TextNode>) -> Vec<ResolvedText> {
    text.into_iter()
        .map(|text| {
            let (text, span) = text.into_parts();
            ResolvedText::new(text, span)
        })
        .collect()
}

fn resolved_node(node: LinkedNode) -> ResolvedNode {
    ResolvedNode::new(node.kind(), node.span())
}

fn invalid_duration(attribute: &Attribute, reason: InvalidDuration) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidDuration,
        attribute.value_span(),
        format!("duration \"{}\" is invalid: {reason}", attribute.value()),
        "use an exact duration such as 3s, 500ms, or 1.5s",
    )
}

fn invalid_audio_gain(attribute: &Attribute, reason: InvalidAudioGain) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidAttributeValue,
        attribute.value_span(),
        format!("audio gain \"{}\" is invalid: {reason}", attribute.value()),
        "use an exact linear gain from 0% through 100%",
    )
}

fn unknown_cue(attribute: &Attribute) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnknownCueReference,
        attribute.value_span(),
        format!(
            "cue \"{}\" is not declared as a resolved event",
            attribute.value()
        ),
        "declare this cue with a valid id and time, or use another cue",
    )
}

fn unused_cue(id: &CueId, primary: SourceSpan) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnusedCue,
        primary,
        format!("cue \"{id}\" is never referenced"),
        "remove the cue or reference it from an overlay",
    )
}

fn unknown_attribute(attribute: &Attribute, kind: ElementKind) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::UnknownAttribute,
        attribute.name().span(),
        format!(
            "attribute \"{}\" is not allowed on <{kind}>",
            attribute.name()
        ),
        format!("remove the \"{}\" attribute", attribute.name()),
    )
}

fn missing_attribute(kind: ElementKind, name: &str, primary: SourceSpan) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::MissingRequiredAttribute,
        primary,
        format!("element <{kind}> requires the \"{name}\" attribute"),
        format!("add {name}=\"...\" to <{kind}>"),
    )
}

fn invalid_asset_reference(attribute: &Attribute, reason: InvalidAssetRef) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidAttributeValue,
        attribute.value_span(),
        format!("attribute \"{}\" is invalid: {reason}", attribute.name()),
        format!(
            "provide a screenplay-relative path for \"{}\"",
            attribute.name()
        ),
    )
}

fn invalid_cue_reference(attribute: &Attribute, reason: InvalidNodeId) -> Diagnostic {
    author_diagnostic(
        DiagnosticCode::InvalidAttributeValue,
        attribute.value_span(),
        format!("attribute \"cue\" is invalid: {reason}"),
        "use a non-empty cue ID without ASCII whitespace",
    )
}

fn conflicting_attributes(first: &Attribute, second: &Attribute) -> Diagnostic {
    let (first, second) = if first.span() <= second.span() {
        (first, second)
    } else {
        (second, first)
    };
    author_diagnostic(
        DiagnosticCode::ConflictingAttributes,
        second.name().span(),
        format!(
            "attributes \"{}\" and \"{}\" define conflicting start rules",
            first.name(),
            second.name(),
        ),
        format!(
            "keep either \"{}\" or \"{}\", not both",
            first.name(),
            second.name()
        ),
    )
    .with_related(
        first.name().span(),
        format!("\"{}\" is first authored here", first.name()),
    )
    .expect("a formatted related message is non-blank")
}

#[cfg(test)]
mod tests {
    use crate::compiler;
    use crate::diagnostics::{Diagnostic, DiagnosticCode};
    use crate::model::SourceId;

    #[test]
    fn warnings_preserve_the_resolved_film() {
        let report = resolve_source(r#"<film><cues><cue id="unused" time="1s" /></cues></film>"#);
        let codes = report
            .diagnostics()
            .iter()
            .map(Diagnostic::code)
            .collect::<Vec<_>>();

        assert_eq!(codes, [DiagnosticCode::UnusedCue]);
        assert!(!report.diagnostics().has_errors());
        assert!(report.film().is_some());
    }

    #[test]
    fn missing_media_sources_remain_valid_for_static_analysis() {
        let report =
            resolve_source("<film><scene><shot><video/><vo>Narration</vo></shot></scene></film>");

        assert!(report.diagnostics().is_empty());
        assert!(report.film().is_some());
    }

    fn resolve_source(source: &str) -> super::ResolveReport {
        let parsed = compiler::parse(SourceId::new(0), source);
        let (document, syntax_diagnostics) = parsed.into_parts();
        assert!(syntax_diagnostics.is_empty());

        let bound = compiler::bind(document);
        let (film, binding_diagnostics) = bound.into_parts();
        assert!(binding_diagnostics.is_empty());

        super::resolve(film.expect("the fixture has one film"))
    }
}
