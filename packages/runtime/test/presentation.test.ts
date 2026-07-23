// Presentation-adapter behavior across decoded video and solved overlays.

import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_PRESENTATION_EFFECTS,
  PresentationRuntimeAdapter,
  RuntimeAdapterError,
  runtimeFrameAt,
  type BrowserPlan,
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

// ── Test presentation boundary ──

interface RecordedVideo {
  readonly element: FakeVideoElement;
  readonly index: number;
  disposed: boolean;
  visible: boolean;
  rejectVisibility(): void;
}

interface RecordedOverlay {
  readonly nodeId: number;
  readonly kind: "callToAction" | "caption" | "title";
  readonly text: string;
  disposed: boolean;
  visible: boolean;
}

interface RecordedFrameEffect {
  readonly appliedFrames: number[];
  disposed: boolean;
}

interface RecordedContainer {
  disposed: boolean;
}

class PresentationRecorder {
  readonly containers: RecordedContainer[] = [];
  readonly effects: RecordedFrameEffect[] = [];
  readonly overlays: RecordedOverlay[] = [];
  readonly videos: RecordedVideo[] = [];
  #rejectEffectCleanup = false;
  #rejectedContainerCleanupIndex: number | undefined;
  #rejectedOverlayNodeId: number | undefined;
  #rejectedVideoCleanupIndex: number | undefined;

  readonly bindings: PresentationBindings = {
    bindFilm: () => this.#bindContainer(),
    bindScene: () => this.#bindContainer(),
    bindShot: () => this.#bindContainer(),
    bindVideo: (placement) => {
      const index = this.videos.length;
      const element = new FakeVideoElement(true);
      const rejectCleanup = index === this.#rejectedVideoCleanupIndex;
      let visibilityError: Error | undefined;
      let visibilityCalls = 0;
      const recorded: RecordedVideo = {
        element,
        index,
        disposed: false,
        visible: false,
        rejectVisibility(): void {
          visibilityError = new Error("video visibility failed");
        },
      };
      this.videos.push(recorded);
      return {
        element,
        source: `./assets/${placement.assetId.slice("sha256:".length)}`,
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
        disposed: false,
        visible: false,
      };
      this.overlays.push(recorded);
      return {
        element: {} as HTMLElement,
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

  rejectOverlayBindingAt(nodeId: number): void {
    this.#rejectedOverlayNodeId = nodeId;
  }

  rejectVideoCleanupAt(index: number): void {
    this.#rejectedVideoCleanupIndex = index;
  }

  visibility(): { videos: boolean[]; overlays: boolean[] } {
    return {
      videos: this.videos.map(({ visible }) => visible),
      overlays: this.overlays.map(({ visible }) => visible),
    };
  }

  allDisposed(): boolean {
    return (
      this.videos.every(
        ({ disposed, element }) => disposed && !element.hasSource,
      ) &&
      this.overlays.every(({ disposed }) => disposed) &&
      this.effects.every(({ disposed }) => disposed) &&
      this.containers.every(({ disposed }) => disposed)
    );
  }

  #bindContainer() {
    const index = this.containers.length;
    const recorded: RecordedContainer = { disposed: false };
    this.containers.push(recorded);
    return {
      element: {} as HTMLElement,
      setVisible: (visible: boolean): void => {
        if (!visible && index === this.#rejectedContainerCleanupIndex) {
          throw new Error("container cleanup failed");
        }
      },
      dispose(): void {
        recorded.disposed = true;
      },
    };
  }
}

function presentationPlan(): BrowserPlan {
  return {
    timelineVersion: 1,
    frameRate: { numerator: 30, denominator: 1 },
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

function video(
  digestByte: number,
  nodeId: number,
  startFrame: number,
  endFrame: number,
) {
  return {
    node: { nodeId, authoredId: null },
    shotId: 2,
    assetId: `sha256:${digestByte.toString().padStart(2, "0").repeat(32)}`,
    interval: { start: startFrame, end: endFrame },
    sourceFrameRate: { numerator: 30, denominator: 1 },
  };
}

function emptyContainer() {
  return {
    element: {} as HTMLElement,
    setVisible(): void {},
    dispose(): void {},
  };
}
