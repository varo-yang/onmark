use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use onmark_core::compiler;
use onmark_core::model::{AssetMetadata, AssetRef, Duration as MediaDuration};
use onmark_core::model::{FrameRate, SourceId, Timebase};
use onmark_core::protocol::{
    BrowserCommand, BrowserEvent, BrowserPlan, BrowserRequest, RequestId, WireFrame,
};
use onmark_render::{BrowserLimits, BrowserSession, EncodedPng};
use url::Url;

#[tokio::test]
#[ignore = "requires ONMARK_CHROME and a built @onmark/runtime package"]
async fn captures_stable_frames_across_the_real_browser_protocol() {
    let chrome = env::var_os("ONMARK_CHROME")
        .map(PathBuf::from)
        .expect("ONMARK_CHROME must name the Chrome executable");
    let fixture = browser_fixture();
    let limits = BrowserLimits::new(320, 180, Duration::from_secs(10), 8 * 1024 * 1024)
        .expect("the fixture limits are bounded");
    let session = BrowserSession::launch(chrome, limits)
        .await
        .expect("Chrome must launch");

    let result = exercise_protocol(&session, &fixture).await;
    let shutdown = session.shutdown().await;

    result.expect("the real browser protocol must capture deterministic frames");
    shutdown.expect("Chrome must shut down cleanly");
}

async fn exercise_protocol(session: &BrowserSession, fixture: &Url) -> Result<(), Box<dyn Error>> {
    session.navigate(fixture.as_str()).await?;
    let plan = gate_one_plan();

    let loaded = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(1),
            BrowserCommand::Load { plan },
        ))
        .await?;
    assert_eq!(loaded.event(), &BrowserEvent::Loaded);

    let prepared = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(2),
            BrowserCommand::Prepare {
                evaluation_start: frame(0),
            },
        ))
        .await?;
    assert_eq!(
        prepared.event(),
        &BrowserEvent::Prepared {
            evaluation_start: frame(0),
        },
    );
    let first = session.capture_png().await?;

    seek(session, 3, 15).await?;
    let selected = session.capture_png().await?;
    seek(session, 4, 15).await?;
    let repeated = session.capture_png().await?;

    assert_png(&first);
    assert_ne!(first, selected);
    assert_eq!(selected, repeated);
    Ok(())
}

async fn seek(session: &BrowserSession, request_id: u32, index: u64) -> Result<(), Box<dyn Error>> {
    let response = session
        .dispatch(&BrowserRequest::new(
            RequestId::new(request_id),
            BrowserCommand::Seek {
                frame: frame(index),
            },
        ))
        .await?;
    assert_eq!(
        response.event(),
        &BrowserEvent::FrameReady {
            frame: frame(index),
        },
    );
    Ok(())
}

fn frame(index: u64) -> WireFrame {
    WireFrame::new(index).expect("fixture frames are browser-safe")
}

fn assert_png(frame: &EncodedPng) {
    assert!(frame.as_bytes().starts_with(b"\x89PNG\r\n\x1a\n"));
}

fn browser_fixture() -> Url {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("render is nested at crates/render");
    let fixture = repository.join("conformance/browser/gate-one.html");
    let runtime = repository.join("packages/runtime/dist/src/index.js");
    assert!(runtime.is_file(), "run `pnpm --dir packages/runtime build`");
    Url::from_file_path(fixture).expect("the fixture path is absolute")
}

fn gate_one_plan() -> BrowserPlan {
    let asset = AssetRef::parse("opening.mp4").expect("the fixture asset is valid");
    let assets = BTreeMap::from([(
        asset,
        AssetMetadata::new(MediaDuration::from_nanos(2_500_000_000)),
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
