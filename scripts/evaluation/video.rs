//! Offline grading for the checked-in source-local video editing experiment.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Attribute, Element, Node};
use serde::Deserialize;

const EVALUATION: &str = "evals/video-editing-syntax";

// ── Grading pipeline

pub(super) fn grade(repository: &Path) -> Result<(), Box<dyn Error>> {
    let evaluation = repository.join(EVALUATION);
    let cases: CaseSet = read_json(&evaluation.join("cases.json"))?;
    let baseline: Baseline = read_json(&evaluation.join("baseline.json"))?;
    let settings: EvaluationSettings = read_json(&evaluation.join("settings.json"))?;
    let expected = case_expectations(cases.cases)?;
    if settings.repetitions == 0 {
        return Err(Box::new(GradingFailed(vec![String::from(
            "evaluation repetitions must be positive",
        )])));
    }
    let mut scores = BTreeMap::new();
    let mut failures = Vec::new();

    for arm in Arm::ALL {
        let mut score = Score::default();
        let mut occurrences = BTreeMap::new();
        for run in 1..=settings.repetitions {
            let filename = format!("{}-run-{run}.json", arm.filename());
            let output: ModelOutput = read_json(&evaluation.join("raw").join(&filename))?;
            grade_output(
                arm,
                &filename,
                output,
                &expected,
                &mut occurrences,
                &mut score,
                &mut failures,
            );
        }
        compare_occurrences(
            arm,
            settings.repetitions,
            &expected,
            &occurrences,
            &mut failures,
        );
        scores.insert(arm.filename(), score);
    }

    compare_baseline(&scores, &baseline, &mut failures);
    if !failures.is_empty() {
        return Err(Box::new(GradingFailed(failures)));
    }

    for (arm, score) in scores {
        println!(
            "{arm}: {}/{} ({} authored bytes)",
            score.passed, score.total, score.authored_bytes,
        );
    }
    println!("admitted: {}", baseline.admitted);
    Ok(())
}

fn case_expectations(
    cases: Vec<CaseExpectation>,
) -> Result<BTreeMap<String, CaseExpectation>, GradingFailed> {
    let mut expected = BTreeMap::new();
    for case in cases {
        let id = case.id.clone();
        if expected.insert(id.clone(), case).is_some() {
            return Err(GradingFailed(vec![format!(
                "case definition {id} is duplicated"
            )]));
        }
    }
    Ok(expected)
}

fn grade_output(
    arm: Arm,
    filename: &str,
    output: ModelOutput,
    expected: &BTreeMap<String, CaseExpectation>,
    occurrences: &mut BTreeMap<String, usize>,
    score: &mut Score,
    failures: &mut Vec<String>,
) {
    let mut seen = BTreeSet::new();
    for result in output.results {
        score.total += 1;
        score.authored_bytes += result.screenplay.len();
        if !seen.insert(result.case_id.clone()) {
            failures.push(format!("{filename}: duplicate case {}", result.case_id));
            continue;
        }
        let Some(expected) = expected.get(&result.case_id) else {
            failures.push(format!("{filename}: unknown case {}", result.case_id));
            continue;
        };
        *occurrences.entry(result.case_id.clone()).or_default() += 1;

        match extract_videos(arm, &result.screenplay) {
            Ok(actual) if actual == expected.videos => score.passed += 1,
            Ok(actual) => failures.push(format!(
                "{filename}: {} differs\n  expected: {:?}\n  actual:   {:?}",
                result.case_id, expected.videos, actual,
            )),
            Err(error) => failures.push(format!("{filename}: {}: {error}", result.case_id)),
        }
    }

    if seen.len() != expected.len() {
        failures.push(format!(
            "{filename}: expected {} distinct cases, found {}",
            expected.len(),
            seen.len(),
        ));
    }
}

