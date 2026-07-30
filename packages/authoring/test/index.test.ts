// Public authored-DOM behavior over a deliberately small browser fake.

import assert from "node:assert/strict";
import test from "node:test";

import {
  combineMotion,
  createDomPresentationBindings,
  type PresentationTarget,
} from "../src/index.js";
import type {
  FrameEffect,
  PresentationResource,
  RuntimePlan,
} from "@onmark/runtime/types";

// ── Authored binding ──

test("binds solved structure without replacing authored HTML", () => {
  const browser = new FakeDocument();
  const bindings = bindingsFor(browser);

  const film = bindings.bindFilm(PLAN);
  const scene = bindings.bindScene(PLAN.scenes[0]!);
  const shot = bindings.bindShot(PLAN.shots[0]!);
  const video = bindings.bindVideo(PLAN.videos[0]!);
  const overlay = bindings.bindOverlay(PLAN.overlays[0]!);

  assert.equal(browser.authored.film, film.element);
  assert.equal(browser.authored.scene, scene.element);
  assert.equal(browser.authored.shot, shot.element);
  assert.equal(browser.authored.video, video.element);
  assert.equal(browser.authored.title, overlay.element);
  assert.deepEqual(tags(browser.body), [
    "om-film",
    "om-scene",
    "om-shot",
    "video",
    "om-title",
    "span",
  ]);
  assert.equal(overlay.element.className, "headline");
  assert.equal(overlay.element.children.length, 1);
  assert.equal(shot.element.id, "hero");
  assert.deepEqual(shot.element.dataset, { omNode: "2" });
  assert.equal(overlay.element.textContent, "Opening");
  assert.equal(video.source, `./assets/${PLAN.videos[0]!.assetId}`);

  film.setVisible(true);
  scene.setVisible(true);
  shot.setVisible(true);
  video.setVisible(true);
  overlay.setVisible(true);
  assert.equal(
    browser.authoredNodes.every(({ hidden }) => !hidden),
    true,
  );

  overlay.dispose();
  video.dispose();
  shot.dispose();
  scene.dispose();
  film.dispose();
  assert.equal(
    browser.authoredNodes.every(({ removed }) => !removed),
    true,
  );
  assert.equal(
    browser.authoredNodes.every(
      ({ dataset }) => dataset["omNode"] === undefined,
    ),
    true,
  );
});

test("applies projected typed fields before motion observes the DOM", async () => {
  const browser = new FakeDocument();
  const featured = new FakeElement("span");
  const untouched = new FakeElement("span");
  browser.authored.accent.setAttribute("data-om-text", "headline");
  browser.authored.shot.setAttribute("data-om-css", "accent progress");
  featured.setAttribute("data-om-show", "featured");
  featured.hidden = true;
  untouched.setAttribute("data-om-text", "otherRegion");
  untouched.textContent = "Fallback";
  browser.authored.shot.append(featured, untouched);

  let motionObservedVariant = false;
  const plan: RuntimePlan = {
    ...PLAN,
    variantFields: [
      { name: "accent", value: { kind: "color", value: "#1a2b3c" } },
      { name: "featured", value: { kind: "boolean", value: true } },
      { name: "headline", value: { kind: "text", value: "Canonical" } },
      { name: "progress", value: { kind: "integer", value: 72 } },
    ],
  };
  const bindings = createDomPresentationBindings({
    document: asBrowserDocument(browser),
    motion: {
      bind() {
        motionObservedVariant =
          browser.authored.accent.textContent === "Canonical" &&
          browser.authored.shot.style.getPropertyValue("--accent") ===
            "#1a2b3c" &&
          browser.authored.shot.style.getPropertyValue("--progress") === "72" &&
          !featured.hidden;
        return { effects: [], resources: [] };
      },
    },
    videoSource: () => "./video.mp4",
  });

  bindings.bindFilm(plan);
  await bindings.bindExtensions(plan);

  assert.equal(motionObservedVariant, true);
  assert.equal(untouched.textContent, "Fallback");
});

