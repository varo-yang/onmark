//! Offline grading for the checked-in typed-variant authoring experiment.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Attribute, Element, Node};
use serde::Deserialize;
use serde_json::{Map, Value};

const EVALUATION: &str = "evals/typed-variants";
const ADMITTED_ARM: &str = "declarative-bindings";
const RUNS_PER_ARM: usize = 2;

// ── Grading pipeline

pub(super) fn grade(repository: &Path) -> Result<(), Box<dyn Error>> {
    let evaluation = repository.join(EVALUATION);
    let cases: CaseSet = read_json(&evaluation.join("cases.json"))?;
    let baseline: Baseline = read_json(&evaluation.join("baseline.json"))?;
    let expected = cases
        .cases
        .into_iter()
        .map(|case| case.id)
        .collect::<BTreeSet<_>>();
    let mut scores = BTreeMap::new();
    let mut failures = Vec::new();

    for arm in Arm::ALL {
        let mut score = Score::default();
        for run in 1..=RUNS_PER_ARM {
            let filename = format!("{}-run-{run}.json", arm.filename());
            let output: ModelOutput = read_json(&evaluation.join("raw").join(&filename))?;
            grade_output(
                arm,
                run,
                &filename,
                output,
                &expected,
                &mut score,
                &mut failures,
            );
        }
        scores.insert(arm.filename(), score);
    }

    compare_baseline(&scores, &baseline, &mut failures);
    if !failures.is_empty() {
        return Err(Box::new(GradingFailed(failures)));
    }

    for (arm, score) in scores {
        println!(
            "{arm}: {}/{}; {} authored bytes",
            score.passed, score.total, score.authored_bytes,
        );
    }
    println!("admitted: {}", baseline.admitted);
    Ok(())
}

fn grade_output(
    arm: Arm,
    run: usize,
    filename: &str,
    output: ModelOutput,
    expected: &BTreeSet<String>,
    score: &mut Score,
    failures: &mut Vec<String>,
) {
    let mut seen = BTreeSet::new();

    for result in output.cases {
        score.total += 1;
        score.authored_bytes += result.film_html.len() + result.variant_json.len();

        if !expected.contains(&result.id) {
            failures.push(format!("{filename}: unknown case {}", result.id));
            continue;
        }
        if !seen.insert(result.id.clone()) {
            failures.push(format!("{filename}: duplicate case {}", result.id));
            continue;
        }
        if case_is_valid(arm, &result) {
            score.passed += 1;
        } else {
            score
                .failed_cases
                .insert(format!("run-{run}:{}", result.id));
        }
    }

    if &seen != expected {
        failures.push(format!(
            "{filename}: expected cases {expected:?}, found {seen:?}"
        ));
    }
}

// ── Candidate semantics

fn case_is_valid(arm: Arm, result: &ModelCase) -> bool {
    let Some(case) = ExpectedCase::get(&result.id) else {
        return false;
    };
    let Ok(variant) = serde_json::from_str::<Map<String, Value>>(&result.variant_json) else {
        return false;
    };
    if variant != case.variant {
        return false;
    }

    let Some(root) = parse_film(&result.film_html) else {
        return false;
    };
    if has_forbidden_authoring(&result.film_html)
        || !case.videos.matches(&video_sources(&root))
        || !arm.has_valid_surface(&root, &result.film_html, &case)
    {
        return false;
    }

    case.has_required_content(&result.film_html)
}

fn parse_film(source: &str) -> Option<Element> {
    let report = compiler::parse(SourceId::new(0), source);
    let (document, diagnostics) = report.into_parts();
    if !diagnostics.is_empty() {
        return None;
    }

    let mut root = None;
    for node in document.nodes() {
        match node {
            Node::Element(element) if root.is_none() => root = Some(element.clone()),
            Node::Text(text) if text.text().trim().is_empty() => {}
            Node::Element(_) | Node::Text(_) => return None,
        }
    }
    let root = root?;
    if root.name().local() != "om-film" {
        return None;
    }
    Some(root)
}

fn has_forbidden_authoring(source: &str) -> bool {
    [
        " start=\"",
        " begin=\"",
        " end=\"",
        " until=\"",
        " track=\"",
        "innerHTML",
        "insertAdjacentHTML",
    ]
    .iter()
    .any(|needle| source.contains(needle))
}

fn video_sources(root: &Element) -> Vec<String> {
    descendants(root)
        .filter(|element| element.name().local() == "video")
        .filter_map(|video| attribute(video, "src"))
        .map(str::to_owned)
        .collect()
}

