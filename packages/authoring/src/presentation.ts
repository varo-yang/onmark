// Authored-HTML bindings for solved film, scene, shot, and content facts.
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
  RuntimeTransition,
  RuntimeVideo,
  TransitionPresentation,
  VideoPresentation,
} from "@onmark/runtime/types";

import {
  EMPTY_PRESENTATION_EXTENSIONS,
  ownExtension,
  ownExtensions,
  type PresentationExtension,
  type PresentationExtensionContext,
  type PresentationElementTargetKind,
  type PresentationTarget,
} from "./motion.js";
import { authoredImageResources } from "./resource.js";

const ELEMENTS = Object.freeze({
  callToAction: "om-cta",
  caption: "om-caption",
  film: "om-film",
  scene: "om-scene",
  shot: "om-shot",
  title: "om-title",
  transition: "om-transition",
  video: "video",
});

// Whole-film artifacts may contain semantic nodes absent from a projected
// plan. Authored display rules cannot make those unbound nodes visible.
const VISIBILITY_RULE = [
  "[data-om-node][hidden],",
  "om-film > om-scene:not([data-om-node]),",
  "om-scene > om-shot:not([data-om-node]),",
  "om-scene > om-transition,",
  "om-shot > :is(video, om-title, om-cta):not([data-om-node]) {",
  "  display: none !important;",
  "}",
  "om-cues, om-cue, om-music, om-sfx, om-vo {",
  "  display: none !important;",
  "}",
].join("\n");

// ── Public contract

/** Resolves one immutable video placement to its materialized browser source. */
export type VideoSource = (placement: RuntimeVideo) => string;

/** Browser effects required to bind solved facts onto authored HTML. */
export interface DomPresentationOptions {
  readonly document: Document;
  readonly motion?: PresentationExtension;
  readonly videoSource: VideoSource;
}

/** Binds solved facts to the semantic elements already present in the document. */
export function createDomPresentationBindings(
  options: DomPresentationOptions,
): PresentationBindings {
  const document = new AuthoredDocument(options.document, options.videoSource);
  const motion =
    options.motion === undefined ? undefined : ownExtension(options.motion);
  const bindings: PresentationBindings = {
    bindFilm: document.bindFilm.bind(document),
    bindScene: document.bindScene.bind(document),
    bindShot: document.bindShot.bind(document),
    bindTransition: document.bindTransition.bind(document),
    bindVideo: document.bindVideo.bind(document),
    bindOverlay: document.bindOverlay.bind(document),
    async bindExtensions(plan) {
      // Static authored images exist before extensions may add their own
      // explicitly owned resources. Snapshot them once to avoid two owners for
      // a motion-created image.
      const images = document.imageElements();
      const extensions =
        motion === undefined
          ? EMPTY_PRESENTATION_EXTENSIONS
          : await motion.bind(document.motionContext(plan));
      const imageResources = authoredImageResources(
        images,
        extensions.resources,
      );
      return ownExtensions({
        effects: extensions.effects,
        resources: [...imageResources, ...extensions.resources],
      });
    },
  };
  return Object.freeze(bindings);
}

// ── Binding lifecycle

/** Single mutable owner of authored-node admission and runtime decoration. */
class AuthoredDocument {
  readonly #document: Document;
  #nodes: AuthoredNodeIndex | undefined;
  readonly #targets: PresentationTarget[] = [];
  readonly #videoSource: VideoSource;

  constructor(document: Document, videoSource: VideoSource) {
    this.#document = document;
    this.#videoSource = videoSource;
  }