test("binds a transition to both adjacent shot elements", async () => {
  const browser = new FakeDocument();
  const transition = new FakeElement("om-transition");
  transition.id = "reveal";
  const closing = authoredScene("unused", "closing", "After");
  browser.authored.scene.append(transition, closing.shot);
  let transitionTarget:
    Extract<PresentationTarget, { readonly kind: "transition" }> | undefined;
  const plan: RuntimePlan = {
    ...PLAN,
    timeline: { start: 0, end: 105 },
    evaluation: { start: 0, end: 105 },
    output: { start: 0, end: 105 },
    scenes: [{ ...PLAN.scenes[0]!, interval: { start: 0, end: 105 } }],
    shots: [
      { ...PLAN.shots[0]!, interval: { start: 0, end: 60 } },
      {
        node: { nodeId: 6, authoredId: "closing" },
        sceneId: 1,
        interval: { start: 45, end: 105 },
      },
    ],
    transitions: [
      {
        incomingShotId: 6,
        interval: { start: 45, end: 60 },
        node: { nodeId: 5, authoredId: "reveal" },
        outgoingShotId: 2,
      },
    ],
    overlays: [
      { ...PLAN.overlays[0]!, interval: { start: 0, end: 60 } },
      {
        node: { nodeId: 7, authoredId: null },
        shotId: 6,
        kind: "title",
        text: "After",
        interval: { start: 45, end: 105 },
      },
    ],
  };
  const bindings = createDomPresentationBindings({
    document: asBrowserDocument(browser),
    motion: {
      bind({ targets }) {
        transitionTarget = targets.find(
          (target) => target.kind === "transition",
        );
        return { effects: [], resources: [] };
      },
    },
    videoSource: () => "./video.mp4",
  });

  bindings.bindFilm(plan);
  bindings.bindScene(plan.scenes[0]!);
  bindings.bindShot(plan.shots[0]!);
  bindings.bindVideo(plan.videos[0]!);
  bindings.bindOverlay(plan.overlays[0]!);
  const bound = bindings.bindTransition(plan.transitions[0]!);
  bindings.bindShot(plan.shots[1]!);
  bindings.bindOverlay(plan.overlays[1]!);
  await bindings.bindExtensions(plan);

  assert.equal(bound.element, transition);
  assert.equal(bound.outgoingElement, browser.authored.shot);
  assert.equal(bound.incomingElement, closing.shot);
  assert.equal(transitionTarget?.outgoingElement, browser.authored.shot);
  assert.equal(transitionTarget?.incomingElement, closing.shot);
});

test("projects native video out of dense foreground identity", () => {
  const browser = new FakeDocument();
  const bindings = bindingsFor(browser);
  const foreground: RuntimePlan = {
    ...PLAN,
    videos: [],
    overlays: [
      {
        ...PLAN.overlays[0]!,
        node: { ...PLAN.overlays[0]!.node, nodeId: 3 },
      },
    ],
  };

  bindings.bindFilm(foreground);
  bindings.bindScene(foreground.scenes[0]!);
  bindings.bindShot(foreground.shots[0]!);
  const overlay = bindings.bindOverlay(foreground.overlays[0]!);

  assert.equal(overlay.element, browser.authored.title);
  assert.equal(browser.authored.video.dataset["omNode"], undefined);
});

test("owns readiness for native authored images", async () => {
  const browser = new FakeDocument();
  const image = new FakeElement("img");
  image.src = "./resources/poster.svg";
  browser.images.push(image);
  const bindings = bindingsFor(browser);

  const extensions = await bindings.bindExtensions(PLAN);
  assert.equal(extensions.resources.length, 1);
  assert.equal(extensions.resources[0]?.kind, "image");

  await extensions.resources[0]?.prepare();
  assert.equal(image.decodeCalls, 1);

  await extensions.resources[0]?.dispose();
  assert.equal(image.sourceRemoved, true);
});

test("allocates native image identities around extension-owned resources", async () => {
  const browser = new FakeDocument();
  const image = new FakeElement("img");
  image.src = "./resources/poster.svg";
  browser.images.push(image);
  const extensionResource = disposableResource(() => {});
  const bindings = createDomPresentationBindings({
    document: browser as unknown as Document,
    motion: {
      bind() {
        return {
          effects: [],
          resources: [
            {
              ...extensionResource,
              id: "authored-image-0",
              kind: "image",
            },
          ],
        };
      },
    },
    videoSource: () => "./video.mp4",
  });

  bindings.bindFilm(PLAN);
  const extensions = await bindings.bindExtensions(PLAN);
  const identities = extensions.resources.map(
    ({ id, kind }) => `${kind}:${id}`,
  );

  assert.equal(new Set(identities).size, identities.length);
});

