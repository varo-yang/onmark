use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use onmark_core::compiler::{self, LinkedElement, LinkedFilm, LinkedShotContent};
use onmark_core::diagnostics::{Diagnostic, Diagnostics};
use onmark_core::model::{SourceId, SourceSpan};
use onmark_core::syntax::TextNode;

#[test]
fn the_gate_one_example_matches_canonical_binding() {
    let source_path = fixture("valid/gate-one.onmark");
    let expected_path = fixture("valid/gate-one.linked.txt");
    let source = fs::read_to_string(&source_path).expect("the binding fixture must be readable");
    let parsed = compiler::parse(SourceId::new(0), &source);
    let (document, syntax_diagnostics) = parsed.into_parts();

    assert!(syntax_diagnostics.is_empty());

    let report = compiler::bind(document);
    let film = report.film().expect("the valid fixture must bind one film");

    assert!(report.diagnostics().is_empty());
    assert_or_update(&expected_path, &render_film(film));
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
    let source_path = fixture(&format!("invalid/{name}.onmark"));
    let expected_path = fixture(&format!("invalid/{name}.diagnostics.txt"));
    let source = fs::read_to_string(&source_path).expect("the binding fixture must be readable");
    let parsed = compiler::parse(SourceId::new(0), &source);
    let (document, syntax_diagnostics) = parsed.into_parts();

    assert!(syntax_diagnostics.is_empty());

    let report = compiler::bind(document);
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/binding")
        .join(relative)
}

fn assert_or_update(path: &Path, actual: &str) {
    if std::env::var_os("ONMARK_UPDATE_GOLDENS").is_some() {
        fs::write(path, actual).expect("the generated golden artifact must be writable");
        return;
    }

    let expected = fs::read_to_string(path).expect(
        "the golden artifact must exist; regenerate with ONMARK_UPDATE_GOLDENS=1 cargo test",
    );
    assert_eq!(actual, expected, "golden mismatch for {}", path.display());
}

fn render_film(film: &LinkedFilm) -> String {
    let mut output = String::from("# onmark binding test rendering; not a wire format\n");
    writeln!(output, "film id={}", id(film.element()))
        .expect("rendering into a String cannot fail");
    render_attributes(&mut output, film.element(), 1);

    if let Some(cues) = film.cues() {
        writeln!(output, "  cues id={}", id(cues.element()))
            .expect("rendering into a String cannot fail");
        render_attributes(&mut output, cues.element(), 2);
        for cue in cues.cues() {
            writeln!(output, "    cue id={}", id(cue.element()))
                .expect("rendering into a String cannot fail");
            render_attributes(&mut output, cue.element(), 3);
        }
    }

    for scene in film.scenes() {
        writeln!(output, "  scene id={}", id(scene.element()))
            .expect("rendering into a String cannot fail");
        render_attributes(&mut output, scene.element(), 2);
        for shot in scene.shots() {
            writeln!(output, "    shot id={}", id(shot.element()))
                .expect("rendering into a String cannot fail");
            render_attributes(&mut output, shot.element(), 3);
            for content in shot.content() {
                let (element, text): (&LinkedElement, &[TextNode]) = match content {
                    LinkedShotContent::Video(video) => {
                        writeln!(output, "      video id={}", id(video.element()))
                            .expect("rendering into a String cannot fail");
                        (video.element(), &[])
                    }
                    LinkedShotContent::VoiceOver(voice_over) => {
                        writeln!(output, "      vo id={}", id(voice_over.element()))
                            .expect("rendering into a String cannot fail");
                        (voice_over.element(), voice_over.text())
                    }
                    LinkedShotContent::Overlay(overlay) => {
                        writeln!(
                            output,
                            "      {} id={}",
                            overlay.element().kind(),
                            id(overlay.element()),
                        )
                        .expect("rendering into a String cannot fail");
                        (overlay.element(), overlay.text())
                    }
                };
                render_attributes(&mut output, element, 4);
                render_text(&mut output, text, 4);
            }
        }
    }

    output.push_str("index\n");
    for (node_id, entry) in film.ids() {
        writeln!(
            output,
            "  {node_id} -> {} @{}",
            entry.kind(),
            span(entry.span())
        )
        .expect("rendering into a String cannot fail");
    }

    output
}

fn render_attributes(output: &mut String, element: &LinkedElement, depth: usize) {
    let indent = "  ".repeat(depth);
    for attribute in element.attributes() {
        writeln!(
            output,
            "{indent}attribute {}=\"{}\" @{} value@{}",
            attribute.name(),
            attribute.value().escape_default(),
            span(attribute.span()),
            span(attribute.value_span()),
        )
        .expect("rendering into a String cannot fail");
    }
}

fn render_text(output: &mut String, nodes: &[TextNode], depth: usize) {
    let indent = "  ".repeat(depth);
    for text in nodes {
        writeln!(
            output,
            "{indent}text \"{}\" @{}",
            text.text().escape_default(),
            span(text.span()),
        )
        .expect("rendering into a String cannot fail");
    }
}

fn id(element: &LinkedElement) -> &str {
    element.id().map_or("-", onmark_core::model::NodeId::as_str)
}

fn render_diagnostics(diagnostics: &Diagnostics) -> String {
    let mut output = String::from("# onmark diagnostic test rendering; not a wire format\n");

    for diagnostic in diagnostics.iter() {
        render_diagnostic(&mut output, diagnostic)
            .expect("rendering diagnostics into a String cannot fail");
    }

    output
}

fn render_diagnostic(output: &mut String, diagnostic: &Diagnostic) -> std::fmt::Result {
    writeln!(
        output,
        "{} {} @{} {}",
        diagnostic.code(),
        diagnostic.severity(),
        span(diagnostic.primary()),
        diagnostic.message(),
    )?;

    if let Some(help) = diagnostic.help() {
        writeln!(output, "  help: {help}")?;
    }

    for related in diagnostic.related() {
        writeln!(
            output,
            "  related @{} {}",
            span(related.span()),
            related.message(),
        )?;
    }

    Ok(())
}

fn span(span: SourceSpan) -> String {
    format!(
        "{}:{}..{}",
        span.source().get(),
        span.start().get(),
        span.end().get(),
    )
}
