//! Stable typed-attribute and reference-resolution facts.

mod conformance;

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use onmark_core::compiler::{
    self, Authored, ResolvedAudio, ResolvedCues, ResolvedElement, ResolvedFilm, ResolvedOverlay,
    ResolvedScene, ResolvedShot, ResolvedShotContent, ResolvedStart, ResolvedText, ResolvedVideo,
    ResolvedVoiceOver,
};
use onmark_core::model::{AssetRef, Duration, EventRef, SourceId};

use conformance::{assert_or_update, fixture, render_diagnostics, span};

#[test]
fn the_gate_one_example_matches_canonical_resolution() {
    assert_valid_fixture("gate-one");
}

#[test]
fn authored_values_match_canonical_resolution() {
    assert_valid_fixture("authored-values");
}

#[test]
fn authored_general_audio_matches_canonical_resolution() {
    assert_valid_fixture("general-audio");
}

#[test]
fn attribute_and_reference_errors_match_stable_diagnostics() {
    let source_path = fixture("resolution", "invalid/attribute-errors.onmark");
    let expected_path = fixture("resolution", "invalid/attribute-errors.diagnostics.txt");
    let report = resolve_fixture(&source_path);

    assert!(report.film().is_none());
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

#[test]
fn authored_audio_errors_match_stable_diagnostics() {
    let source_path = fixture("resolution", "invalid/audio-errors.onmark");
    let expected_path = fixture("resolution", "invalid/audio-errors.diagnostics.txt");
    let report = resolve_fixture(&source_path);

    assert!(report.film().is_none());
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

#[test]
fn deferred_timing_attributes_remain_outside_gate_one() {
    let source_path = fixture("resolution", "invalid/deferred-timing-attributes.onmark");
    let expected_path = fixture(
        "resolution",
        "invalid/deferred-timing-attributes.diagnostics.txt",
    );
    let report = resolve_fixture(&source_path);

    assert!(report.film().is_none());
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

fn assert_valid_fixture(name: &str) {
    let source_path = fixture("resolution", &format!("valid/{name}.onmark"));
    let expected_path = fixture("resolution", &format!("valid/{name}.resolved.txt"));
    let report = resolve_fixture(&source_path);

    assert!(report.diagnostics().is_empty());
    assert_or_update(
        &expected_path,
        &ResolvedFilmRenderer::render(report.film().expect("the valid fixture must resolve")),
    );
}

fn resolve_fixture(path: &Path) -> compiler::ResolveReport {
    let source = fs::read_to_string(path).expect("the resolution fixture must be readable");
    let parsed = compiler::parse(SourceId::new(0), &source);
    let (document, syntax_diagnostics) = parsed.into_parts();
    assert!(syntax_diagnostics.is_empty());

    let bound = compiler::bind(document);
    let (film, binding_diagnostics) = bound.into_parts();
    assert!(binding_diagnostics.is_empty());

    compiler::resolve(film.expect("the fixture has one structurally valid film"))
}

fn id(element: &ResolvedElement) -> &str {
    element.id().map_or("-", onmark_core::model::NodeId::as_str)
}

fn duration(value: Option<&Authored<Duration>>) -> String {
    value.map_or_else(|| "-".to_owned(), |value| value.value().to_string())
}

fn asset(value: Option<&Authored<AssetRef>>) -> &str {
    value.map_or("-", |value| value.value().as_str())
}

fn start(value: &ResolvedStart) -> String {
    match value {
        ResolvedStart::ShotStart => "shot-start".to_owned(),
        ResolvedStart::Delayed(delay) => format!("delay:{}", delay.value()),
        ResolvedStart::Cue(event) => match event.value() {
            EventRef::Cue(id) => format!("cue:{id}"),
        },
    }
}

struct ResolvedFilmRenderer {
    output: String,
}

impl ResolvedFilmRenderer {
    fn render(film: &ResolvedFilm) -> String {
        let mut renderer = Self {
            output: String::from("# onmark resolution test rendering; not a wire format\n"),
        };

        renderer
            .render_film(film)
            .expect("rendering into a String cannot fail");
        renderer.output
    }

    fn render_film(&mut self, film: &ResolvedFilm) -> std::fmt::Result {
        writeln!(self.output, "film id={}", id(film.element()))?;

        if let Some(cues) = film.cues() {
            self.render_cues(cues)?;
        }

        for music in film.music() {
            self.render_audio(music, "  ")?;
        }

        for scene in film.scenes() {
            self.render_scene(scene)?;
        }

        self.render_index(film)
    }

    fn render_cues(&mut self, cues: &ResolvedCues) -> std::fmt::Result {
        writeln!(self.output, "  cues id={}", id(cues.element()))?;

        for cue in cues.cues() {
            writeln!(
                self.output,
                "    cue id={} id@{} time={} time@{}",
                cue.id().value(),
                span(cue.id().span()),
                cue.time().value(),
                span(cue.time().span()),
            )?;
        }

        Ok(())
    }

    fn render_scene(&mut self, scene: &ResolvedScene) -> std::fmt::Result {
        writeln!(self.output, "  scene id={}", id(scene.element()))?;

        for shot in scene.shots() {
            self.render_shot(shot)?;
        }

        Ok(())
    }

    fn render_shot(&mut self, shot: &ResolvedShot) -> std::fmt::Result {
        writeln!(
            self.output,
            "    shot id={} duration={}",
            id(shot.element()),
            duration(shot.duration()),
        )?;

        for content in shot.content() {
            self.render_content(content)?;
        }

        for effect in shot.sound_effects() {
            self.render_audio(effect, "      ")?;
        }

        Ok(())
    }

    fn render_content(&mut self, content: &ResolvedShotContent) -> std::fmt::Result {
        match content {
            ResolvedShotContent::Video(video) => self.render_video(video),
            ResolvedShotContent::VoiceOver(voice_over) => self.render_voice_over(voice_over),
            ResolvedShotContent::Overlay(overlay) => self.render_overlay(overlay),
        }
    }

    fn render_video(&mut self, video: &ResolvedVideo) -> std::fmt::Result {
        writeln!(
            self.output,
            "      video id={} src={} delay={}",
            id(video.element()),
            asset(video.src()),
            duration(video.delay()),
        )
    }

    fn render_audio(&mut self, audio: &ResolvedAudio, indent: &str) -> std::fmt::Result {
        writeln!(
            self.output,
            "{indent}{} id={} src={} delay={} gain={}/{}",
            audio.element().kind(),
            id(audio.element()),
            audio.src().value(),
            duration(audio.delay()),
            audio.gain().numerator(),
            audio.gain().denominator(),
        )
    }

    fn render_voice_over(&mut self, voice_over: &ResolvedVoiceOver) -> std::fmt::Result {
        writeln!(
            self.output,
            "      vo id={} src={} delay={}",
            id(voice_over.element()),
            asset(voice_over.src()),
            duration(voice_over.delay()),
        )?;

        self.render_text(voice_over.text())
    }

    fn render_overlay(&mut self, overlay: &ResolvedOverlay) -> std::fmt::Result {
        writeln!(
            self.output,
            "      {} id={} start={}",
            overlay.element().kind(),
            id(overlay.element()),
            start(overlay.start()),
        )?;

        self.render_text(overlay.text())
    }

    fn render_text(&mut self, text: &[ResolvedText]) -> std::fmt::Result {
        for text in text {
            writeln!(
                self.output,
                "        text \"{}\" @{}",
                text.text().escape_default(),
                span(text.span()),
            )?;
        }

        Ok(())
    }

    fn render_index(&mut self, film: &ResolvedFilm) -> std::fmt::Result {
        self.output.push_str("index\n");

        for (node_id, node) in film.ids() {
            writeln!(
                self.output,
                "  {node_id} -> {} @{}",
                node.kind(),
                span(node.declared_at()),
            )?;
        }

        Ok(())
    }
}
