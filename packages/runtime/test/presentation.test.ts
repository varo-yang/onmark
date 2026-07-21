// Presentation-adapter behavior across decoded video and solved overlays.

import assert from "node:assert/strict";
import test from "node:test";

import {
  PresentationRuntimeAdapter,
  RuntimeAdapterError,
  runtimeFrameAt,
  type BrowserPlan,
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
    recorder.overlays.map(({ componentId, kind, text }) => ({
      componentId,
      kind,
      text,
    })),
    [
      { componentId: 4, kind: "title", text: "Opening" },
      { componentId: 9, kind: "callToAction", text: "Buy now" },
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

test("releases every bound effect after one cleanup failure", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  await adapter.load(presentationPlan());
  recorder.videos[0]?.rejectVisibility();

  await assert.rejects(adapter.dispose(), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
});

test("releases videos and overlays after frame-effect cleanup fails", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  await adapter.load(presentationPlan());
  recorder.rejectEffectCleanup();

  await assert.rejects(adapter.dispose(), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
});

test("releases earlier effects when a later binding fails", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  recorder.rejectOverlayBindingAt(9);

  await assert.rejects(adapter.load(presentationPlan()), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
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
  readonly componentId: number;
  readonly kind: "callToAction" | "caption" | "title";
  readonly text: string;
  disposed: boolean;
  visible: boolean;
}

interface RecordedFrameEffect {
  readonly appliedFrames: number[];
  disposed: boolean;
}

class PresentationRecorder {
  readonly effects: RecordedFrameEffect[] = [];
  readonly overlays: RecordedOverlay[] = [];
  readonly videos: RecordedVideo[] = [];
  #rejectEffectCleanup = false;
  #rejectedOverlayComponentId: number | undefined;
  #rejectedVideoCleanupIndex: number | undefined;

  readonly bindings: PresentationBindings = {
    bindVideo: (placement, index) => {
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
      if (placement.componentId === this.#rejectedOverlayComponentId) {
        throw new Error("overlay binding failed");
      }
      const recorded: RecordedOverlay = {
        componentId: placement.componentId,
        kind: placement.kind,
        text: placement.text,
        disposed: false,
        visible: false,
      };
      this.overlays.push(recorded);
      return {
        setVisible(visible): void {
          recorded.visible = visible;
        },
        dispose(): void {
          recorded.disposed = true;
        },
      };
    },
    bindFrameEffects: () => {
      const recorded: RecordedFrameEffect = {
        appliedFrames: [],
        disposed: false,
      };
      this.effects.push(recorded);
      return [
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
      ];
    },
    bindResources: () => [],
  };

  rejectEffectCleanup(): void {
    this.#rejectEffectCleanup = true;
  }

  rejectOverlayBindingAt(componentId: number): void {
    this.#rejectedOverlayComponentId = componentId;
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
      this.effects.every(({ disposed }) => disposed)
    );
  }
}

function presentationPlan(): BrowserPlan {
  return {
    timelineVersion: 1,
    frameRate: { numerator: 30, denominator: 1 },
    evaluation: { start: 10, end: 30 },
    output: { start: 10, end: 30 },
    videos: [video(1, 10, 20), video(2, 20, 30)],
    overlays: [
      {
        componentId: 4,
        kind: "title",
        text: "Opening",
        interval: { start: 10, end: 30 },
      },
      {
        componentId: 9,
        kind: "callToAction",
        text: "Buy now",
        interval: { start: 20, end: 30 },
      },
    ],
  };
}

function video(digestByte: number, startFrame: number, endFrame: number) {
  return {
    assetId: `sha256:${digestByte.toString().padStart(2, "0").repeat(32)}`,
    interval: { start: startFrame, end: endFrame },
    sourceFrameRate: { numerator: 30, denominator: 1 },
  };
}
