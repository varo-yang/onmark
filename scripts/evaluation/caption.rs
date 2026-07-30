//! Offline grading for the checked-in caption-track syntax experiment.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Attribute, Element, Node};
use serde::Deserialize;
use serde::de::DeserializeOwned;

const EVALUATION: &str = "evals/caption-track-syntax";

pub(super) fn grade(repository: &Path) -> Result<(), Box<dyn Error>> {
    let evaluation = repository.join(EVALUATION);
    let cases: CaseSet = read_json(&evaluation.join("cases.json"))?;
    let settings: Settings = read_json(&evaluation.join("settings.json"))?;
    let baseline: Baseline = read_json(&evaluation.join("baseline.json"))?;
    let expected = index_cases(cases.cases)?;
    let mut scores = BTreeMap::new();
    let mut failures = Vec::new();

    for arm in Arm::ALL {
        let score = grade_arm(
            arm,
            settings.repetitions,
            &evaluation,
            &expected,
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

fn grade_arm(
    arm: Arm,
    repetitions: usize,
    evaluation: &Path,
    expected: &BTreeMap<String, CaseExpectation>,
    failures: &mut Vec<String>,
) -> Score {
    let mut score = Score::default();
    let mut occurrences = BTreeMap::new();

    if repetitions == 0 {
        failures.push(String::from("evaluation repetitions must be positive"));
        return score;
    }

    for run in 1..=repetitions {
        let filename = format!("{}-run-{run}.json", arm.filename());
        let path = evaluation.join("raw").join(&filename);
        let output = match read_json::<ModelOutput>(&path) {
            Ok(output) => output,
            Err(error) => {
                failures.push(format!("{filename}: {error}"));
                continue;
            }
        };
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

    for id in expected.keys() {
        let actual = occurrences.get(id).copied().unwrap_or_default();
        if actual != repetitions {
            failures.push(format!(
                "{}: expected case {id} {repetitions} times, found {actual}",
                arm.filename(),
            ));
        }
    }
    score
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
        *occurrences.entry(result.case_id.clone()).or_insert(0) += 1;

        let Some(case) = expected.get(&result.case_id) else {
            failures.push(format!("{filename}: unknown case {}", result.case_id));
            continue;
        };
        if !seen.insert(result.case_id.clone()) {
            failures.push(format!("{filename}: duplicate case {}", result.case_id));
            continue;
        }

        match validate_result(arm, &result, case) {
            Ok(()) => score.passed += 1,
            Err(error) => failures.push(format!("{filename}: {}: {error}", result.case_id)),
        }
    }
}

fn validate_result(
    arm: Arm,
    result: &ModelCase,
    expected: &CaseExpectation,
) -> Result<(), InvalidScreenplay> {
    if result.selected_tracks != expected.selected_tracks {
        return Err(InvalidScreenplay::new("selected track order differs"));
    }
    if has_forbidden_authoring(&result.screenplay) {
        return Err(InvalidScreenplay::new(
            "screenplay contains a forbidden timing or scripting surface",
        ));
    }

    let film = parse_film(&result.screenplay)?;
    validate_film(arm, film, expected)?;
    validate_style(&result.screenplay, expected.style.as_deref())
}

fn parse_film(source: &str) -> Result<Element, InvalidScreenplay> {
    let report = compiler::parse(SourceId::new(0), source);
    let (document, diagnostics) = report.into_parts();
    if !diagnostics.is_empty() {
        return Err(InvalidScreenplay::new("screenplay is not well-formed HTML"));
    }

    let films = document
        .nodes()
        .iter()
        .filter_map(element_node)
        .flat_map(descendants)
        .filter(|element| element.name().local() == "om-film")
        .collect::<Vec<_>>();
    if films.len() != 1 {
        return Err(InvalidScreenplay::new(
            "screenplay must contain exactly one om-film",
        ));
    }
    Ok(films[0].clone())
}

fn validate_film(
    arm: Arm,
    film: Element,
    expected: &CaseExpectation,
) -> Result<(), InvalidScreenplay> {
    require_attributes(&film, &[])?;
    let children = direct_children(&film).collect::<Vec<_>>();
    let tracks = children
        .iter()
        .copied()
        .filter(|element| element.name().local() == arm.element())
        .collect::<Vec<_>>();
    let scenes = children
        .iter()
        .copied()
        .filter(|element| element.name().local() == "om-scene")
        .collect::<Vec<_>>();

    if tracks.len() != expected.tracks.len() {
        return Err(InvalidScreenplay::new("caption track count differs"));
    }
    if scenes.len() != 1 || tracks.len() + scenes.len() != children.len() {
        return Err(InvalidScreenplay::new(
            "film contains an unexpected semantic child",
        ));
    }

    for (track, expected) in tracks.into_iter().zip(&expected.tracks) {
        validate_track(track, expected)?;
    }
    validate_scene(scenes[0], &expected.video)
}

fn validate_track(track: &Element, expected: &TrackExpectation) -> Result<(), InvalidScreenplay> {
    require_attributes(track, &["id", "src", "lang"])?;
    require_empty(track)?;
    require_attribute(track, "id", &expected.id)?;
    require_attribute(track, "src", &expected.src)?;
    require_attribute(track, "lang", &expected.lang)
}

fn validate_scene(scene: &Element, video_source: &str) -> Result<(), InvalidScreenplay> {
    require_attributes(scene, &[])?;
    let shots = direct_children(scene).collect::<Vec<_>>();
    if shots.len() != 1 || shots[0].name().local() != "om-shot" {
        return Err(InvalidScreenplay::new(
            "scene must contain exactly one shot",
        ));
    }

    let shot = shots[0];
    require_attributes(shot, &[])?;
    let videos = direct_children(shot).collect::<Vec<_>>();
    if videos.len() != 1 || videos[0].name().local() != "video" {
        return Err(InvalidScreenplay::new(
            "shot must contain exactly one video",
        ));
    }

    let video = videos[0];
    require_attributes(video, &["src"])?;
    require_empty(video)?;
    require_attribute(video, "src", video_source)
}

fn validate_style(source: &str, expected: Option<&str>) -> Result<(), InvalidScreenplay> {
    let compact_source = compact(source);
    match expected {
        Some(style) if compact_source.contains(&compact(style)) => Ok(()),
        Some(_) => Err(InvalidScreenplay::new("required track CSS differs")),
        None if !source.contains("om-caption[") => Ok(()),
        None => Err(InvalidScreenplay::new("unrequested track CSS is present")),
    }
}

fn has_forbidden_authoring(source: &str) -> bool {
    [
        "<script",
        "<track",
        "<om-cue",
        " start=\"",
        " begin=\"",
        " end=\"",
        " until=\"",
        " duration=\"",
        " delay=\"",
    ]
    .iter()
    .any(|needle| source.contains(needle))
}

fn require_attributes(element: &Element, expected: &[&str]) -> Result<(), InvalidScreenplay> {
    let actual = element
        .attributes()
        .iter()
        .map(|attribute| attribute.name().local())
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    if actual == expected {
        return Ok(());
    }
    Err(InvalidScreenplay::new(format!(
        "attributes on <{}> differ",
        element.name(),
    )))
}

fn require_attribute(
    element: &Element,
    name: &str,
    expected: &str,
) -> Result<(), InvalidScreenplay> {
    let actual = element
        .attributes()
        .iter()
        .find(|attribute| attribute.name().local() == name)
        .map(Attribute::value);
    if actual == Some(expected) {
        return Ok(());
    }
    Err(InvalidScreenplay::new(format!(
        "attribute {name} on <{}> differs",
        element.name(),
    )))
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

fn direct_children(element: &Element) -> impl DoubleEndedIterator<Item = &Element> {
    element.children().iter().filter_map(element_node)
}

fn descendants(root: &Element) -> impl Iterator<Item = &Element> {
    let mut pending = vec![root];
    std::iter::from_fn(move || {
        let element = pending.pop()?;
        pending.extend(direct_children(element).rev());
        Some(element)
    })
}

fn element_node(node: &Node) -> Option<&Element> {
    match node {
        Node::Element(element) => Some(element),
        Node::Text(_) => None,
    }
}

fn compact(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn index_cases(
    cases: Vec<CaseExpectation>,
) -> Result<BTreeMap<String, CaseExpectation>, GradingFailed> {
    let mut indexed = BTreeMap::new();
    for case in cases {
        let id = case.id.clone();
        if indexed.insert(id.clone(), case).is_some() {
            return Err(GradingFailed(vec![format!(
                "case definition {id} is duplicated",
            )]));
        }
    }
    Ok(indexed)
}

fn compare_baseline(
    scores: &BTreeMap<&str, Score>,
    baseline: &Baseline,
    failures: &mut Vec<String>,
) {
    let expected = [
        ("captions-element", &baseline.captions_element),
        ("caption-track-element", &baseline.caption_track_element),
    ];
    for (arm, baseline) in expected {
        if scores.get(arm) != Some(baseline) {
            failures.push(format!("{arm}: score differs from baseline"));
        }
    }
    if baseline.admitted != Arm::CaptionsElement.filename() {
        failures.push(String::from("baseline admits the wrong caption spelling"));
    }
    if baseline.reason.trim().is_empty() {
        failures.push(String::from("baseline admission reason is blank"));
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Clone, Copy)]
enum Arm {
    CaptionsElement,
    CaptionTrackElement,
}

impl Arm {
    const ALL: [Self; 2] = [Self::CaptionsElement, Self::CaptionTrackElement];

    const fn filename(self) -> &'static str {
        match self {
            Self::CaptionsElement => "captions-element",
            Self::CaptionTrackElement => "caption-track-element",
        }
    }

    const fn element(self) -> &'static str {
        match self {
            Self::CaptionsElement => "om-captions",
            Self::CaptionTrackElement => "om-caption-track",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Settings {
    repetitions: usize,
}

#[derive(Debug, Deserialize)]
struct CaseSet {
    cases: Vec<CaseExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaseExpectation {
    id: String,
    tracks: Vec<TrackExpectation>,
    selected_tracks: Vec<String>,
    video: String,
    style: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TrackExpectation {
    id: String,
    src: String,
    lang: String,
}

#[derive(Debug, Deserialize)]
struct ModelOutput {
    results: Vec<ModelCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelCase {
    case_id: String,
    screenplay: String,
    selected_tracks: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Baseline {
    #[serde(rename = "captions-element")]
    captions_element: Score,
    #[serde(rename = "caption-track-element")]
    caption_track_element: Score,
    admitted: String,
    reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Score {
    passed: usize,
    total: usize,
    authored_bytes: usize,
}

#[derive(Debug)]
struct InvalidScreenplay(Box<str>);

impl InvalidScreenplay {
    fn new(message: impl Into<Box<str>>) -> Self {
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
        formatter.write_str("caption-track syntax evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}