test("owns bound and omitted semantic visibility independently of authored CSS", () => {
  const browser = new FakeDocument();
  const bindings = bindingsFor(browser);
  const film = bindings.bindFilm(PLAN);

  const visibility = browser.head.children[0];
  assert.equal(
    visibility?.textContent,
    [
      "[data-om-node][hidden],",
      "[data-om-show][hidden],",
      "om-film > om-scene:not([data-om-node]),",
      "om-scene > om-shot:not([data-om-node]),",
      "om-scene > om-transition,",
      "om-shot > :is(video, om-title, om-cta):not([data-om-node]) {",
      "  display: none !important;",
      "}",
      "om-cues, om-cue, om-captions, om-music, om-sfx, om-vo {",
      "  display: none !important;",
      "}",
    ].join("\n"),
  );

  film.dispose();
  assert.equal(visibility?.removed, true);
});

test("maps every overlay role to one stable semantic element", () => {
  const browser = new FakeDocument();
  browser.authored.shot.append(new FakeElement("om-cta"));
  const bindings = bindingsFor(browser);
  bindings.bindFilm(PLAN);
  bindings.bindScene(PLAN.scenes[0]!);
  bindings.bindShot(PLAN.shots[0]!);

  const title = bindings.bindOverlay(PLAN.overlays[0]!);
  const callToAction = bindings.bindOverlay({
    ...PLAN.overlays[0]!,
    node: { nodeId: 5, authoredId: null },
    kind: "callToAction",
  });
  const caption = bindings.bindOverlay({
    ...PLAN.overlays[0]!,
    node: { nodeId: 6, authoredId: null },
    shotId: null,
    kind: "caption",
    captionTrack: { id: "en", language: "en-US" },
  });

  assert.equal(title.element.localName, "om-title");
  assert.equal(callToAction.element.localName, "om-cta");
  assert.deepEqual(caption.element.dataset, {
    omNode: "6",
    track: "en",
  });
  assert.equal(caption.element.lang, "en-US");
  assert.equal(
    browser.authored.film.children.includes(
      caption.element as unknown as FakeElement,
    ),
    true,
  );
});

test("binds dense local node identity in a projected region document", () => {
  const browser = new FakeDocument();
  browser.authored.scene.remove();
  const later = authoredScene("later", "later-shot", "Later");
  browser.authored.film.append(later.scene);
  const bindings = bindingsFor(browser);

  bindings.bindFilm({
    ...PLAN,
    scenes: [
      {
        node: { nodeId: 1, authoredId: "later" },
        interval: { start: 60, end: 120 },
      },
    ],
    shots: [
      {
        node: { nodeId: 2, authoredId: "later-shot" },
        sceneId: 1,
        interval: { start: 60, end: 120 },
      },
    ],
    videos: [],
    overlays: [
      {
        node: { nodeId: 3, authoredId: null },
        shotId: 2,
        kind: "title",
        text: "Later",
        interval: { start: 60, end: 120 },
      },
    ],
  });
  const scene = bindings.bindScene({
    node: { nodeId: 1, authoredId: "later" },
    interval: { start: 60, end: 120 },
  });
  const shot = bindings.bindShot({
    node: { nodeId: 2, authoredId: "later-shot" },
    sceneId: 1,
    interval: { start: 60, end: 120 },
  });
  const overlay = bindings.bindOverlay({
    node: { nodeId: 3, authoredId: null },
    shotId: 2,
    kind: "title",
    text: "Later",
    interval: { start: 60, end: 120 },
  });

  assert.equal(scene.element, later.scene);
  assert.equal(shot.element, later.shot);
  assert.equal(overlay.element, later.title);
});

test("does not give presentation wrappers screenplay ownership", () => {
  const browser = new FakeDocument();
  const wrapper = new FakeElement("div");
  wrapper.append(new FakeElement("om-film"));
  browser.body.append(wrapper);
  const bindings = bindingsFor(browser);

  const film = bindings.bindFilm(PLAN);

  assert.equal(film.element, browser.authored.film);
});

// ── Extension boundary ──

