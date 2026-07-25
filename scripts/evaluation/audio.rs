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

// ── Grading pipeline

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
        scores.insert(arm.filename(), score);
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

// ── Screenplay facts

fn extract_facts(arm: Arm, screenplay: &str) -> Result<FilmFacts, InvalidScreenplay> {
    let normalized = expand_empty_elements(screenplay);
    let report = compiler::parse(SourceId::new(0), &normalized);
    let (document, diagnostics) = report.into_parts();
    if !diagnostics.is_empty() {
        return Err(InvalidScreenplay::new(
            "screenplay is not well-formed markup",
        ));
    }
    let root = only_element(document.nodes(), "document")?;
    require_name(root, "film")?;
    require_attributes(root, &[])?;

    FactExtractor::new(arm).extract(root)
}

fn expand_empty_elements(source: &str) -> String {
    let mut expanded = source.to_owned();
    let mut search_from = 0;
    while let Some(relative_end) = expanded[search_from..].find("/>") {
        let end = search_from + relative_end;
        let Some(start) = expanded[..end].rfind('<') else {
            break;
        };
        let name = expanded[start + 1..end]
            .split_ascii_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned();
        expanded.replace_range(end..end + 2, &format!("></{name}>"));
        search_from = end + name.len() + 3;
    }
    expanded
}

struct FactExtractor {
    arm: Arm,
    facts: FilmFacts,
    cta_seen: bool,
}

impl FactExtractor {
    fn new(arm: Arm) -> Self {
        Self {
            arm,
            facts: FilmFacts::default(),
            cta_seen: false,
        }
    }

    fn extract(mut self, root: &Element) -> Result<FilmFacts, InvalidScreenplay> {
        for child in elements(root.children(), "film")? {
            match child.name().local() {
                "scene" => {
                    let scene = self.extract_scene(child)?;
                    self.facts.scenes.push(scene);
                }
                "cues" => {
                    if self.facts.cue.replace(extract_cues(child)?).is_some() {
                        return Err(InvalidScreenplay::new(
                            "screenplay contains more than one cues container",
                        ));
                    }
                }
                _ if self.arm.is_music(child) => {
                    self.facts
                        .music
                        .push(extract_audio(self.arm, child, AudioRole::Music)?)
                }
                name => return Err(unexpected_element(name, "film")),
            }
        }

        if self.facts.cue.is_some() != self.cta_seen {
            return Err(InvalidScreenplay::new(
                "cue declaration and call-to-action must appear together",
            ));
        }
        Ok(self.facts)
    }

    fn extract_scene(&mut self, scene: &Element) -> Result<Vec<String>, InvalidScreenplay> {
        require_attributes(scene, &[])?;
        let scene_index = self.facts.scenes.len();
        let mut videos = Vec::new();

        for (shot_index, shot) in elements(scene.children(), "scene")?.into_iter().enumerate() {
            require_name(shot, "shot")?;
            require_attributes(shot, &[])?;
            self.extract_shot(shot, scene_index, shot_index, &mut videos)?;
        }
        Ok(videos)
    }

    fn extract_shot(
        &mut self,
        shot: &Element,
        scene: usize,
        shot_index: usize,
        videos: &mut Vec<String>,
    ) -> Result<(), InvalidScreenplay> {
        for child in elements(shot.children(), "shot")? {
            match child.name().local() {
                "video" => {
                    require_attributes(child, &["src"])?;
                    require_empty(child)?;
                    videos.push(attribute(child, "src")?.to_owned());
                }
                "cta" => {
                    if self.cta_seen {
                        return Err(InvalidScreenplay::new(
                            "screenplay contains more than one call-to-action",
                        ));
                    }
                    self.cta_seen = true;
                }
                _ if self.arm.is_effect(child) => {
                    let audio = extract_audio(self.arm, child, AudioRole::SoundEffect)?;
                    self.facts.effects.push(EffectExpectation {
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

// ── Markup queries

fn elements<'a>(nodes: &'a [Node], parent: &str) -> Result<Vec<&'a Element>, InvalidScreenplay> {
    let mut elements = Vec::new();
    for node in nodes {
        match node {
            Node::Element(element) => {
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
    if element.name().local() == expected {
        return Ok(());
    }
    Err(InvalidScreenplay::new(format!(
        "expected <{expected}>, found <{}>",
        element.name(),
    )))
}

fn require_attributes(element: &Element, allowed: &[&str]) -> Result<(), InvalidScreenplay> {
    for attribute in element.attributes() {
        if !allowed.contains(&attribute.name().local()) {
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
        .find_map(|attribute| attribute_named(attribute, name).then_some(attribute.value()))
}

fn attribute_named(attribute: &Attribute, name: &str) -> bool {
    attribute.name().local() == name
}

fn unexpected_element(name: &str, parent: &str) -> InvalidScreenplay {
    InvalidScreenplay::new(format!("unexpected <{name}> inside <{parent}>"))
}

// ── Baseline contract

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

// ── Evaluation model

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
        formatter.write_str("audio syntax evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}
