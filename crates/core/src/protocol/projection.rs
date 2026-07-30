//! Timeline-to-browser projection under one already selected evaluation interval.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{ElementKind, FrameInterval, FrozenAssetId, VideoTiming};
use crate::timeline::{
    TimelineCaption, TimelineContent, TimelineElement, TimelineIr, TimelineOverlay, TimelineScene,
    TimelineShot, TimelineShotIndex, TimelineText, TimelineTransition, TimelineVideo,
};

use super::frame::WireInterval;
use super::plan::{
    BrowserCaptionTrack, BrowserNode, BrowserNodeId, BrowserOverlay, BrowserOverlayKind,
    BrowserScene, BrowserShot, BrowserTransition, BrowserVideo, BrowserVideoTiming,
    InvalidBrowserPlan, MAX_BROWSER_OVERLAYS, MAX_BROWSER_SCENES, MAX_BROWSER_SHOTS,
    MAX_BROWSER_TEXT_BYTES, MAX_BROWSER_TRANSITIONS, MAX_BROWSER_VIDEOS, text_exceeds_limit,
};

pub(super) struct BrowserProjection {
    pub(super) film: BrowserNode,
    pub(super) scenes: Vec<BrowserScene>,
    pub(super) shots: Vec<BrowserShot>,
    pub(super) transitions: Vec<BrowserTransition>,
    pub(super) videos: Vec<BrowserVideo>,
    pub(super) overlays: Vec<BrowserOverlay>,
}

pub(super) struct ProjectionBuilder<'a> {
    evaluation: FrameInterval,
    source_timings: &'a BTreeMap<FrozenAssetId, VideoTiming>,
    selected_shots: Option<&'a BTreeSet<TimelineShotIndex>>,
    next_node_id: u32,
    next_shot_index: usize,
    scenes: Vec<BrowserScene>,
    shots: Vec<BrowserShot>,
    transitions: Vec<BrowserTransition>,
    videos: Vec<BrowserVideo>,
    overlays: Vec<BrowserOverlay>,
    overlay_text_bytes: usize,
}

struct PendingTransition {
    node: BrowserNode,
    outgoing_shot_id: BrowserNodeId,
    interval: WireInterval,
}

impl PendingTransition {
    fn finish(self, incoming_shot_id: BrowserNodeId) -> BrowserTransition {
        BrowserTransition::new(
            self.node,
            self.outgoing_shot_id,
            incoming_shot_id,
            self.interval,
        )
    }
}

impl<'a> ProjectionBuilder<'a> {
    pub(super) fn new(
        evaluation: FrameInterval,
        source_timings: &'a BTreeMap<FrozenAssetId, VideoTiming>,
        selected_shots: Option<&'a BTreeSet<TimelineShotIndex>>,
    ) -> Self {
        Self {
            evaluation,
            source_timings,
            selected_shots,
            next_node_id: 0,
            next_shot_index: 0,
            scenes: Vec::new(),
            shots: Vec::new(),
            transitions: Vec::new(),
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
        self.validate_shot_selection()?;
        for caption in timeline.captions() {
            self.project_caption(caption)?;
        }

        Ok(BrowserProjection {
            film,
            scenes: self.scenes,
            shots: self.shots,
            transitions: self.transitions,
            videos: self.videos,
            overlays: self.overlays,
        })
    }

    fn validate_shot_selection(&self) -> Result<(), InvalidBrowserPlan> {
        let Some(selected) = self.selected_shots else {
            return Ok(());
        };
        if selected.is_empty() || selected.len() != self.shots.len() {
            return Err(InvalidBrowserPlan::InvalidShotSelection);
        }
        Ok(())
    }

    fn project_scene(&mut self, scene: &TimelineScene) -> Result<(), InvalidBrowserPlan> {
        let interval = scene.timing().interval();
        let first_shot = self.next_shot_index;
        self.next_shot_index = self
            .next_shot_index
            .checked_add(scene.shots().len())
            .ok_or(InvalidBrowserPlan::TooManyShots)?;
        if !self.scene_selected(scene, first_shot, self.next_shot_index) {
            return Ok(());
        }
        let node = self.node(scene.element())?;
        let scene_id = node.id();
        if self.scenes.len() >= MAX_BROWSER_SCENES {
            return Err(InvalidBrowserPlan::TooManyScenes);
        }
        self.scenes
            .push(BrowserScene::new(node, WireInterval::try_from(interval)?));
        let mut previous: Option<(TimelineShotIndex, BrowserNodeId)> = None;
        for (offset, shot) in scene.shots().iter().enumerate() {
            let index = TimelineShotIndex::new(
                first_shot
                    .checked_add(offset)
                    .ok_or(InvalidBrowserPlan::TooManyShots)?,
            );
            if !self.shot_selected(index, shot) {
                previous = None;
                continue;
            }
            let transition = match (previous, shot.incoming_transition()) {
                (Some((previous_index, previous_id)), Some(transition))
                    if previous_index.get().checked_add(1) == Some(index.get()) =>
                {
                    Some(self.prepare_transition(transition, previous_id)?)
                }
                _ => None,
            };
            let shot_id = self.project_shot(shot, scene_id)?;
            if let Some(transition) = transition {
                self.transitions.push(transition.finish(shot_id));
            }
            previous = Some((index, shot_id));
        }
        Ok(())
    }

    fn scene_selected(&self, scene: &TimelineScene, first: usize, end: usize) -> bool {
        match self.selected_shots {
            Some(selected) => selected
                .range(TimelineShotIndex::new(first)..TimelineShotIndex::new(end))
                .next()
                .is_some(),
            None => scene.timing().interval().intersects(self.evaluation),
        }
    }

    fn shot_selected(&self, index: TimelineShotIndex, shot: &TimelineShot) -> bool {
        self.selected_shots.map_or_else(
            || shot.timing().interval().intersects(self.evaluation),
            |selected| selected.contains(&index),
        )
    }

    fn project_shot(
        &mut self,
        shot: &TimelineShot,
        scene_id: BrowserNodeId,
    ) -> Result<BrowserNodeId, InvalidBrowserPlan> {
        let interval = shot.timing().interval();
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
        Ok(shot_id)
    }

    fn prepare_transition(
        &mut self,
        transition: &TimelineTransition,
        outgoing_shot_id: BrowserNodeId,
    ) -> Result<PendingTransition, InvalidBrowserPlan> {
        if self.transitions.len() >= MAX_BROWSER_TRANSITIONS {
            return Err(InvalidBrowserPlan::TooManyTransitions);
        }
        let node = self.node(transition.element())?;
        Ok(PendingTransition {
            node,
            outgoing_shot_id,
            interval: WireInterval::try_from(transition.timing().interval())?,
        })
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
        .checked_add(overlay.text_bytes())
        .ok_or(InvalidBrowserPlan::BrowserTextBudget)?;
    if *overlay_text_bytes > MAX_BROWSER_TEXT_BYTES {
        return Err(InvalidBrowserPlan::BrowserTextBudget);
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
        None,
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
        Some(BrowserCaptionTrack::new(
            caption.track_id(),
            caption.language(),
        )),
        caption.text().into(),
        WireInterval::try_from(interval)?,
    ))
}
