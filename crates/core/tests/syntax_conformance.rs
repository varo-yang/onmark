mod conformance;

use std::fmt::Write as _;
use std::fs;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Node, SourceDocument};

use conformance::{assert_or_update, fixture, render_diagnostics, span};

#[test]
fn valid_source_matches_canonical_syntax_rendering() {
    let source_path = fixture("syntax", "valid/minimal.onmark");
    let expected_path = fixture("syntax", "valid/minimal.ast.txt");
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

#[test]
fn an_unclosed_doctype_cannot_hide_following_markup() {
    assert_invalid_fixture("unclosed-doctype");
}

fn assert_invalid_fixture(name: &str) {
    let source_path = fixture("syntax", &format!("invalid/{name}.onmark"));
    let expected_path = fixture("syntax", &format!("invalid/{name}.diagnostics.txt"));
    let source =
        fs::read_to_string(&source_path).expect("the invalid syntax fixture must be readable");
    let report = compiler::parse(SourceId::new(0), &source);

    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
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
