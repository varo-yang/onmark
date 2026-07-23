// Semantic DOM projection for solved film, scene, shot, and content facts.
// Rust owns structure and timing; this module owns browser node lifetimes.

import type {
  ContainerPresentation,
  OverlayPresentation,
  PresentationBindings,
  RuntimeNode,
  RuntimeOverlay,
  RuntimePlan,
  RuntimeScene,
  RuntimeShot,
  RuntimeVideo,
  VideoPresentation,
} from "@onmark/runtime/types";

import {
  EMPTY_PRESENTATION_EXTENSIONS,
  ownExtension,
  ownExtensions,
  type PresentationExtension,
  type PresentationExtensionContext,
  type PresentationTarget,
  type PresentationTargetKind,
} from "./motion.js";

// ── Public contract ──

/** Stable semantic classes emitted by the DOM authoring surface. */
export const PRESENTATION_CLASSES = Object.freeze({
  film: "onmark-film",
  scene: "onmark-scene",
  shot: "onmark-shot",
  video: "onmark-video",
  overlay: "onmark-overlay",
  title: "onmark-title",
  callToAction: "onmark-call-to-action",
  caption: "onmark-caption",
});

/** Resolves one immutable video placement to its materialized browser source. */
export type VideoSource = (placement: RuntimeVideo) => string;

/** Browser effects required to project solved facts into one document. */
export interface DomPresentationOptions {
  readonly document: Document;
  readonly motion?: PresentationExtension;
  readonly videoSource: VideoSource;
}

/** Creates deterministic semantic DOM bindings without owning visual design. */
export function createDomPresentationBindings(
  options: DomPresentationOptions,
): PresentationBindings {
  const projection = new DomProjection(options.document, options.videoSource);
  const motion =
    options.motion === undefined ? undefined : ownExtension(options.motion);
  const bindings: PresentationBindings = {
    bindFilm: projection.bindFilm.bind(projection),
    bindScene: projection.bindScene.bind(projection),
    bindShot: projection.bindShot.bind(projection),
    bindVideo: projection.bindVideo.bind(projection),
    bindOverlay: projection.bindOverlay.bind(projection),
    async bindExtensions(plan) {
      if (motion === undefined) {
        return EMPTY_PRESENTATION_EXTENSIONS;
      }
      const extensions = await motion.bind(projection.motionContext(plan));
      return ownExtensions(extensions);
    },
  };
  return Object.freeze(bindings);
}

// ── Semantic projection ──

class DomProjection {
  readonly #document: Document;
  readonly #elements = new Map<number, HTMLElement>();
  readonly #targets: PresentationTarget[] = [];
  readonly #videoSource: VideoSource;

  constructor(document: Document, videoSource: VideoSource) {
    this.#document = document;
    this.#videoSource = videoSource;
  }

  bindFilm(node: RuntimeNode): ContainerPresentation {
    const element = this.#document.createElement("main");
    this.#bindNode(element, node, PRESENTATION_CLASSES.film);
    this.#document.body.append(element);
    this.#record("film", element, node, { start: 0, end: 0 });
    return presentation(element);
  }

  bindScene(scene: RuntimeScene): ContainerPresentation {
    const element = this.#document.createElement("section");
    this.#bindNode(element, scene.node, PRESENTATION_CLASSES.scene);
    this.#film().append(element);
    this.#record("scene", element, scene.node, scene.interval);
    return presentation(element);
  }

  bindShot(shot: RuntimeShot): ContainerPresentation {
    const element = this.#document.createElement("article");
    this.#bindNode(element, shot.node, PRESENTATION_CLASSES.shot);
    this.#parent(shot.sceneId).append(element);
    this.#record("shot", element, shot.node, shot.interval);
    return presentation(element);
  }

  bindVideo(placement: RuntimeVideo): VideoPresentation {
    const element = this.#document.createElement("video");
    this.#bindNode(element, placement.node, PRESENTATION_CLASSES.video);
    element.muted = true;
    element.playsInline = true;
    this.#parent(placement.shotId).append(element);
    this.#record("video", element, placement.node, placement.interval);

    return {
      ...presentation(element),
      element,
      source: this.#videoSource(placement),
    };
  }

  bindOverlay(placement: RuntimeOverlay): OverlayPresentation {
    const element = this.#document.createElement(overlayTag(placement.kind));
    const kind = placement.kind;
    const className = `${PRESENTATION_CLASSES.overlay} ${overlayClass(kind)}`;
    this.#bindNode(element, placement.node, className);
    element.textContent = placement.text;
    this.#overlayParent(placement).append(element);
    this.#record(kind, element, placement.node, placement.interval);
    return presentation(element);
  }

  motionContext(plan: RuntimePlan): PresentationExtensionContext {
    const film = this.#targets[0];
    if (film === undefined || film.kind !== "film") {
      throw new Error("semantic DOM motion requires a bound film root");
    }
    const targets = this.#targets.map((target) =>
      target.kind === "film"
        ? Object.freeze({ ...target, interval: plan.evaluation })
        : target,
    );
    return Object.freeze({
      frameRate: plan.frameRate,
      targets: Object.freeze(targets),
    });
  }

  #bindNode(element: HTMLElement, node: RuntimeNode, className: string): void {
    element.className = className;
    element.dataset["onmarkNode"] = String(node.nodeId);
    if (node.authoredId !== undefined && node.authoredId !== null) {
      element.id = node.authoredId;
      element.dataset["onmarkId"] = node.authoredId;
    }
    element.hidden = true;
    this.#elements.set(node.nodeId, element);
  }

  #record(
    kind: PresentationTargetKind,
    element: HTMLElement,
    node: RuntimeNode,
    interval: RuntimePlan["evaluation"],
  ): void {
    this.#targets.push(Object.freeze({ kind, element, interval, node }));
  }

  #film(): HTMLElement {
    const film = this.#targets[0]?.element;
    if (film === undefined) {
      throw new Error("semantic DOM scene requires a bound film root");
    }
    return film;
  }

  #parent(nodeId: number): HTMLElement {
    const parent = this.#elements.get(nodeId);
    if (parent === undefined) {
      throw new Error(`semantic DOM parent ${nodeId} is not bound`);
    }
    return parent;
  }

  #overlayParent(placement: RuntimeOverlay): HTMLElement {
    if (placement.shotId === undefined || placement.shotId === null) {
      return this.#film();
    }
    return this.#parent(placement.shotId);
  }
}

// ── Browser elements ──

function presentation(element: HTMLElement): ContainerPresentation {
  return {
    element,
    setVisible(visible): void {
      element.hidden = !visible;
    },
    dispose(): void {
      element.remove();
    },
  };
}

function overlayTag(kind: RuntimeOverlay["kind"]): "div" | "h1" {
  return kind === "title" ? "h1" : "div";
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
