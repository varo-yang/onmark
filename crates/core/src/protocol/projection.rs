//! Timeline-to-browser projection under one already selected evaluation interval.

use std::collections::BTreeMap;

use crate::model::{ElementKind, FrameInterval, FrozenAssetId, VideoTiming};
use crate::timeline::{
    TimelineCaption, TimelineContent, TimelineElement, TimelineIr, TimelineOverlay, TimelineScene,
    TimelineShot, TimelineText, TimelineVideo,
};

use super::frame::WireInterval;
use super::plan::{
    BrowserNode, BrowserNodeId, BrowserOverlay, BrowserOverlayKind, BrowserScene, BrowserShot,
    BrowserVideo, BrowserVideoTiming, InvalidBrowserPlan, MAX_BROWSER_OVERLAY_TEXT_BYTES,
    MAX_BROWSER_OVERLAYS, MAX_BROWSER_SCENES, MAX_BROWSER_SHOTS, MAX_BROWSER_VIDEOS,
    text_exceeds_limit,
};

pub(super) struct BrowserProjection {
    pub(super) film: BrowserNode,
    pub(super) scenes: Vec<BrowserScene>,
    pub(super) shots: Vec<BrowserShot>,
    pub(super) videos: Vec<BrowserVideo>,
    pub(super) overlays: Vec<BrowserOverlay>,
}

pub(super) struct ProjectionBuilder<'a> {
    evaluation: FrameInterval,
    source_timings: &'a BTreeMap<FrozenAssetId, VideoTiming>,
    next_node_id: u32,
    scenes: Vec<BrowserScene>,
    shots: Vec<BrowserShot>,
    videos: Vec<BrowserVideo>,
    overlays: Vec<BrowserOverlay>,
    overlay_text_bytes: usize,
}

impl<'a> ProjectionBuilder<'a> {
    pub(super) fn new(
        evaluation: FrameInterval,
        source_timings: &'a BTreeMap<FrozenAssetId, VideoTiming>,
    ) -> Self {
        Self {
            evaluation,
            source_timings,
            next_node_id: 0,
            scenes: Vec::new(),
            shots: Vec::new(),
            videos: Vec::new(),
            overlays: Vec::new(),
            overlay_text_bytes: 0,
        }
    }

    pub(super) fn project(
        mut self,
        timeline: &TimelineIr,
    ) -> Result<BrowserProjection, InvalidBrowserPlan> {
        let film = self.node(timeline.element())?;
        for scene in timeline.scenes() {
            self.project_scene(scene)?;
        }
        for caption in timeline.captions() {
            self.project_caption(caption)?;
        }

        Ok(BrowserProjection {
            film,
            scenes: self.scenes,
            shots: self.shots,
            videos: self.videos,
            overlays: self.overlays,
        })
    }

    fn project_scene(&mut self, scene: &TimelineScene) -> Result<(), InvalidBrowserPlan> {
        let interval = scene.timing().interval();
        if !interval.intersects(self.evaluation) {
            return Ok(());
        }
        let node = self.node(scene.element())?;
        let scene_id = node.id();
        if self.scenes.len() >= MAX_BROWSER_SCENES {
            return Err(InvalidBrowserPlan::TooManyScenes);
        }
        self.scenes
            .push(BrowserScene::new(node, WireInterval::try_from(interval)?));
        for shot in scene.shots() {
            self.project_shot(shot, scene_id)?;
        }
        Ok(())
    }

    fn project_shot(
        &mut self,
        shot: &TimelineShot,
        scene_id: BrowserNodeId,
    ) -> Result<(), InvalidBrowserPlan> {
        let interval = shot.timing().interval();
        if !interval.intersects(self.evaluation) {
            return Ok(());
        }
        let node = self.node(shot.element())?;
        let shot_id = node.id();
        if self.shots.len() >= MAX_BROWSER_SHOTS {
            return Err(InvalidBrowserPlan::TooManyShots);
        }
        self.shots.push(BrowserShot::new(
            node,
            scene_id,
            WireInterval::try_from(interval)?,
        ));
        for content in shot.content() {
            self.project_content(content, shot_id)?;
        }
        Ok(())
    }

    fn project_content(
        &mut self,
        content: &TimelineContent,
        shot_id: BrowserNodeId,
    ) -> Result<(), InvalidBrowserPlan> {
        match content {
            TimelineContent::Video(video) => self.project_video(video, shot_id),
            TimelineContent::VoiceOver(_) => Ok(()),
            TimelineContent::Overlay(overlay) => self.project_overlay(overlay, shot_id),
        }
    }

    fn project_video(
        &mut self,
        video: &TimelineVideo,
        shot_id: BrowserNodeId,
    ) -> Result<(), InvalidBrowserPlan> {
        let interval = video.timing().interval();
        if !interval.intersects(self.evaluation) {
            return Ok(());
        }
        if !self.evaluation.contains_interval(interval) {
            return Err(InvalidBrowserPlan::VideoCrossesEvaluation);
        }
        if self.videos.len() >= MAX_BROWSER_VIDEOS {
            return Err(InvalidBrowserPlan::TooManyVideos);
        }
        let node = self.node(video.element())?;
        self.videos
            .push(browser_video(video, node, shot_id, self.source_timings)?);
        Ok(())
    }

