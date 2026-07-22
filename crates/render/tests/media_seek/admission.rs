//! Correctness and performance admission for the layered-media candidate.

#[path = "admission/color.rs"]
mod color;
#[path = "admission/correctness.rs"]
mod correctness;
#[path = "admission/performance.rs"]
mod performance;

use url::Url;

use onmark_core::protocol::RequestId;

use super::layered::{LayeredCompositor, LayeredJob, LayeredOutput, LayeredSegment, PixelProbe};
use super::{
    BENCHMARK_FRAME_COUNT, StrategyFixture, confirm, dispose, experiment_dimensions, frame,
    launch_transparent_browser, load_and_prepare, stage,
};

pub(super) async fn capture_layered_sequence(
    fixture: &StrategyFixture,
    presentation: &Url,
    output_name: &str,
) -> LayeredOutput {
    capture_layered_segment(
        fixture,
        presentation,
        output_name,
        &fixture.indices,
        LayeredSegment::new(0, BENCHMARK_FRAME_COUNT),
    )
    .await
}

pub(super) async fn capture_layered_segment(
    fixture: &StrategyFixture,
    presentation: &Url,
    output_name: &str,
    indices: &[u64],
    segment: LayeredSegment,
) -> LayeredOutput {
    capture_layered_observation(fixture, presentation, output_name, indices, segment, &[]).await
}

pub(super) async fn capture_layered_observation(
    fixture: &StrategyFixture,
    presentation: &Url,
    output_name: &str,
    indices: &[u64],
    segment: LayeredSegment,
    probes: &[PixelProbe],
) -> LayeredOutput {
    let job = LayeredJob::new(
        &fixture.media,
        fixture.root().join(output_name),
        fixture.source_frame_rate,
        fixture.frame_rate,
        fixture.color_profile,
        experiment_dimensions(),
        segment,
    )
    .with_probes(probes);
    let mut compositor = LayeredCompositor::start(job);
    let mut session = launch_transparent_browser().await;
    load_and_prepare(&mut session, presentation, &fixture.plan)
        .await
        .expect("the transparent presentation must prepare");

    for (offset, index) in indices.iter().copied().enumerate() {
        let request_offset = request_offset(offset);
        stage(&session, RequestId::new(3 + request_offset), index)
            .await
            .expect("the transparent presentation frame must stage");
        let captured = session
            .capture_png(frame(index), fixture.plan.frame_rate())
            .await
            .expect("the transparent presentation frame must capture");
        compositor.write_overlay(&captured).await;
        confirm(&session, RequestId::new(4 + request_offset), index)
            .await
            .expect("the transparent presentation frame must confirm");
    }

    dispose(&session, indices.len())
        .await
        .expect("the transparent presentation must dispose");
    session
        .shutdown()
        .await
        .expect("headless shell must shut down after layered capture");
    compositor.finish().await
}

pub(super) fn request_offset(offset: usize) -> u32 {
    u32::try_from(offset * 2).expect("the admission fixture is small")
}
