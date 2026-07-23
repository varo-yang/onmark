//! Timeline-to-browser projection under one already selected evaluation interval.

use std::collections::BTreeMap;

use crate::model::{ElementKind, FrameInterval, FrameRate, FrozenAssetId};
use crate::timeline::{
    TimelineCaption, TimelineContent, TimelineElement, TimelineIr, TimelineOverlay, TimelineScene,
    TimelineShot, TimelineText, TimelineVideo,
};

use super::{
    BrowserNode, BrowserNodeId, BrowserOverlay, BrowserOverlayKind, BrowserScene, BrowserShot,
    BrowserVideo, InvalidBrowserPlan, MAX_BROWSER_OVERLAY_TEXT_BYTES, MAX_BROWSER_OVERLAYS,
    MAX_BROWSER_SCENES, MAX_BROWSER_SHOTS, MAX_BROWSER_VIDEOS, WireInterval, text_exceeds_limit,
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
    source_frame_rates: &'a BTreeMap<FrozenAssetId, FrameRate>,
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
        source_frame_rates: &'a BTreeMap<FrozenAssetId, FrameRate>,
    ) -> Self {
        Self {
            evaluation,
            source_frame_rates,
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
        let node = self.node(scene.element())?;
        let scene_id = node.id();
        if let Some(interval) = intersection(scene.timing().interval(), self.evaluation) {
            if self.scenes.len() >= MAX_BROWSER_SCENES {
                return Err(InvalidBrowserPlan::TooManyScenes);
            }
            self.scenes.push(BrowserScene {
                node,
                interval: WireInterval::try_from(interval)?,
            });
        }
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
        let node = self.node(shot.element())?;
        let shot_id = node.id();
        if let Some(interval) = intersection(shot.timing().interval(), self.evaluation) {
            if self.shots.len() >= MAX_BROWSER_SHOTS {
                return Err(InvalidBrowserPlan::TooManyShots);
            }
            self.shots.push(BrowserShot {
                node,
                scene_id,
                interval: WireInterval::try_from(interval)?,
            });
        }
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
        let node = self.node(video.element())?;
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
        self.videos.push(browser_video(
            video,
            node,
            shot_id,
            self.source_frame_rates,
        )?);
        Ok(())
    }

    fn project_overlay(
        &mut self,
        overlay: &TimelineOverlay,
        shot_id: BrowserNodeId,
    ) -> Result<(), InvalidBrowserPlan> {
        let node = self.node(overlay.element())?;
        let interval = overlay.timing().interval();
        if !interval.intersects(self.evaluation) {
            return Ok(());
        }
        if !self.evaluation.contains_interval(interval) {
            return Err(InvalidBrowserPlan::OverlayCrossesEvaluation);
        }
        let overlay = browser_overlay(overlay, node, shot_id)?;
        push_browser_overlay(&mut self.overlays, &mut self.overlay_text_bytes, overlay)
    }

    fn project_caption(&mut self, caption: &TimelineCaption) -> Result<(), InvalidBrowserPlan> {
        let node = self.synthetic_node()?;
        let Some(interval) = intersection(caption.interval(), self.evaluation) else {
            return Ok(());
        };
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

fn intersection(left: FrameInterval, right: FrameInterval) -> Option<FrameInterval> {
    let start = left.start().max(right.start());
    let end = left.end().min(right.end());
    (start < end).then(|| {
        FrameInterval::new(start, end).expect("ordered intersection bounds form an interval")
    })
}

fn browser_video(
    video: &TimelineVideo,
    node: BrowserNode,
    shot_id: BrowserNodeId,
    source_frame_rates: &BTreeMap<FrozenAssetId, FrameRate>,
) -> Result<BrowserVideo, InvalidBrowserPlan> {
    let asset_id = video.asset_id();
    let rate = source_frame_rates
        .get(&asset_id)
        .copied()
        .ok_or(InvalidBrowserPlan::MissingSourceFrameRate(asset_id))?;
    Ok(BrowserVideo {
        node,
        shot_id,
        asset_id: asset_id.to_string().into_boxed_str(),
        asset_identity: asset_id,
        interval: WireInterval::try_from(video.timing().interval())?,
        source_frame_rate: rate.into(),
    })
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
    Ok(BrowserOverlay {
        node,
        shot_id: Some(shot_id),
        kind,
        text: text.into_boxed_str(),
        interval: WireInterval::try_from(overlay.timing().interval())?,
    })
}

fn browser_caption(
    caption: &TimelineCaption,
    node: BrowserNode,
    interval: FrameInterval,
) -> Result<BrowserOverlay, InvalidBrowserPlan> {
    if text_exceeds_limit(caption.text()) {
        return Err(InvalidBrowserPlan::CaptionTextTooLong);
    }
    Ok(BrowserOverlay {
        node,
        shot_id: None,
        kind: BrowserOverlayKind::Caption,
        text: caption.text().into(),
        interval: WireInterval::try_from(interval)?,
    })
}