impl Arm {
    fn has_valid_surface(self, root: &Element, source: &str, case: &ExpectedCase) -> bool {
        match self {
            Self::Declarative => valid_declarative(root, source, case),
            Self::Module => valid_module(root, source, case),
            Self::Placeholder => valid_placeholders(root, source, case),
        }
    }
}

fn valid_declarative(root: &Element, source: &str, case: &ExpectedCase) -> bool {
    valid_field_declarations(root, case)
        && field_placements(root, BindingSyntax::Declarative) == case.placements
        && valid_authored_defaults(root, source, case, BindingSyntax::Declarative)
}

fn valid_module(root: &Element, source: &str, case: &ExpectedCase) -> bool {
    let Some(module) = direct_children(root)
        .find(|element| element.name().local() == "script")
        .filter(|element| {
            attribute(element, "type") == Some("module")
                && attribute(element, "data-om-motion").is_some()
        })
    else {
        return false;
    };
    let code = compact(&element_text(module));
    let schema_is_complete = case.fields.iter().all(|field| {
        code.contains(&format!(
            "{}:{}({})",
            field.name,
            field.kind.constructor(),
            field.kind.javascript_literal(field.default),
        ))
    });
    let values_are_used = case
        .fields
        .iter()
        .all(|field| code.contains(&format!("values.{}", field.name)));

    source.contains("from \"onmark/variant\"")
        && code.contains("defineVariant({fields:")
        && code.contains("bind({document,values})")
        && schema_is_complete
        && values_are_used
        && valid_authored_defaults(root, source, case, BindingSyntax::Module)
        && valid_module_writes(&code, case)
}

fn valid_module_writes(code: &str, case: &ExpectedCase) -> bool {
    let text_fields_are_safe = case
        .fields
        .iter()
        .filter(|field| field.kind == FieldKind::Text)
        .all(|field| code.contains(&format!(".textContent=values.{}", field.name)));
    let boolean_fields_are_safe = case
        .fields
        .iter()
        .filter(|field| field.kind == FieldKind::Boolean)
        .all(|field| code.contains(&format!(".hidden=!values.{}", field.name)));
    let style_fields_are_bound = case
        .fields
        .iter()
        .filter(|field| matches!(field.kind, FieldKind::Color | FieldKind::Integer))
        .all(|field| {
            code.contains(&format!("setProperty(\"--{}\",", field.name))
                && code.contains(&format!("values.{}", field.name))
        });

    text_fields_are_safe && boolean_fields_are_safe && style_fields_are_bound
}

fn valid_placeholders(root: &Element, source: &str, case: &ExpectedCase) -> bool {
    valid_field_declarations(root, case)
        && case.fields.iter().all(|field| {
            let placeholder = format!("{{{{{}}}}}", field.name);
            let conditional = format!("{{{{#if {}}}}}", field.name);
            match field.kind {
                FieldKind::Boolean => source.contains(&conditional) && source.contains("{{/if}}"),
                FieldKind::Text | FieldKind::Color | FieldKind::Integer => {
                    source.contains(&placeholder)
                }
            }
        })
        && !source.contains("</om-title><script>alert(1)</script>")
}

fn valid_authored_defaults(
    root: &Element,
    source: &str,
    case: &ExpectedCase,
    syntax: BindingSyntax,
) -> bool {
    let compacted = compact(source);
    case.fields.iter().all(|field| match field.kind {
        FieldKind::Text => syntax.text_default_is_authored(root, field),
        FieldKind::Boolean => syntax.boolean_default_is_authored(root, field),
        FieldKind::Color | FieldKind::Integer => {
            compacted.contains(&format!("--{}:{}", field.name, field.default))
        }
    })
}