fn compare_occurrences(
    arm: Arm,
    repetitions: usize,
    expected: &BTreeMap<String, CaseExpectation>,
    occurrences: &BTreeMap<String, usize>,
    failures: &mut Vec<String>,
) {
    for id in expected.keys() {
        let count = occurrences.get(id).copied().unwrap_or_default();
        if count != repetitions {
            failures.push(format!(
                "{}: case {id} occurs {count} times instead of once per repetition",
                arm.filename(),
            ));
        }
    }
}

fn compare_baseline(
    scores: &BTreeMap<&str, Score>,
    baseline: &Baseline,
    failures: &mut Vec<String>,
) {
    let expected = [
        (Arm::TrimEdges.filename(), baseline.trim_edges),
        (Arm::TrimRange.filename(), baseline.trim_range),
    ];
    for (arm, baseline) in expected {
        if scores.get(arm) != Some(&baseline) {
            failures.push(format!(
                "{arm}: expected baseline {baseline:?}, found {:?}",
                scores.get(arm),
            ));
        }
    }
    if baseline.admitted != Arm::TrimRange.filename() {
        failures.push(format!(
            "baseline admits {}, expected {}",
            baseline.admitted,
            Arm::TrimRange.filename(),
        ));
    }
    if baseline.reason.trim().is_empty() {
        failures.push(String::from("baseline admission reason is blank"));
    }
}

// ── Screenplay facts

fn extract_videos(arm: Arm, screenplay: &str) -> Result<Vec<VideoExpectation>, InvalidScreenplay> {
    let report = compiler::parse(SourceId::new(0), screenplay);
    let (document, diagnostics) = report.into_parts();
    if !diagnostics.is_empty() {
        return Err(InvalidScreenplay::new(
            "screenplay is not well-formed markup",
        ));
    }

    let film = only_element(document.nodes(), "document")?;
    require_name(film, "om-film")?;
    require_attributes(film, &[])?;
    let scene = only_element(film.children(), "om-film")?;
    require_name(scene, "om-scene")?;
    require_attributes(scene, &[])?;

    let mut videos = Vec::new();
    for shot in elements(scene.children(), "om-scene")? {
        require_name(shot, "om-shot")?;
        require_attributes(shot, &[])?;
        let video = only_element(shot.children(), "om-shot")?;
        videos.push(extract_video(arm, video)?);
    }
    Ok(videos)
}

fn extract_video(arm: Arm, video: &Element) -> Result<VideoExpectation, InvalidScreenplay> {
    require_name(video, "video")?;
    require_empty(video)?;

    match arm {
        Arm::TrimEdges => {
            require_attributes(video, &["src", "trim-in", "trim-out", "speed"])?;
            Ok(VideoExpectation {
                src: attribute(video, "src")?.to_owned(),
                trim_in: optional_attribute(video, "trim-in").map(str::to_owned),
                trim_out: optional_attribute(video, "trim-out").map(str::to_owned),
                speed: optional_attribute(video, "speed").map(str::to_owned),
            })
        }
        Arm::TrimRange => {
            require_attributes(video, &["src", "trim", "speed"])?;
            let (trim_in, trim_out) = parse_trim(optional_attribute(video, "trim"))?;
            Ok(VideoExpectation {
                src: attribute(video, "src")?.to_owned(),
                trim_in,
                trim_out,
                speed: optional_attribute(video, "speed").map(str::to_owned),
            })
        }
    }
}

fn parse_trim(value: Option<&str>) -> Result<(Option<String>, Option<String>), InvalidScreenplay> {
    let Some(value) = value else {
        return Ok((None, None));
    };
    let Some((start, end)) = value.split_once("..") else {
        return Err(InvalidScreenplay::new(
            "trim range does not contain one `..` separator",
        ));
    };
    if start.is_empty() && end.is_empty() {
        return Err(InvalidScreenplay::new("trim range has no bound"));
    }
    if end.contains("..") {
        return Err(InvalidScreenplay::new(
            "trim range contains more than one separator",
        ));
    }

    Ok((nonempty(start), nonempty(end)))
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn elements<'a>(nodes: &'a [Node], parent: &str) -> Result<Vec<&'a Element>, InvalidScreenplay> {
    let mut elements = Vec::new();
    for node in nodes {
        match node {
            Node::Element(element) => elements.push(element),
            Node::Text(text) if text.text().trim().is_empty() => {}
            Node::Text(_) => {
                return Err(InvalidScreenplay::new(format!(
                    "unexpected text inside <{parent}>",
                )));
            }
        }
    }
    Ok(elements)
}

