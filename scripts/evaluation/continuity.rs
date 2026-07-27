//! Offline grading for the checked-in media-continuity spelling experiment.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Attribute, Element, Node};
use serde::Deserialize;

const EVALUATION: &str = "evals/media-continuity-syntax";

// ── Grading pipeline

pub(super) fn grade(repository: &Path) -> Result<(), Box<dyn Error>> {
    let evaluation = repository.join(EVALUATION);
    let cases: CaseSet = read_json(&evaluation.join("cases.json"))?;
    let baseline: Baseline = read_json(&evaluation.join("baseline.json"))?;
    let settings: EvaluationSettings = read_json(&evaluation.join("settings.json"))?;
    let expected = index_cases(cases.cases)?;
    require_positive_repetitions(settings.repetitions)?;

    let mut scores = BTreeMap::new();
    let mut failures = Vec::new();
    for arm in Arm::ALL {
        let score = grade_arm(
            arm,
            settings.repetitions,
            &evaluation,
            &expected,
            &mut failures,
        )?;
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

fn grade_arm(
    arm: Arm,
    repetitions: usize,
    evaluation: &Path,
    expected: &BTreeMap<String, CaseExpectation>,
    failures: &mut Vec<String>,
) -> Result<Score, Box<dyn Error>> {
    let mut score = Score::default();
    let mut occurrences = BTreeMap::new();
    for run in 1..=repetitions {
        let filename = format!("{}-run-{run}.json", arm.filename());
        let output: ModelOutput = read_json(&evaluation.join("raw").join(&filename))?;
        grade_output(
            arm,
            &filename,
            output,
            expected,
            &mut occurrences,
            &mut score,
            failures,
        );
    }
    compare_occurrences(arm, repetitions, expected, &occurrences, failures);
    Ok(score)
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
        (Arm::LoopHold.filename(), baseline.loop_hold),
        (Arm::PlaysHoldLast.filename(), baseline.plays_hold_last),
    ];
    for (arm, baseline) in expected {
        if scores.get(arm) != Some(&baseline) {
            failures.push(format!(
                "{arm}: expected baseline {baseline:?}, found {:?}",
                scores.get(arm),
            ));
        }
    }
    if baseline.admitted != Arm::PlaysHoldLast.filename() {
        failures.push(format!(
            "baseline admits {}, expected {}",
            baseline.admitted,
            Arm::PlaysHoldLast.filename(),
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
    require_element(film, "om-film", &[])?;
    let scene = only_element(film.children(), "om-film")?;
    require_element(scene, "om-scene", &[])?;

    elements(scene.children(), "om-scene")?
        .into_iter()
        .map(|shot| extract_shot(arm, shot))
        .collect()
}

fn extract_shot(arm: Arm, shot: &Element) -> Result<VideoExpectation, InvalidScreenplay> {
    require_element(shot, "om-shot", &[])?;
    let video = only_element(shot.children(), "om-shot")?;
    require_empty(video)?;
    extract_video(arm, video)
}

fn extract_video(arm: Arm, video: &Element) -> Result<VideoExpectation, InvalidScreenplay> {
    let names = arm.attributes();
    require_element(video, "video", names)?;
    let plays = optional_attribute(video, arm.plays())
        .map(parse_plays)
        .transpose()?
        .unwrap_or(1);

    Ok(VideoExpectation {
        src: attribute(video, "src")?.to_owned(),
        trim: optional_attribute(video, "trim").map(str::to_owned),
        speed: optional_attribute(video, "speed").map(str::to_owned),
        plays,
        hold_last: optional_attribute(video, arm.hold()).map(str::to_owned),
    })
}

fn parse_plays(value: &str) -> Result<u32, InvalidScreenplay> {
    value
        .parse()
        .map_err(|_| InvalidScreenplay::new("play count is not an unsigned integer"))
}

fn require_element(
    element: &Element,
    name: &str,
    attributes: &[&str],
) -> Result<(), InvalidScreenplay> {
    if element.name().local() != name {
        return Err(InvalidScreenplay::new(format!(
            "expected <{name}>, found <{}>",
            element.name(),
        )));
    }
    for attribute in element.attributes() {
        if !attributes.contains(&attribute.name().local()) {
            return Err(InvalidScreenplay::new(format!(
                "unexpected attribute \"{}\" on <{name}>",
                attribute.name(),
            )));
        }
    }
    if attributes.contains(&"src") {
        attribute(element, "src")?;
    }
    Ok(())
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

fn index_cases(
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

fn require_positive_repetitions(repetitions: usize) -> Result<(), GradingFailed> {
    if repetitions > 0 {
        return Ok(());
    }
    Err(GradingFailed(vec![String::from(
        "evaluation repetitions must be positive",
    )]))
}

// ── Checked-in evaluation data

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Clone, Copy)]
enum Arm {
    LoopHold,
    PlaysHoldLast,
}

impl Arm {
    const ALL: [Self; 2] = [Self::LoopHold, Self::PlaysHoldLast];

    const fn filename(self) -> &'static str {
        match self {
            Self::LoopHold => "loop-hold",
            Self::PlaysHoldLast => "plays-hold-last",
        }
    }

    const fn attributes(self) -> &'static [&'static str] {
        match self {
            Self::LoopHold => &["src", "trim", "speed", "loop", "hold"],
            Self::PlaysHoldLast => &["src", "trim", "speed", "plays", "hold-last"],
        }
    }

    const fn plays(self) -> &'static str {
        match self {
            Self::LoopHold => "loop",
            Self::PlaysHoldLast => "plays",
        }
    }

    const fn hold(self) -> &'static str {
        match self {
            Self::LoopHold => "hold",
            Self::PlaysHoldLast => "hold-last",
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
    trim: Option<String>,
    speed: Option<String>,
    #[serde(default = "one")]
    plays: u32,
    hold_last: Option<String>,
}

const fn one() -> u32 {
    1
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
    #[serde(rename = "loop-hold")]
    loop_hold: Score,
    #[serde(rename = "plays-hold-last")]
    plays_hold_last: Score,
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
        formatter.write_str("media continuity syntax evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}
