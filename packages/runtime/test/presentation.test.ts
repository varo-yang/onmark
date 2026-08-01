// Presentation-adapter behavior across decoded video and solved overlays.

import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_BROWSER_VISUAL_FINDINGS,
  MAX_PRESENTATION_EFFECTS,
  PresentationRuntimeAdapter,
  RuntimeAdapterError,
  runtimeFrameAt,
  type BrowserPlan,
  type BrowserVideo,
  type FrameEffect,
  type PresentationBindings,
} from "../src/index.js";
import { FakeVideoElement } from "./fake-video-element.js";

// ── Presentation lifecycle ──

test("presents videos and overlays on their Rust-owned intervals", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  const plan = presentationPlan();

  await adapter.load(plan);
  assert.deepEqual(
    recorder.videos.map(({ element }) => element.src),
    plan.videos.map(
      ({ assetId }) => `./assets/${assetId.slice("sha256:".length)}`,
    ),
  );
  assert.deepEqual(
    recorder.overlays.map(({ nodeId, kind, text }) => ({
      nodeId,
      kind,
      text,
    })),
    [
      { nodeId: 4, kind: "title", text: "Opening" },
      { nodeId: 9, kind: "callToAction", text: "Buy now" },
    ],
  );

  await adapter.prepare(runtimeFrameAt(10, plan.frameRate));
  const firstFrame = adapter.seek(runtimeFrameAt(10, plan.frameRate));
  recorder.videos[0]?.element.emit("seeked");
  await firstFrame;
  const firstConfirmation = adapter.confirm(runtimeFrameAt(10, plan.frameRate));
  recorder.videos[0]?.element.present(0);
  await firstConfirmation;
  assert.deepEqual(recorder.visibility(), {
    videos: [true, false],
    overlays: [true, false],
  });

  const secondFrame = adapter.seek(runtimeFrameAt(20, plan.frameRate));
  recorder.videos[1]?.element.emit("seeked");
  await secondFrame;
  const secondConfirmation = adapter.confirm(
    runtimeFrameAt(20, plan.frameRate),
  );
  recorder.videos[1]?.element.present(0);
  await secondConfirmation;
  assert.deepEqual(recorder.visibility(), {
    videos: [false, true],
    overlays: [true, true],
  });

  await adapter.dispose();
  assert.equal(recorder.allDisposed(), true);
});

test("measures layout-only videos without decoding their media", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  const plan = presentationPlan();

  await adapter.load(plan, "layoutOnly");
  const layout = await adapter.prepare(runtimeFrameAt(10, plan.frameRate));

  assert.deepEqual(layout, [
    {
      nodeId: 3,
      objectFit: "cover",
      objectPosition: { x: 500_000, y: 500_000 },
      rectangle: { x: 0, y: 0, width: 16, height: 9 },
    },
    {
      nodeId: 8,
      objectFit: "cover",
      objectPosition: { x: 500_000, y: 500_000 },
      rectangle: { x: 0, y: 0, width: 16, height: 9 },
    },
  ]);
  assert.deepEqual(
    recorder.videos.map(({ element }) => element.hasSource),
    [false, false],
  );
  assert.deepEqual(
    recorder.videos.map(({ layoutVisible }) => layoutVisible),
    [false, false],
  );

  await adapter.dispose();
  assert.equal(recorder.allDisposed(), true);
});

test("reports authored layout failures through the typed runtime boundary", async () => {
  const recorder = new PresentationRecorder();
  recorder.rejectLayoutAt(1);
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  const plan = presentationPlan();

  await adapter.load(plan, "layoutOnly");
  await assert.rejects(
    adapter.prepare(runtimeFrameAt(10, plan.frameRate)),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "video layout is outside the admitted CSS subset",
  );
  assert.deepEqual(
    recorder.videos.map(({ layoutVisible }) => layoutVisible),
    [false, false],
  );

  await adapter.dispose();
  assert.equal(recorder.allDisposed(), true);
});