fn valid_field_declarations(root: &Element, case: &ExpectedCase) -> bool {
    let mut containers =
        direct_children(root).filter(|element| element.name().local() == "om-fields");
    let Some(fields) = containers.next() else {
        return false;
    };
    if containers.next().is_some() {
        return false;
    }

    let actual = direct_children(fields)
        .map(field_declaration)
        .collect::<Option<BTreeMap<_, _>>>();
    let expected = case
        .fields
        .iter()
        .map(|field| {
            (
                field.name.to_owned(),
                (field.kind.spelling().to_owned(), field.default.to_owned()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    actual == Some(expected)
}

fn field_declaration(element: &Element) -> Option<(String, (String, String))> {
    if element.name().local() != "om-field" || !element.children().is_empty() {
        return None;
    }
    let attributes = attributes(element);
    if attributes.len() != 3 {
        return None;
    }
    Some((
        attributes.get("name")?.to_string(),
        (
            attributes.get("type")?.to_string(),
            attributes.get("default")?.to_string(),
        ),
    ))
}

fn field_placements(root: &Element, syntax: BindingSyntax) -> BTreeMap<String, usize> {
    descendants(root)
        .filter(|element| element.name().local() == "om-shot")
        .enumerate()
        .flat_map(|(index, shot)| {
            syntax
                .fields(shot)
                .into_iter()
                .map(move |field| (field, index))
        })
        .collect()
}

impl BindingSyntax {
    fn fields(self, shot: &Element) -> BTreeSet<String> {
        let mut fields = BTreeSet::new();
        for element in descendants(shot) {
            for name in ["data-om-text", "data-om-show"] {
                if let Some(field) = attribute(element, name) {
                    fields.insert(field.to_owned());
                }
            }
            if let Some(bound) = attribute(element, "data-om-css") {
                fields.extend(bound.split_ascii_whitespace().map(str::to_owned));
            }
        }
        fields
    }

    fn text_default_is_authored(self, root: &Element, field: &Field) -> bool {
        match self {
            Self::Declarative => descendants(root).any(|element| {
                attribute(element, "data-om-text") == Some(field.name)
                    && direct_text(element).contains(field.default)
            }),
            Self::Module => descendants(root).any(|element| {
                element.name().local() != "script" && direct_text(element).contains(field.default)
            }),
        }
    }

    fn boolean_default_is_authored(self, root: &Element, field: &Field) -> bool {
        let target = match self {
            Self::Declarative => descendants(root)
                .find(|element| attribute(element, "data-om-show") == Some(field.name)),
            Self::Module => descendants(root).find(|element| {
                element.name().local() != "script"
                    && direct_text(element).contains(field.target_text)
            }),
        };
        let Some(target) = target else {
            return false;
        };
        let hidden = attribute(target, "hidden").is_some();
        hidden == (field.default == "false")
    }
}

// ── Evaluation expectations

struct ExpectedCase {
    videos: VideoExpectation,
    fields: Vec<Field>,
    variant: Map<String, Value>,
    placements: BTreeMap<String, usize>,
    required_content: Vec<&'static str>,
}

impl ExpectedCase {
    fn get(id: &str) -> Option<Self> {
        match id {
            "product-offer" => Some(Self::new(
                &["media/product.mp4"],
                &[
                    Field::text("headline", "Summer edit"),
                    Field::color("accent", "#ff4d36"),
                    Field::integer("progress", "72"),
                    Field::boolean("featured", "false", "Featured"),
                ],
                &[
                    ("headline", json_string("Night edition")),
                    ("accent", json_string("#72f1b8")),
                    ("progress", Value::from(88)),
                    ("featured", Value::from(true)),
                ],
                &[
                    ("headline", 0),
                    ("accent", 0),
                    ("progress", 0),
                    ("featured", 0),
                ],
                &["Featured", "--accent", "--progress", "var(--accent)"],
            )),
            "regional-copy" => Some(Self::new(
                &["media/hello.mp4", "media/offer.mp4"],
                &[
                    Field::text("greeting", "Hello"),
                    Field::text("offer", "Save 20%"),
                ],
                &[("offer", json_string("Save 30%"))],
                &[("greeting", 0), ("offer", 1)],
                &[],
            )),
            "boolean-badge" => Some(Self::new(
                &["media/status.mp4"],
                &[
                    Field::text("status", "Ready"),
                    Field::boolean("showStatus", "true", "Ready"),
                ],
                &[
                    ("status", json_string("Rendering")),
                    ("showStatus", Value::from(false)),
                ],
                &[("status", 0), ("showStatus", 0)],
                &[],
            )),
            "rename-field" => Some(Self::new(
                VideoExpectation::AnySingle,
                &[Field::text("headline", "Exact video")],
                &[("headline", json_string("Exact variants"))],
                &[("headline", 0)],
                &[],
            )),
            "add-local-field" => Some(Self::new(
                &["media/a.mp4", "media/b.mp4"],
                &[Field::text("legal", "Terms apply")],
                &[("legal", json_string("Limited regions"))],
                &[("legal", 1)],
                &[],
            )),
            "literal-untrusted-text" => Some(Self::new(
                &["media/safe.mp4"],
                &[Field::text("message", "Safe text")],
                &[(
                    "message",
                    json_string("</om-title><script>alert(1)</script>"),
                )],
                &[("message", 0)],
                &[],
            )),
            _ => None,
        }
    }

    fn new(
        videos: impl Into<VideoExpectation>,
        fields: &[Field],
        variant: &[(&str, Value)],
        placements: &[(&str, usize)],
        required_content: &[&'static str],
    ) -> Self {
        Self {
            videos: videos.into(),
            fields: fields.to_vec(),
            variant: variant
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect(),
            placements: placements
                .iter()
                .map(|(name, index)| ((*name).to_owned(), *index))
                .collect(),
            required_content: required_content.to_vec(),
        }
    }

    fn has_required_content(&self, source: &str) -> bool {
        self.required_content
            .iter()
            .all(|needle| source.contains(needle))
    }
}

enum VideoExpectation {
    Exact(Vec<String>),
    AnySingle,
}

impl VideoExpectation {
    fn matches(&self, actual: &[String]) -> bool {
        match self {
            Self::Exact(expected) => actual == expected,
            Self::AnySingle => actual.len() == 1,
        }
    }
}

impl From<&[&str]> for VideoExpectation {
    fn from(values: &[&str]) -> Self {
        Self::Exact(values.iter().map(|value| (*value).to_owned()).collect())
    }
}

impl<const N: usize> From<&[&str; N]> for VideoExpectation {
    fn from(values: &[&str; N]) -> Self {
        Self::from(values.as_slice())
    }
}

#[derive(Clone)]
struct Field {
    name: &'static str,
    kind: FieldKind,
    default: &'static str,
    target_text: &'static str,
}

impl Field {
    const fn text(name: &'static str, default: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::Text,
            default,
            target_text: default,
        }
    }

    const fn integer(name: &'static str, default: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::Integer,
            default,
            target_text: "",
        }
    }

    const fn color(name: &'static str, default: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::Color,
            default,
            target_text: "",
        }
    }

    const fn boolean(name: &'static str, default: &'static str, target_text: &'static str) -> Self {
        Self {
            name,
            kind: FieldKind::Boolean,
            default,
            target_text,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FieldKind {
    Text,
    Integer,
    Color,
    Boolean,
}

impl FieldKind {
    const fn spelling(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Color => "color",
            Self::Boolean => "boolean",
        }
    }

    const fn constructor(self) -> &'static str {
        match self {
            Self::Text => "textField",
            Self::Integer => "integerField",
            Self::Color => "colorField",
            Self::Boolean => "booleanField",
        }
    }

    fn javascript_literal(self, value: &str) -> String {
        match self {
            Self::Text | Self::Color => format!("\"{value}\""),
            Self::Integer | Self::Boolean => value.to_owned(),
        }
    }
}

fn json_string(value: &str) -> Value {
    Value::String(value.to_owned())
}

// ── Syntax helpers

fn direct_children(element: &Element) -> impl DoubleEndedIterator<Item = &Element> {
    element.children().iter().filter_map(|node| match node {
        Node::Element(child) => Some(child),
        Node::Text(_) => None,
    })
}

fn descendants(root: &Element) -> impl Iterator<Item = &Element> {
    let mut pending = vec![root];
    std::iter::from_fn(move || {
        let element = pending.pop()?;
        pending.extend(direct_children(element).rev());
        Some(element)
    })
}

fn attribute<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element
        .attributes()
        .iter()
        .find(|attribute| attribute.name().local() == name)
        .map(Attribute::value)
}

fn attributes(element: &Element) -> BTreeMap<&str, &str> {
    element
        .attributes()
        .iter()
        .map(|attribute| (attribute.name().local(), attribute.value()))
        .collect()
}

fn element_text(element: &Element) -> String {
    let mut text = String::new();
    collect_text(element, &mut text);
    text
}

fn direct_text(element: &Element) -> String {
    element
        .children()
        .iter()
        .filter_map(|node| match node {
            Node::Text(text) => Some(text.text()),
            Node::Element(_) => None,
        })
        .collect()
}

fn collect_text(element: &Element, text: &mut String) {
    for child in element.children() {
        match child {
            Node::Element(element) => collect_text(element, text),
            Node::Text(node) => text.push_str(node.text()),
        }
    }
}

fn compact(source: &str) -> String {
    let mut compacted = String::with_capacity(source.len());
    let mut quote = None;
    let mut escaped = false;

    for character in source.chars() {
        if let Some(delimiter) = quote {
            compacted.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == delimiter {
                quote = None;
            }
            continue;
        }

        if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
            compacted.push(character);
        } else if !character.is_whitespace() {
            compacted.push(character);
        }
    }

    compacted
}

// ── Baseline contract

fn compare_baseline(
    scores: &BTreeMap<&str, Score>,
    baseline: &Baseline,
    failures: &mut Vec<String>,
) {
    for (arm, expected) in [
        ("declarative-bindings", &baseline.declarative_bindings),
        ("module-bindings", &baseline.module_bindings),
        ("placeholders", &baseline.placeholders),
    ] {
        match scores.get(arm) {
            Some(actual) if actual == expected => {}
            Some(actual) => failures.push(format!(
                "{arm}: baseline {expected:?} differs from {actual:?}"
            )),
            None => failures.push(format!("{arm}: baseline names an unknown arm")),
        }
    }
    if baseline.admitted != ADMITTED_ARM {
        failures.push(format!(
            "admitted arm {:?} differs from {ADMITTED_ARM:?}",
            baseline.admitted,
        ));
    }
    if baseline.reason.trim().is_empty() {
        failures.push(String::from("admission reason must not be blank"));
    }
    compare_architecture(&baseline.architecture, failures);
}

fn compare_architecture(baseline: &BTreeMap<String, Architecture>, failures: &mut Vec<String>) {
    for arm in Arm::ALL {
        let expected = arm.architecture();
        match baseline.get(arm.filename()) {
            Some(actual) if actual == &expected => {}
            Some(actual) => failures.push(format!(
                "{}: architecture {actual:?} differs from {expected:?}",
                arm.filename(),
            )),
            None => failures.push(format!(
                "{}: architecture baseline is missing",
                arm.filename(),
            )),
        }
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Box<dyn Error>> {
    let source = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&source)?)
}

// ── Evaluation model

#[derive(Clone, Copy)]
enum Arm {
    Declarative,
    Module,
    Placeholder,
}

impl Arm {
    const ALL: [Self; 3] = [Self::Declarative, Self::Module, Self::Placeholder];

    const fn filename(self) -> &'static str {
        match self {
            Self::Declarative => "declarative-bindings",
            Self::Module => "module-bindings",
            Self::Placeholder => "placeholders",
        }
    }

    const fn architecture(self) -> Architecture {
        match self {
            Self::Declarative => Architecture::new(true, true, true, true, true, false),
            Self::Module => Architecture::new(true, true, false, true, false, true),
            Self::Placeholder => Architecture::new(false, false, true, false, true, false),
        }
    }
}