    fn project_overlay(
        &mut self,
        overlay: &TimelineOverlay,
        shot_id: BrowserNodeId,
    ) -> Result<(), InvalidBrowserPlan> {
        let interval = overlay.timing().interval();
        if !interval.intersects(self.evaluation) {
            return Ok(());
        }
        let node = self.node(overlay.element())?;
        let overlay = browser_overlay(overlay, node, shot_id)?;
        push_browser_overlay(&mut self.overlays, &mut self.overlay_text_bytes, overlay)
    }

    fn project_caption(&mut self, caption: &TimelineCaption) -> Result<(), InvalidBrowserPlan> {
        let interval = caption.interval();
        if !interval.intersects(self.evaluation) {
            return Ok(());
        }
        let node = self.synthetic_node()?;
        let caption = browser_caption(caption, node, interval)?;
        push_browser_overlay(&mut self.overlays, &mut self.overlay_text_bytes, caption)
    }

    fn node(&mut self, element: &TimelineElement) -> Result<BrowserNode, InvalidBrowserPlan> {
        let id = self.take_node_id()?;
        Ok(BrowserNode::new(id, element.id()))
    }

    fn synthetic_node(&mut self) -> Result<BrowserNode, InvalidBrowserPlan> {
        let id = self.take_node_id()?;
        Ok(BrowserNode::new(id, None))
    }

    fn take_node_id(&mut self) -> Result<BrowserNodeId, InvalidBrowserPlan> {
        let id = BrowserNodeId::new(self.next_node_id);
        self.next_node_id = self
            .next_node_id
            .checked_add(1)
            .ok_or(InvalidBrowserPlan::TooManyNodes)?;
        Ok(id)
    }
}

fn push_browser_overlay(
    overlays: &mut Vec<BrowserOverlay>,
    overlay_text_bytes: &mut usize,
    overlay: BrowserOverlay,
) -> Result<(), InvalidBrowserPlan> {
    if overlays.len() >= MAX_BROWSER_OVERLAYS {
        return Err(InvalidBrowserPlan::TooManyOverlays);
    }
    *overlay_text_bytes = overlay_text_bytes
        .checked_add(overlay.text().len())
        .ok_or(InvalidBrowserPlan::OverlayTextBudget)?;
    if *overlay_text_bytes > MAX_BROWSER_OVERLAY_TEXT_BYTES {
        return Err(InvalidBrowserPlan::OverlayTextBudget);
    }
    overlays.push(overlay);
    Ok(())
}

fn browser_video(
    video: &TimelineVideo,
    node: BrowserNode,
    shot_id: BrowserNodeId,
    source_timings: &BTreeMap<FrozenAssetId, VideoTiming>,
) -> Result<BrowserVideo, InvalidBrowserPlan> {
    let asset_id = video.asset_id();
    let timing = source_timings
        .get(&asset_id)
        .ok_or(InvalidBrowserPlan::MissingSourceTiming(asset_id))?;
    Ok(BrowserVideo::new(
        node,
        shot_id,
        asset_id,
        WireInterval::try_from(video.timing().interval())?,
        BrowserVideoTiming::from_model(timing)?,
        video.source(),
    ))
}

fn browser_overlay(
    overlay: &TimelineOverlay,
    node: BrowserNode,
    shot_id: BrowserNodeId,
) -> Result<BrowserOverlay, InvalidBrowserPlan> {
    let element_kind = overlay.element().kind();
    let kind = match element_kind {
        ElementKind::Title => BrowserOverlayKind::Title,
        ElementKind::CallToAction => BrowserOverlayKind::CallToAction,
        _ => return Err(InvalidBrowserPlan::InvalidOverlayKind(element_kind)),
    };
    let text = overlay
        .text()
        .iter()
        .map(TimelineText::text)
        .collect::<String>();
    if text_exceeds_limit(&text) {
        return Err(InvalidBrowserPlan::OverlayTextTooLong(element_kind));
    }
    Ok(BrowserOverlay::new(
        node,
        Some(shot_id),
        kind,
        text.into_boxed_str(),
        WireInterval::try_from(overlay.timing().interval())?,
    ))
}

fn browser_caption(
    caption: &TimelineCaption,
    node: BrowserNode,
    interval: FrameInterval,
) -> Result<BrowserOverlay, InvalidBrowserPlan> {
    if text_exceeds_limit(caption.text()) {
        return Err(InvalidBrowserPlan::CaptionTextTooLong);
    }
    Ok(BrowserOverlay::new(
        node,
        None,
        BrowserOverlayKind::Caption,
        caption.text().into(),
        WireInterval::try_from(interval)?,
    ))
}