test("presents both adjacent shots throughout a transition overlap", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  const baseline = presentationPlan();
  const plan: BrowserPlan = {
    ...baseline,
    timeline: { start: 0, end: 105 },
    evaluation: { start: 0, end: 105 },
    output: { start: 0, end: 105 },
    scenes: [{ ...baseline.scenes[0]!, interval: { start: 0, end: 105 } }],
    shots: [
      { ...baseline.shots[0]!, interval: { start: 0, end: 60 } },
      {
        interval: { start: 45, end: 105 },
        node: { authoredId: "after", nodeId: 4 },
        sceneId: 1,
      },
    ],
    transitions: [
      {
        incomingShotId: 4,
        interval: { start: 45, end: 60 },
        node: { authoredId: "reveal", nodeId: 3 },
        outgoingShotId: 2,
      },
    ],
    videos: [],
    overlays: [],
  };

  await adapter.load(plan);
  await adapter.prepare(runtimeFrameAt(0, plan.frameRate));
  const before = runtimeFrameAt(30, plan.frameRate);
  await adapter.seek(before);
  await adapter.confirm(before);
  assert.deepEqual(recorder.shotVisibility(), [true, false]);

  const overlap = runtimeFrameAt(45, plan.frameRate);
  await adapter.seek(overlap);
  await adapter.confirm(overlap);
  assert.deepEqual(recorder.shotVisibility(), [true, true]);

  const after = runtimeFrameAt(60, plan.frameRate);
  await adapter.seek(after);
  await adapter.confirm(after);
  assert.deepEqual(recorder.shotVisibility(), [false, true]);
  await adapter.dispose();
  assert.equal(recorder.allDisposed(), true);
});

test("loads independent videos concurrently", async () => {
  const elements: FakeVideoElement[] = [];
  const adapter = new PresentationRuntimeAdapter(videoBindings(elements), 100);
  const plan = { ...presentationPlan(), overlays: [] };

  const loading = adapter.load(plan);
  await Promise.resolve();
  await Promise.resolve();
  const started = elements.map(({ hasSource }) => hasSource);
  for (const element of elements) {
    element.emit("loadeddata");
  }
  await nextTurn();
  for (const element of elements) {
    element.emit("loadeddata");
  }
  await loading;

  assert.deepEqual(started, [true, true]);
  await adapter.dispose();
});

test("bounds concurrent browser video work", async () => {
  const elements: FakeVideoElement[] = [];
  const adapter = new PresentationRuntimeAdapter(videoBindings(elements), 100);
  const baseline = presentationPlan();
  const plan = {
    ...baseline,
    overlays: [],
    videos: Array.from({ length: 5 }, (_, index) =>
      video(index + 1, index + 3, 10, 20),
    ),
  };

  const loading = adapter.load(plan);
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(elements.filter(({ hasSource }) => hasSource).length, 4);

  for (const element of elements) {
    element.emit("loadeddata");
  }
  await nextTurn();
  assert.equal(elements.filter(({ hasSource }) => hasSource).length, 5);
  for (const element of elements) {
    element.emit("loadeddata");
  }
  await loading;
  await adapter.dispose();
});

test("reports concurrent video failures in authored order", async () => {
  const elements: FakeVideoElement[] = [];
  const adapter = new PresentationRuntimeAdapter(videoBindings(elements), 100);
  const plan = { ...presentationPlan(), overlays: [] };

  const loading = adapter.load(plan);
  const rejected = assert.rejects(
    loading,
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "video data failed to load",
  );
  await Promise.resolve();
  await Promise.resolve();
  const second = elements[1];
  const first = elements[0];
  assert.ok(first);
  assert.ok(second);
  second.releaseError = new Error("second video release failed");
  second.emit("error");
  await Promise.resolve();
  first.emit("error");

  await rejected;
});

