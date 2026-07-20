//! Offline grading for the checked-in authored-audio syntax experiment.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Attribute, Element, Node};
use serde::Deserialize;

const EVALUATION: &str = "evals/audio-syntax";
const ADMITTED_ARM: &str = "semantic-elements";

pub(super) fn grade(repository: &Path) -> Result<(), Box<dyn Error>> {
    let evaluation = repository.join(EVALUATION);
    let cases: CaseSet = read_json(&evaluation.join("cases.json"))?;
    let baseline: Baseline = read_json(&evaluation.join("baseline.json"))?;
    let expected = cases
        .cases
        .into_iter()
        .map(|case| (case.id.clone(), case))
        .collect::<BTreeMap<_, _>>();
    let mut scores = BTreeMap::new();
    let mut failures = Vec::new();

    for arm in Arm::ALL {
        let mut score = Score::default();
        let mut occurrences = BTreeMap::new();
        for run in 1..=2 {
            for batch in 1..=2 {
                let filename = format!("{}-run-{run}-batch-{batch}.json", arm.filename());
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
        }
        compare_occurrences(arm, &expected, &occurrences, &mut failures);
        scores.insert(arm.baseline_key(), score);
    }

    compare_baseline(&scores, &baseline, &mut failures);
    if !failures.is_empty() {
        return Err(Box::new(GradingFailed(failures)));
    }

    for (arm, score) in scores {
        println!("{arm}: {}/{}", score.passed, score.total);
    }
    println!("admitted: {}", baseline.admitted);
    Ok(())
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
        if !seen.insert(result.case_id.clone()) {
            failures.push(format!("{filename}: duplicate case {}", result.case_id));
            continue;
        }
        let Some(expected) = expected.get(&result.case_id) else {
            failures.push(format!("{filename}: unknown case {}", result.case_id));
            continue;
        };
        *occurrences.entry(result.case_id.clone()).or_default() += 1;

        match extract_facts(arm, &result.screenplay) {
            Ok(actual) if actual == expected.facts() => score.passed += 1,
            Ok(actual) => failures.push(format!(
                "{filename}: {} differs\n  expected: {:?}\n  actual:   {:?}",
                result.case_id,
                expected.facts(),
                actual,
            )),
            Err(error) => failures.push(format!("{filename}: {}: {error}", result.case_id)),
        }
    }

    if seen.len() != 5 {
        failures.push(format!(
            "{filename}: expected 5 distinct cases, found {}",
            seen.len(),
        ));
    }
}

fn extract_facts(arm: Arm, screenplay: &str) -> Result<FilmFacts, InvalidScreenplay> {
    let report = compiler::parse(SourceId::new(0), screenplay);
    let (document, diagnostics) = report.into_parts();
    if !diagnostics.is_empty() {
        return Err(InvalidScreenplay::new(
            "screenplay is not well-formed markup",
        ));
    }
    let root = only_element(document.nodes(), "document")?;
    require_name(root, "film")?;
    require_attributes(root, &[])?;

    let mut facts = FilmFacts::default();
    let mut scene_index = 0;
    let mut cta_seen = false;
    for child in elements(root.children(), "film")? {
        match child.name().local() {
            "scene" => {
                facts.scenes.push(extract_scene(
                    arm,
                    child,
                    scene_index,
                    &mut facts.effects,
                    &mut cta_seen,
                )?);
                scene_index += 1;
            }
            "cues" => {
                if facts.cue.replace(extract_cues(child)?).is_some() {
                    return Err(InvalidScreenplay::new(
                        "screenplay contains more than one cues container",
                    ));
                }
            }
            _ if arm.is_music(child) => {
                facts
                    .music
                    .push(extract_audio(arm, child, AudioRole::Music)?)
            }
            name => return Err(unexpected_element(name, "film")),
        }
    }

    if facts.cue.is_some() != cta_seen {
        return Err(InvalidScreenplay::new(
            "cue declaration and call-to-action must appear together",
        ));
    }

    Ok(facts)
}

fn extract_scene(
    arm: Arm,
    scene: &Element,
    scene_index: usize,
    effects: &mut Vec<EffectExpectation>,
    cta_seen: &mut bool,
) -> Result<Vec<String>, InvalidScreenplay> {
    require_attributes(scene, &[])?;
    let mut videos = Vec::new();

    for (shot_index, shot) in elements(scene.children(), "scene")?.into_iter().enumerate() {
        require_name(shot, "shot")?;
        require_attributes(shot, &[])?;
        extract_shot(
            arm,
            shot,
            scene_index,
            shot_index,
            &mut videos,
            effects,
            cta_seen,
        )?;
    }

    Ok(videos)
}

