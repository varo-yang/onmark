//! Stable typed-variant schemas, values, scopes, and authored diagnostics.

mod conformance;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

use onmark_core::compiler::{
    self, LinkedVariantScope, ResolvedFilm, ResolvedVariantValues, VariantBindingSink,
};
use onmark_core::model::{FrameRate, SourceId, Timebase, VariantValue};
use onmark_core::render_graph::RenderGraph;
use onmark_core::timeline::{TimelineIr, TimelineVariantScope};

use conformance::{assert_or_update, fixture, render_diagnostics, span};

#[test]
fn canonical_defaults_and_overrides_keep_exact_region_dependencies() {
    let source_path = fixture("compiler/variants", "valid/typed-variants.html");
    let variant_path = fixture("compiler/variants", "valid/typed-variants.json");
    let expected_path = fixture("compiler/variants", "valid/typed-variants.variant.txt");
    let source = fs::read_to_string(source_path).expect("the screenplay fixture must be readable");
    let variant =
        fs::read_to_string(variant_path).expect("the external variant fixture must be readable");
    let film = resolve_source(&source);
    let report = compiler::resolve_variant(film, SourceId::new(1), &variant);

    assert!(report.diagnostics().is_empty());
    let film = report
        .film()
        .expect("the valid variant must preserve the film");
    let timeline = solve(film.clone());
    let graph = RenderGraph::from_timeline(
        &timeline,
        onmark_core::model::PresentationTemporalCapability::RandomAccess,
    )
    .expect("the solved fixture has complete render ownership");
    let actual = VariantRenderer::render(film, &timeline, &graph);

    assert_or_update(&expected_path, &actual);
}

