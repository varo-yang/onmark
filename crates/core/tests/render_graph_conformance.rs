//! Dependency-driven partition facts for the Gate-two graph boundary.

mod conformance;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

use onmark_core::compiler;
use onmark_core::model::{
    AssetMetadata, AssetRef, Duration, FrameInterval, FrameRate, FrozenAsset, FrozenAssetId,
    SourceId, Timebase, VideoMetadata, VideoTiming,
};
use onmark_core::render_graph::{PartitionPlan, RenderGraph};
use onmark_core::timeline::TimelineIr;

use conformance::{assert_or_update, fixture};

#[test]
fn independent_shots_form_separate_scoped_units() {
    let source_path = fixture("render-graph", "valid/two-independent-shots.onmark");
    let expected_path = fixture("render-graph", "valid/two-independent-shots.plan.txt");
    let assets = frozen_assets([
        ("opening.mp4", "1s"),
        ("closing.mp4", "2s"),
        ("voice.mp3", "1s"),
    ]);
    let timeline = solve_fixture(&source_path, &assets);
    let graph = RenderGraph::from_timeline(&timeline);
    let mut renderer = PlanRenderer::new();
    renderer
        .render_graph(&graph)
        .expect("rendering into a String cannot fail");
    let plan = graph.into_partition();
    renderer
        .render_plan(&plan)
        .expect("rendering into a String cannot fail");

    assert_eq!(plan.units().len(), 2);
    assert_or_update(&expected_path, &renderer.finish());
}

fn solve_fixture(
    source_path: &std::path::Path,
    assets: &BTreeMap<AssetRef, FrozenAsset>,
) -> TimelineIr {
    let source =
        fs::read_to_string(source_path).expect("the render-graph fixture must be readable");
    let (document, diagnostics) = compiler::parse(SourceId::new(0), &source).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::bind(document).into_parts();
    assert!(diagnostics.is_empty());
    let (film, diagnostics) = compiler::resolve(film.expect("the fixture binds")).into_parts();
    assert!(diagnostics.is_empty());
    let report = compiler::solve(
        film.expect("the fixture resolves"),
        assets,
        Timebase::new(FrameRate::new(30, 1).expect("the fixture frame rate is valid")),
    )
    .expect("the fixture catalog is complete");
    let (timeline, diagnostics) = report.into_parts();
    assert!(diagnostics.is_empty());
    timeline.expect("the fixture solves")
}

fn frozen_assets<const N: usize>(entries: [(&str, &str); N]) -> BTreeMap<AssetRef, FrozenAsset> {
    entries
        .into_iter()
        .enumerate()
        .map(|(index, (name, duration))| {
            let reference = AssetRef::parse(name).expect("the fixture asset reference is valid");
            let duration = Duration::parse(duration).expect("the fixture duration is valid");
            let id = FrozenAssetId::from_sha256(
                [u8::try_from(index + 1).expect("the fixture catalog is small"); 32],
            );
            let metadata = if name.ends_with(".mp3") {
                AssetMetadata::audio(duration)
            } else {
                let video = VideoMetadata::new(
                    duration,
                    "h264",
                    "yuv420p",
                    VideoTiming::Constant(
                        FrameRate::new(30, 1).expect("the fixture frame rate is valid"),
                    ),
                )
                .expect("the fixture video metadata is valid");
                AssetMetadata::video(duration, video)
            };
            (reference, FrozenAsset::new(id, metadata))
        })
        .collect()
}

struct PlanRenderer {
    output: String,
}

impl PlanRenderer {
    fn new() -> Self {
        Self {
            output: String::from("# onmark render-graph test rendering; not a wire format\n"),
        }
    }

    fn render_graph(&mut self, graph: &RenderGraph) -> std::fmt::Result {
        writeln!(
            self.output,
            "graph interval={} regions={}",
            frames(graph.interval()),
            graph.regions().len(),
        )?;

        for region in graph.regions() {
            writeln!(
                self.output,
                "  region evaluation={} output={} assets={}",
                frames(region.evaluation()),
                frames(region.output()),
                assets(region.media_assets()),
            )?;
        }

        Ok(())
    }

    fn render_plan(&mut self, plan: &PartitionPlan) -> std::fmt::Result {
        writeln!(
            self.output,
            "plan interval={} units={}",
            frames(plan.interval()),
            plan.units().len(),
        )?;

        for unit in plan.units() {
            writeln!(
                self.output,
                "  unit evaluation={} output={} assets={}",
                frames(unit.evaluation()),
                frames(unit.output()),
                assets(unit.media_assets()),
            )?;
        }

        Ok(())
    }

    fn finish(self) -> String {
        self.output
    }
}

fn frames(interval: FrameInterval) -> String {
    format!("{}..{}", interval.start().get(), interval.end().get())
}

fn assets<'a>(assets: impl Iterator<Item = &'a FrozenAssetId>) -> String {
    assets
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}