test("seeks simultaneously visible videos concurrently", async () => {
  const elements: FakeVideoElement[] = [];
  const adapter = new PresentationRuntimeAdapter(
    videoBindings(elements, true),
    100,
  );
  const baseline = presentationPlan();
  const plan = {
    ...baseline,
    overlays: [],
    videos: baseline.videos.map((placement) => ({
      ...placement,
      interval: { start: 10, end: 20 },
    })),
  };

  await adapter.load(plan);
  const frame = runtimeFrameAt(10, plan.frameRate);
  const seeking = adapter.seek(frame);
  await Promise.resolve();
  const started = elements.map(({ seekCount }) => seekCount);
  for (const element of elements) {
    element.emit("seeked");
  }
  await nextTurn();
  for (const element of elements) {
    element.emit("seeked");
  }
  await seeking;

  assert.deepEqual(started, [1, 1]);
  for (const element of elements) {
    element.present(0);
  }
  await adapter.confirm(frame);
  await adapter.dispose();
});

test("applies frame effects at each exact authored frame", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  const plan = { ...presentationPlan(), videos: [] };

  await adapter.load(plan);
  await adapter.prepare(runtimeFrameAt(10, plan.frameRate));
  for (const index of [17, 12, 17]) {
    const frame = runtimeFrameAt(index, plan.frameRate);
    await adapter.seek(frame);
    await adapter.confirm(frame);
  }

  assert.deepEqual(recorder.effects[0]?.appliedFrames, [17, 12, 17]);
  await adapter.dispose();
  assert.equal(recorder.allDisposed(), true);
});

test("owns frame-effect behavior before author objects can mutate", async () => {
  const applied: string[] = [];
  const effect: FrameEffect = {
    apply(): void {
      applied.push("owned");
    },
    dispose(): void {
      applied.push("disposed");
    },
  };
  const bindings = emptyBindings([effect]);
  const adapter = new PresentationRuntimeAdapter(bindings, 100);
  const plan = { ...presentationPlan(), videos: [], overlays: [] };

  await adapter.load(plan);
  effect.apply = () => {
    applied.push("mutated");
  };
  effect.dispose = () => {
    applied.push("mutated-disposal");
  };
  await adapter.prepare(runtimeFrameAt(10, plan.frameRate));
  const frame = runtimeFrameAt(10, plan.frameRate);
  await adapter.seek(frame);
  await adapter.confirm(frame);
  await adapter.dispose();

  assert.deepEqual(applied, ["owned", "disposed"]);
});

test("releases every bound effect after one cleanup failure", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  await adapter.load(presentationPlan());
  recorder.videos[0]?.rejectVisibility();

  await assert.rejects(adapter.dispose(), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
});

test("releases frame effects in reverse ownership order", async () => {
  const released: number[] = [];
  const effects = [1, 2, 3].map((identity): FrameEffect => ({
    apply(): void {},
    dispose(): void {
      released.push(identity);
    },
  }));
  const adapter = new PresentationRuntimeAdapter(emptyBindings(effects), 100);
  const plan = { ...presentationPlan(), videos: [], overlays: [] };

  await adapter.load(plan);
  await adapter.dispose();

  assert.deepEqual(released, [3, 2, 1]);
});

test("bounds retained frame effects and releases the rejected collection", async () => {
  let disposed = 0;
  let resourceDisposed = false;
  const effects = Array.from(
    { length: MAX_PRESENTATION_EFFECTS + 1 },
    (): FrameEffect => ({
      apply(): void {},
      dispose(): void {
        disposed += 1;
      },
    }),
  );
  const bindings: PresentationBindings = {
    ...emptyBindings(effects),
    async bindExtensions() {
      return {
        effects,
        resources: [
          {
            id: "owned-resource",
            kind: "custom",
            prepare(): void {},
            dispose(): void {
              resourceDisposed = true;
            },
          },
        ],
      };
    },
  };
  const adapter = new PresentationRuntimeAdapter(bindings, 100);
  const plan = { ...presentationPlan(), videos: [], overlays: [] };

  await assert.rejects(
    adapter.load(plan),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "presentation frame-effect count exceeds its limit",
  );

  assert.equal(disposed, effects.length);
  assert.equal(resourceDisposed, true);
});

test("releases videos and overlays after frame-effect cleanup fails", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  await adapter.load(presentationPlan());
  recorder.rejectEffectCleanup();

  await assert.rejects(adapter.dispose(), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
});