#[test]
fn declaration_and_binding_errors_match_stable_diagnostics() {
    let source_path = fixture(
        "compiler/variants",
        "invalid/declarations-and-bindings.html",
    );
    let expected_path = fixture(
        "compiler/variants",
        "invalid/declarations-and-bindings.diagnostics.txt",
    );
    let source = fs::read_to_string(source_path).expect("the screenplay fixture must be readable");
    let parsed = compiler::parse(SourceId::new(0), &source);
    let (document, syntax_diagnostics) = parsed.into_parts();
    assert!(syntax_diagnostics.is_empty());
    let bound = compiler::bind(document);
    let (film, binding_diagnostics) = bound.into_parts();
    assert!(binding_diagnostics.is_empty());
    let report = compiler::resolve(film.expect("the fixture is structurally valid"));

    assert!(report.film().is_none());
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

#[test]
fn external_value_errors_match_stable_diagnostics() {
    let source_path = fixture("compiler/variants", "valid/typed-variants.html");
    let variant_path = fixture("compiler/variants", "invalid/typed-variants.json");
    let expected_path = fixture(
        "compiler/variants",
        "invalid/typed-variants.diagnostics.txt",
    );
    let source = fs::read_to_string(source_path).expect("the screenplay fixture must be readable");
    let variant =
        fs::read_to_string(variant_path).expect("the external variant fixture must be readable");
    let film = resolve_source(&source);
    let report = compiler::resolve_variant(film, SourceId::new(1), &variant);

    assert!(report.film().is_none());
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

fn resolve_source(source: &str) -> ResolvedFilm {
    let (document, syntax_diagnostics) = compiler::parse(SourceId::new(0), source).into_parts();
    assert!(syntax_diagnostics.is_empty());
    let (film, binding_diagnostics) = compiler::bind(document).into_parts();
    assert!(binding_diagnostics.is_empty());
    let (film, resolution_diagnostics) =
        compiler::resolve(film.expect("the fixture has one film")).into_parts();
    assert!(!resolution_diagnostics.has_errors());
    film.expect("the valid fixture must resolve")
}

fn solve(film: ResolvedFilm) -> TimelineIr {
    let rate = FrameRate::new(30, 1).expect("30 fps is valid");
    let report = compiler::solve(film, &BTreeMap::new(), Timebase::new(rate))
        .expect("the fixture references no media");

    assert!(report.diagnostics().is_empty());
    report.into_parts().0.expect("the valid fixture must solve")
}

struct VariantRenderer {
    output: String,
}

impl VariantRenderer {
    fn render(film: &ResolvedFilm, timeline: &TimelineIr, graph: &RenderGraph) -> String {
        let mut renderer = Self {
            output: String::from("# onmark variant test rendering; not a wire format\n"),
        };
        renderer
            .render_schema(film)
            .expect("rendering into a String cannot fail");
        renderer
            .render_values(film.variant_values())
            .expect("rendering into a String cannot fail");
        renderer
            .render_timeline(timeline)
            .expect("rendering into a String cannot fail");
        renderer
            .render_graph(graph)
            .expect("rendering into a String cannot fail");
        renderer.output
    }

    fn render_schema(&mut self, film: &ResolvedFilm) -> std::fmt::Result {
        self.output.push_str("schema\n");
        for (_, field) in film.variants().fields() {
            writeln!(
                self.output,
                "  {} kind={} default={} @{}",
                field.name(),
                field.kind(),
                field.default(),
                span(field.declared_at()),
            )?;
        }
        self.output.push_str("bindings\n");
        for binding in film.variants().bindings() {
            writeln!(
                self.output,
                "  {} sink={} scope={} @{}",
                binding.field(),
                sink(binding.sink()),
                linked_scope(binding.scope()),
                span(binding.authored_at()),
            )?;
        }
        Ok(())
    }

    fn render_values(&mut self, values: &ResolvedVariantValues) -> std::fmt::Result {
        self.output.push_str("values\n");
        for (name, value) in values.iter() {
            writeln!(
                self.output,
                "  {name} kind={} value={}",
                value.kind(),
                quoted(value),
            )?;
        }
        Ok(())
    }

    fn render_timeline(&mut self, timeline: &TimelineIr) -> std::fmt::Result {
        self.output.push_str("timeline\n");
        for field in timeline.variants() {
            write!(
                self.output,
                "  {}={} scopes=",
                field.name(),
                quoted(field.value()),
            )?;
            for (index, scope) in field.scopes().iter().enumerate() {
                if index > 0 {
                    self.output.push(',');
                }
                write!(self.output, "{}", timeline_scope(*scope))?;
            }
            self.output.push('\n');
        }
        Ok(())
    }

    fn render_graph(&mut self, graph: &RenderGraph) -> std::fmt::Result {
        self.output.push_str("regions\n");
        for (index, region) in graph.regions().iter().enumerate() {
            write!(self.output, "  {index} fields=")?;
            for (field_index, field) in region.variant_fields().enumerate() {
                if field_index > 0 {
                    self.output.push(',');
                }
                write!(self.output, "{field}")?;
            }
            self.output.push('\n');
        }
        Ok(())
    }
}

fn sink(sink: VariantBindingSink) -> &'static str {
    match sink {
        VariantBindingSink::Text => "text",
        VariantBindingSink::Css => "css",
        VariantBindingSink::Show => "show",
    }
}

fn linked_scope(scope: LinkedVariantScope) -> String {
    match scope {
        LinkedVariantScope::Film => "film".to_owned(),
        LinkedVariantScope::Scene(span_value) => format!("scene@{}", span(span_value)),
        LinkedVariantScope::Shot(span_value) => format!("shot@{}", span(span_value)),
        LinkedVariantScope::Transition(span_value) => {
            format!("transition@{}", span(span_value))
        }
    }
}

fn timeline_scope(scope: TimelineVariantScope) -> String {
    match scope {
        TimelineVariantScope::Film => "film".to_owned(),
        TimelineVariantScope::Scene {
            first_shot,
            shot_count,
        } => format!("scene:{}+{shot_count}", first_shot.get()),
        TimelineVariantScope::Shot(shot) => format!("shot:{}", shot.get()),
        TimelineVariantScope::Transition { outgoing, incoming } => {
            format!("transition:{}+{}", outgoing.get(), incoming.get())
        }
    }
}

fn quoted(value: &VariantValue) -> String {
    match value {
        VariantValue::Text(value) => format!("{value:?}"),
        VariantValue::Integer(value) => value.to_string(),
        VariantValue::Boolean(value) => value.to_string(),
        VariantValue::Color(value) => value.to_string(),
    }
}
