// Browser DOM bindings for screenplay-authored video and overlay facts.
// Layout remains presentation-owned; runtime remains the sole timing owner.

import type {
  OverlayPresentation,
  PresentationBindings,
  RuntimeOverlay,
  RuntimeVideo,
  VideoPresentation,
} from "@onmark/runtime/types";

/** Stable semantic classes emitted by the Gate-one DOM authoring surface. */
export const PRESENTATION_CLASSES = Object.freeze({
  video: "onmark-video",
  overlay: "onmark-overlay",
  title: "onmark-title",
  callToAction: "onmark-call-to-action",
});

/** Resolves one immutable video placement to its materialized browser source. */
export type VideoSource = (placement: RuntimeVideo) => string;

/** Browser effects required to bind solved facts into an author-owned document. */
export interface DomPresentationOptions {
  readonly document: Document;
  readonly videoSource: VideoSource;
}

/** Creates deterministic DOM bindings without owning layout or frame timing. */
export function createDomPresentationBindings(
  options: DomPresentationOptions,
): PresentationBindings {
  const { document, videoSource } = options;
  const bindings: PresentationBindings = {
    bindVideo(placement, index) {
      return bindVideo(document, videoSource, placement, index);
    },
    bindOverlay(placement, index) {
      return bindOverlay(document, placement, index);
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
  }
}