fn extract_shot(
    arm: Arm,
    shot: &Element,
    scene: usize,
    shot_index: usize,
    videos: &mut Vec<String>,
    effects: &mut Vec<EffectExpectation>,
    cta_seen: &mut bool,
) -> Result<(), InvalidScreenplay> {
    for child in elements(shot.children(), "shot")? {
        match child.name().local() {
            "video" => {
                require_attributes(child, &["src"])?;
                require_empty(child)?;
                videos.push(attribute(child, "src")?.to_owned());
            }
            "cta" => {
                if *cta_seen {
                    return Err(InvalidScreenplay::new(
                        "screenplay contains more than one call-to-action",
                    ));
                }
                *cta_seen = true;
            }
            _ if arm.is_effect(child) => {
                let audio = extract_audio(arm, child, AudioRole::SoundEffect)?;
                effects.push(EffectExpectation {
                    scene,
                    shot: shot_index,
                    src: audio.src,
                    delay: audio.delay,
                    gain: audio.gain,
                });
            }
            name => return Err(unexpected_element(name, "shot")),
        }
    }
    Ok(())
}

fn extract_audio(
    arm: Arm,
    element: &Element,
    role: AudioRole,
) -> Result<AudioExpectation, InvalidScreenplay> {
    require_empty(element)?;
    let allowed = match arm {
        Arm::SemanticElements => &["src", "delay", "gain"][..],
        Arm::GenericAudio => &["kind", "src", "delay", "gain"][..],
    };
    require_attributes(element, allowed)?;
    if arm == Arm::GenericAudio {
        let expected_kind = role.generic_kind();
        if attribute(element, "kind")? != expected_kind {
            return Err(InvalidScreenplay::new("generic audio has the wrong kind"));
        }
    }

    Ok(AudioExpectation {
        src: attribute(element, "src")?.to_owned(),
        delay: optional_attribute(element, "delay").map(str::to_owned),
        gain: optional_attribute(element, "gain").map(str::to_owned),
    })
}

fn extract_cues(element: &Element) -> Result<CueExpectation, InvalidScreenplay> {
    require_attributes(element, &[])?;
    let cue = only_element(element.children(), "cues")?;
    require_name(cue, "cue")?;
    require_attributes(cue, &["id", "time"])?;
    require_empty(cue)?;

    Ok(CueExpectation {
        id: attribute(cue, "id")?.to_owned(),
        time: attribute(cue, "time")?.to_owned(),
        text: String::from("Buy now"),
    })
}

fn elements<'a>(nodes: &'a [Node], parent: &str) -> Result<Vec<&'a Element>, InvalidScreenplay> {
    let mut elements = Vec::new();
    for node in nodes {
        match node {
            Node::Element(element) => {
                if element.name().prefix().is_some() {
                    return Err(InvalidScreenplay::new(format!(
                        "qualified element <{}> is not part of the evaluation language",
                        element.name(),
                    )));
                }
                if element.name().local() == "cta" {
                    validate_cta(element)?;
                }
                elements.push(element);
            }
            Node::Text(text) if text.text().trim().is_empty() => {}
            Node::Text(_) => {
                return Err(InvalidScreenplay::new(format!(
                    "unexpected text inside <{parent}>"
                )));
            }
        }
    }
    Ok(elements)
}

fn validate_cta(element: &Element) -> Result<(), InvalidScreenplay> {
    require_attributes(element, &["cue"])?;
    if attribute(element, "cue")? != "offer" {
        return Err(InvalidScreenplay::new(
            "call-to-action refers to the wrong cue",
        ));
    }
    let mut text = String::new();
    for node in element.children() {
        match node {
            Node::Text(run) => text.push_str(run.text()),
            Node::Element(_) => {
                return Err(InvalidScreenplay::new(
                    "call-to-action contains a nested element",
                ));
            }
        }
    }
    if text != "Buy now" {
        return Err(InvalidScreenplay::new("call-to-action text differs"));
    }
    Ok(())
}

fn only_element<'a>(nodes: &'a [Node], parent: &str) -> Result<&'a Element, InvalidScreenplay> {
    let elements = elements(nodes, parent)?;
    match elements.as_slice() {
        [element] => Ok(element),
        _ => Err(InvalidScreenplay::new(format!(
            "<{parent}> must contain exactly one element"
        ))),
    }
}

fn require_name(element: &Element, expected: &str) -> Result<(), InvalidScreenplay> {
    if element.name().prefix().is_none() && element.name().local() == expected {
        return Ok(());
    }
    Err(InvalidScreenplay::new(format!(
        "expected <{expected}>, found <{}>",
        element.name(),
    )))
}

fn require_attributes(element: &Element, allowed: &[&str]) -> Result<(), InvalidScreenplay> {
    for attribute in element.attributes() {
        if attribute.name().prefix().is_some() || !allowed.contains(&attribute.name().local()) {
            return Err(InvalidScreenplay::new(format!(
                "unexpected attribute {} on <{}>",
                attribute.name(),
                element.name(),
            )));
        }
    }
    Ok(())
}

fn require_empty(element: &Element) -> Result<(), InvalidScreenplay> {
    if element.children().iter().all(|node| match node {
        Node::Text(text) => text.text().trim().is_empty(),
        Node::Element(_) => false,
    }) {
        return Ok(());
    }
    Err(InvalidScreenplay::new(format!(
        "<{}> must be empty",
        element.name(),
    )))
}