test("releases every structural container after one cleanup failure", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  await adapter.load(presentationPlan());
  recorder.rejectContainerCleanupAt(2);

  await assert.rejects(adapter.dispose(), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
});

test("releases earlier browser nodes when later binding fails", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  recorder.rejectOverlayBindingAt(9);

  await assert.rejects(adapter.load(presentationPlan()), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
  await assert.rejects(
    adapter.load(presentationPlan()),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "presentation load requires the empty state",
  );
});

test("releases owned effects when extension resources are invalid", async () => {
  let effectDisposed = false;
  const duplicate = {
    id: "hero-font",
    kind: "font" as const,
    prepare(): void {},
    dispose(): void {},
  };
  const bindings: PresentationBindings = {
    ...emptyBindings([]),
    async bindExtensions() {
      return {
        effects: [
          {
            apply(): void {},
            dispose(): void {
              effectDisposed = true;
            },
          },
        ],
        resources: [duplicate, duplicate],
      };
    },
  };
  const adapter = new PresentationRuntimeAdapter(bindings, 100);
  const plan = { ...presentationPlan(), videos: [], overlays: [] };

  await assert.rejects(
    adapter.load(plan),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "presentation resource identity is duplicated",
  );

  assert.equal(effectDisposed, true);
});

test("reports incomplete cleanup after presentation loading fails", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  recorder.rejectVideoCleanupAt(0);
  recorder.rejectOverlayBindingAt(9);

  await assert.rejects(
    adapter.load(presentationPlan()),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "presentation load failed and cleanup was incomplete",
  );
  assert.equal(recorder.allDisposed(), true);
  await assert.rejects(
    adapter.load(presentationPlan()),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "presentation load requires the empty state",
  );
});

test("rejects an invalid readiness policy before binding browser effects", () => {
  const recorder = new PresentationRecorder();

  assert.throws(
    () => new PresentationRuntimeAdapter(recorder.bindings, 0),
    TypeError,
  );
  assert.deepEqual(recorder.videos, []);
  assert.deepEqual(recorder.overlays, []);
});

test("reports bounded layout defects for active semantic elements", async () => {
  const recorder = new PresentationRecorder();
  const plan = presentationPlan();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  await adapter.load(plan, "omitted");
  await adapter.prepare(runtimeFrameAt(10, plan.frameRate));
  recorder.setShotLayout(0, { height: 0, width: 0 });
  recorder.setOverlayLayout(4, { height: 0, width: 0 });
  recorder.setOverlayLayout(9, {
    clientHeight: 20,
    clientWidth: 40,
    overflowX: "hidden",
    overflowY: "clip",
    scrollHeight: 30,
    scrollWidth: 50,
  });

  const frame = runtimeFrameAt(25, plan.frameRate);
  await adapter.seek(frame);
  assert.deepEqual(await adapter.confirm(frame), [
    { nodeId: 2, issue: "emptyBox" },
    { nodeId: 4, issue: "emptyBox" },
    { nodeId: 9, issue: "clippedHorizontally" },
    { nodeId: 9, issue: "clippedVertically" },
  ]);
  await adapter.dispose();
});

test("retains the canonical visual prefix without failing the frame", async () => {
  const recorder = new PresentationRecorder();
  const baseline = presentationPlan();
  const overlays = Array.from(
    { length: MAX_BROWSER_VISUAL_FINDINGS + 1 },
    (_, index) => ({
      node: { nodeId: index + 3, authoredId: `overlay-${index}` },
      shotId: 2,
      kind: "title" as const,
      text: `Overlay ${index}`,
      interval: { start: 10, end: 30 },
    }),
  );
  const plan: BrowserPlan = { ...baseline, videos: [], overlays };
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  await adapter.load(plan, "omitted");
  await adapter.prepare(runtimeFrameAt(10, plan.frameRate));
  for (const overlay of recorder.overlays) {
    overlay.layout.layout.height = 0;
  }

  const frame = runtimeFrameAt(10, plan.frameRate);
  await adapter.seek(frame);
  const findings = await adapter.confirm(frame);

  assert.equal(findings.length, MAX_BROWSER_VISUAL_FINDINGS);
  assert.deepEqual(findings[0], { nodeId: 3, issue: "emptyBox" });
  assert.deepEqual(findings.at(-1), {
    nodeId: MAX_BROWSER_VISUAL_FINDINGS + 2,
    issue: "emptyBox",
  });
  await adapter.dispose();
});

