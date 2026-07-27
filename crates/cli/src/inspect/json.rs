//! Versioned machine projection of Timeline IR and render-region facts.

use std::io::{self, Write};

use onmark_core::model::{EventRef, FrameInterval, MediaSource, NodeId, PlaybackRate, SourceSpan};
use onmark_core::timeline::{
    TimelineAudio, TimelineCaption, TimelineContent, TimelineElement, TimelineEvent, TimelineIr,
    TimelineScene, TimelineShot, TimelineText, TimelineTiming, TimingReason,
};
use serde::Serialize;

use crate::check::{Inspection, RegionInspection, Validation};
use crate::diagnostic::JsonDiagnostic;

const REPORT_VERSION: u16 = 2;

pub(super) fn write(validation: &Validation) -> io::Result<()> {
    let report = &validation.report;
    let document = InspectReport {
        version: REPORT_VERSION,
        command: "inspect",
        valid: validation.inspection.is_some(),
        source: report.path.display().to_string(),
        diagnostics: report
            .diagnostics
            .iter()
            .map(JsonDiagnostic::from)
            .collect(),
        inspection: validation.inspection.as_ref().map(JsonInspection::from),
    };
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &document)?;
    writeln!(stdout)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InspectReport<'a> {
    version: u16,
    command: &'static str,
    valid: bool,
    source: String,
    diagnostics: Vec<JsonDiagnostic<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inspection: Option<JsonInspection<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonInspection<'a> {
    timeline: JsonTimeline<'a>,
    frozen_assets: usize,
    regions: Vec<JsonRegion<'a>>,
}

impl<'a> From<&'a Inspection> for JsonInspection<'a> {
    fn from(inspection: &'a Inspection) -> Self {
        Self {
            timeline: JsonTimeline::from(&inspection.timeline),
            frozen_assets: inspection.assets,
            regions: inspection.regions.iter().map(JsonRegion::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonTimeline<'a> {
    version: u16,
    frame_rate: JsonFrameRate,
    interval: JsonInterval,
    film: JsonElement<'a>,
    events: Vec<JsonEvent<'a>>,
    scenes: Vec<JsonScene<'a>>,
    audio: Vec<JsonAudio<'a>>,
    captions: Vec<JsonCaption<'a>>,
}

impl<'a> From<&'a TimelineIr> for JsonTimeline<'a> {
    fn from(timeline: &'a TimelineIr) -> Self {
        Self {
            version: timeline.version().get(),
            frame_rate: timeline.timebase().frame_rate().into(),
            interval: timeline.interval().into(),
            film: timeline.element().into(),
            events: timeline
                .events()
                .map(|(id, event)| JsonEvent::new(id.as_str(), event))
                .collect(),
            scenes: timeline.scenes().iter().map(JsonScene::from).collect(),
            audio: timeline.audio().map(JsonAudio::from).collect(),
            captions: timeline.captions().iter().map(JsonCaption::from).collect(),
        }
    }
}

#[derive(Clone, Copy, Serialize)]
struct JsonFrameRate {
    numerator: u32,
    denominator: u32,
}

impl From<onmark_core::model::FrameRate> for JsonFrameRate {
    fn from(frame_rate: onmark_core::model::FrameRate) -> Self {
        Self {
            numerator: frame_rate.numerator(),
            denominator: frame_rate.denominator(),
        }
    }
}

#[derive(Clone, Copy, Serialize)]
struct JsonInterval {
    start: u64,
    end: u64,
}

impl From<FrameInterval> for JsonInterval {
    fn from(interval: FrameInterval) -> Self {
        Self {
            start: interval.start().get(),
            end: interval.end().get(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSpan {
    start_byte: u64,
    end_byte: u64,
}

impl From<SourceSpan> for JsonSpan {
    fn from(span: SourceSpan) -> Self {
        Self {
            start_byte: span.start().get(),
            end_byte: span.end().get(),
        }
    }
}

#[derive(Serialize)]
struct JsonElement<'a> {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    span: JsonSpan,
}

impl<'a> From<&'a TimelineElement> for JsonElement<'a> {
    fn from(element: &'a TimelineElement) -> Self {
        Self {
            kind: element.kind().as_str(),
            id: element.id().map(NodeId::as_str),
            span: element.span().into(),
        }
    }
}

#[derive(Serialize)]
struct JsonEvent<'a> {
    id: &'a str,
    frame: u64,
    span: JsonSpan,
}

impl<'a> JsonEvent<'a> {
    fn new(id: &'a str, event: &TimelineEvent) -> Self {
        Self {
            id,
            frame: event.at().get(),
            span: event.authored_at().into(),
        }
    }
}

#[derive(Serialize)]
struct JsonScene<'a> {
    element: JsonElement<'a>,
    timing: JsonTiming<'a>,
    shots: Vec<JsonShot<'a>>,
}

impl<'a> From<&'a TimelineScene> for JsonScene<'a> {
    fn from(scene: &'a TimelineScene) -> Self {
        Self {
            element: scene.element().into(),
            timing: scene.timing().into(),
            shots: scene.shots().iter().map(JsonShot::from).collect(),
        }
    }
}

