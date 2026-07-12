// Behavioral contract for the sequential browser runtime session.
// A recording adapter replaces only the not-yet-implemented browser boundary.

import assert from "node:assert/strict";
import test from "node:test";

import {
  RuntimeAdapterError,
  RuntimeSession,
  type BrowserPlan,
  type BrowserRequest,
  type BrowserResponse,
  type RuntimeAdapter,
  type RuntimeFrame,
  type RuntimePlan,
} from "../src/index.js";

const plan: BrowserPlan = {
  timelineVersion: 1,
  frameRate: { numerator: 30, denominator: 1 },
  evaluation: { start: 10, end: 20 },
  output: { start: 10, end: 20 },
  videos: [
    {
      assetId:
        "sha256:0101010101010101010101010101010101010101010101010101010101010101",
      interval: { start: 12, end: 18 },
      sourceFrameRate: { numerator: 24, denominator: 1 },
    },
  ],
};

// ── Protocol progression ──

test("executes the Gate-one protocol in order", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  assert.deepEqual(await session.dispatch(request(1, { type: "load", plan })), {
    version: 1,
    requestId: 1,
    event: { type: "loaded" },
  });
  assert.deepEqual(
    await session.dispatch(
      request(2, { type: "prepare", evaluationStart: 10 }),
    ),
    {
      version: 1,
      requestId: 2,
      event: { type: "prepared", evaluationStart: 10 },
    },
  );
  assert.deepEqual(
    await session.dispatch(request(3, { type: "seek", frame: 15 })),
    {
      version: 1,
      requestId: 3,
      event: { type: "frameReady", frame: 15 },
    },
  );
  assert.deepEqual(await session.dispatch(request(4, { type: "dispose" })), {
    version: 1,
    requestId: 4,
    event: { type: "disposed" },
  });
  assert.deepEqual(adapter.operations, [
    "load",
    "prepare:10",
    "seek:15",
    "dispose",
  ]);
  assert.deepEqual(adapter.preparedFrame, { index: 10, timeSeconds: 1 / 3 });
  assert.deepEqual(adapter.seekFrames, [{ index: 15, timeSeconds: 0.5 }]);
});

test("rejects commands that violate session state or evaluation bounds", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  const beforeLoad = await session.dispatch(
    request(1, { type: "seek", frame: 10 }),
  );
  assertFailure(beforeLoad, "invalidRequest");

  await session.dispatch(request(2, { type: "load", plan }));
  const wrongStart = await session.dispatch(
    request(3, { type: "prepare", evaluationStart: 11 }),
  );
  assertFailure(wrongStart, "invalidRequest");

  await session.dispatch(request(4, { type: "prepare", evaluationStart: 10 }));
  const outside = await session.dispatch(
    request(5, { type: "seek", frame: 20 }),
  );
  assertFailure(outside, "invalidRequest");
  assert.deepEqual(adapter.operations, ["load", "prepare:10"]);
});

// ── Concurrency, failures, and ownership ──

test("rejects concurrent commands instead of growing a hidden queue", async () => {
  let finishLoad!: () => void;
  const adapter = new RecordingAdapter();
  adapter.loadBarrier = new Promise<void>((resolve) => {
    finishLoad = resolve;
  });
  const session = new RuntimeSession(adapter);

  const loading = session.dispatch(request(1, { type: "load", plan }));
  const concurrent = await session.dispatch(request(2, { type: "dispose" }));

  assertFailure(concurrent, "invalidRequest");
  finishLoad();
  assert.equal((await loading).event.type, "loaded");
});

test("translates typed adapter failures and contains untyped exceptions", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  await session.dispatch(request(1, { type: "load", plan }));
  adapter.prepareError = new RuntimeAdapterError(
    "readinessTimeout",
    "fonts did not become ready",
    ["font:Inter"],
  );
  const timeout = await session.dispatch(
    request(2, { type: "prepare", evaluationStart: 10 }),
  );
  assert.deepEqual(timeout.event, {
    type: "failed",
    code: "readinessTimeout",
    message: "fonts did not become ready",
    pendingResources: ["font:Inter"],
  });

  adapter.prepareError = undefined;
  await session.dispatch(request(3, { type: "prepare", evaluationStart: 10 }));
  adapter.seekError = new Error("vendor-specific failure");
  const internal = await session.dispatch(
    request(4, { type: "seek", frame: 10 }),
  );
  assertFailure(internal, "internal");
  if (internal.event.type === "failed") {
    assert.equal(
      internal.event.message,
      "runtime adapter threw an untyped error",
    );
  }
});

test("reserves readiness timeouts for operations that wait for a frame", async () => {
  const adapter = new RecordingAdapter();
  adapter.loadError = new RuntimeAdapterError(
    "readinessTimeout",
    "browser launch timed out",
    ["browser"],
  );
  const session = new RuntimeSession(adapter);

  const failure = await session.dispatch(request(1, { type: "load", plan }));

  assertFailure(failure, "loadFailed");
});

