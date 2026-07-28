//! Offline grading for the checked-in audio-envelope syntax experiment.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use onmark_core::compiler;
use onmark_core::model::SourceId;
use onmark_core::syntax::{Attribute, Element, Node};
use serde::Deserialize;

const EVALUATION: &str = "evals/audio-envelope-syntax";
const ADMITTED_ARM: Arm = Arm::FadeAttributes;

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
    expected: &BTreeMap<String, ScreenplayExpectation>,
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

    compare_occurrences(arm, repetitions, expected, &occurrences, failures);
    score
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
    extract_film(arm, film)
}

fn extract_film(arm: Arm, film: &Element) -> Result<ScreenplayExpectation, InvalidScreenplay> {
    let mut music = Vec::new();
    let mut scenes = Vec::new();

    for child in elements(film.children(), "om-film")? {
        match child.name().local() {
            "om-music" => music.push(extract_track(arm, TrackRole::Music, child)?),
            "om-scene" => scenes.push(extract_scene(arm, child)?),
            name => return Err(unexpected_element(name, "om-film")),
        }
    }

    Ok(ScreenplayExpectation { music, scenes })
}

fn extract_scene(arm: Arm, scene: &Element) -> Result<Vec<ShotExpectation>, InvalidScreenplay> {
    require_element(scene, "om-scene", &[])?;
    let mut shots = Vec::new();

    for child in elements(scene.children(), "om-scene")? {
        require_element(child, "om-shot", &[])?;
        shots.push(extract_shot(arm, child)?);
    }

    Ok(shots)
}

fn extract_shot(arm: Arm, shot: &Element) -> Result<ShotExpectation, InvalidScreenplay> {
    let mut video = None;
    let mut voice_overs = Vec::new();
    let mut effects = Vec::new();

    for child in elements(shot.children(), "om-shot")? {
        match child.name().local() {
            "video" => {
                if video.replace(extract_video(child)?).is_some() {
                    return Err(InvalidScreenplay::new("shot contains more than one video"));
                }
            }
            "om-vo" => voice_overs.push(extract_track(arm, TrackRole::VoiceOver, child)?),
            "om-sfx" => effects.push(extract_track(arm, TrackRole::SoundEffect, child)?),
            name => return Err(unexpected_element(name, "om-shot")),
        }
    }

    let Some(video) = video else {
        return Err(InvalidScreenplay::new("shot is missing its video"));
    };

    Ok(ShotExpectation {
        video,
        voice_overs,
        effects,
    })
}

fn extract_video(video: &Element) -> Result<String, InvalidScreenplay> {
    require_element(video, "video", &["src"])?;
    require_empty(video)?;
    Ok(attribute(video, "src")?.to_owned())
}

fn extract_track(
    arm: Arm,
    role: TrackRole,
    element: &Element,
) -> Result<TrackExpectation, InvalidScreenplay> {
    require_element(element, role.element_name(), role.attributes(arm))?;
    let body = AudioBody::extract(element)?;
    let (fade_in, fade_out) = arm.extract_fades(element, body.envelope)?;
    let text = role.extract_text(body.text)?;

    Ok(TrackExpectation {
        src: attribute(element, "src")?.to_owned(),
        delay: optional_attribute(element, "delay").map(str::to_owned),
        gain: optional_attribute(element, "gain").map(str::to_owned),
        fade_in,
        fade_out,
        text,
    })
}

struct AudioBody<'a> {
    envelope: Option<&'a Element>,
    text: String,
}