// ── Test presentation boundary ──

interface RecordedVideo {
  readonly element: FakeVideoElement;
  readonly index: number;
  disposed: boolean;
  layoutVisible: boolean;
  visible: boolean;
  rejectVisibility(): void;
}

interface RecordedOverlay {
  readonly nodeId: number;
  readonly kind: "callToAction" | "caption" | "title";
  readonly text: string;
  readonly layout: TestLayoutElement;
  disposed: boolean;
  visible: boolean;
}

interface RecordedFrameEffect {
  readonly appliedFrames: number[];
  disposed: boolean;
}

interface RecordedContainer {
  readonly layout: TestLayoutElement;
  disposed: boolean;
  visible: boolean;
}

interface RecordedTransition {
  disposed: boolean;
}

interface TestLayout {
  height: number;
  width: number;
  clientHeight: number;
  clientWidth: number;
  overflowX: string;
  overflowY: string;
  scrollHeight: number;
  scrollWidth: number;
}

interface TestLayoutElement {
  readonly element: HTMLElement;
  readonly layout: TestLayout;
}

function testLayoutElement(): TestLayoutElement {
  const layout: TestLayout = {
    height: 9,
    width: 16,
    clientHeight: 9,
    clientWidth: 16,
    overflowX: "visible",
    overflowY: "visible",
    scrollHeight: 9,
    scrollWidth: 16,
  };
  const element = {
    get clientHeight(): number {
      return layout.clientHeight;
    },
    get clientWidth(): number {
      return layout.clientWidth;
    },
    get scrollHeight(): number {
      return layout.scrollHeight;
    },
    get scrollWidth(): number {
      return layout.scrollWidth;
    },
    getBoundingClientRect(): Pick<DOMRect, "height" | "width"> {
      return { height: layout.height, width: layout.width };
    },
    ownerDocument: {
      defaultView: {
        getComputedStyle: () => ({
          overflowX: layout.overflowX,
          overflowY: layout.overflowY,
        }),
      },
    },
  } as unknown as HTMLElement;
  return { element, layout };
}

class PresentationRecorder {
  readonly containers: RecordedContainer[] = [];
  readonly effects: RecordedFrameEffect[] = [];
  readonly overlays: RecordedOverlay[] = [];
  readonly transitions: RecordedTransition[] = [];
  readonly videos: RecordedVideo[] = [];
  #rejectEffectCleanup = false;
  #rejectedContainerCleanupIndex: number | undefined;
  #rejectedLayoutIndex: number | undefined;
  #rejectedOverlayNodeId: number | undefined;
  #rejectedVideoCleanupIndex: number | undefined;