#[derive(Clone, Copy)]
enum BindingSyntax {
    Declarative,
    Module,
}

#[derive(Debug, Deserialize)]
struct CaseSet {
    cases: Vec<CaseMetadata>,
}

#[derive(Debug, Deserialize)]
struct CaseMetadata {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ModelOutput {
    cases: Vec<ModelCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelCase {
    id: String,
    film_html: String,
    variant_json: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Baseline {
    declarative_bindings: Score,
    module_bindings: Score,
    placeholders: Score,
    architecture: BTreeMap<String, Architecture>,
    admitted: String,
    reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Score {
    passed: usize,
    total: usize,
    authored_bytes: usize,
    failed_cases: BTreeSet<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Architecture {
    parses_authored_html_once: bool,
    readable_without_runtime: bool,
    statically_locatable_dependencies: bool,
    reusable_bundle_across_variants: bool,
    safe_literal_text_by_construction: bool,
    requires_arbitrary_binding_code: bool,
}

impl Architecture {
    const fn new(
        parses_authored_html_once: bool,
        readable_without_runtime: bool,
        statically_locatable_dependencies: bool,
        reusable_bundle_across_variants: bool,
        safe_literal_text_by_construction: bool,
        requires_arbitrary_binding_code: bool,
    ) -> Self {
        Self {
            parses_authored_html_once,
            readable_without_runtime,
            statically_locatable_dependencies,
            reusable_bundle_across_variants,
            safe_literal_text_by_construction,
            requires_arbitrary_binding_code,
        }
    }
}

// ── Failures

#[derive(Debug)]
struct GradingFailed(Vec<String>);

impl fmt::Display for GradingFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("typed-variant authoring evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}