fn only_element<'a>(nodes: &'a [Node], parent: &str) -> Result<&'a Element, InvalidScreenplay> {
    let elements = elements(nodes, parent)?;
    let [element] = elements.as_slice() else {
        return Err(InvalidScreenplay::new(format!(
            "<{parent}> must contain exactly one element",
        )));
    };
    Ok(element)
}

fn require_name(element: &Element, expected: &str) -> Result<(), InvalidScreenplay> {
    let actual = element.name().local();
    if actual == expected {
        return Ok(());
    }
    Err(InvalidScreenplay::new(format!(
        "expected <{expected}>, found <{actual}>",
    )))
}

fn require_attributes(element: &Element, allowed: &[&str]) -> Result<(), InvalidScreenplay> {
    for attribute in element.attributes() {
        if !allowed.contains(&attribute.name().local()) {
            return Err(InvalidScreenplay::new(format!(
                "unexpected attribute \"{}\" on <{}>",
                attribute.name(),
                element.name(),
            )));
        }
    }
    if allowed.contains(&"src") {
        attribute(element, "src")?;
    }
    Ok(())
}

fn attribute<'a>(element: &'a Element, name: &str) -> Result<&'a str, InvalidScreenplay> {
    optional_attribute(element, name).ok_or_else(|| {
        InvalidScreenplay::new(format!(
            "missing attribute \"{name}\" on <{}>",
            element.name(),
        ))
    })
}

fn optional_attribute<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element
        .attributes()
        .iter()
        .find(|attribute| attribute.name().local() == name)
        .map(Attribute::value)
}

fn require_empty(element: &Element) -> Result<(), InvalidScreenplay> {
    let empty = element
        .children()
        .iter()
        .all(|node| matches!(node, Node::Text(text) if text.text().trim().is_empty()));
    if empty {
        return Ok(());
    }
    Err(InvalidScreenplay::new(format!(
        "<{}> must be empty",
        element.name(),
    )))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

// ── Evaluation model

#[derive(Clone, Copy)]
enum Arm {
    TrimEdges,
    TrimRange,
}

impl Arm {
    const ALL: [Self; 2] = [Self::TrimEdges, Self::TrimRange];

    const fn filename(self) -> &'static str {
        match self {
            Self::TrimEdges => "trim-edges",
            Self::TrimRange => "trim-range",
        }
    }
}

#[derive(Debug, Deserialize)]
struct CaseSet {
    cases: Vec<CaseExpectation>,
}

#[derive(Debug, Deserialize)]
struct CaseExpectation {
    id: String,
    videos: Vec<VideoExpectation>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct VideoExpectation {
    src: String,
    trim_in: Option<String>,
    trim_out: Option<String>,
    speed: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelOutput {
    results: Vec<ModelResult>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelResult {
    case_id: String,
    screenplay: String,
}

#[derive(Debug, Deserialize)]
struct EvaluationSettings {
    repetitions: usize,
}

#[derive(Debug, Deserialize)]
struct Baseline {
    #[serde(rename = "trim-edges")]
    trim_edges: Score,
    #[serde(rename = "trim-range")]
    trim_range: Score,
    admitted: String,
    reason: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Score {
    passed: usize,
    total: usize,
    authored_bytes: usize,
}

// ── Failures

#[derive(Debug)]
struct InvalidScreenplay(String);

impl InvalidScreenplay {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for InvalidScreenplay {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for InvalidScreenplay {}

#[derive(Debug)]
struct GradingFailed(Vec<String>);

impl fmt::Display for GradingFailed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("video editing syntax evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}
