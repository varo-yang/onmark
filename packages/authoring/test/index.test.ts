// Public DOM-binding behavior over a controllable browser-document capability.

import assert from "node:assert/strict";
import test from "node:test";

import {
  PRESENTATION_CLASSES,
  createDomPresentationBindings,
} from "../src/index.js";
import type {
  FrameEffect,
  RuntimeOverlay,
  RuntimePlan,
  RuntimeVideo,
} from "@onmark/runtime/types";

test("binds video and overlay facts into semantic presentation nodes", () => {
  const browser = new FakeDocument();
  const bindings = createDomPresentationBindings({
    document: asBrowserDocument(browser),
    videoSource: ({ assetId }) => `./assets/${assetId}`,
  });

  const video = bindings.bindVideo(VIDEO, 3);
  const overlay = bindings.bindOverlay(OVERLAY, 4);
  const videoNode = nodeAt(browser, 0);
  const overlayNode = nodeAt(browser, 1);

  assert.equal(videoNode.className, PRESENTATION_CLASSES.video);
  assert.deepEqual(videoNode.dataset, { onmarkPlacement: "3" });
  assert.equal(videoNode.hidden, true);
  assert.equal(videoNode.muted, true);
  assert.equal(videoNode.playsInline, true);
  assert.equal(videoNode.tagName, "video");
  assert.equal(video.source, `./assets/${VIDEO.assetId}`);
  assert.equal(
    overlayNode.className,
    `${PRESENTATION_CLASSES.overlay} ${PRESENTATION_CLASSES.title}`,
  );
  assert.deepEqual(overlayNode.dataset, { onmarkPlacement: "4" });
  assert.equal(overlayNode.hidden, true);
  assert.equal(overlayNode.tagName, "div");
  assert.equal(overlayNode.textContent, "Opening");

  video.setVisible(true);
  overlay.setVisible(true);
  assert.equal(
    browser.nodes.every(({ hidden }) => !hidden),
    true,
  );

  video.dispose();
  overlay.dispose();
  assert.equal(
    browser.nodes.every(({ removed }) => removed),
    true,
  );
});

test("maps every overlay role to its stable semantic class", () => {
  const browser = new FakeDocument();
  const bindings = createDomPresentationBindings({
    document: asBrowserDocument(browser),
    videoSource: () => "unused",
  });

  bindings.bindOverlay({ ...OVERLAY, kind: "title" }, 0);
  bindings.bindOverlay({ ...OVERLAY, kind: "callToAction" }, 1);
  bindings.bindOverlay({ ...OVERLAY, kind: "caption" }, 2);

  assert.deepEqual(
    browser.nodes.map(({ className }) => className),
    [
      `${PRESENTATION_CLASSES.overlay} ${PRESENTATION_CLASSES.title}`,
      `${PRESENTATION_CLASSES.overlay} ${PRESENTATION_CLASSES.callToAction}`,
      `${PRESENTATION_CLASSES.overlay} ${PRESENTATION_CLASSES.caption}`,
    ],
  );
});

test("binds one immutable frame-effect collection to the loaded plan", () => {
  const browser = new FakeDocument();
  const effect: FrameEffect = {
    apply(): void {},
    dispose(): void {},
  };
  let received: RuntimePlan | undefined;
  const bindings = createDomPresentationBindings({
    document: asBrowserDocument(browser),
    frameEffects(plan) {
      received = plan;
      return [effect];
    },
    videoSource: () => "unused",
  });

  const effects = bindings.bindFrameEffects(PLAN);

  assert.equal(received, PLAN);
  assert.deepEqual(effects, [effect]);
  assert.equal(Object.isFrozen(effects), true);
});

const PLAN: RuntimePlan = {
  timelineVersion: 1,
  frameRate: { numerator: 30, denominator: 1 },
  evaluation: { start: 0, end: 1 },
  output: { start: 0, end: 1 },
  videos: [],
  overlays: [],
};

const VIDEO: RuntimeVideo = {
  assetId:
    "sha256:0101010101010101010101010101010101010101010101010101010101010101",
  interval: { start: 0, end: 1 },
  sourceFrameRate: { numerator: 30, denominator: 1 },
};

const OVERLAY: RuntimeOverlay = {
  kind: "title",
  text: "Opening",
  interval: { start: 0, end: 1 },
};

class FakeDocument {
  readonly nodes: FakeElement[] = [];
  readonly body = {
    append: (element: FakeElement): void => {
      this.nodes.push(element);
    },
  };

  createElement(tagName: string): FakeElement {
    return new FakeElement(tagName);
  }
}

class FakeElement {
  className = "";
  readonly dataset: Record<string, string> = {};
  hidden = false;
  muted = false;
  playsInline = false;
  removed = false;
  textContent: string | null = null;

  constructor(readonly tagName: string) {}

  remove(): void {
    this.removed = true;
  }
}

function asBrowserDocument(document: FakeDocument): Document {
  // The fake deliberately implements only the DOM capabilities the public
  // authoring facade consumes.
  return document as unknown as Document;
}

function nodeAt(document: FakeDocument, index: number): FakeElement {
  const node = document.nodes[index];
  assert.ok(node);
  return node;
}
