//! Frame-selection, repeatability, and partition evidence.

use std::env;

use onmark_core::model::FrameRate;
use sha2::{Digest as _, Sha256};
use url::Url;

use super::color::empty_transparent_fixture;
use super::{capture_layered_segment, capture_layered_sequence};
use crate::decoder::decode_sequence;
use crate::layered::LayeredSegment;
use crate::{
    BENCHMARK_FRAME_COUNT, StrategyFixture, experiment_dimensions, transparent_overlay_fixture,
};

#[tokio::test]
#[ignore = "requires pinned Linux Chromium, FFmpeg, and ffprobe"]
async fn selects_exact_cfr_frames() {
    assert_linux();

    let source_rate = FrameRate::new(24, 1).expect("the source rate is valid");
    let output_rate = FrameRate::new(30, 1).expect("the output rate is valid");
    let fixture = StrategyFixture::build_resampled(source_rate, output_rate).await;
    let source_indices = (0..source_frame_count(source_rate, output_rate))
        .map(|index| u64::try_from(index).expect("the source fixture is small"))
        .collect::<Vec<_>>();
    let source_frames = decode_sequence(
        &fixture.media,
        source_rate,
        experiment_dimensions(),
        &source_indices,
    )
    .await;
    let source_hashes = source_frames
        .iter()
        .map(|pixels| <[u8; 32]>::from(Sha256::digest(pixels)))
        .collect::<Vec<_>>();
    let output = capture_layered_sequence(
        &fixture,
        &empty_transparent_fixture(),
        "layered-rate-conversion.mp4",
    )
    .await;

    for (output_index, actual) in output.fingerprints.iter().enumerate() {
        let source_index = selected_source_frame(output_index, source_rate, output_rate);
        assert_eq!(
            actual, &source_hashes[source_index],
            "output frame {output_index} must select source frame {source_index}",
        );
    }
}

#[tokio::test]
#[ignore = "requires pinned Linux Chromium, FFmpeg, and ffprobe"]
async fn repeats_and_matches_partitions() {
    assert_linux();

    let fixture = StrategyFixture::build().await;
    let presentation = transparent_overlay_fixture();
    let first = capture_layered_sequence(&fixture, &presentation, "layered-whole-first.mp4").await;
    let second =
        capture_layered_sequence(&fixture, &presentation, "layered-whole-second.mp4").await;

    assert_eq!(
        first.fingerprints, second.fingerprints,
        "independent layered runs must reproduce every canonical frame",
    );
    assert_partition_equivalence(&fixture, &presentation, &first.fingerprints).await;
}

impl StrategyFixture {
    async fn build_resampled(source_rate: FrameRate, output_rate: FrameRate) -> Self {
        let directory = crate::experiment_directory("rate-conversion");
        let indices = (0..BENCHMARK_FRAME_COUNT).collect::<Vec<_>>();
        let media = directory.path().join("rate-conversion.mp4");
        crate::generate_video(&media, source_rate, crate::FixtureTiming::Constant).await;
        let admitted = crate::admitted_source_video(&media, crate::FixtureTiming::Constant)
            .await
            .expect("the rate fixture must satisfy layered-media admission");
        assert_eq!(admitted.frame_rate, source_rate);

        Self {
            directory,
            frame_rate: output_rate,
            source_frame_rate: source_rate,
            color_profile: admitted.color_profile,
            indices: indices.clone(),
            media,
            expected: crate::expected_cfr_frames(output_rate, &indices),
            plan: crate::browser_plan_with_source_rate(output_rate, source_rate),
        }
    }
}

async fn assert_partition_equivalence(
    fixture: &StrategyFixture,
    presentation: &Url,
    whole: &[[u8; 32]],
) {
    let split = fixture.indices.len() / 2;
    let split_frame = u64::try_from(split).expect("the partition fixture is small");
    let (first_indices, second_indices) = fixture.indices.split_at(split);
    let first = capture_layered_segment(
        fixture,
        presentation,
        "layered-part-first.mp4",
        first_indices,
        LayeredSegment::new(0, split_frame),
    )
    .await;
    let second = capture_layered_segment(
        fixture,
        presentation,
        "layered-part-second.mp4",
        second_indices,
        LayeredSegment::new(
            split_frame,
            u64::try_from(second_indices.len()).expect("the partition fixture is small"),
        ),
    )
    .await;

    let partitioned = first
        .fingerprints
        .into_iter()
        .chain(second.fingerprints)
        .collect::<Vec<_>>();
    assert_eq!(
        partitioned, whole,
        "independent layered partitions must reproduce the whole-film sequence",
    );
}

fn source_frame_count(source_rate: FrameRate, output_rate: FrameRate) -> usize {
    selected_source_frame(
        usize::try_from(BENCHMARK_FRAME_COUNT - 1).expect("the benchmark is small"),
        source_rate,
        output_rate,
    )
    .checked_add(1)
    .expect("the source-frame count must fit this process")
}

fn selected_source_frame(
    output_index: usize,
    source_rate: FrameRate,
    output_rate: FrameRate,
) -> usize {
    let output_index =
        u128::try_from(output_index).expect("the output index must fit exact frame arithmetic");
    let sample_midpoint = output_index
        .checked_mul(2)
        .and_then(|index| index.checked_add(1))
        .expect("the output midpoint must fit exact frame arithmetic");
    let numerator = exact_product([
        sample_midpoint,
        u128::from(output_rate.denominator()),
        u128::from(source_rate.numerator()),
    ]);
    let denominator = exact_product([
        2,
        u128::from(output_rate.numerator()),
        u128::from(source_rate.denominator()),
    ]);
    usize::try_from(numerator / denominator)
        .expect("the selected source frame must fit this process")
}

fn exact_product(factors: [u128; 3]) -> u128 {
    factors
        .into_iter()
        .try_fold(1_u128, u128::checked_mul)
        .expect("the fixture rates must fit exact frame arithmetic")
}

fn assert_linux() {
    assert_eq!(
        env::consts::OS,
        "linux",
        "layered conformance is Linux-only"
    );
}
