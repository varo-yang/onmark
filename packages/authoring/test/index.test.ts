// Public semantic DOM behavior over a deliberately small browser fake.

import assert from "node:assert/strict";
import test from "node:test";

import {
  PRESENTATION_CLASSES,
  createDomPresentationBindings,
  combineMotion,
} from "../src/index.js";
import type {
  FrameEffect,
  PresentationResource,
  RuntimePlan,
} from "@onmark/runtime/types";

// ── Semantic projection ──

test("projects solved structure into nested semantic nodes", () => {
  const browser = new FakeDocument();
  const bindings = bindingsFor(browser);

  const film = bindings.bindFilm(PLAN.film);
  const scene = bindings.bindScene(PLAN.scenes[0]!);
  const shot = bindings.bindShot(PLAN.shots[0]!);
  const video = bindings.bindVideo(PLAN.videos[0]!);
  const overlay = bindings.bindOverlay(PLAN.overlays[0]!);

  assert.equal(browser.body.children[0], film.element);
  assert.deepEqual(tags(browser.body), [
    "main",
    "section",
    "article",
    "video",
    "h1",
  ]);
  assert.equal(film.element.className, PRESENTATION_CLASSES.film);
  assert.equal(scene.element.className, PRESENTATION_CLASSES.scene);
  assert.equal(shot.element.className, PRESENTATION_CLASSES.shot);
  assert.equal(
    (video.element as unknown as FakeElement).className,
    PRESENTATION_CLASSES.video,
  );
  assert.equal(
    overlay.element.className,
    `${PRESENTATION_CLASSES.overlay} ${PRESENTATION_CLASSES.title}`,
  );
  assert.equal(shot.element.id, "hero");
  assert.deepEqual(shot.element.dataset, {
    onmarkId: "hero",
    onmarkNode: "2",
  });
  assert.equal(overlay.element.textContent, "Opening");
  assert.equal(video.source, `./assets/${PLAN.videos[0]!.assetId}`);

  film.setVisible(true);
  scene.setVisible(true);
  shot.setVisible(true);
  video.setVisible(true);
  overlay.setVisible(true);
  assert.equal(
    browser.created.every(({ hidden }) => !hidden),
    true,
  );

  overlay.dispose();
  video.dispose();
  shot.dispose();
  scene.dispose();
  film.dispose();
  assert.equal(
    browser.created.every(({ removed }) => removed),
    true,
  );
});

test("maps every overlay role to one stable semantic element", () => {
  const browser = new FakeDocument();
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

  assert.equal(title.element.tagName, "h1");
  assert.equal(callToAction.element.tagName, "div");
  assert.equal(
    browser.body.children[0]?.children.includes(
      caption.element as unknown as FakeElement,
    ),
    true,
  );
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

class FakeDocument {
  readonly body = new FakeElement("body");
  readonly created: FakeElement[] = [];

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

  constructor(readonly tagName: string) {}

  append(element: FakeElement): void {
    element.parent = this;
    this.children.push(element);
  }

  remove(): void {
    this.removed = true;
  }
}

function asBrowserDocument(document: FakeDocument): Document {
  return document as unknown as Document;
}

function tags(root: FakeElement): string[] {
  const result: string[] = [];
  for (const child of root.children) {
    result.push(child.tagName, ...tags(child));
  }
  return result;
}