impl<'a> AudioBody<'a> {
    fn extract(element: &'a Element) -> Result<Self, InvalidScreenplay> {
        let mut envelope = None;
        let mut text = String::new();

        for child in element.children() {
            match child {
                Node::Element(child) if child.name().local() == "om-envelope" => {
                    if envelope.replace(child).is_some() {
                        return Err(InvalidScreenplay::new(
                            "audio contains more than one envelope",
                        ));
                    }
                }
                Node::Element(child) => {
                    return Err(unexpected_element(
                        child.name().local(),
                        element.name().local(),
                    ));
                }
                Node::Text(run) => text.push_str(run.text()),
            }
        }

        Ok(Self { envelope, text })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Arm {
    FadeAttributes,
    EnvelopeElement,
}

impl Arm {
    const ALL: [Self; 2] = [Self::FadeAttributes, Self::EnvelopeElement];

    const fn filename(self) -> &'static str {
        match self {
            Self::FadeAttributes => "fade-attributes",
            Self::EnvelopeElement => "envelope-element",
        }
    }

    fn extract_fades(
        self,
        element: &Element,
        envelope: Option<&Element>,
    ) -> Result<(Option<String>, Option<String>), InvalidScreenplay> {
        match self {
            Self::FadeAttributes => {
                if envelope.is_some() {
                    return Err(InvalidScreenplay::new(
                        "attribute spelling contains an envelope element",
                    ));
                }
                Ok((
                    optional_attribute(element, "fade-in").map(str::to_owned),
                    optional_attribute(element, "fade-out").map(str::to_owned),
                ))
            }
            Self::EnvelopeElement => extract_envelope(envelope),
        }
    }
}

fn extract_envelope(
    envelope: Option<&Element>,
) -> Result<(Option<String>, Option<String>), InvalidScreenplay> {
    let Some(envelope) = envelope else {
        return Ok((None, None));
    };

    require_element(envelope, "om-envelope", &["in", "out"])?;
    require_empty(envelope)?;
    let fade_in = optional_attribute(envelope, "in").map(str::to_owned);
    let fade_out = optional_attribute(envelope, "out").map(str::to_owned);
    if fade_in.is_none() && fade_out.is_none() {
        return Err(InvalidScreenplay::new("audio envelope has no fade edge"));
    }

    Ok((fade_in, fade_out))
}

#[derive(Clone, Copy)]
enum TrackRole {
    Music,
    VoiceOver,
    SoundEffect,
}

impl TrackRole {
    const fn element_name(self) -> &'static str {
        match self {
            Self::Music => "om-music",
            Self::VoiceOver => "om-vo",
            Self::SoundEffect => "om-sfx",
        }
    }

    const fn attributes(self, arm: Arm) -> &'static [&'static str] {
        match (self, arm) {
            (Self::Music, Arm::FadeAttributes) => &["src", "gain", "fade-in", "fade-out"],
            (Self::VoiceOver, Arm::FadeAttributes) => &["src", "delay", "fade-in", "fade-out"],
            (Self::SoundEffect, Arm::FadeAttributes) => {
                &["src", "delay", "gain", "fade-in", "fade-out"]
            }
            (Self::Music, Arm::EnvelopeElement) => &["src", "gain"],
            (Self::VoiceOver, Arm::EnvelopeElement) => &["src", "delay"],
            (Self::SoundEffect, Arm::EnvelopeElement) => &["src", "delay", "gain"],
        }
    }

    fn extract_text(self, text: String) -> Result<Option<String>, InvalidScreenplay> {
        let text = normalize_text(&text);
        match self {
            Self::VoiceOver if text.is_empty() => {
                Err(InvalidScreenplay::new("voice-over inscription is empty"))
            }
            Self::VoiceOver => Ok(Some(text)),
            Self::Music | Self::SoundEffect if text.is_empty() => Ok(None),
            Self::Music | Self::SoundEffect => {
                Err(InvalidScreenplay::new("non-narrative audio contains text"))
            }
        }
    }
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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

fn unexpected_element(name: &str, parent: &str) -> InvalidScreenplay {
    InvalidScreenplay::new(format!("unexpected <{name}> inside <{parent}>"))
}