  bindFilm(plan: RuntimePlan): ContainerPresentation {
    if (this.#nodes !== undefined) {
      throw new Error("authored HTML film is already bound");
    }
    this.#nodes = collectAuthoredNodes(this.#document, plan);
    const element = requiredNode(this.#nodes, plan.film, "film", ELEMENTS.film);
    const visibility = visibilityStyle(this.#document);
    const bound = bindElement(element, plan.film, () => visibility.remove());
    this.#record("film", element, plan.film, { start: 0, end: 0 });
    return bound;
  }

  bindScene(scene: RuntimeScene): ContainerPresentation {
    const element = requiredNode(
      this.#nodeIndex(),
      scene.node,
      "scene",
      ELEMENTS.scene,
    );
    const bound = bindElement(element, scene.node);
    this.#record("scene", element, scene.node, scene.interval);
    return bound;
  }

  bindShot(shot: RuntimeShot): ContainerPresentation {
    const element = requiredNode(
      this.#nodeIndex(),
      shot.node,
      "shot",
      ELEMENTS.shot,
    );
    const bound = bindElement(element, shot.node);
    this.#record("shot", element, shot.node, shot.interval);
    return bound;
  }

  bindVideo(placement: RuntimeVideo): VideoPresentation {
    const element = requiredNode(
      this.#nodeIndex(),
      placement.node,
      "video",
      ELEMENTS.video,
    ) as HTMLVideoElement;
    const bound = bindElement(element, placement.node);
    element.muted = true;
    element.playsInline = true;
    this.#record("video", element, placement.node, placement.interval);
    return {
      ...bound,
      element,
      source: this.#videoSource(placement),
    };
  }

  bindTransition(placement: RuntimeTransition): TransitionPresentation {
    const nodes = this.#nodeIndex();
    const element = requiredNode(
      nodes,
      placement.node,
      "transition",
      ELEMENTS.transition,
    );
    const outgoingElement = requiredNodeId(
      nodes,
      placement.outgoingShotId,
      "outgoing shot",
      ELEMENTS.shot,
    );
    const incomingElement = requiredNodeId(
      nodes,
      placement.incomingShotId,
      "incoming shot",
      ELEMENTS.shot,
    );
    const bound = bindElement(element, placement.node);
    this.#targets.push(
      Object.freeze({
        element,
        incomingElement,
        interval: placement.interval,
        kind: "transition",
        node: placement.node,
        outgoingElement,
      }),
    );
    return Object.freeze({
      dispose: bound.dispose,
      element,
      incomingElement,
      outgoingElement,
    });
  }

  bindOverlay(placement: RuntimeOverlay): OverlayPresentation {
    if (placement.kind === "caption") {
      return this.#bindCaption(placement);
    }

    const expected =
      placement.kind === "title" ? ELEMENTS.title : ELEMENTS.callToAction;
    const element = requiredNode(
      this.#nodeIndex(),
      placement.node,
      placement.kind,
      expected,
    );
    const bound = bindElement(element, placement.node);
    this.#record(placement.kind, element, placement.node, placement.interval);
    return bound;
  }

  motionContext(plan: RuntimePlan): PresentationExtensionContext {
    const film = this.#targets[0];
    if (film === undefined || film.kind !== "film") {
      throw new Error("authored HTML motion requires a bound film root");
    }
    const targets = this.#targets.map((target) =>
      target.kind === "film"
        ? Object.freeze({ ...target, interval: plan.timeline })
        : target,
    );
    return Object.freeze({
      frameRate: plan.frameRate,
      targets: Object.freeze(targets),
    });
  }

  imageElements(): readonly HTMLImageElement[] {
    return Array.from(this.#document.images).filter((element) =>
      element.hasAttribute("src"),
    );
  }

  #bindCaption(placement: RuntimeOverlay): OverlayPresentation {
    const element = this.#document.createElement(ELEMENTS.caption);
    element.textContent = placement.text;
    this.#nodeIndex().film.append(element);
    const bound = bindElement(element, placement.node, () => element.remove());
    this.#record("caption", element, placement.node, placement.interval);
    return bound;
  }

  #record(
    kind: PresentationElementTargetKind,
    element: HTMLElement,
    node: RuntimeNode,
    interval: RuntimePlan["evaluation"],
  ): void {
    this.#targets.push(Object.freeze({ kind, element, interval, node }));
  }

  #nodeIndex(): AuthoredNodeIndex {
    if (this.#nodes === undefined) {
      throw new Error("authored HTML nodes require a bound film");
    }
    return this.#nodes;
  }
}

// ── Authored identity

