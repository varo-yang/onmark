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
    recorder.overlays.map(({ index, kind, text }) => ({ index, kind, text })),
    [
      { index: 0, kind: "title", text: "Opening" },
      { index: 1, kind: "callToAction", text: "Buy now" },
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

test("releases every bound effect after one cleanup failure", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  await adapter.load(presentationPlan());
  recorder.videos[0]?.rejectVisibility();

  await assert.rejects(adapter.dispose(), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
});

test("releases earlier effects when a later binding fails", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  recorder.rejectOverlayBindingAt(1);

  await assert.rejects(adapter.load(presentationPlan()), RuntimeAdapterError);

  assert.equal(recorder.allDisposed(), true);
});

test("reports incomplete cleanup after presentation loading fails", async () => {
  const recorder = new PresentationRecorder();
  const adapter = new PresentationRuntimeAdapter(recorder.bindings, 100);
  recorder.rejectVideoCleanupAt(0);
  recorder.rejectOverlayBindingAt(1);

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
  readonly index: number;
  readonly kind: "callToAction" | "caption" | "title";
  readonly text: string;
  disposed: boolean;
  visible: boolean;
}

class PresentationRecorder {
  readonly overlays: RecordedOverlay[] = [];
  readonly videos: RecordedVideo[] = [];
  #rejectedOverlayIndex: number | undefined;
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
    bindOverlay: (placement, index) => {
      if (index === this.#rejectedOverlayIndex) {
        throw new Error("overlay binding failed");
      }
      const recorded: RecordedOverlay = {
        index,
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
  };

  rejectOverlayBindingAt(index: number): void {
    this.#rejectedOverlayIndex = index;
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
      ) && this.overlays.every(({ disposed }) => disposed)
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
      { kind: "title", text: "Opening", interval: { start: 10, end: 30 } },
      {
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
