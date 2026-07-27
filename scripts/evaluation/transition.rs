//! Offline grading for the checked-in shot-transition syntax experiment.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Attribute, Element, Node};
use serde::Deserialize;

const EVALUATION: &str = "evals/transition-syntax";

pub(super) fn grade(repository: &Path) -> Result<(), Box<dyn Error>> {
    let evaluation = repository.join(EVALUATION);
    let cases: CaseSet = read_json(&evaluation.join("cases.json"))?;
    let baseline: Baseline = read_json(&evaluation.join("baseline.json"))?;
    let settings: EvaluationSettings = read_json(&evaluation.join("settings.json"))?;
    let expected = index_cases(cases.cases)?;
    require_repetitions(settings.repetitions)?;

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
    expected: &BTreeMap<String, ScreenplayExpectation>,
    failures: &mut Vec<String>,
) -> Result<Score, Box<dyn Error>> {
    let mut score = Score::default();
    let mut occurrences = BTreeMap::new();
    for run in 1..=repetitions {
        let filename = format!("{}-run-{run}.json", arm.filename());
        let output = read_json(&evaluation.join("raw").join(&filename))?;
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
    expected: &BTreeMap<String, ScreenplayExpectation>,
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

        match extract_screenplay(arm, &result.screenplay) {
            Ok(actual) if actual == *expected => score.passed += 1,
            Ok(actual) => failures.push(format!(
                "{filename}: {} differs\n  expected: {expected:?}\n  actual:   {actual:?}",
                result.case_id,
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
    expected: &BTreeMap<String, ScreenplayExpectation>,
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

fn index_cases(
    cases: Vec<CaseExpectation>,
) -> Result<BTreeMap<String, ScreenplayExpectation>, GradingFailed> {
    let mut expected = BTreeMap::new();
    for case in cases {
        if expected.insert(case.id.clone(), case.screenplay).is_some() {
            return Err(GradingFailed(vec![format!(
                "case definition {} is duplicated",
                case.id,
            )]));
        }
    }
    Ok(expected)
}

fn require_repetitions(repetitions: usize) -> Result<(), GradingFailed> {
    if repetitions > 0 {
        return Ok(());
    }
    Err(GradingFailed(vec![String::from(
        "evaluation repetitions must be positive",
    )]))
}

fn compare_baseline(
    scores: &BTreeMap<&str, Score>,
    baseline: &Baseline,
    failures: &mut Vec<String>,
) {
    let expected = [
        (Arm::BoundaryElement.filename(), baseline.boundary_element),
        (
            Arm::IncomingAttribute.filename(),
            baseline.incoming_attribute,
        ),
    ];
    for (arm, expected) in expected {
        if scores.get(arm) != Some(&expected) {
            failures.push(format!(
                "{arm}: expected baseline {expected:?}, found {:?}",
                scores.get(arm),
            ));
        }
    }
    if baseline.admitted != Arm::BoundaryElement.filename() {
        failures.push(format!(
            "baseline admits {}, expected {}",
            baseline.admitted,
            Arm::BoundaryElement.filename(),
        ));
    }
    if baseline.reason.trim().is_empty() {
        failures.push(String::from("baseline admission reason is blank"));
    }
}

fn extract_screenplay(
    arm: Arm,
    screenplay: &str,
) -> Result<ScreenplayExpectation, InvalidScreenplay> {
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
    Sequence::extract(arm, scene)
}

struct Sequence {
    arm: Arm,
    shots: Vec<ShotExpectation>,
    transitions: Vec<TransitionExpectation>,
    pending_duration: Option<String>,
}

impl Sequence {
    fn extract(arm: Arm, scene: &Element) -> Result<ScreenplayExpectation, InvalidScreenplay> {
        let mut sequence = Self {
            arm,
            shots: Vec::new(),
            transitions: Vec::new(),
            pending_duration: None,
        };
        for child in elements(scene.children(), "om-scene")? {
            sequence.push(child)?;
        }
        sequence.finish()
    }

    fn push(&mut self, element: &Element) -> Result<(), InvalidScreenplay> {
        match element.name().local() {
            "om-shot" => self.push_shot(element),
            "om-transition" if self.arm == Arm::BoundaryElement => self.push_transition(element),
            name => Err(InvalidScreenplay::new(format!(
                "unexpected <{name}> inside <om-scene>",
            ))),
        }
    }

    fn push_transition(&mut self, element: &Element) -> Result<(), InvalidScreenplay> {
        require_element(element, "om-transition", &["duration"])?;
        require_empty(element)?;
        if self.shots.is_empty() {
            return Err(InvalidScreenplay::new("transition has no preceding shot"));
        }
        if self.pending_duration.is_some() {
            return Err(InvalidScreenplay::new("transitions are adjacent"));
        }
        self.pending_duration = Some(attribute(element, "duration")?.to_owned());
        Ok(())
    }

    fn push_shot(&mut self, element: &Element) -> Result<(), InvalidScreenplay> {
        let allowed = match self.arm {
            Arm::BoundaryElement => &["id"][..],
            Arm::IncomingAttribute => &["id", "transition-in"][..],
        };
        require_element(element, "om-shot", allowed)?;
        let shot = ShotExpectation {
            id: attribute(element, "id")?.to_owned(),
            src: extract_video(element)?,
        };
        let incoming = optional_attribute(element, "transition-in");

        if let Some(previous) = self.shots.last() {
            let duration = match self.arm {
                Arm::BoundaryElement => self.pending_duration.take(),
                Arm::IncomingAttribute => incoming.map(str::to_owned),
            };
            if let Some(duration) = duration {
                self.transitions.push(TransitionExpectation {
                    from: previous.id.clone(),
                    to: shot.id.clone(),
                    duration,
                });
            }
        } else if incoming.is_some() {
            return Err(InvalidScreenplay::new(
                "first shot cannot have an incoming transition",
            ));
        }

        self.shots.push(shot);
        Ok(())
    }

    fn finish(self) -> Result<ScreenplayExpectation, InvalidScreenplay> {
        if self.pending_duration.is_some() {
            return Err(InvalidScreenplay::new("transition has no following shot"));
        }
        Ok(ScreenplayExpectation {
            shots: self.shots,
            transitions: self.transitions,
        })
    }
}

fn extract_video(shot: &Element) -> Result<String, InvalidScreenplay> {
    let video = only_element(shot.children(), "om-shot")?;
    require_element(video, "video", &["src"])?;
    require_empty(video)?;
    Ok(attribute(video, "src")?.to_owned())
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

fn require_element(
    element: &Element,
    expected: &str,
    allowed_attributes: &[&str],
) -> Result<(), InvalidScreenplay> {
    let actual = element.name().local();
    if actual != expected {
        return Err(InvalidScreenplay::new(format!(
            "expected <{expected}>, found <{actual}>",
        )));
    }
    for attribute in element.attributes() {
        if !allowed_attributes.contains(&attribute.name().local()) {
            return Err(InvalidScreenplay::new(format!(
                "unexpected attribute \"{}\" on <{actual}>",
                attribute.name(),
            )));
        }
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
    if element
        .children()
        .iter()
        .all(|node| matches!(node, Node::Text(text) if text.text().trim().is_empty()))
    {
        return Ok(());
    }
    Err(InvalidScreenplay::new(format!(
        "<{}> must be empty",
        element.name(),
    )))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Arm {
    BoundaryElement,
    IncomingAttribute,
}

impl Arm {
    const ALL: [Self; 2] = [Self::BoundaryElement, Self::IncomingAttribute];

    const fn filename(self) -> &'static str {
        match self {
            Self::BoundaryElement => "boundary-element",
            Self::IncomingAttribute => "incoming-attribute",
        }
    }
}

#[derive(Debug, Deserialize)]
struct CaseSet {
    cases: Vec<CaseExpectation>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct CaseExpectation {
    id: String,
    #[serde(flatten)]
    screenplay: ScreenplayExpectation,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct ScreenplayExpectation {
    shots: Vec<ShotExpectation>,
    transitions: Vec<TransitionExpectation>,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct ShotExpectation {
    id: String,
    src: String,
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct TransitionExpectation {
    from: String,
    to: String,
    duration: String,
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
    #[serde(rename = "boundary-element")]
    boundary_element: Score,
    #[serde(rename = "incoming-attribute")]
    incoming_attribute: Score,
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
        formatter.write_str("shot-transition syntax evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}