interface AuthoredNodeIndex {
  readonly film: HTMLElement;
  readonly elements: readonly HTMLElement[];
}

/**
 * Indexes renderable semantic elements by the accepted plan's dense preorder.
 *
 * Whole-film and region documents each receive the matching Browser Plan.
 * Authored IDs, rather than protocol node IDs, carry cross-build identity.
 */
function collectAuthoredNodes(
  document: Document,
  plan: RuntimePlan,
): AuthoredNodeIndex {
  const films = semanticChildren(document.body, ELEMENTS.film);
  if (films.length !== 1) {
    throw new Error("authored HTML requires exactly one om-film element");
  }
  const film = films[0]!;
  const indexed = [film];
  const browserVideoShots = new Set(plan.videos.map(({ shotId }) => shotId));
  for (const scene of semanticChildren(film, ELEMENTS.scene)) {
    indexed.push(scene);
    const structure = semanticChildren(
      scene,
      `${ELEMENTS.shot}, ${ELEMENTS.transition}`,
    );
    for (const element of structure) {
      if (element.matches(ELEMENTS.transition)) {
        indexed.push(element);
        continue;
      }
      const shot = element;
      indexed.push(shot);
      // Browser Plans assign dense semantic preorder IDs. The shot just
      // admitted is therefore the parent ID carried by projected videos.
      const shotNodeId = indexed.length - 1;
      const contentSelector = browserVideoShots.has(shotNodeId)
        ? `${ELEMENTS.video}, ${ELEMENTS.title}, ${ELEMENTS.callToAction}`
        : `${ELEMENTS.title}, ${ELEMENTS.callToAction}`;
      indexed.push(...semanticChildren(shot, contentSelector));
    }
  }
  return Object.freeze({ film, elements: Object.freeze(indexed) });
}

function requiredNodeId(
  nodes: AuthoredNodeIndex,
  nodeId: number,
  role: string,
  selector: string,
): HTMLElement {
  return requiredNode(nodes, { nodeId }, role, selector);
}

function requiredNode(
  nodes: AuthoredNodeIndex,
  node: RuntimeNode,
  role: string,
  selector: string,
): HTMLElement {
  const element = nodes.elements[node.nodeId];
  if (element === undefined) {
    throw new Error(
      `authored HTML has no ${role} element for node ${node.nodeId}`,
    );
  }
  if (!element.matches(selector)) {
    throw new Error(
      `authored HTML node ${node.nodeId} is not a ${role} element`,
    );
  }
  return element;
}

function semanticChildren(parent: Element, selector: string): HTMLElement[] {
  return elements(parent.children).filter((element) =>
    element.matches(selector),
  );
}

function elements(collection: ArrayLike<Element>): HTMLElement[] {
  return Array.from(collection, (element) => element as HTMLElement);
}

// ── Browser decoration

function bindElement(
  element: HTMLElement,
  node: RuntimeNode,
  release?: () => void,
): ContainerPresentation {
  requireAuthoredId(element, node);
  const previousNode = element.dataset["omNode"];
  const previouslyHidden = element.hidden;
  element.dataset["omNode"] = String(node.nodeId);
  element.hidden = true;

  return {
    element,
    setVisible(visible): void {
      element.hidden = !visible;
    },
    dispose(): void {
      element.hidden = previouslyHidden;
      restoreDataset(element, "omNode", previousNode);
      release?.();
    },
  };
}

function requireAuthoredId(element: HTMLElement, node: RuntimeNode): void {
  const expected = node.authoredId ?? "";
  if (element.id !== expected) {
    throw new Error(
      `authored HTML node identity differs: expected "${expected}", found "${element.id}"`,
    );
  }
}

function restoreDataset(
  element: HTMLElement,
  name: string,
  value: string | undefined,
): void {
  if (value === undefined) {
    delete element.dataset[name];
    return;
  }
  element.dataset[name] = value;
}

function visibilityStyle(document: Document): HTMLStyleElement {
  const style = document.createElement("style");
  style.textContent = VISIBILITY_RULE;
  document.head.append(style);
  return style;
}