fn index_cases(
    cases: Vec<CaseExpectation>,
) -> Result<BTreeMap<String, ScreenplayExpectation>, GradingFailed> {
    let mut expected = BTreeMap::new();

    for case in cases {
        let (id, screenplay) = case.into_parts();
        if expected.insert(id.clone(), screenplay).is_some() {
            return Err(GradingFailed(vec![format!(
                "case definition {id} is duplicated",
            )]));
        }
    }

    Ok(expected)
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

fn compare_baseline(
    scores: &BTreeMap<&str, Score>,
    baseline: &Baseline,
    failures: &mut Vec<String>,
) {
    let expected = [
        (Arm::FadeAttributes.filename(), baseline.fade_attributes),
        (Arm::EnvelopeElement.filename(), baseline.envelope_element),
    ];

    for (arm, expected) in expected {
        if scores.get(arm) != Some(&expected) {
            failures.push(format!(
                "{arm}: expected baseline {expected:?}, found {:?}",
                scores.get(arm),
            ));
        }
    }
    if baseline.admitted != ADMITTED_ARM.filename() {
        failures.push(format!(
            "baseline admits {}, expected {}",
            baseline.admitted,
            ADMITTED_ARM.filename(),
        ));
    }
    if baseline.reason.trim().is_empty() {
        failures.push(String::from("baseline admission reason is blank"));
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
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
    #[serde(default)]
    music: Vec<TrackExpectation>,
    scenes: Vec<Vec<ShotExpectation>>,
}

impl CaseExpectation {
    fn into_parts(self) -> (String, ScreenplayExpectation) {
        (
            self.id,
            ScreenplayExpectation {
                music: self.music,
                scenes: self.scenes,
            },
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ScreenplayExpectation {
    #[serde(default)]
    music: Vec<TrackExpectation>,
    scenes: Vec<Vec<ShotExpectation>>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ShotExpectation {
    video: String,
    #[serde(default)]
    voice_overs: Vec<TrackExpectation>,
    #[serde(default)]
    effects: Vec<TrackExpectation>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
struct TrackExpectation {
    src: String,
    delay: Option<String>,
    gain: Option<String>,
    fade_in: Option<String>,
    fade_out: Option<String>,
    text: Option<String>,
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
    #[serde(rename = "fade-attributes")]
    fade_attributes: Score,
    #[serde(rename = "envelope-element")]
    envelope_element: Score,
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
        formatter.write_str("audio-envelope syntax evaluation differs from its baseline")?;
        for failure in &self.0 {
            write!(formatter, "\n- {failure}")?;
        }
        Ok(())
    }
}

impl Error for GradingFailed {}

#[cfg(test)]
mod tests {
    use super::{Arm, extract_screenplay};

    #[test]
    fn both_spellings_extract_the_same_audio_facts() {
        let attributes = extract_screenplay(
            Arm::FadeAttributes,
            r#"
                <om-film>
                  <om-music src="bed.wav" fade-in="200ms" fade-out="400ms"></om-music>
                  <om-scene>
                    <om-shot>
                      <video src="clip.mp4"></video>
                      <om-vo src="voice.wav" fade-in="100ms">Exact words.</om-vo>
                    </om-shot>
                  </om-scene>
                </om-film>
            "#,
        )
        .expect("the attribute spelling is valid evaluation input");
        let element = extract_screenplay(
            Arm::EnvelopeElement,
            r#"
                <om-film>
                  <om-music src="bed.wav">
                    <om-envelope in="200ms" out="400ms"></om-envelope>
                  </om-music>
                  <om-scene>
                    <om-shot>
                      <video src="clip.mp4"></video>
                      <om-vo src="voice.wav">
                        <om-envelope in="100ms"></om-envelope>
                        Exact words.
                      </om-vo>
                    </om-shot>
                  </om-scene>
                </om-film>
            "#,
        )
        .expect("the element spelling is valid evaluation input");

        assert_eq!(attributes, element);
    }

    #[test]
    fn one_arm_cannot_smuggle_in_the_other_spelling() {
        let error = extract_screenplay(
            Arm::FadeAttributes,
            r#"
                <om-film>
                  <om-music src="bed.wav">
                    <om-envelope in="200ms"></om-envelope>
                  </om-music>
                  <om-scene>
                    <om-shot><video src="clip.mp4"></video></om-shot>
                  </om-scene>
                </om-film>
            "#,
        )
        .expect_err("the attribute arm must reject an envelope child");

        assert_eq!(
            error.to_string(),
            "attribute spelling contains an envelope element",
        );
    }
}
