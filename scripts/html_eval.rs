//! Offline grading for the checked-in native HTML authoring experiment.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Element, Node};
use serde::Deserialize;

const EVALUATION: &str = "evals/html-authoring";
const ADMITTED_ARM: &str = "html";

pub(super) fn grade(repository: &Path) -> Result<(), Box<dyn Error>> {
    let evaluation = repository.join(EVALUATION);
    let cases: CaseSet = read_json(&evaluation.join("cases.json"))?;
    let baseline: Baseline = read_json(&evaluation.join("baseline.json"))?;
    let mut scores = BTreeMap::new();
    let mut failures = Vec::new();

    for arm in Arm::ALL {
        let mut score = Score::default();
        for run in 1..=2 {
            for batch in &cases.batches {
                let filename = format!("{}-run-{run}-batch-{}.json", arm.filename(), batch.id);
                let output: ModelOutput = read_json(&evaluation.join("raw").join(&filename))?;
                grade_output(
                    arm,
                    run,
                    &filename,
                    output,
                    batch,
                    &mut score,
                    &mut failures,
                );
            }
        }
        scores.insert(arm.filename(), score);
    }

    compare_baseline(&scores, &baseline, &mut failures);
    if !failures.is_empty() {
        return Err(Box::new(GradingFailed(failures)));
    }

    for (arm, score) in scores {
        println!(
            "{arm}: {}/{}; {} files; {} authored bytes",
            score.passed, score.total, score.files, score.authored_bytes,
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
    batch: &CaseBatch,
    score: &mut Score,
    failures: &mut Vec<String>,
) {
    let expected = batch.cases.iter().cloned().collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();

    for result in output.results {
        score.total += 1;
        score.files += result.files.len();
        score.authored_bytes += result
            .files
            .iter()
            .map(|file| file.content.len())
            .sum::<usize>();

        if !expected.contains(&result.case_id) {
            failures.push(format!("{filename}: unknown case {}", result.case_id));
            continue;
        }
        if !seen.insert(result.case_id.clone()) {
            failures.push(format!("{filename}: duplicate case {}", result.case_id));
            continue;
        }
        if project_is_valid(arm, &result.case_id, &result.files) {
            score.passed += 1;
        } else {
            score
                .failed_cases
                .insert(format!("run-{run}:{}", result.case_id));
        }
    }

    if seen != expected {
        failures.push(format!(
            "{filename}: expected cases {expected:?}, found {seen:?}"
        ));
    }
}

fn project_is_valid(arm: Arm, case: &str, files: &[AuthoredFile]) -> bool {
    let Some(project) = Project::new(arm, files) else {
        return false;
    };
    if !language_is_valid(project.markup, arm) || has_free_coordinates(project.markup) {
        return false;
    }
    if arm == Arm::Html && lacks_native_html_spelling(project.markup) {
        return false;
    }

    match case {
        "simple-hero" => project.has_all(&[
            "media/hero.mp4",
            "Compile intent.",
            "brightness",
            "opacity: 0",
            "y: 36",
            "0.45",
        ]),
        "three-beats" => {
            project.has_all(&[
                "media/a.mp4",
                "media/b.mp4",
                "media/c.mp4",
                "id=\"story\"",
                "id=\"compile\"",
                "id=\"render\"",
                "clipPath",
                "0.55",
            ]) && !project.all.contains("switch")
        }
        "cue-cta" => {
            has_direct_cues(project.markup, arm)
                && project.has_all(&[
                    "id=\"offer\"",
                    "time=\"2s\"",
                    "cue=\"offer\"",
                    "Buy now",
                    "border-radius",
                    "scale: 0.8",
                    "opacity: 0",
                ])
        }
        "nested-emphasis" => valid_nested_emphasis(arm, &project),
        "decorative-layer" => valid_decorative_layer(arm, &project),
        "edit-copy" => {
            project.has_all(&[
                "Compile what you mean.",
                "#d7ff43",
                "font-size: 80px",
                "#hero",
                "#111",
            ]) && !project.all.contains("Old message")
        }
        "append-shot" => project.has_all(&[
            "id=\"one\"",
            "id=\"two\"",
            "id=\"three\"",
            "one.mp4",
            "two.mp4",
            "three.mp4",
            "background: red",
            "background: blue",
            "background: green",
        ]),
        "add-cued-cta" => {
            has_direct_cues(project.markup, arm)
                && !project.markup.contains("duration=")
                && project.has_all(&[
                    "offer.mp4",
                    "Offer",
                    "id=\"buy\"",
                    "time=\"1500ms\"",
                    "cue=\"buy\"",
                    "Buy now",
                ])
        }
        "repair-markup" => project.has_all(&["clip.mp4", "Exact"]),
        "reject-coordinate-edit" => {
            has_direct_cues(project.markup, arm)
                && project.has_all(&[
                    "id=\"title-at-three\"",
                    "time=\"3s\"",
                    "cue=\"title-at-three\"",
                    "Later",
                ])
        }
        _ => false,
    }
}

fn valid_nested_emphasis(arm: Arm, project: &Project<'_>) -> bool {
    if arm == Arm::Html {
        return project.has_all(&[
            "<span class=\"accent\">native.</span>",
            "Write ",
            " Render exact.",
            ".accent",
            "selectors",
        ]);
    }
    valid_advanced_presentation(project)
        && project
            .markup
            .contains("<om-title>Write native. Render exact.</om-title>")
        && project.all.contains("createElement(\"span\")")
        && project.has_all(&["native.", "Render exact.", ".accent", "selectors"])
}

fn valid_decorative_layer(arm: Arm, project: &Project<'_>) -> bool {
    if arm == Arm::Html {
        return project.has_all(&[
            "<div class=\"grid\" aria-hidden=\"true\"></div>",
            "No tracks",
            "linear-gradient",
            "selectors",
        ]) && project.all.matches("linear-gradient").count() >= 2;
    }
    valid_advanced_presentation(project)
        && project.markup.contains("<om-title>No tracks</om-title>")
        && (project.all.contains("className = \"grid\"")
            || project.all.contains("className: \"grid\""))
        && project.has_all(&["linear-gradient", "selectors"])
        && project.all.matches("linear-gradient").count() >= 2
}

fn valid_advanced_presentation(project: &Project<'_>) -> bool {
    let Some(presentation) = project.files.get("presentation.ts") else {
        return false;
    };
    project.files.contains_key("film.onmark")
        && presentation.contains("PresentationBindings")
        && presentation.contains("PresentationRuntimeAdapter")
        && presentation.contains("installRuntimeHost")
}

fn language_is_valid(source: &str, arm: Arm) -> bool {
    let normalized = normalized_markup(source, arm);
    let report = compiler::parse(SourceId::new(0), &normalized);
    let (document, diagnostics) = report.into_parts();
    if diagnostics.has_errors() {
        return false;
    }
    let Some(root) = single_root(document.nodes()) else {
        return false;
    };
    if !has_name(root, arm.root()) {
        return false;
    }
    if arm == Arm::Screenplay {
        return true;
    }

    let bound = compiler::bind(document);
    let (film, diagnostics) = bound.into_parts();
    if diagnostics.has_errors() {
        return false;
    }
    let Some(film) = film else {
        return false;
    };
    let resolved = compiler::resolve(film);
    let (film, diagnostics) = resolved.into_parts();
    film.is_some() && !diagnostics.has_errors()
}

fn has_direct_cues(source: &str, arm: Arm) -> bool {
    let normalized = normalized_markup(source, arm);
    let report = compiler::parse(SourceId::new(0), &normalized);
    let (document, diagnostics) = report.into_parts();
    if !diagnostics.is_empty() {
        return false;
    }
    let Some(root) = single_root(document.nodes()) else {
        return false;
    };
    root.children().iter().any(|node| {
        matches!(
            node,
            Node::Element(element)
                if has_name(element, arm.cues())
        )
    })
}

fn normalized_markup(source: &str, arm: Arm) -> Cow<'_, str> {
    if arm == Arm::Html {
        return Cow::Borrowed(source);
    }
    // The historical screenplay arm used XML empty-element spelling. Expand
    // only its two admitted empty elements before checking it with the current
    // HTML tokenizer; the frozen authored bytes remain untouched.
    Cow::Owned(expand_empty_elements(source))
}

fn expand_empty_elements(source: &str) -> String {
    let mut expanded = source.to_owned();
    for tag in ["video", "cue"] {
        let mut search_from = 0;
        while let Some(relative_start) = expanded[search_from..].find(&format!("<{tag}")) {
            let start = search_from + relative_start;
            let Some(relative_end) = expanded[start..].find("/>") else {
                break;
            };
            let end = start + relative_end;
            expanded.replace_range(end..end + 2, &format!("></{tag}>"));
            search_from = end + tag.len() + 3;
        }
    }
    expanded
}

fn single_root(nodes: &[Node]) -> Option<&Element> {
    let mut root = None;
    for node in nodes {
        match node {
            Node::Element(element) if root.is_none() => root = Some(element),
            Node::Text(text) if text.text().trim().is_empty() => {}
            Node::Element(_) | Node::Text(_) => return None,
        }
    }
    root
}

fn has_name(element: &Element, expected: &str) -> bool {
    element.name().local() == expected
}

fn has_free_coordinates(markup: &str) -> bool {
    ["start=\"", "begin=\"", "end=\"", "until=\"", "track=\""]
        .iter()
        .any(|attribute| markup.contains(attribute))
}

fn lacks_native_html_spelling(markup: &str) -> bool {
    markup.contains("/>") || !markup.contains("</video>")
}

fn compare_baseline(
    scores: &BTreeMap<&str, Score>,
    baseline: &Baseline,
    failures: &mut Vec<String>,
) {
    for (arm, expected) in [
        ("screenplay", &baseline.screenplay),
        ("html", &baseline.html),
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
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Box<dyn Error>> {
    let source = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&source)?)
}

struct Project<'a> {
    all: String,
    files: BTreeMap<&'a str, &'a str>,
    markup: &'a str,
}