fn attribute<'a>(element: &'a Element, name: &str) -> Result<&'a str, InvalidScreenplay> {
    optional_attribute(element, name)
        .ok_or_else(|| InvalidScreenplay::new(format!("<{}> is missing {name}", element.name())))
}

fn optional_attribute<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element
        .attributes()
        .iter()
        .find_map(|attribute| unqualified(attribute, name).then_some(attribute.value()))
}

fn unqualified(attribute: &Attribute, name: &str) -> bool {
    attribute.name().prefix().is_none() && attribute.name().local() == name
}

fn unexpected_element(name: &str, parent: &str) -> InvalidScreenplay {
    InvalidScreenplay::new(format!("unexpected <{name}> inside <{parent}>"))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Box<dyn Error>> {
    let source = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&source)?)
}

fn compare_baseline(
    scores: &BTreeMap<&str, Score>,
    baseline: &Baseline,
    failures: &mut Vec<String>,
) {
    let expected_scores = [
        ("semantic-elements", baseline.semantic_elements),
        ("generic-audio", baseline.generic_audio),
    ];
    for (arm, expected) in expected_scores {
        match scores.get(arm) {
            Some(actual) if *actual == expected => {}
            Some(actual) => failures.push(format!(
                "{arm}: baseline {}/{} differs from {}/{}",
                expected.passed, expected.total, actual.passed, actual.total,
            )),
            None => failures.push(format!("{arm}: baseline names an unknown arm")),
        }
    }

    if baseline.admitted != ADMITTED_ARM {
        failures.push(format!(
            "admitted arm {:?} differs from the recorded decision {ADMITTED_ARM:?}",
            baseline.admitted,
        ));
    }
    if baseline.reason.trim().is_empty() {
        failures.push(String::from("admission reason must not be blank"));
    }
}

fn compare_occurrences(
    arm: Arm,
    expected: &BTreeMap<String, CaseExpectation>,
    occurrences: &BTreeMap<String, usize>,
    failures: &mut Vec<String>,
) {
    for id in expected.keys() {
        let count = occurrences.get(id).copied().unwrap_or_default();
        if count != 2 {
            failures.push(format!(
                "{}: case {id} occurs {count} times instead of once per repetition",
                arm.filename(),
            ));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Arm {
    SemanticElements,
    GenericAudio,
}

#[derive(Clone, Copy)]
enum AudioRole {
    Music,
    SoundEffect,
}

impl AudioRole {
    const fn generic_kind(self) -> &'static str {
        match self {
            Self::Music => "music",
            Self::SoundEffect => "sound-effect",
        }
    }
}

impl Arm {
    const ALL: [Self; 2] = [Self::SemanticElements, Self::GenericAudio];

    const fn filename(self) -> &'static str {
        match self {
            Self::SemanticElements => "semantic-elements",
            Self::GenericAudio => "generic-audio",
        }
    }

    const fn baseline_key(self) -> &'static str {
        self.filename()
    }

    fn is_music(self, element: &Element) -> bool {
        match self {
            Self::SemanticElements => element.name().local() == "music",
            Self::GenericAudio => {
                element.name().local() == "audio"
                    && optional_attribute(element, "kind") == Some("music")
            }
        }
    }

    fn is_effect(self, element: &Element) -> bool {
        match self {
            Self::SemanticElements => element.name().local() == "sfx",
            Self::GenericAudio => {
                element.name().local() == "audio"
                    && optional_attribute(element, "kind") == Some("sound-effect")
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct CaseSet {
    cases: Vec<CaseExpectation>,
}

#[derive(Clone, Debug, Deserialize)]
struct CaseExpectation {
    id: String,
    scenes: Vec<Vec<String>>,
    #[serde(default)]
    music: Vec<AudioExpectation>,
    #[serde(default)]
    effects: Vec<EffectExpectation>,
    cue: Option<CueExpectation>,
}

impl CaseExpectation {
    fn facts(&self) -> FilmFacts {
        FilmFacts {
            scenes: self.scenes.clone(),
            music: self.music.clone(),
            effects: self.effects.clone(),
            cue: self.cue.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct AudioExpectation {
    src: String,
    delay: Option<String>,
    gain: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct EffectExpectation {
    scene: usize,
    shot: usize,
    src: String,
    delay: Option<String>,
    gain: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct CueExpectation {
    id: String,
    time: String,
    text: String,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct FilmFacts {
    scenes: Vec<Vec<String>>,
    music: Vec<AudioExpectation>,
    effects: Vec<EffectExpectation>,
    cue: Option<CueExpectation>,
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
struct Baseline {
    #[serde(rename = "semantic-elements")]
    semantic_elements: Score,
    #[serde(rename = "generic-audio")]
    generic_audio: Score,
    admitted: String,
    reason: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
struct Score {
    passed: usize,
    total: usize,
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
        formatter.write_str("audio syntax evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}
