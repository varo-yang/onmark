mod conformance;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use onmark_core::compiler;
use onmark_core::model::{AssetMetadata, AssetRef, Duration, FrameRate, SourceId, Timebase};
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, BrowserResponse, ProtocolFailure,
    ProtocolFailureCode, RequestId, WireFrame,
};

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
        request(4, BrowserCommand::Dispose),
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
        vec![Box::from("font:Inter")],
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
        response(3, BrowserEvent::FrameReady { frame: frame(15) }),
        response(3, BrowserEvent::Failed(timeout)),
        response(4, BrowserEvent::Disposed),
    ];

    assert_or_update(
        &fixture("protocol", "browser-responses-v1.jsonl"),
        &render_json_lines(&responses),
    );
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
    let asset = AssetRef::parse("opening.mp4").expect("the fixture asset is valid");
    let assets = BTreeMap::from([(
        asset,
        AssetMetadata::new(Duration::from_nanos(2_500_000_000)),
    )]);
    let parsed = compiler::parse(
        SourceId::new(0),
        r#"<film><scene><shot><video src="opening.mp4" /></shot></scene></film>"#,
    );
    let (document, diagnostics) = parsed.into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::bind(document).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
    assert!(diagnostics.is_empty());
    let rate = FrameRate::new(30, 1).expect("the fixture frame rate is valid");
    let solved = compiler::solve(
        film.expect("the fixture resolves"),
        &assets,
        Timebase::new(rate),
    )
    .expect("the fixture metadata is complete");
    assert!(solved.diagnostics().is_empty());

    BrowserPlan::try_from(solved.timeline().expect("the fixture solves"))
        .expect("the fixture timeline fits the browser frame domain")
}

fn render_json_lines(values: &[impl serde::Serialize]) -> String {
    let mut output = String::new();
    for value in values {
        let encoded = serde_json::to_string(value).expect("protocol fixtures must serialize");
        writeln!(output, "{encoded}").expect("writing protocol JSON into a String cannot fail");
    }
    output
}