impl<'a> Project<'a> {
    fn new(arm: Arm, files: &'a [AuthoredFile]) -> Option<Self> {
        let mut by_path = BTreeMap::new();
        for file in files {
            if file.path.trim().is_empty() || file.content.trim().is_empty() {
                return None;
            }
            if by_path
                .insert(file.path.as_str(), file.content.as_str())
                .is_some()
            {
                return None;
            }
        }
        if arm == Arm::Html && !is_single_html_document(&by_path) {
            return None;
        }
        let markup = by_path.get(arm.markup()).copied()?;
        let all = by_path.values().copied().collect::<Vec<_>>().join("\n");
        Some(Self {
            all,
            files: by_path,
            markup,
        })
    }

    fn has_all(&self, needles: &[&str]) -> bool {
        needles.iter().all(|needle| self.all.contains(needle))
    }
}

fn is_single_html_document(files: &BTreeMap<&str, &str>) -> bool {
    files.len() == 1 && files.contains_key("film.html")
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Arm {
    Screenplay,
    Html,
}

impl Arm {
    const ALL: [Self; 2] = [Self::Screenplay, Self::Html];

    const fn filename(self) -> &'static str {
        match self {
            Self::Screenplay => "screenplay",
            Self::Html => "html",
        }
    }

    const fn markup(self) -> &'static str {
        match self {
            Self::Screenplay => "film.onmark",
            Self::Html => "film.html",
        }
    }

    const fn root(self) -> &'static str {
        match self {
            Self::Screenplay => "film",
            Self::Html => "om-film",
        }
    }

    const fn cues(self) -> &'static str {
        match self {
            Self::Screenplay => "cues",
            Self::Html => "om-cues",
        }
    }
}

#[derive(Debug, Deserialize)]
struct CaseSet {
    batches: Vec<CaseBatch>,
}

#[derive(Debug, Deserialize)]
struct CaseBatch {
    id: usize,
    cases: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelOutput {
    results: Vec<ModelResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelResult {
    case_id: String,
    files: Vec<AuthoredFile>,
}

#[derive(Debug, Deserialize)]
struct AuthoredFile {
    path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct Baseline {
    screenplay: Score,
    html: Score,
    admitted: String,
    reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Score {
    passed: usize,
    total: usize,
    files: usize,
    authored_bytes: usize,
    failed_cases: BTreeSet<String>,
}

#[derive(Debug)]
struct GradingFailed(Vec<String>);

impl fmt::Display for GradingFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("native HTML authoring evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}