  readonly bindings: PresentationBindings = {
    bindFilm: () => this.#bindContainer(),
    bindScene: () => this.#bindContainer(),
    bindShot: () => this.#bindContainer(),
    bindTransition: () => {
      const recorded: RecordedTransition = { disposed: false };
      this.transitions.push(recorded);
      return {
        dispose(): void {
          recorded.disposed = true;
        },
        element: {} as HTMLElement,
        incomingElement: {} as HTMLElement,
        outgoingElement: {} as HTMLElement,
      };
    },
    bindVideo: (placement) => {
      const index = this.videos.length;
      const element = new FakeVideoElement(true);
      const rejectCleanup = index === this.#rejectedVideoCleanupIndex;
      const rejectLayout = index === this.#rejectedLayoutIndex;
      let visibilityError: Error | undefined;
      let visibilityCalls = 0;
      const recorded: RecordedVideo = {
        element,
        index,
        disposed: false,
        layoutVisible: false,
        visible: false,
        rejectVisibility(): void {
          visibilityError = new Error("video visibility failed");
        },
      };
      this.videos.push(recorded);
      return {
        element,
        source: `./assets/${placement.assetId.slice("sha256:".length)}`,
        setLayoutVisible(visible): void {
          recorded.layoutVisible = visible;
        },
        measureLayout() {
          if (rejectLayout) {
            throw new Error("video layout is outside the admitted CSS subset");
          }
          return {
            nodeId: placement.node.nodeId,
            objectFit: "cover" as const,
            objectPosition: { x: 500_000, y: 500_000 },
            rectangle: { x: 0, y: 0, width: 16, height: 9 },
          };
        },
        setVisible(visible): void {
          visibilityCalls += 1;
          if (rejectCleanup && visibilityCalls > 1) {
            throw new Error("video cleanup failed");
          }
          if (visibilityError !== undefined) {
            throw visibilityError;
          }
          recorded.visible = visible;
        },
        dispose(): void {
          recorded.disposed = true;
        },
      };
    },
    bindOverlay: (placement) => {
      if (placement.node.nodeId === this.#rejectedOverlayNodeId) {
        throw new Error("overlay binding failed");
      }
      const recorded: RecordedOverlay = {
        nodeId: placement.node.nodeId,
        kind: placement.kind,
        text: placement.text,
        layout: testLayoutElement(),
        disposed: false,
        visible: false,
      };
      this.overlays.push(recorded);
      return {
        element: recorded.layout.element,
        setVisible(visible): void {
          recorded.visible = visible;
        },
        dispose(): void {
          recorded.disposed = true;
        },
      };
    },
    bindExtensions: async () => {
      const recorded: RecordedFrameEffect = {
        appliedFrames: [],
        disposed: false,
      };
      this.effects.push(recorded);
      return {
        effects: [
          {
            async apply(frame): Promise<void> {
              await Promise.resolve();
              recorded.appliedFrames.push(frame.index);
            },
            dispose: async (): Promise<void> => {
              recorded.disposed = true;
              if (this.#rejectEffectCleanup) {
                throw new Error("frame-effect cleanup failed");
              }
            },
          },
        ],
        resources: [],
      };
    },
  };

  rejectEffectCleanup(): void {
    this.#rejectEffectCleanup = true;
  }

  rejectContainerCleanupAt(index: number): void {
    this.#rejectedContainerCleanupIndex = index;
  }

  rejectLayoutAt(index: number): void {
    this.#rejectedLayoutIndex = index;
  }

  rejectOverlayBindingAt(nodeId: number): void {
    this.#rejectedOverlayNodeId = nodeId;
  }

  rejectVideoCleanupAt(index: number): void {
    this.#rejectedVideoCleanupIndex = index;
  }

  setOverlayLayout(nodeId: number, layout: Partial<TestLayout>): void {
    const overlay = this.overlays.find(
      (candidate) => candidate.nodeId === nodeId,
    );
    assert.ok(overlay);
    Object.assign(overlay.layout.layout, layout);
  }

  setShotLayout(index: number, layout: Partial<TestLayout>): void {
    const shot = this.containers.at(index + 2);
    assert.ok(shot);
    Object.assign(shot.layout.layout, layout);
  }

  visibility(): { videos: boolean[]; overlays: boolean[] } {
    return {
      videos: this.videos.map(({ visible }) => visible),
      overlays: this.overlays.map(({ visible }) => visible),
    };
  }

  shotVisibility(): boolean[] {
    return this.containers.slice(2).map(({ visible }) => visible);
  }

  allDisposed(): boolean {
    return (
      this.videos.every(
        ({ disposed, element }) => disposed && !element.hasSource,
      ) &&
      this.overlays.every(({ disposed }) => disposed) &&
      this.transitions.every(({ disposed }) => disposed) &&
      this.effects.every(({ disposed }) => disposed) &&
      this.containers.every(({ disposed }) => disposed)
    );
  }

