//! Stable structural-binding facts and authored diagnostics.

mod conformance;

use std::fmt::Write as _;
use std::fs;

use onmark_core::compiler::{
    self, LinkedAudio, LinkedCues, LinkedElement, LinkedFilm, LinkedScene, LinkedShot,
    LinkedShotContent,
};
use onmark_core::model::SourceId;

use conformance::{assert_or_update, fixture, render_diagnostics, span};

#[test]
fn the_gate_one_example_matches_canonical_binding() {
    assert_valid_fixture("gate-one");
}

#[test]
fn authored_general_audio_matches_canonical_binding() {
    assert_valid_fixture("general-audio");
}

#[test]
fn native_html_remains_presentation_owned_during_binding() {
    assert_valid_fixture("native-html");
}

#[test]
fn structural_errors_match_stable_diagnostics() {
    assert_invalid_fixture("structural-errors");
}

#[test]
fn root_errors_match_stable_diagnostics() {
    assert_invalid_fixture("root-errors");
}

fn assert_invalid_fixture(name: &str) {
    let source_path = fixture("binding", &format!("invalid/{name}.html"));
    let expected_path = fixture("binding", &format!("invalid/{name}.diagnostics.txt"));
    let source = fs::read_to_string(&source_path).expect("the binding fixture must be readable");
    let parsed = compiler::parse(SourceId::new(0), &source);
    let (document, syntax_diagnostics) = parsed.into_parts();

    assert!(syntax_diagnostics.is_empty());

    let report = compiler::bind(document);
    assert!(report.film().is_none());
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

fn assert_valid_fixture(name: &str) {
    let source_path = fixture("binding", &format!("valid/{name}.html"));
    let expected_path = fixture("binding", &format!("valid/{name}.linked.txt"));
    let source = fs::read_to_string(&source_path).expect("the binding fixture must be readable");
    let (document, diagnostics) = compiler::parse(SourceId::new(0), &source).into_parts();
    assert!(diagnostics.is_empty());
    let report = compiler::bind(document);

    assert!(report.diagnostics().is_empty());
    assert_or_update(
        &expected_path,
        &LinkedFilmRenderer::render(report.film().expect("the valid fixture must bind")),
    );
}

fn id(element: &LinkedElement) -> &str {
    element.id().map_or("-", onmark_core::model::NodeId::as_str)
}

struct LinkedFilmRenderer {
    output: String,
}

impl LinkedFilmRenderer {
    fn render(film: &LinkedFilm) -> String {
        let mut renderer = Self {
            output: String::from("# onmark binding test rendering; not a wire format\n"),
        };

        renderer
            .render_film(film)
            .expect("rendering into a String cannot fail");
        renderer.output
    }

    fn render_film(&mut self, film: &LinkedFilm) -> std::fmt::Result {
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

    fn render_cues(&mut self, cues: &LinkedCues) -> std::fmt::Result {
        writeln!(self.output, "  cues id={}", id(cues.element()))?;

        for cue in cues.cues() {
            writeln!(self.output, "    cue id={}", id(cue.element()))?;
        }

        Ok(())
    }

    fn render_scene(&mut self, scene: &LinkedScene) -> std::fmt::Result {
        writeln!(self.output, "  scene id={}", id(scene.element()))?;

        for shot in scene.shots() {
            self.render_shot(shot)?;
        }

        Ok(())
    }

    fn render_shot(&mut self, shot: &LinkedShot) -> std::fmt::Result {
        writeln!(self.output, "    shot id={}", id(shot.element()))?;

        for content in shot.content() {
            self.render_content(content)?;
        }

        for effect in shot.sound_effects() {
            self.render_audio(effect, "      ")?;
        }

        Ok(())
    }

    fn render_content(&mut self, content: &LinkedShotContent) -> std::fmt::Result {
        let element = content.element();

        writeln!(self.output, "      {} id={}", element.kind(), id(element))
    }

    fn render_audio(&mut self, audio: &LinkedAudio, indent: &str) -> std::fmt::Result {
        writeln!(
            self.output,
            "{indent}{} id={}",
            audio.element().kind(),
            id(audio.element()),
        )
    }

    fn render_index(&mut self, film: &LinkedFilm) -> std::fmt::Result {
        self.output.push_str("index\n");

        for (node_id, entry) in film.ids() {
            writeln!(
                self.output,
                "  {node_id} -> {} @{}",
                entry.kind(),
                span(entry.span()),
            )?;
        }

        Ok(())
    }
}
