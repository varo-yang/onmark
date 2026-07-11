use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use onmark_core::compiler;
use onmark_core::diagnostics::{Diagnostic, Diagnostics};
use onmark_core::model::{SourceId, SourceSpan};
use onmark_core::syntax::{Node, SourceDocument};

#[test]
fn valid_source_matches_canonical_syntax_rendering() {
    let source_path = fixture("valid/minimal.onmark");
    let expected_path = fixture("valid/minimal.ast.txt");
    let source =
        fs::read_to_string(&source_path).expect("the valid syntax fixture must be readable");
    let report = compiler::parse(SourceId::new(0), &source);

    assert!(report.diagnostics().is_empty());
    assert_or_update(&expected_path, &render_document(report.document()));
}

#[test]
fn invalid_source_matches_stable_diagnostics() {
    assert_invalid_fixture("structural-errors");
}

#[test]
fn nested_unclosed_elements_match_stable_diagnostics() {
    assert_invalid_fixture("nested-unclosed-elements");
}

#[test]
fn a_doctype_internal_subset_produces_one_diagnostic() {
    assert_invalid_fixture("doctype-internal-subset");
}

fn assert_invalid_fixture(name: &str) {
    let source_path = fixture(&format!("invalid/{name}.onmark"));
    let expected_path = fixture(&format!("invalid/{name}.diagnostics.txt"));
    let source =
        fs::read_to_string(&source_path).expect("the invalid syntax fixture must be readable");
    let report = compiler::parse(SourceId::new(0), &source);

    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance/syntax")
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

fn render_document(document: &SourceDocument) -> String {
    let mut output = String::from("# onmark syntax test rendering; not a wire format\ndocument\n");

    for node in document.nodes() {
        render_node(&mut output, node, 1).expect("rendering into a String cannot fail");
    }

    output
}

fn render_node(output: &mut String, node: &Node, depth: usize) -> std::fmt::Result {
    let indent = "  ".repeat(depth);

    match node {
        Node::Element(element) => {
            writeln!(
                output,
                "{indent}element {} @{}",
                element.name(),
                span(element.span()),
            )?;

            for attribute in element.attributes() {
                writeln!(
                    output,
                    "{indent}  attribute {}=\"{}\" @{} value@{}",
                    attribute.name(),
                    attribute.value().escape_default(),
                    span(attribute.span()),
                    span(attribute.value_span()),
                )?;
            }

            for child in element.children() {
                render_node(output, child, depth + 1)?;
            }
        }
        Node::Text(text) => {
            writeln!(
                output,
                "{indent}text \"{}\" @{}",
                text.text().escape_default(),
                span(text.span()),
            )?;
        }
    }

    Ok(())
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
