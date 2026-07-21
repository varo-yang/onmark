//! Frozen source-color evidence for the layered-media candidate.

use std::env;
use std::path::Path;

use onmark_core::model::FrameRate;
use tempfile::tempdir;
use tokio::process::Command;
use tokio::time::timeout;
use url::Url;

use super::capture_layered_observation;
use crate::layered::{LayeredSegment, PixelProbe};
use crate::{
    BENCHMARK_FRAME_COUNT, PROCESS_DEADLINE, StrategyFixture, experiment_dimensions, required_path,
};

const CHANNEL_ERROR_BOUND: u8 = 4;

#[tokio::test]
#[ignore = "requires pinned Linux Chromium, FFmpeg, ffprobe, and built runtime"]
async fn keeps_bt709_patches_within_bound() {
    assert_eq!(
        env::consts::OS,
        "linux",
        "layered conformance is Linux-only"
    );

    let fixture = StrategyFixture::build_color().await;
    let presentation = empty_transparent_fixture();
    let patches = color_patches(experiment_dimensions());
    let probes = patches.iter().map(|patch| patch.probe).collect::<Vec<_>>();
    let output = capture_layered_observation(
        &fixture,
        &presentation,
        "layered-color.mp4",
        &fixture.indices,
        LayeredSegment::new(0, BENCHMARK_FRAME_COUNT),
        &probes,
    )
    .await;

    for patch in patches {
        assert_color_patch(patch, output.sample(patch.probe));
    }
}

impl StrategyFixture {
    async fn build_color() -> Self {
        let directory = tempdir().expect("the color-fixture directory must be available");
        let frame_rate = FrameRate::new(30, 1).expect("the color-fixture rate is valid");
        let indices = (0..BENCHMARK_FRAME_COUNT).collect::<Vec<_>>();
        let media = directory.path().join("colors.mp4");
        generate_color_video(&media, frame_rate).await;
        let admitted = crate::admitted_source_video(&media, crate::FixtureTiming::Constant)
            .await
            .expect("the color fixture must satisfy layered-media admission");
        assert_eq!(admitted.frame_rate, frame_rate);

        Self {
            directory,
            frame_rate,
            source_frame_rate: frame_rate,
            color_profile: admitted.color_profile,
            indices: indices.clone(),
            media,
            expected: crate::expected_cfr_frames(frame_rate, &indices),
            plan: crate::browser_plan(frame_rate),
        }
    }
}

async fn generate_color_video(output: &Path, frame_rate: FrameRate) {
    let (width, height) = experiment_dimensions();
    let half_width = width / 2;
    let half_height = height / 2;
    let rate = format!("{}/{}", frame_rate.numerator(), frame_rate.denominator());
    let source = format!(
        concat!(
            "color=c=red:size={half_width}x{half_height}:rate={rate}:d=2.5[tl];",
            "color=c=lime:size={half_width}x{half_height}:rate={rate}:d=2.5[tr];",
            "color=c=blue:size={half_width}x{half_height}:rate={rate}:d=2.5[bl];",
            "color=c=white:size={half_width}x{half_height}:rate={rate}:d=2.5[br];",
            "[tl][tr]hstack[top];[bl][br]hstack[bottom];[top][bottom]vstack",
        ),
        half_width = half_width,
        half_height = half_height,
        rate = rate,
    );
    let generated = Command::new(required_path("ONMARK_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-f", "lavfi", "-i", &source])
        .args([
            "-an",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-x264-params",
            "colorprim=bt709:transfer=bt709:colormatrix=bt709:range=limited",
            "-colorspace",
            "bt709",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_range",
            "tv",
            "-y",
        ])
        .arg(output)
        .output();
    let generated = timeout(PROCESS_DEADLINE, generated)
        .await
        .expect("the color fixture must finish before its deadline")
        .expect("FFmpeg must generate the color fixture");
    crate::assert_process_succeeded("color-fixture generation", &generated);
}

#[derive(Clone, Copy)]
struct ColorPatch {
    name: &'static str,
    probe: PixelProbe,
    expected: [u8; 4],
}

fn color_patches((width, height): (u32, u32)) -> Vec<ColorPatch> {
    let locations = [
        ("red", width / 4, height / 4, [255, 0, 0, 255]),
        ("green", width * 3 / 4, height / 4, [0, 255, 0, 255]),
        ("blue", width / 4, height * 3 / 4, [0, 0, 255, 255]),
        ("white", width * 3 / 4, height * 3 / 4, [255; 4]),
    ];
    [0, BENCHMARK_FRAME_COUNT - 1]
        .into_iter()
        .flat_map(|frame| {
            locations.map(move |(name, x, y, expected)| ColorPatch {
                name,
                probe: PixelProbe::new(frame, x, y),
                expected,
            })
        })
        .collect()
}

fn assert_color_patch(patch: ColorPatch, actual: [u8; 4]) {
    for (channel, (&expected, &actual)) in patch.expected.iter().zip(&actual).enumerate() {
        assert_color_channel(patch.name, channel, expected, actual);
    }
}

fn assert_color_channel(name: &str, channel: usize, expected: u8, actual: u8) {
    assert!(
        expected.abs_diff(actual) <= CHANNEL_ERROR_BOUND,
        "{name} patch channel {channel} expected {expected}, observed {actual}, bound {CHANNEL_ERROR_BOUND}",
    );
}

pub(super) fn empty_transparent_fixture() -> Url {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/transparent-empty.html");
    Url::from_file_path(fixture).expect("the fixture path is absolute")
}
