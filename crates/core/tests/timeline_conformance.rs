mod conformance;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;

use onmark_core::compiler;
use onmark_core::model::{
    AssetMetadata, AssetRef, Duration, EventRef, FrameInterval, FrameRate, SourceId, Timebase,
};
use onmark_core::timeline::{
    TimelineContent, TimelineElement, TimelineIr, TimelineTiming, TimingReason,
};

use conformance::{assert_or_update, fixture, render_diagnostics};

#[test]
fn explicit_duration_and_overlays_match_canonical_timeline() {
    assert_valid_fixture("explicit-duration", BTreeMap::new());
}

#[test]
fn media_duration_and_longest_content_match_canonical_timeline() {
    let assets = asset_metadata([("clip.mp4", "2s"), ("voice.mp3", "1s")]);

    assert_valid_fixture("media-duration", assets);
}

#[test]
fn timing_errors_match_stable_diagnostics() {
    let source_path = fixture("timeline", "invalid/timing-errors.onmark");
    let expected_path = fixture("timeline", "invalid/timing-errors.diagnostics.txt");
    let assets = asset_metadata([("clip.mp4", "2s")]);
    let report = solve_fixture(&source_path, &assets).expect("all referenced assets were probed");

    assert!(report.timeline().is_none());
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

#[test]
fn missing_asset_metadata_is_a_typed_failure() {
    let source_path = fixture("timeline", "valid/media-duration.onmark");
    let error = solve_fixture(&source_path, &BTreeMap::new())
        .expect_err("the fixture references assets absent from the catalog");
    let asset = AssetRef::parse("clip.mp4").expect("the fixture asset reference is valid");

    assert_eq!(error, compiler::SolveError::MissingAssetMetadata(asset));
}

#[test]
fn frame_domain_overflow_matches_stable_diagnostics() {
    let source_path = fixture("timeline", "invalid/frame-overflow.onmark");
    let expected_path = fixture("timeline", "invalid/frame-overflow.diagnostics.txt");
    let rate = FrameRate::new(u32::MAX, 1).expect("the maximum frame rate is valid");
    let report = solve_fixture_at(&source_path, &BTreeMap::new(), rate)
        .expect("the fixture references no external assets");

    assert!(report.timeline().is_none());
    assert_or_update(&expected_path, &render_diagnostics(report.diagnostics()));
}

fn assert_valid_fixture(name: &str, assets: BTreeMap<AssetRef, AssetMetadata>) {
    let source_path = fixture("timeline", &format!("valid/{name}.onmark"));
    let expected_path = fixture("timeline", &format!("valid/{name}.timeline.txt"));
    let solved = solve_fixture(&source_path, &assets).expect("all fixture assets were probed");

    assert!(solved.diagnostics().is_empty());
    let timeline = solved.timeline().expect("the valid fixture must solve");
    assert_or_update(&expected_path, &TimelineRenderer::render(timeline));
}

fn solve_fixture(
    source_path: &std::path::Path,
    assets: &BTreeMap<AssetRef, AssetMetadata>,
) -> Result<compiler::SolveReport, compiler::SolveError> {
    let rate = FrameRate::new(30, 1).expect("the fixture frame rate is valid");
    solve_fixture_at(source_path, assets, rate)
}

fn solve_fixture_at(
    source_path: &std::path::Path,
    assets: &BTreeMap<AssetRef, AssetMetadata>,
    rate: FrameRate,
) -> Result<compiler::SolveReport, compiler::SolveError> {
    let source = fs::read_to_string(source_path).expect("the timeline fixture must be readable");
    let parsed = compiler::parse(SourceId::new(0), &source);
    let (document, syntax_diagnostics) = parsed.into_parts();
    assert!(syntax_diagnostics.is_empty());
    let bound = compiler::bind(document);
    let (film, binding_diagnostics) = bound.into_parts();
    assert!(binding_diagnostics.is_empty());
    let resolved = compiler::resolve(film.expect("the fixture contains one film"));
    let (film, resolution_diagnostics) = resolved.into_parts();
    assert!(resolution_diagnostics.is_empty());
    compiler::solve(
        film.expect("the fixture resolves"),
        assets,
        Timebase::new(rate),
    )
}

fn asset_metadata<const N: usize>(entries: [(&str, &str); N]) -> BTreeMap<AssetRef, AssetMetadata> {
    entries
        .into_iter()
        .map(|(asset, duration)| {
            let asset = AssetRef::parse(asset).expect("the fixture asset reference is valid");
            let duration = Duration::parse(duration).expect("the fixture duration is valid");
            (asset, AssetMetadata::new(duration))
        })
        .collect()
}

struct TimelineRenderer {
    output: String,
}

impl TimelineRenderer {
    fn render(timeline: &TimelineIr) -> String {
        let mut renderer = Self {
            output: String::from("# onmark timeline test rendering; not a wire format\n"),
        };
        renderer
            .render_timeline(timeline)
            .expect("rendering into a String cannot fail");
        renderer.output
    }

    fn render_timeline(&mut self, timeline: &TimelineIr) -> std::fmt::Result {
        let rate = timeline.timebase().frame_rate();
        writeln!(
            self.output,
            "timeline v={} fps={}/{} film={} interval={}",
            timeline.version().get(),
            rate.numerator(),
            rate.denominator(),
            element(timeline.element()),
            frames(timeline.interval()),
        )?;

        for (id, event) in timeline.events() {
            writeln!(self.output, "  event {id}={}f", event.at().get())?;
        }

        for scene in timeline.scenes() {
            writeln!(
                self.output,
                "  scene {} {}",
                element(scene.element()),
                timing(scene.timing()),
            )?;

            for shot in scene.shots() {
                writeln!(
                    self.output,
                    "    shot {} {}",
                    element(shot.element()),
                    timing(shot.timing()),
                )?;

                for content in shot.content() {
                    self.render_content(content)?;
                }
            }
        }

        Ok(())
    }

    fn render_content(&mut self, content: &TimelineContent) -> std::fmt::Result {
        match content {
            TimelineContent::Video(video) => writeln!(
                self.output,
                "      video {} {} asset={}",
                element(video.element()),
                timing(video.timing()),
                video.asset(),
            ),
            TimelineContent::VoiceOver(voice_over) => writeln!(
                self.output,
                "      vo {} {} asset={} text={:?}",
                element(voice_over.element()),
                timing(voice_over.timing()),
                voice_over.asset(),
                text(voice_over.text()),
            ),
            TimelineContent::Overlay(overlay) => writeln!(
                self.output,
                "      overlay {} {} text={:?}",
                element(overlay.element()),
                timing(overlay.timing()),
                text(overlay.text()),
            ),
        }
    }
}

fn element(element: &TimelineElement) -> String {
    let id = element.id().map_or("-", onmark_core::model::NodeId::as_str);
    format!("<{}> id={id}", element.kind())
}

fn timing(timing: &TimelineTiming) -> String {
    format!(
        "interval={} start={} end={}",
        frames(timing.interval()),
        reason(timing.start_reason()),
        reason(timing.end_reason()),
    )
}

fn frames(interval: FrameInterval) -> String {
    format!("{}..{}", interval.start().get(), interval.end().get())
}

fn reason(reason: &TimingReason) -> String {
    match reason {
        TimingReason::FilmStart => "film-start".to_owned(),
        TimingReason::Sequential => "sequential".to_owned(),
        TimingReason::ShotStart => "shot-start".to_owned(),
        TimingReason::AuthoredDelay(_) => "authored-delay".to_owned(),
        TimingReason::Event { event, .. } => match event {
            EventRef::Cue(id) => format!("event:{id}"),
        },
        TimingReason::ExplicitDuration(_) => "explicit-duration".to_owned(),
        TimingReason::AssetDuration => "asset-duration".to_owned(),
        TimingReason::LongestContent(_) => "longest-content".to_owned(),
        TimingReason::Children => "children".to_owned(),
        TimingReason::ShotEnd => "shot-end".to_owned(),
        _ => panic!("the fixture renderer must cover every emitted timing reason"),
    }
}

fn text(runs: &[onmark_core::timeline::TimelineText]) -> String {
    runs.iter().map(|run| run.text()).collect()
}