test("delivers one immutable semantic view to local motion", async () => {
  const browser = new FakeDocument();
  const effect: FrameEffect = {
    apply(): void {},
    dispose(): void {},
  };
  let targetKinds: readonly string[] = [];
  const bindings = createDomPresentationBindings({
    document: asBrowserDocument(browser),
    motion: {
      bind(context) {
        targetKinds = context.targets.map(({ kind }) => kind);
        assert.equal(Object.isFrozen(context.targets), true);
        assert.deepEqual(context.targets[0]?.interval, PLAN.timeline);
        return { effects: [effect], resources: [] };
      },
    },
    videoSource: () => "unused",
  });

  bindings.bindFilm(PLAN);
  bindings.bindScene(PLAN.scenes[0]!);
  bindings.bindShot(PLAN.shots[0]!);
  bindings.bindVideo(PLAN.videos[0]!);
  bindings.bindOverlay(PLAN.overlays[0]!);
  const extensions = await bindings.bindExtensions(PLAN);

  assert.deepEqual(targetKinds, ["film", "scene", "shot", "video", "title"]);
  assert.equal(extensions.effects.length, 1);
  assert.notEqual(extensions.effects[0], effect);
  assert.equal(Object.isFrozen(extensions.effects), true);
});

test("binds one immutable resource collection through motion", async () => {
  const browser = new FakeDocument();
  const resource: PresentationResource = {
    id: "poster",
    kind: "image",
    prepare(): void {},
    dispose(): void {},
  };
  const bindings = createDomPresentationBindings({
    document: asBrowserDocument(browser),
    motion: {
      bind() {
        return { effects: [], resources: [resource] };
      },
    },
    videoSource: () => "unused",
  });

  bindings.bindFilm(PLAN);
  const extensions = await bindings.bindExtensions(PLAN);

  assert.equal(extensions.resources[0]?.id, resource.id);
  assert.notEqual(extensions.resources[0], resource);
  assert.equal(Object.isFrozen(extensions.resources), true);
});

test("releases prior extensions when later motion binding fails", async () => {
  const browser = new FakeDocument();
  const released: string[] = [];
  const effect = disposableEffect(() => {
    released.push("effect");
    throw new Error("effect cleanup failed");
  });
  const motion = combineMotion(
    {
      bind() {
        return {
          effects: [effect],
          resources: [disposableResource(() => released.push("resource"))],
        };
      },
    },
    {
      bind(): never {
        effect.dispose = () => {
          released.push("mutated");
        };
        throw new Error("motion binding failed");
      },
    },
  );
  const bindings = createDomPresentationBindings({
    document: asBrowserDocument(browser),
    motion,
    videoSource: () => "unused",
  });

  bindings.bindFilm(PLAN);
  await assert.rejects(bindings.bindExtensions(PLAN), AggregateError);
  assert.deepEqual(released, ["effect", "resource"]);
});

// ── Fixture ──

const PLAN: RuntimePlan = {
  timelineVersion: 6,
  frameRate: { numerator: 30, denominator: 1 },
  timeline: { start: 0, end: 90 },
  evaluation: { start: 0, end: 60 },
  output: { start: 0, end: 60 },
  film: { nodeId: 0, authoredId: "film" },
  scenes: [
    {
      node: { nodeId: 1, authoredId: "opening" },
      interval: { start: 0, end: 60 },
    },
  ],
  shots: [
    {
      node: { nodeId: 2, authoredId: "hero" },
      sceneId: 1,
      interval: { start: 0, end: 60 },
    },
  ],
  transitions: [],
  variantFields: [],
  videos: [
    {
      node: { nodeId: 3, authoredId: null },
      shotId: 2,
      assetId:
        "sha256:0101010101010101010101010101010101010101010101010101010101010101",
      interval: { start: 0, end: 60 },
      sourceTiming: {
        kind: "constant",
        frameRate: { numerator: 30, denominator: 1 },
      },
      source: {
        startNanoseconds: "0",
        endNanoseconds: "2000000000",
        naturalEndNanoseconds: "2000000000",
        playbackRate: { numerator: 1, denominator: 1 },
        plays: 1,
        holdLastNanoseconds: "0",
      },
    },
  ],
  overlays: [
    {
      node: { nodeId: 4, authoredId: null },
      shotId: 2,
      kind: "title",
      text: "Opening",
      interval: { start: 0, end: 60 },
    },
  ],
};

function disposableEffect(dispose: () => void): FrameEffect {
  return { apply(): void {}, dispose };
}

