// Browser-video lifecycle tests through one controllable media capability.

import assert from "node:assert/strict";
import test from "node:test";

import {
  DecodedVideo,
  RuntimeAdapterError,
  VideoRuntimeAdapter,
  materializedVideoSource,
  runtimeFrameAt,
  type BrowserPlan,
  type BrowserVideoElement,
} from "../src/index.js";

const selection = {
  mediaTimeSeconds: 2 / 30,
  seekTimeSeconds: 2.5 / 30,
};

// ── Decoded-video resource ──

test("loads and confirms one exact decoded source frame", async () => {
  const element = new FakeVideoElement();
  const video = new DecodedVideo(element, 100);

  const loading = video.load("./assets/sha256/source");
  assert.equal(element.src, "./assets/sha256/source");
  assert.equal(element.loadCount, 1);
  element.emit("loadeddata");
  await loading;

  const presenting = video.present(selection);
  assert.equal(element.currentTime, selection.seekTimeSeconds);
  element.emit("seeked");
  element.present(selection.mediaTimeSeconds);
  await presenting;
});

test("reuses an already confirmed source frame without seeking again", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element);

  const first = video.present(selection);
  element.emit("seeked");
  element.present(selection.mediaTimeSeconds);
  await first;
  const seekCount = element.seekCount;

  await video.present(selection);

  assert.equal(element.seekCount, seekCount);
  assert.equal(element.pendingFrameCallbacks, 0);
});

test("waits past unrelated decoded frames and removes every observer", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element);

  const presenting = video.present(selection);
  element.emit("seeked");
  element.present(0);
  assert.equal(element.pendingFrameCallbacks, 1);
  element.present(selection.mediaTimeSeconds);
  await presenting;

  assert.equal(element.listenerCount, 0);
  assert.equal(element.pendingFrameCallbacks, 0);
});

test("reports bounded readiness failures and cleans the pending frame wait", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element, 5);

  await assert.rejects(
    video.present(selection),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.kind === "readinessTimeout" &&
      error.pendingResources.includes("video-frame"),
  );
  assert.equal(element.listenerCount, 0);
  assert.equal(element.pendingFrameCallbacks, 0);
});

test("cleans media observers after synchronous browser failures", async () => {
  const loadingElement = new FakeVideoElement();
  loadingElement.loadError = new Error("browser load failed");
  const loadingVideo = new DecodedVideo(loadingElement, 100);

  await assert.rejects(
    loadingVideo.load("./assets/sha256/source"),
    RuntimeAdapterError,
  );
  assert.equal(loadingElement.listenerCount, 0);
  assert.equal(loadingElement.hasSource, false);

  const seekingElement = new FakeVideoElement();
  const seekingVideo = await loadedVideo(seekingElement);
  seekingElement.frameCallbackError = new Error("callback unavailable");

  await assert.rejects(seekingVideo.present(selection), RuntimeAdapterError);
  assert.equal(seekingElement.listenerCount, 0);
  assert.equal(seekingElement.pendingFrameCallbacks, 0);
});

test("releases media bytes and makes disposal terminal", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element);

  video.dispose();

  assert.equal(element.hasSource, false);
  assert.equal(element.loadCount, 2);
  await assert.rejects(video.present(selection), RuntimeAdapterError);
});

test("rejects readiness deadlines outside the browser timer budget", () => {
  assert.throws(() => new DecodedVideo(new FakeVideoElement(), 0), TypeError);
  assert.throws(
    () => new DecodedVideo(new FakeVideoElement(), 86_400_001),
    TypeError,
  );
});

// ── Runtime adapter ──

test("coordinates presentation-owned videos without owning their layout", async () => {
  const presentations: RecordedPresentation[] = [];
  const adapter = new VideoRuntimeAdapter((placement, index) => {
    const element = new FakeVideoElement(true);
    const presentation = { element, index, visible: false };
    presentations.push(presentation);
    return {
      element,
      source: `./assets/${placement.assetId.slice("sha256:".length)}`,
      setVisible(visible): void {
        presentation.visible = visible;
      },
    };
  }, 100);
  const plan = videoPlan();

  await adapter.load(plan);
  assert.deepEqual(
    presentations.map(({ element }) => element.src),
    [
      `./assets/${plan.videos[0]?.assetId.slice("sha256:".length)}`,
      `./assets/${plan.videos[1]?.assetId.slice("sha256:".length)}`,
    ],
  );

  const firstFrame = adapter.prepare(runtimeFrameAt(10, plan.frameRate));
  presentations[0]?.element.emit("seeked");
  presentations[0]?.element.present(0);
  await firstFrame;
  assert.deepEqual(
    presentations.map(({ index, visible }) => ({ index, visible })),
    [
      { index: 0, visible: true },
      { index: 1, visible: false },
    ],
  );

  const secondFrame = adapter.seek(runtimeFrameAt(20, plan.frameRate));
  presentations[1]?.element.emit("seeked");
  presentations[1]?.element.present(0);
  await secondFrame;
  assert.equal(presentations[0]?.visible, false);
  assert.equal(presentations[1]?.visible, true);

  await adapter.dispose();
  assert.equal(
    presentations.every(
      ({ element, visible }) => !element.hasSource && !visible,
    ),
    true,
  );
});