  #bindContainer() {
    const index = this.containers.length;
    const recorded: RecordedContainer = {
      layout: testLayoutElement(),
      disposed: false,
      visible: false,
    };
    this.containers.push(recorded);
    return {
      element: recorded.layout.element,
      setVisible: (visible: boolean): void => {
        if (!visible && index === this.#rejectedContainerCleanupIndex) {
          throw new Error("container cleanup failed");
        }
        recorded.visible = visible;
      },
      dispose(): void {
        recorded.disposed = true;
      },
    };
  }
}

function presentationPlan(): BrowserPlan {
  return {
    timelineVersion: 7,
    frameRate: { numerator: 30, denominator: 1 },
    timeline: { start: 0, end: 40 },
    evaluation: { start: 10, end: 30 },
    output: { start: 10, end: 30 },
    film: { nodeId: 0, authoredId: "film" },
    scenes: [
      {
        node: { nodeId: 1, authoredId: "scene" },
        interval: { start: 10, end: 30 },
      },
    ],
    shots: [
      {
        node: { nodeId: 2, authoredId: "shot" },
        sceneId: 1,
        interval: { start: 10, end: 30 },
      },
    ],
    transitions: [],
    variantFields: [],
    videos: [video(1, 3, 10, 20), video(2, 8, 20, 30)],
    overlays: [
      {
        node: { nodeId: 4, authoredId: "opening" },
        shotId: 2,
        kind: "title",
        text: "Opening",
        interval: { start: 10, end: 30 },
      },
      {
        node: { nodeId: 9, authoredId: "cta" },
        shotId: 2,
        kind: "callToAction",
        text: "Buy now",
        interval: { start: 20, end: 30 },
      },
    ],
  };
}

function emptyBindings(effects: readonly FrameEffect[]): PresentationBindings {
  return {
    bindFilm() {
      return emptyContainer();
    },
    bindScene() {
      return emptyContainer();
    },
    bindShot() {
      return emptyContainer();
    },
    bindTransition(): never {
      throw new Error("the empty fixture contains no transition");
    },
    bindVideo(): never {
      throw new Error("the empty fixture contains no video");
    },
    bindOverlay(): never {
      throw new Error("the empty fixture contains no overlay");
    },
    async bindExtensions() {
      return { effects, resources: [] };
    },
  };
}

function videoBindings(
  elements: FakeVideoElement[],
  loadAutomatically = false,
): PresentationBindings {
  return {
    ...emptyBindings([]),
    bindVideo(placement) {
      const element = new FakeVideoElement(loadAutomatically);
      elements.push(element);
      return {
        element,
        source: `./assets/${placement.assetId.slice("sha256:".length)}`,
        setLayoutVisible(): void {},
        measureLayout() {
          return {
            nodeId: placement.node.nodeId,
            objectFit: "cover" as const,
            objectPosition: { x: 500_000, y: 500_000 },
            rectangle: { x: 0, y: 0, width: 16, height: 9 },
          };
        },
        setVisible(): void {},
        dispose(): void {},
      };
    },
  };
}

function nextTurn(): Promise<void> {
  return new Promise((resolve) => {
    setImmediate(resolve);
  });
}

function video(
  digestByte: number,
  nodeId: number,
  startFrame: number,
  endFrame: number,
): BrowserVideo {
  return {
    node: { nodeId, authoredId: null },
    shotId: 2,
    assetId: `sha256:${digestByte.toString().padStart(2, "0").repeat(32)}`,
    interval: { start: startFrame, end: endFrame },
    sourceTiming: {
      kind: "constant",
      frameRate: { numerator: 30, denominator: 1 },
    },
    source: {
      startNanoseconds: "0",
      endNanoseconds: "333333333",
      naturalEndNanoseconds: "333333333",
      playbackRate: { numerator: 1, denominator: 1 },
      plays: 1,
      holdLastNanoseconds: "0",
    },
  };
}

function emptyContainer() {
  const layout = testLayoutElement();
  return {
    element: layout.element,
    setVisible(): void {},
    dispose(): void {},
  };
}
