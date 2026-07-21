// Browser DOM bindings for solved video, overlay, and imported-caption facts.
// Layout remains presentation-owned; runtime remains the sole timing owner.

import type {
  FrameEffect,
  OverlayPresentation,
  PresentationBindings,
  RuntimeOverlay,
  RuntimePlan,
  RuntimeVideo,
  VideoPresentation,
} from "@onmark/runtime/types";

/** Stable semantic classes emitted by the DOM authoring surface. */
export const PRESENTATION_CLASSES = Object.freeze({
  video: "onmark-video",
  overlay: "onmark-overlay",
  title: "onmark-title",
  callToAction: "onmark-call-to-action",
  caption: "onmark-caption",
});

/** Resolves one immutable video placement to its materialized browser source. */
export type VideoSource = (placement: RuntimeVideo) => string;

/** Creates the paused effects owned by one loaded presentation. */
export type FrameEffectFactory = (plan: RuntimePlan) => readonly FrameEffect[];

/** Browser effects required to bind solved facts into an author-owned document. */
export interface DomPresentationOptions {
  readonly document: Document;
  readonly frameEffects?: FrameEffectFactory;
  readonly videoSource: VideoSource;
}

/** Creates deterministic DOM bindings without owning layout or frame timing. */
export function createDomPresentationBindings(
  options: DomPresentationOptions,
): PresentationBindings {
  const { document, frameEffects, videoSource } = options;
  const bindings: PresentationBindings = {
    bindVideo(placement, index) {
      return bindVideo(document, videoSource, placement, index);
    },
    bindOverlay(placement, index) {
      return bindOverlay(document, placement, index);
    },
    bindFrameEffects(plan) {
      return Object.freeze([...(frameEffects?.(plan) ?? [])]);
    },
  };
  return Object.freeze(bindings);
}

function bindVideo(
  document: Document,
  videoSource: VideoSource,
  placement: RuntimeVideo,
  index: number,
): VideoPresentation {
  const source = videoSource(placement);
  const element = document.createElement("video");
  element.className = PRESENTATION_CLASSES.video;
  element.dataset["onmarkPlacement"] = String(index);
  element.muted = true;
  element.playsInline = true;
  element.hidden = true;
  document.body.append(element);

  return {
    element,
    source,
    setVisible(visible): void {
      element.hidden = !visible;
    },
    dispose(): void {
      element.remove();
    },
  };
}

function bindOverlay(
  document: Document,
  placement: RuntimeOverlay,
  index: number,
): OverlayPresentation {
  const element = document.createElement("div");
  element.className = `${PRESENTATION_CLASSES.overlay} ${overlayClass(placement.kind)}`;
  element.dataset["onmarkPlacement"] = String(index);
  element.textContent = placement.text;
  element.hidden = true;
  document.body.append(element);

  return {
    setVisible(visible): void {
      element.hidden = !visible;
    },
    dispose(): void {
      element.remove();
    },
  };
}

function overlayClass(kind: RuntimeOverlay["kind"]): string {
  switch (kind) {
    case "title":
      return PRESENTATION_CLASSES.title;
    case "callToAction":
      return PRESENTATION_CLASSES.callToAction;
    case "caption":
      return PRESENTATION_CLASSES.caption;
  }
}
