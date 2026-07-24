// Public authored-DOM behavior over a deliberately small browser fake.

import assert from "node:assert/strict";
import test from "node:test";

import { combineMotion, createDomPresentationBindings } from "../src/index.js";
import type {
  FrameEffect,
  PresentationResource,
  RuntimePlan,
} from "@onmark/runtime/types";

// ── Authored binding ──

test("binds solved structure without replacing authored HTML", () => {
  const browser = new FakeDocument();
  const bindings = bindingsFor(browser);

  const film = bindings.bindFilm(PLAN.film);
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

test("owns bound and omitted semantic visibility independently of authored CSS", () => {
  const browser = new FakeDocument();
  const bindings = bindingsFor(browser);
  const film = bindings.bindFilm(PLAN.film);

  const visibility = browser.head.children[0];
  assert.equal(
    visibility?.textContent,
    [
      "[data-om-node][hidden],",
      "om-film > om-scene:not([data-om-node]),",
      "om-scene > om-shot:not([data-om-node]),",
      "om-shot > :is(video, om-title, om-cta):not([data-om-node]) {",
      "  display: none !important;",
      "}",
      "om-cues, om-cue, om-music, om-sfx, om-vo {",
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
  bindings.bindFilm(PLAN.film);
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
  });

  assert.equal(title.element.localName, "om-title");
  assert.equal(callToAction.element.localName, "om-cta");
  assert.equal(
    browser.authored.film.children.includes(
      caption.element as unknown as FakeElement,
    ),
    true,
  );
});

test("uses whole-film node identity when a partition omits earlier scenes", () => {
  const browser = new FakeDocument();
  const later = authoredScene("later", "later-shot", "Later");
  browser.authored.film.append(later.scene);
  const bindings = bindingsFor(browser);

  bindings.bindFilm(PLAN.film);
  const scene = bindings.bindScene({
    node: { nodeId: 5, authoredId: "later" },
    interval: { start: 60, end: 120 },
  });
  const shot = bindings.bindShot({
    node: { nodeId: 6, authoredId: "later-shot" },
    sceneId: 5,
    interval: { start: 60, end: 120 },
  });
  const overlay = bindings.bindOverlay({
    node: { nodeId: 7, authoredId: null },
    shotId: 6,
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

  const film = bindings.bindFilm(PLAN.film);

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
        assert.deepEqual(context.targets[0]?.interval, PLAN.evaluation);
        return { effects: [effect], resources: [] };
      },
    },
    videoSource: () => "unused",
  });

  bindings.bindFilm(PLAN.film);
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

  bindings.bindFilm(PLAN.film);
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

  bindings.bindFilm(PLAN.film);
  await assert.rejects(bindings.bindExtensions(PLAN), AggregateError);
  assert.deepEqual(released, ["effect", "resource"]);
});

// ── Fixture ──

const PLAN: RuntimePlan = {
  timelineVersion: 1,
  frameRate: { numerator: 30, denominator: 1 },
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
  videos: [
    {
      node: { nodeId: 3, authoredId: null },
      shotId: 2,
      assetId:
        "sha256:0101010101010101010101010101010101010101010101010101010101010101",
      interval: { start: 0, end: 60 },
      sourceFrameRate: { numerator: 30, denominator: 1 },
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
}

class FakeElement {
  readonly children: FakeElement[] = [];
  readonly dataset: Record<string, string> = {};
  className = "";
  hidden = false;
  id = "";
  muted = false;
  parent: FakeElement | undefined;
  playsInline = false;
  removed = false;
  textContent: string | null = null;

  constructor(readonly localName: string) {}

  get tagName(): string {
    return this.localName.toUpperCase();
  }

  append(...elements: FakeElement[]): void {
    for (const element of elements) {
      element.parent = this;
      this.children.push(element);
    }
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