test("derives the materialized source from the Rust-owned bundle layout", () => {
  const placement = videoPlan().videos[0];
  assert.ok(placement);

  assert.equal(
    materializedVideoSource(placement),
    "./assets/sha256/0101010101010101010101010101010101010101010101010101010101010101",
  );
});

test("releases every video even when presentation cleanup fails", async () => {
  const element = new FakeVideoElement(true);
  let rejectVisibility = false;
  const adapter = new VideoRuntimeAdapter(
    () => ({
      element,
      source: "./assets/sha256/source",
      setVisible(): void {
        if (rejectVisibility) {
          throw new Error("presentation cleanup failed");
        }
      },
    }),
    100,
  );

  await adapter.load(singleVideoPlan());
  rejectVisibility = true;
  await assert.rejects(adapter.dispose(), RuntimeAdapterError);

  assert.equal(element.hasSource, false);
});

// ── Test support ──

async function loadedVideo(
  element: FakeVideoElement,
  timeoutMilliseconds = 100,
): Promise<DecodedVideo> {
  const video = new DecodedVideo(element, timeoutMilliseconds);
  const loading = video.load("./assets/sha256/source");
  element.emit("loadeddata");
  await loading;
  return video;
}

type VideoEvent = "error" | "loadeddata" | "seeked";

interface RecordedPresentation {
  readonly element: FakeVideoElement;
  readonly index: number;
  visible: boolean;
}

class FakeVideoElement implements BrowserVideoElement {
  readonly #listeners = new Map<VideoEvent, Set<() => void>>();
  readonly #frameCallbacks = new Map<
    number,
    (now: number, metadata: { readonly mediaTime: number }) => void
  >();
  #nextFrameCallback = 1;
  #currentTime = 0;
  #hasSource = false;
  #src = "";
  readonly #loadAutomatically: boolean;
  frameCallbackError: Error | undefined;
  loadCount = 0;
  loadError: Error | undefined;
  seekCount = 0;

  constructor(loadAutomatically = false) {
    this.#loadAutomatically = loadAutomatically;
  }

  get currentTime(): number {
    return this.#currentTime;
  }

  set currentTime(value: number) {
    this.#currentTime = value;
    this.seekCount += 1;
  }

  get hasSource(): boolean {
    return this.#hasSource;
  }

  get listenerCount(): number {
    let count = 0;
    for (const listeners of this.#listeners.values()) {
      count += listeners.size;
    }
    return count;
  }

  get pendingFrameCallbacks(): number {
    return this.#frameCallbacks.size;
  }

  get src(): string {
    return this.#src;
  }

  set src(value: string) {
    this.#src = value;
    this.#hasSource = true;
  }

  addEventListener(type: VideoEvent, listener: () => void): void {
    const listeners = this.#listeners.get(type) ?? new Set();
    listeners.add(listener);
    this.#listeners.set(type, listeners);
  }

  cancelVideoFrameCallback(handle: number): void {
    this.#frameCallbacks.delete(handle);
  }

  load(): void {
    this.loadCount += 1;
    if (this.loadError !== undefined) {
      throw this.loadError;
    }
    if (this.#loadAutomatically && this.#hasSource) {
      queueMicrotask(() => this.emit("loadeddata"));
    }
  }

  removeAttribute(name: "src"): void {
    assert.equal(name, "src");
    this.#src = "";
    this.#hasSource = false;
  }

  removeEventListener(type: VideoEvent, listener: () => void): void {
    this.#listeners.get(type)?.delete(listener);
  }

  requestVideoFrameCallback(
    callback: (now: number, metadata: { readonly mediaTime: number }) => void,
  ): number {
    if (this.frameCallbackError !== undefined) {
      throw this.frameCallbackError;
    }
    const handle = this.#nextFrameCallback;
    this.#nextFrameCallback += 1;
    this.#frameCallbacks.set(handle, callback);
    return handle;
  }

  emit(type: VideoEvent): void {
    for (const listener of this.#listeners.get(type) ?? []) {
      listener();
    }
  }

  present(mediaTime: number): void {
    const callbacks = [...this.#frameCallbacks.values()];
    this.#frameCallbacks.clear();
    for (const callback of callbacks) {
      callback(0, { mediaTime });
    }
  }
}

function videoPlan(): BrowserPlan {
  return {
    timelineVersion: 1,
    frameRate: { numerator: 30, denominator: 1 },
    evaluation: { start: 10, end: 30 },
    output: { start: 10, end: 30 },
    videos: [
      {
        assetId:
          "sha256:0101010101010101010101010101010101010101010101010101010101010101",
        interval: { start: 10, end: 20 },
        sourceFrameRate: { numerator: 30, denominator: 1 },
      },
      {
        assetId:
          "sha256:0202020202020202020202020202020202020202020202020202020202020202",
        interval: { start: 20, end: 30 },
        sourceFrameRate: { numerator: 30, denominator: 1 },
      },
    ],
  };
}

function singleVideoPlan(): BrowserPlan {
  const plan = videoPlan();
  const video = plan.videos[0];
  assert.ok(video);
  return {
    ...plan,
    evaluation: { start: 10, end: 20 },
    output: { start: 10, end: 20 },
    videos: [video],
  };
}