function disposableResource(dispose: () => void): PresentationResource {
  return { id: "test", kind: "custom", prepare(): void {}, dispose };
}

function bindingsFor(browser: FakeDocument) {
  return createDomPresentationBindings({
    document: asBrowserDocument(browser),
    videoSource: ({ assetId }) => `./assets/${assetId}`,
  });
}

function authoredScene(sceneId: string, shotId: string, title: string) {
  const scene = new FakeElement("om-scene");
  const shot = new FakeElement("om-shot");
  const overlay = new FakeElement("om-title");
  scene.id = sceneId;
  shot.id = shotId;
  overlay.textContent = title;
  shot.append(overlay);
  scene.append(shot);
  return { scene, shot, title: overlay };
}

class FakeDocument {
  readonly body = new FakeElement("body");
  readonly head = new FakeElement("head");
  readonly images: FakeElement[] = [];
  readonly created: FakeElement[] = [];
  readonly authored = authoredTree();
  readonly authoredNodes = Object.values(this.authored);

  constructor() {
    this.body.append(this.authored.film);
  }

  createElement(tagName: string): FakeElement {
    const element = new FakeElement(tagName);
    this.created.push(element);
    return element;
  }

  querySelectorAll<T extends Element>(_selector: string): NodeListOf<T> {
    return descendants(this.body).filter(
      (element) =>
        element.getAttribute("data-om-text") !== null ||
        element.getAttribute("data-om-css") !== null ||
        element.getAttribute("data-om-show") !== null,
    ) as unknown as NodeListOf<T>;
  }
}

class FakeElement {
  readonly #attributes = new Map<string, string>();
  readonly children: FakeElement[] = [];
  readonly dataset: Record<string, string> = {};
  className = "";
  decodeCalls = 0;
  hidden = false;
  id = "";
  lang = "";
  muted = false;
  parent: FakeElement | undefined;
  playsInline = false;
  removed = false;
  sourceRemoved = false;
  src = "";
  readonly style = new FakeStyle();
  textContent: string | null = null;

  constructor(readonly localName: string) {}

  get tagName(): string {
    return this.localName.toUpperCase();
  }

  hasAttribute(name: string): boolean {
    return name === "src" ? this.src.length > 0 : this.#attributes.has(name);
  }

  getAttribute(name: string): string | null {
    return this.#attributes.get(name) ?? null;
  }

  setAttribute(name: string, value: string): void {
    this.#attributes.set(name, value);
  }

  append(...elements: FakeElement[]): void {
    for (const element of elements) {
      element.parent = this;
      this.children.push(element);
    }
  }

  async decode(): Promise<void> {
    this.decodeCalls += 1;
  }

  matches(selector: string): boolean {
    return selector
      .split(",")
      .some((candidate) => candidate.trim() === this.localName);
  }

  remove(): void {
    this.removed = true;
    if (this.parent !== undefined) {
      const index = this.parent.children.indexOf(this);
      if (index >= 0) {
        this.parent.children.splice(index, 1);
      }
    }
  }

  removeAttribute(name: string): void {
    assert.equal(name, "src");
    this.sourceRemoved = true;
    this.src = "";
  }
}

class FakeStyle {
  readonly #properties = new Map<string, string>();
  visibility = "";

  getPropertyValue(name: string): string {
    return this.#properties.get(name) ?? "";
  }

  setProperty(name: string, value: string): void {
    this.#properties.set(name, value);
  }
}

function authoredTree() {
  const film = new FakeElement("om-film");
  film.id = "film";
  const scene = new FakeElement("om-scene");
  scene.id = "opening";
  const shot = new FakeElement("om-shot");
  shot.id = "hero";
  const video = new FakeElement("video");
  const title = new FakeElement("om-title");
  title.className = "headline";
  title.textContent = "Opening";
  const accent = new FakeElement("span");
  title.append(accent);
  shot.append(video, title);
  scene.append(shot);
  film.append(scene);
  return { accent, film, scene, shot, title, video };
}

function asBrowserDocument(document: FakeDocument): Document {
  return document as unknown as Document;
}

function tags(root: FakeElement): string[] {
  const result: string[] = [];
  for (const child of root.children) {
    result.push(child.localName, ...tags(child));
  }
  return result;
}

function descendants(root: FakeElement): FakeElement[] {
  return root.children.flatMap((child) => [child, ...descendants(child)]);
}