#[derive(Serialize)]
struct JsonShot<'a> {
    element: JsonElement<'a>,
    timing: JsonTiming<'a>,
    content: Vec<JsonContent<'a>>,
}

impl<'a> From<&'a TimelineShot> for JsonShot<'a> {
    fn from(shot: &'a TimelineShot) -> Self {
        Self {
            element: shot.element().into(),
            timing: shot.timing().into(),
            content: shot.content().iter().map(JsonContent::from).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonContent<'a> {
    element: JsonElement<'a>,
    timing: JsonTiming<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<JsonMediaSource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
}

impl<'a> From<&'a TimelineContent> for JsonContent<'a> {
    fn from(content: &'a TimelineContent) -> Self {
        match content {
            TimelineContent::Video(video) => Self {
                element: video.element().into(),
                timing: video.timing().into(),
                asset_id: Some(video.asset_id().to_string()),
                source: Some(video.source().into()),
                text: None,
            },
            TimelineContent::VoiceOver(voice_over) => Self {
                element: voice_over.element().into(),
                timing: voice_over.timing().into(),
                asset_id: Some(voice_over.asset_id().to_string()),
                source: None,
                text: Some(text(voice_over.text())),
            },
            TimelineContent::Overlay(overlay) => Self {
                element: overlay.element().into(),
                timing: overlay.timing().into(),
                asset_id: None,
                source: None,
                text: Some(text(overlay.text())),
            },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonMediaSource {
    start_nanoseconds: String,
    end_nanoseconds: String,
    natural_end_nanoseconds: String,
    playback_rate: JsonPlaybackRate,
    plays: u32,
    hold_last_nanoseconds: String,
}

impl From<MediaSource> for JsonMediaSource {
    fn from(source: MediaSource) -> Self {
        let interval = source.interval();
        Self {
            start_nanoseconds: interval.start().as_nanos().to_string(),
            end_nanoseconds: interval.end().as_nanos().to_string(),
            natural_end_nanoseconds: source.natural_duration().as_nanos().to_string(),
            playback_rate: source.playback_rate().into(),
            plays: source.plays().get(),
            hold_last_nanoseconds: source.hold_last().as_nanos().to_string(),
        }
    }
}

#[derive(Serialize)]
struct JsonPlaybackRate {
    numerator: u32,
    denominator: u32,
}

impl From<PlaybackRate> for JsonPlaybackRate {
    fn from(rate: PlaybackRate) -> Self {
        Self {
            numerator: rate.numerator(),
            denominator: rate.denominator(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonAudio<'a> {
    kind: &'static str,
    timing: JsonTiming<'a>,
    asset_id: String,
    gain: JsonGain,
}

impl<'a> From<&'a TimelineAudio> for JsonAudio<'a> {
    fn from(audio: &'a TimelineAudio) -> Self {
        let gain = audio.gain();
        Self {
            kind: audio.kind().as_str(),
            timing: audio.timing().into(),
            asset_id: audio.asset_id().to_string(),
            gain: JsonGain {
                numerator: gain.numerator(),
                denominator: gain.denominator(),
            },
        }
    }
}

#[derive(Serialize)]
struct JsonGain {
    numerator: u32,
    denominator: u32,
}

#[derive(Serialize)]
struct JsonCaption<'a> {
    interval: JsonInterval,
    text: &'a str,
    timing_span: JsonSpan,
    text_span: JsonSpan,
}

impl<'a> From<&'a TimelineCaption> for JsonCaption<'a> {
    fn from(caption: &'a TimelineCaption) -> Self {
        Self {
            interval: caption.interval().into(),
            text: caption.text(),
            timing_span: caption.timing_span().into(),
            text_span: caption.text_span().into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonTiming<'a> {
    interval: JsonInterval,
    start_reason: JsonReason<'a>,
    end_reason: JsonReason<'a>,
}

impl<'a> From<&'a TimelineTiming> for JsonTiming<'a> {
    fn from(timing: &'a TimelineTiming) -> Self {
        Self {
            interval: timing.interval().into(),
            start_reason: timing.start_reason().into(),
            end_reason: timing.end_reason().into(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonReason<'a> {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    event: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    span: Option<JsonSpan>,
}

impl<'a> From<&'a TimingReason> for JsonReason<'a> {
    fn from(reason: &'a TimingReason) -> Self {
        Self {
            kind: reason.as_str(),
            event: reason.event().map(event_id),
            span: reason.authored_at().map(JsonSpan::from),
        }
    }
}

fn event_id(event: &EventRef) -> &str {
    match event {
        EventRef::Cue(id) => id.as_str(),
    }
}

fn text(runs: &[TimelineText]) -> String {
    runs.iter().map(TimelineText::text).collect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonRegion<'a> {
    evaluation: JsonInterval,
    output: JsonInterval,
    visual_mode: &'a str,
    capture_cadence: &'a str,
    bundle_id: &'a str,
}

impl<'a> From<&'a RegionInspection> for JsonRegion<'a> {
    fn from(region: &'a RegionInspection) -> Self {
        Self {
            evaluation: JsonInterval {
                start: region.evaluation_start,
                end: region.evaluation_end,
            },
            output: JsonInterval {
                start: region.output_start,
                end: region.output_end,
            },
            visual_mode: region.visual_mode,
            capture_cadence: region.capture_cadence,
            bundle_id: &region.bundle_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use onmark_core::model::{
        AssetMetadata, AssetRef, Duration, FrameRate, FrozenAsset, FrozenAssetId, Timebase,
        VideoDimensions, VideoMetadata, VideoTiming,
    };

    use super::JsonTimeline;
    use crate::compilation;

    #[test]
    fn projects_timing_provenance_without_reconstructing_the_timeline() {
        let source = concat!(
            r#"<om-film id="film"><om-cues>"#,
            r#"<om-cue id="reveal" time="1s"></om-cue>"#,
            r#"</om-cues><om-scene id="scene"><om-shot id="shot" duration="2s">"#,
            r#"<om-title id="title" cue="reveal">Exact.</om-title>"#,
            "</om-shot></om-scene></om-film>",
        );
        let (film, diagnostics) = compilation::resolve(source).into_parts();
        assert!(diagnostics.is_empty());
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let (timeline, diagnostics) = compilation::solve(
            film.expect("the fixture resolves"),
            &BTreeMap::new(),
            Timebase::new(rate),
            diagnostics,
        )
        .expect("the fixture solves")
        .into_parts();
        assert!(diagnostics.is_empty());

        let document = serde_json::to_value(JsonTimeline::from(
            &timeline.expect("the fixture produces Timeline IR"),
        ))
        .expect("the inspection projection serializes");

        assert_eq!(document["events"][0]["id"], "reveal");
        assert_eq!(document["events"][0]["frame"], 30);
        assert_eq!(
            document["scenes"][0]["shots"][0]["content"][0]["timing"]["startReason"]["kind"],
            "event",
        );
        assert_eq!(
            document["scenes"][0]["shots"][0]["content"][0]["timing"]["startReason"]["event"],
            "reveal",
        );
        assert_eq!(
            document["scenes"][0]["shots"][0]["content"][0]["text"],
            "Exact.",
        );
    }

    #[test]
    fn projects_exact_video_source_facts() {
        let source = concat!(
            "<om-film><om-scene><om-shot>",
            r#"<video src="clip.mp4" trim="4s..10s" speed="2x" plays="2" hold-last="1s"></video>"#,
            "</om-shot></om-scene></om-film>",
        );
        let (film, diagnostics) = compilation::resolve(source).into_parts();
        assert!(diagnostics.is_empty());
        let rate = FrameRate::new(30, 1).expect("the fixture rate is valid");
        let assets = BTreeMap::from([fixture_video("clip.mp4", rate)]);
        let (timeline, diagnostics) = compilation::solve(
            film.expect("the fixture resolves"),
            &assets,
            Timebase::new(rate),
            diagnostics,
        )
        .expect("the fixture solves")
        .into_parts();
        assert!(diagnostics.is_empty());

        let document = serde_json::to_value(JsonTimeline::from(
            &timeline.expect("the fixture produces Timeline IR"),
        ))
        .expect("the inspection projection serializes");
        let video = &document["scenes"][0]["shots"][0]["content"][0];

        assert_eq!(video["source"]["startNanoseconds"], "4000000000");
        assert_eq!(video["source"]["endNanoseconds"], "10000000000");
        assert_eq!(video["source"]["naturalEndNanoseconds"], "12000000000");
        assert_eq!(video["source"]["playbackRate"]["numerator"], 2);
        assert_eq!(video["source"]["playbackRate"]["denominator"], 1);
        assert_eq!(video["source"]["plays"], 2);
        assert_eq!(video["source"]["holdLastNanoseconds"], "1000000000");
    }

    fn fixture_video(asset: &str, rate: FrameRate) -> (AssetRef, FrozenAsset) {
        let duration = Duration::parse("12s").expect("the fixture duration is valid");
        let dimensions =
            VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive");
        let video = VideoMetadata::new(
            duration,
            dimensions,
            "h264",
            "yuv420p",
            VideoTiming::Constant(rate),
        )
        .expect("the fixture video metadata is normalized");
        let metadata = AssetMetadata::video(duration, video);

        (
            AssetRef::parse(asset).expect("the fixture asset reference is valid"),
            FrozenAsset::new(FrozenAssetId::from_sha256([1; 32]), metadata),
        )
    }
}