test("takes ownership of plan facts and makes disposal terminal", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);
  const mutablePlan = structuredClone(plan);

  await session.dispatch(request(1, { type: "load", plan: mutablePlan }));
  mutablePlan.evaluation.start = 12;
  await session.dispatch(request(2, { type: "prepare", evaluationStart: 10 }));
  adapter.disposeError = new RuntimeAdapterError(
    "operation",
    "browser cleanup failed",
  );
  const cleanup = await session.dispatch(request(3, { type: "dispose" }));
  const disposed = await session.dispatch(
    request(4, { type: "seek", frame: 10 }),
  );

  assertFailure(cleanup, "internal");
  assertFailure(disposed, "invalidRequest");
  assert.deepEqual(adapter.loadedPlan, plan);
});

test("rejects interval relationships outside the browser plan contract", async () => {
  const reversedEvaluation = structuredClone(plan);
  reversedEvaluation.evaluation = { start: 20, end: 10 };
  const reversedOutput = structuredClone(plan);
  reversedOutput.output = { start: 20, end: 10 };
  const escapedOutput = structuredClone(plan);
  escapedOutput.output = { start: 9, end: 20 };

  for (const invalidPlan of [
    reversedEvaluation,
    reversedOutput,
    escapedOutput,
  ]) {
    const adapter = new RecordingAdapter();
    const session = new RuntimeSession(adapter);
    const rejected = await session.dispatch(
      request(1, { type: "load", plan: invalidPlan }),
    );

    assertFailure(rejected, "invalidRequest");
    assert.deepEqual(adapter.operations, []);
  }
});

test("rejects invalid browser video facts before adapter loading", async () => {
  const emptyVideo = structuredClone(plan);
  firstVideo(emptyVideo).interval = { start: 12, end: 12 };
  const escapedVideo = structuredClone(plan);
  firstVideo(escapedVideo).interval = { start: 9, end: 18 };

  for (const invalidPlan of [emptyVideo, escapedVideo]) {
    const adapter = new RecordingAdapter();
    const session = new RuntimeSession(adapter);
    const rejected = await session.dispatch(
      request(1, { type: "load", plan: invalidPlan }),
    );

    assertFailure(rejected, "invalidRequest");
    assert.deepEqual(adapter.operations, []);
  }
});

test("keeps the owned plan immutable after passing it to the adapter", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  await session.dispatch(request(1, { type: "load", plan }));
  const loadedPlan = adapter.loadedPlan;
  assert.ok(loadedPlan);
  assert.equal(Reflect.set(loadedPlan.frameRate, "numerator", 60), false);
  assert.equal(Reflect.set(firstVideo(loadedPlan).interval, "start", 0), false);

  await session.dispatch(request(2, { type: "prepare", evaluationStart: 10 }));
  await session.dispatch(request(3, { type: "seek", frame: 15 }));
  assert.deepEqual(adapter.seekFrames, [{ index: 15, timeSeconds: 0.5 }]);
});

// ── Test support ──

function request(
  requestId: number,
  command: BrowserRequest["command"],
): BrowserRequest {
  return { version: 1, requestId, command };
}

function firstVideo<Video>(plan: { readonly videos: readonly Video[] }): Video {
  const video = plan.videos[0];
  assert.ok(video);
  return video;
}

function assertFailure(response: BrowserResponse, code: FailureCode): void {
  assert.equal(response.event.type, "failed");
  if (response.event.type === "failed") {
    assert.equal(response.event.code, code);
  }
}

type FailureCode = Extract<
  BrowserResponse["event"],
  { type: "failed" }
>["code"];

class RecordingAdapter implements RuntimeAdapter {
  readonly operations: string[] = [];
  loadedPlan: RuntimePlan | undefined;
  loadBarrier: Promise<void> | undefined;
  loadError: Error | undefined;
  prepareError: Error | undefined;
  seekError: Error | undefined;
  disposeError: Error | undefined;
  preparedFrame: RuntimeFrame | undefined;
  readonly seekFrames: RuntimeFrame[] = [];

  async load(plan: RuntimePlan): Promise<void> {
    this.operations.push("load");
    this.loadedPlan = plan;
    if (this.loadError !== undefined) {
      throw this.loadError;
    }
    if (this.loadBarrier !== undefined) {
      await this.loadBarrier;
    }
  }

  async prepare(frame: RuntimeFrame): Promise<void> {
    this.operations.push(`prepare:${frame.index}`);
    this.preparedFrame = frame;
    if (this.prepareError !== undefined) {
      throw this.prepareError;
    }
  }

  async seek(frame: RuntimeFrame): Promise<void> {
    this.operations.push(`seek:${frame.index}`);
    this.seekFrames.push(frame);
    if (this.seekError !== undefined) {
      throw this.seekError;
    }
  }

  async dispose(): Promise<void> {
    this.operations.push("dispose");
    if (this.disposeError !== undefined) {
      throw this.disposeError;
    }
  }
}
