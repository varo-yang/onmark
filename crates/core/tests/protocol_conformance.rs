//! Versioned browser protocol fixtures projected from solved core facts.

mod conformance;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use onmark_core::compiler;
use onmark_core::model::{
    AssetMetadata, AssetRef, Duration, FrameRate, FrozenAsset, FrozenAssetId, SourceId, Timebase,
    VideoDimensions, VideoMetadata, VideoTiming,
};
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, BrowserResponse, BundleManifest,
    InvalidBrowserPlan, ProtocolFailure, ProtocolFailureCode, RequestId, WireFrame,
};
use onmark_core::timeline::TimelineIr;

use conformance::{assert_or_update, fixture};

#[test]
fn gate_one_browser_requests_match_the_versioned_wire_contract() {
    let plan = gate_one_plan();
    let requests = [
        request(1, BrowserCommand::Load { plan }),
        request(
            2,
            BrowserCommand::Prepare {
                evaluation_start: frame(0),
            },
        ),
        request(3, BrowserCommand::Seek { frame: frame(15) }),
        request(4, BrowserCommand::Confirm { frame: frame(15) }),
        request(5, BrowserCommand::Dispose),
    ];

    assert_or_update(
        &fixture("protocol", "browser-requests-v1.jsonl"),
        &render_json_lines(&requests),
    );
}

#[test]
fn gate_one_browser_responses_match_the_versioned_wire_contract() {
    let timeout = ProtocolFailure::new(
        ProtocolFailureCode::ReadinessTimeout,
        "frame 15 did not become ready",
        vec![Box::from("video-frame")],
    )
    .expect("the fixture failure is actionable");
    let responses = [
        response(1, BrowserEvent::Loaded),
        response(
            2,
            BrowserEvent::Prepared {
                evaluation_start: frame(0),
            },
        ),
        response(3, BrowserEvent::FrameStaged { frame: frame(15) }),
        response(4, BrowserEvent::FrameReady { frame: frame(15) }),
        response(4, BrowserEvent::Failed(timeout)),
        response(5, BrowserEvent::Disposed),
    ];

    assert_or_update(
        &fixture("protocol", "browser-responses-v1.jsonl"),
        &render_json_lines(&responses),
    );
}

#[test]
fn browser_plan_requires_an_admitted_rate_for_every_video() {
    let (timeline, asset_id, _rate) = gate_one_timeline();

    assert_eq!(
        BrowserPlan::from_timeline(&timeline, &BTreeMap::new()),
        Err(InvalidBrowserPlan::MissingSourceFrameRate(asset_id)),
    );
}

#[test]
fn browser_plan_retains_solved_structure_and_content_ownership() {
    let plan = serde_json::to_value(gate_one_plan()).expect("the browser plan must serialize");

    assert_eq!(
        plan["film"],
        serde_json::json!({ "nodeId": 0, "authoredId": null }),
    );
    assert_eq!(
        plan["scenes"],
        serde_json::json!([{
            "node": { "nodeId": 1, "authoredId": null },
            "interval": { "start": 0, "end": 75 }
        }]),
    );
    assert_eq!(
        plan["shots"],
        serde_json::json!([{
            "node": { "nodeId": 2, "authoredId": null },
            "sceneId": 1,
            "interval": { "start": 0, "end": 75 }
        }]),
    );
    assert_eq!(
        plan["videos"],
        serde_json::json!([{
            "node": { "nodeId": 3, "authoredId": null },
            "shotId": 2,
            "assetId": "sha256:0101010101010101010101010101010101010101010101010101010101010101",
            "interval": { "start": 0, "end": 75 },
            "sourceFrameRate": { "numerator": 30, "denominator": 1 }
        }]),
    );
    assert_eq!(
        plan["overlays"],
        serde_json::json!([
            {
                "node": { "nodeId": 4, "authoredId": null },
                "shotId": 2,
                "kind": "title",
                "text": "Opening",
                "interval": { "start": 15, "end": 75 }
            },
            {
                "node": { "nodeId": 5, "authoredId": null },
                "shotId": 2,
                "kind": "callToAction",
                "text": "Buy now",
                "interval": { "start": 45, "end": 75 }
            }
        ]),
    );
}

#[test]
fn current_bundle_manifest_matches_the_versioned_wire_contract() {
    assert_bundle_manifest("bundle-v1/manifest.json");
}

fn assert_bundle_manifest(name: &str) {
    let path = fixture("protocol", name);
    let source = std::fs::read_to_string(&path).expect("the bundle fixture must be readable");
    let manifest = serde_json::from_str::<BundleManifest>(&source)
        .expect("the bundle fixture must satisfy the Rust wire contract");
    let mut encoded = serde_json::to_string_pretty(&manifest)
        .expect("the bundle manifest must serialize deterministically");
    encoded.push('\n');

    assert_or_update(&path, &encoded);
}

fn request(request_id: u32, command: BrowserCommand) -> BrowserRequest {
    BrowserRequest::new(RequestId::new(request_id), command)
}

fn response(request_id: u32, event: BrowserEvent) -> BrowserResponse {
    BrowserResponse::new(RequestId::new(request_id), event)
}

fn frame(index: u64) -> WireFrame {
    WireFrame::new(index).expect("fixture frames are browser-safe")
}

fn gate_one_plan() -> BrowserPlan {
    let (timeline, asset_id, rate) = gate_one_timeline();
    let source_frame_rates = BTreeMap::from([(asset_id, rate)]);

    BrowserPlan::from_timeline(&timeline, &source_frame_rates)
        .expect("the fixture timeline fits the browser frame domain")
}

fn gate_one_timeline() -> (TimelineIr, FrozenAssetId, FrameRate) {
    let rate = FrameRate::new(30, 1).expect("the fixture frame rate is valid");
    let asset_id = FrozenAssetId::from_sha256([1; 32]);
    let duration = Duration::from_nanos(2_500_000_000);
    let video = VideoMetadata::new(
        duration,
        VideoDimensions::new(1_920, 1_080).expect("fixture dimensions are positive"),
        "h264",
        "yuv420p",
        VideoTiming::Constant(rate),
    )
    .expect("the fixture video metadata is normalized");
    let asset = AssetRef::parse("opening.mp4").expect("the fixture asset is valid");
    let assets = BTreeMap::from([(
        asset,
        FrozenAsset::new(asset_id, AssetMetadata::video(duration, video)),
    )]);
    let parsed = compiler::parse(
        SourceId::new(0),
        concat!(
            "<om-film><om-scene><om-shot>",
            r#"<video src="opening.mp4"></video>"#,
            r#"<om-title delay="500ms">Opening</om-title>"#,
            r#"<om-cta delay="1.5s">Buy now</om-cta>"#,
            "</om-shot></om-scene></om-film>",
        ),
    );
    let (document, diagnostics) = parsed.into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::bind(document).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
    assert!(diagnostics.is_empty());
    let solved = compiler::solve(
        film.expect("the fixture resolves"),
        &assets,
        Timebase::new(rate),
    )
    .expect("the fixture metadata is complete");
    let (timeline, diagnostics) = solved.into_parts();
    assert!(diagnostics.is_empty());

    (timeline.expect("the fixture solves"), asset_id, rate)
}

fn render_json_lines(values: &[impl serde::Serialize]) -> String {
    let mut output = String::new();
    for value in values {
        let encoded = serde_json::to_string(value).expect("protocol fixtures must serialize");
        writeln!(output, "{encoded}").expect("writing protocol JSON into a String cannot fail");
    }
    output
}
