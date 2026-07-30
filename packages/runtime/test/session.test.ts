// Behavioral contract for the sequential browser runtime session.
// A recording adapter isolates protocol behavior from browser effects.

import assert from "node:assert/strict";
import test from "node:test";

import {
  BROWSER_PROTOCOL_VERSION,
  MAX_BROWSER_MEDIA_LAYOUTS,
  MAX_FAILURE_MESSAGE_CHARACTERS,
  MAX_PENDING_RESOURCE_CHARACTERS,
  MAX_PENDING_RESOURCES,
  RuntimeAdapterError,
  RuntimeSession,
  type BrowserPlan,
  type BrowserMediaLayout,
  type BrowserRequest,
  type BrowserResponse,
  type RuntimeAdapter,
  type RuntimeFrame,
  type RuntimeMediaMode,
  type RuntimePlan,
} from "../src/index.js";

const plan: BrowserPlan = {
  timelineVersion: 6,
  frameRate: { numerator: 30, denominator: 1 },
  timeline: { start: 0, end: 30 },
  evaluation: { start: 10, end: 20 },
  output: { start: 10, end: 20 },
  film: { nodeId: 0, authoredId: "film" },
  scenes: [
    {
      node: { nodeId: 1, authoredId: "scene" },
      interval: { start: 10, end: 20 },
    },
  ],
  shots: [
    {
      node: { nodeId: 2, authoredId: "shot" },
      sceneId: 1,
      interval: { start: 10, end: 20 },
    },
  ],
  transitions: [],
  variantFields: [],
  videos: [
    {
      node: { nodeId: 3, authoredId: "video" },
      shotId: 2,
      assetId:
        "sha256:0101010101010101010101010101010101010101010101010101010101010101",
      interval: { start: 12, end: 18 },
      sourceTiming: {
        kind: "constant",
        frameRate: { numerator: 24, denominator: 1 },
      },
      source: {
        startNanoseconds: "0",
        endNanoseconds: "200000000",
        naturalEndNanoseconds: "200000000",
        playbackRate: { numerator: 1, denominator: 1 },
        plays: 1,
        holdLastNanoseconds: "0",
      },
    },
  ],
  overlays: [
    {
      node: { nodeId: 4, authoredId: "title" },
      shotId: 2,
      kind: "title",
      text: "Opening",
      interval: { start: 12, end: 18 },
    },
  ],
};

// ── Protocol progression ──

test("executes the browser protocol in order", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  assert.deepEqual(await session.dispatch(request(1, { type: "load", plan })), {
    version: BROWSER_PROTOCOL_VERSION,
    requestId: 1,
    event: { type: "loaded" },
  });
  assert.deepEqual(
    await session.dispatch(
      request(2, { type: "prepare", evaluationStart: 10 }),
    ),
    {
      version: BROWSER_PROTOCOL_VERSION,
      requestId: 2,
      event: { type: "prepared", evaluationStart: 10, mediaLayout: [] },
    },
  );
  assert.deepEqual(
    await session.dispatch(request(3, { type: "seek", frame: 15 })),
    {
      version: BROWSER_PROTOCOL_VERSION,
      requestId: 3,
      event: { type: "frameStaged", frame: 15 },
    },
  );
  assert.deepEqual(
    await session.dispatch(request(4, { type: "confirm", frame: 15 })),
    {
      version: BROWSER_PROTOCOL_VERSION,
      requestId: 4,
      event: { type: "frameReady", frame: 15 },
    },
  );
  assert.deepEqual(await session.dispatch(request(5, { type: "dispose" })), {
    version: BROWSER_PROTOCOL_VERSION,
    requestId: 5,
    event: { type: "disposed" },
  });
  assert.deepEqual(adapter.operations, [
    "load",
    "prepare:10",
    "seek:15",
    "confirm:15",
    "dispose",
  ]);
  assert.deepEqual(adapter.preparedFrame, { index: 10, timeSeconds: 1 / 3 });
  assert.deepEqual(adapter.seekFrames, [{ index: 15, timeSeconds: 0.5 }]);
  assert.deepEqual(adapter.confirmedFrames, [{ index: 15, timeSeconds: 0.5 }]);
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

  const beforeStage = await session.dispatch(
    request(6, { type: "confirm", frame: 10 }),
  );
  assertFailure(beforeStage, "invalidRequest");

  await session.dispatch(request(7, { type: "seek", frame: 10 }));
  const secondSeek = await session.dispatch(
    request(8, { type: "seek", frame: 11 }),
  );
  const wrongConfirmation = await session.dispatch(
    request(9, { type: "confirm", frame: 11 }),
  );
  assertFailure(secondSeek, "invalidRequest");
  assertFailure(wrongConfirmation, "invalidRequest");
  assert.deepEqual(adapter.operations, ["load", "prepare:10", "seek:10"]);
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

test("makes a failed preparation terminal until disposal", async () => {
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

  const retry = await session.dispatch(
    request(3, { type: "prepare", evaluationStart: 10 }),
  );
  assertFailure(retry, "invalidRequest");
  assert.equal(
    (await session.dispatch(request(4, { type: "dispose" }))).event.type,
    "disposed",
  );
  assert.deepEqual(adapter.operations, ["load", "prepare:10", "dispose"]);
});

test("makes a failed load terminal until disposal", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);
  adapter.loadError = new RuntimeAdapterError(
    "operation",
    "presentation binding failed",
  );

  const failure = await session.dispatch(request(1, { type: "load", plan }));
  assertFailure(failure, "loadFailed");

  const retry = await session.dispatch(request(2, { type: "load", plan }));
  assertFailure(retry, "invalidRequest");
  assert.equal(
    (await session.dispatch(request(3, { type: "dispose" }))).event.type,
    "disposed",
  );
  assert.deepEqual(adapter.operations, ["load", "dispose"]);
});

test("makes a failed confirmation terminal until disposal", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  await session.dispatch(request(1, { type: "load", plan }));
  await session.dispatch(request(2, { type: "prepare", evaluationStart: 10 }));
  await session.dispatch(request(3, { type: "seek", frame: 10 }));
  adapter.confirmError = new RuntimeAdapterError(
    "readinessTimeout",
    "decoded frame did not reach the compositor",
    ["video:3"],
  );

  const failure = await session.dispatch(
    request(4, { type: "confirm", frame: 10 }),
  );
  assertFailure(failure, "readinessTimeout");

  const retry = await session.dispatch(
    request(5, { type: "confirm", frame: 10 }),
  );
  assertFailure(retry, "invalidRequest");
  assert.equal(
    (await session.dispatch(request(6, { type: "dispose" }))).event.type,
    "disposed",
  );
});

test("contains untyped adapter exceptions", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  await session.dispatch(request(1, { type: "load", plan }));
  await session.dispatch(request(2, { type: "prepare", evaluationStart: 10 }));
  adapter.seekError = new Error("vendor-specific failure");
  const internal = await session.dispatch(
    request(3, { type: "seek", frame: 10 }),
  );
  assertFailure(internal, "internal");
  if (internal.event.type === "failed") {
    assert.equal(
      internal.event.message,
      "runtime adapter threw an untyped error",
    );
  }

  const retry = await session.dispatch(request(4, { type: "seek", frame: 10 }));
  assertFailure(retry, "invalidRequest");
  assert.equal(
    (await session.dispatch(request(5, { type: "dispose" }))).event.type,
    "disposed",
  );
  assert.deepEqual(adapter.operations, [
    "load",
    "prepare:10",
    "seek:10",
    "dispose",
  ]);
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

test("bounds typed adapter failure details before encoding", () => {
  assert.throws(
    () =>
      new RuntimeAdapterError(
        "operation",
        "x".repeat(MAX_FAILURE_MESSAGE_CHARACTERS + 1),
      ),
    TypeError,
  );
  assert.throws(
    () =>
      new RuntimeAdapterError(
        "operation",
        "rendering failed",
        Array.from({ length: MAX_PENDING_RESOURCES + 1 }, () => "resource"),
      ),
    TypeError,
  );
  assert.throws(
    () =>
      new RuntimeAdapterError("operation", "rendering failed", [
        "x".repeat(MAX_PENDING_RESOURCE_CHARACTERS + 1),
      ]),
    TypeError,
  );
});

test("takes ownership of plan facts and makes disposal terminal", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);
  const mutablePlan = structuredClone(plan);

  await session.dispatch(request(1, { type: "load", plan: mutablePlan }));
  mutablePlan.timeline.end = 21;
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

test("retains immutable caption track metadata in the adapter snapshot", async () => {
  const captioned = structuredClone(plan);
  captioned.overlays.push({
    node: { nodeId: 5, authoredId: null },
    shotId: null,
    kind: "caption",
    captionTrack: { id: "en", language: "en-US" },
    text: "Exact captions",
    interval: { start: 10, end: 20 },
  });
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  const loaded = await session.dispatch(
    request(1, { type: "load", plan: captioned }),
  );

  assert.equal(loaded.event.type, "loaded");
  assert.deepEqual(adapter.loadedPlan?.overlays[1]?.captionTrack, {
    id: "en",
    language: "en-US",
  });
});

test("retains complete structural timing across an evaluation window", async () => {
  const projected = structuredClone(plan);
  projected.scenes[0]!.interval = { start: 0, end: 30 };
  projected.shots[0]!.interval = { start: 0, end: 30 };
  firstOverlay(projected).interval = { start: 5, end: 25 };
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  const loaded = await session.dispatch(
    request(1, { type: "load", plan: projected }),
  );

  assert.equal(loaded.event.type, "loaded");
  assert.deepEqual(adapter.loadedPlan, projected);
});

test("rejects interval relationships outside the browser plan contract", async () => {
  const noncanonicalFrameRate = structuredClone(plan);
  noncanonicalFrameRate.frameRate = { numerator: 60, denominator: 2 };
  const escapedEvaluation = structuredClone(plan);
  escapedEvaluation.timeline = { start: 11, end: 20 };
  const reversedEvaluation = structuredClone(plan);
  reversedEvaluation.evaluation = { start: 20, end: 10 };
  const reversedOutput = structuredClone(plan);
  reversedOutput.output = { start: 20, end: 10 };
  const emptyOutput = structuredClone(plan);
  emptyOutput.output = { start: 10, end: 10 };
  const escapedOutput = structuredClone(plan);
  escapedOutput.output = { start: 9, end: 20 };
  const escapedShot = structuredClone(plan);
  escapedShot.scenes[0]!.interval = { start: 12, end: 18 };
  const escapedVideo = structuredClone(plan);
  escapedVideo.shots[0]!.interval = { start: 13, end: 17 };
  const crossSceneTransition = structuredClone(plan);
  crossSceneTransition.scenes = [
    {
      node: { nodeId: 1, authoredId: "first-scene" },
      interval: { start: 10, end: 15 },
    },
    {
      node: { nodeId: 3, authoredId: "second-scene" },
      interval: { start: 14, end: 20 },
    },
  ];
  crossSceneTransition.shots = [
    {
      node: { nodeId: 2, authoredId: "first-shot" },
      sceneId: 1,
      interval: { start: 10, end: 15 },
    },
    {
      node: { nodeId: 4, authoredId: "second-shot" },
      sceneId: 3,
      interval: { start: 14, end: 20 },
    },
  ];
  crossSceneTransition.transitions = [
    {
      node: { nodeId: 5, authoredId: "cross-scene" },
      outgoingShotId: 2,
      incomingShotId: 4,
      interval: { start: 14, end: 15 },
    },
  ];
  crossSceneTransition.videos = [];
  crossSceneTransition.overlays = [];
  const partialTransition = structuredClone(crossSceneTransition);
  partialTransition.scenes = [
    {
      node: { nodeId: 1, authoredId: "scene" },
      interval: { start: 10, end: 20 },
    },
  ];
  partialTransition.shots[1]!.sceneId = 1;
  partialTransition.shots[0]!.interval = { start: 10, end: 17 };
  partialTransition.shots[1]!.interval = { start: 14, end: 20 };
  partialTransition.transitions[0]!.interval = { start: 15, end: 17 };

  for (const invalidPlan of [
    noncanonicalFrameRate,
    escapedEvaluation,
    reversedEvaluation,
    reversedOutput,
    emptyOutput,
    escapedOutput,
    escapedShot,
    escapedVideo,
    crossSceneTransition,
    partialTransition,
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

test("rejects invalid or duplicate authored node identity", async () => {
  const invalidIdentity = structuredClone(plan);
  invalidIdentity.film.authoredId = "bad id";
  const duplicateIdentity = structuredClone(plan);
  duplicateIdentity.shots[0]!.node.authoredId = "scene";

  for (const invalidPlan of [invalidIdentity, duplicateIdentity]) {
    const adapter = new RecordingAdapter();
    const session = new RuntimeSession(adapter);
    const rejected = await session.dispatch(
      request(1, { type: "load", plan: invalidPlan }),
    );

    assertFailure(rejected, "invalidRequest");
    assert.deepEqual(adapter.operations, []);
  }
});

test("rejects a sparse unit-local node identity", async () => {
  const sparse = structuredClone(plan);
  firstOverlay(sparse).node.nodeId = 9;
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);

  const rejected = await session.dispatch(
    request(1, { type: "load", plan: sparse }),
  );

  assertFailure(rejected, "invalidRequest");
  assert.deepEqual(adapter.operations, []);
});

test("rejects invalid browser video facts before adapter loading", async () => {
  const emptyVideo = structuredClone(plan);
  firstVideo(emptyVideo).interval = { start: 12, end: 12 };
  const escapedVideo = structuredClone(plan);
  firstVideo(escapedVideo).interval = { start: 9, end: 18 };
  const escapedSource = structuredClone(plan);
  firstVideo(escapedSource).source.endNanoseconds = "200000001";
  const mismatchedDuration = structuredClone(plan);
  firstVideo(mismatchedDuration).source.endNanoseconds = "100000000";
  const noncanonicalRate = structuredClone(plan);
  firstVideo(noncanonicalRate).source.playbackRate = {
    numerator: 2,
    denominator: 2,
  };
  const noncanonicalSourceRate = structuredClone(plan);
  firstVideo(noncanonicalSourceRate).sourceTiming = {
    kind: "constant",
    frameRate: { numerator: 48, denominator: 2 },
  };
  const emptyPlayback = structuredClone(plan);
  firstVideo(emptyPlayback).source.plays = 0;
  const noncanonicalHold = structuredClone(plan);
  firstVideo(noncanonicalHold).source.holdLastNanoseconds = "00";
  const overflowingSource = structuredClone(plan);
  firstVideo(overflowingSource).source = {
    startNanoseconds: "18446744073509551616",
    endNanoseconds: "18446744073709551616",
    naturalEndNanoseconds: "18446744073709551616",
    playbackRate: { numerator: 1, denominator: 1 },
    plays: 1,
    holdLastNanoseconds: "0",
  };

  for (const invalidPlan of [
    emptyVideo,
    escapedVideo,
    escapedSource,
    mismatchedDuration,
    noncanonicalRate,
    noncanonicalSourceRate,
    emptyPlayback,
    noncanonicalHold,
    overflowingSource,
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

test("rejects invalid browser overlay facts before adapter loading", async () => {
  const emptyOverlay = structuredClone(plan);
  firstOverlay(emptyOverlay).interval = { start: 12, end: 12 };
  const escapedOverlay = structuredClone(plan);
  firstOverlay(escapedOverlay).interval = { start: 9, end: 31 };
  const unrelatedOverlay = structuredClone(plan);
  firstOverlay(unrelatedOverlay).interval = { start: 0, end: 5 };
  const duplicateComponent = structuredClone(plan);
  duplicateComponent.overlays.push({ ...firstOverlay(duplicateComponent) });
  const noncanonicalComponent = structuredClone(plan);
  noncanonicalComponent.overlays.push({
    ...firstOverlay(noncanonicalComponent),
    node: { nodeId: 5, authoredId: "second-title" },
  });
  noncanonicalComponent.overlays.reverse();
  const captionWithoutTrack = structuredClone(plan);
  Object.assign(firstOverlay(captionWithoutTrack), {
    kind: "caption",
    shotId: null,
  });
  const authoredOverlayWithTrack = structuredClone(plan);
  firstOverlay(authoredOverlayWithTrack).captionTrack = {
    id: "en",
    language: "en",
  };
  const malformedTrack = structuredClone(plan);
  malformedTrack.overlays.push({
    node: { nodeId: 5, authoredId: null },
    shotId: null,
    kind: "caption",
    captionTrack: { id: "bad id", language: "en_US" },
    text: "Broken",
    interval: { start: 10, end: 20 },
  });

  for (const invalidPlan of [
    emptyOverlay,
    escapedOverlay,
    unrelatedOverlay,
    duplicateComponent,
    noncanonicalComponent,
    captionWithoutTrack,
    authoredOverlayWithTrack,
    malformedTrack,
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

test("rejects noncanonical typed variant facts before adapter loading", async () => {
  const unordered = structuredClone(plan);
  unordered.variantFields = [
    { name: "headline", value: { kind: "text", value: "Opening" } },
    { name: "accent", value: { kind: "color", value: "#ff4d36" } },
  ];
  const invalidName = structuredClone(plan);
  invalidName.variantFields = [
    { name: "Headline", value: { kind: "text", value: "Opening" } },
  ];
  const invalidColor = structuredClone(plan);
  invalidColor.variantFields = [
    { name: "accent", value: { kind: "color", value: "#FF4D36" } },
  ];
  const unsafeInteger = structuredClone(plan);
  unsafeInteger.variantFields = [
    {
      name: "progress",
      value: { kind: "integer", value: Number.MAX_SAFE_INTEGER + 1 },
    },
  ];
  const oversizedText = structuredClone(plan);
  oversizedText.variantFields = [
    { name: "headline", value: { kind: "text", value: "x".repeat(16_385) } },
  ];
  const oversizedUtf8Text = structuredClone(plan);
  oversizedUtf8Text.variantFields = [
    { name: "headline", value: { kind: "text", value: "界".repeat(5_462) } },
  ];
  const oversizedTextBudget = structuredClone(plan);
  oversizedTextBudget.variantFields = Array.from(
    { length: 64 },
    (_, index) => ({
      name: `field${String(index).padStart(3, "0")}`,
      value: { kind: "text" as const, value: "x".repeat(16_384) },
    }),
  );

  for (const invalidPlan of [
    unordered,
    invalidName,
    invalidColor,
    unsafeInteger,
    oversizedText,
    oversizedUtf8Text,
    oversizedTextBudget,
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

test("keeps the owned plan immutable after passing it to the adapter", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);
  const variantPlan: BrowserPlan = {
    ...plan,
    variantFields: [
      { name: "headline", value: { kind: "text", value: "Opening" } },
    ],
  };

  await session.dispatch(request(1, { type: "load", plan: variantPlan }));
  const loadedPlan = adapter.loadedPlan;
  assert.ok(loadedPlan);
  assert.equal(Reflect.set(loadedPlan.frameRate, "numerator", 60), false);
  assert.equal(Reflect.set(firstVideo(loadedPlan).interval, "start", 0), false);
  assert.equal(Reflect.set(firstOverlay(loadedPlan), "text", "Changed"), false);
  assert.equal(
    Reflect.set(loadedPlan.variantFields[0]!.value, "value", "Changed"),
    false,
  );

  await session.dispatch(request(2, { type: "prepare", evaluationStart: 10 }));
  await session.dispatch(request(3, { type: "seek", frame: 15 }));
  assert.deepEqual(adapter.seekFrames, [{ index: 15, timeSeconds: 0.5 }]);
});

test("returns canonical layout evidence from a layout-only load", async () => {
  const adapter = new RecordingAdapter();
  adapter.mediaLayout = [
    {
      nodeId: 3,
      objectFit: "cover",
      objectPosition: { x: 500_000, y: 500_000 },
      rectangle: { x: 80, y: 45, width: 640, height: 360 },
    },
  ];
  const session = new RuntimeSession(adapter);

  await session.dispatch(
    request(1, { type: "load", plan, mediaMode: "layoutOnly" }),
  );
  const prepared = await session.dispatch(
    request(2, { type: "prepare", evaluationStart: 10 }),
  );

  assert.equal(adapter.mediaMode, "layoutOnly");
  assert.deepEqual(prepared.event, {
    type: "prepared",
    evaluationStart: 10,
    mediaLayout: adapter.mediaLayout,
  });
});

test("rejects layout-only work beyond the protocol media bound", async () => {
  const adapter = new RecordingAdapter();
  const session = new RuntimeSession(adapter);
  const prototype = firstVideo(plan);
  const oversized: BrowserPlan = {
    ...plan,
    overlays: [],
    videos: Array.from(
      { length: MAX_BROWSER_MEDIA_LAYOUTS + 1 },
      (_, index) => ({
        ...prototype,
        node: { authoredId: `video-${index}`, nodeId: index + 3 },
      }),
    ),
  };

  const response = await session.dispatch(
    request(1, {
      type: "load",
      plan: oversized,
      mediaMode: "layoutOnly",
    }),
  );

  assert.deepEqual(response.event, {
    type: "failed",
    code: "invalidRequest",
    message: "layout-only plan exceeds the browser media-layout limit",
    pendingResources: [],
  });
  assert.deepEqual(adapter.operations, []);
});

// ── Test support ──

function request(
  requestId: number,
  command:
    | BrowserRequest["command"]
    | {
        readonly type: "load";
        readonly plan: BrowserPlan;
      },
): BrowserRequest {
  const complete =
    command.type === "load" && !("mediaMode" in command)
      ? { ...command, mediaMode: "decoded" as const }
      : command;
  return { version: BROWSER_PROTOCOL_VERSION, requestId, command: complete };
}

function firstVideo<Video>(plan: { readonly videos: readonly Video[] }): Video {
  const video = plan.videos[0];
  assert.ok(video);
  return video;
}

function firstOverlay<Overlay>(plan: {
  readonly overlays: readonly Overlay[];
}): Overlay {
  const overlay = plan.overlays[0];
  assert.ok(overlay);
  return overlay;
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
  mediaMode: RuntimeMediaMode | undefined;
  mediaLayout: BrowserMediaLayout = [];
  loadBarrier: Promise<void> | undefined;
  loadError: Error | undefined;
  prepareError: Error | undefined;
  seekError: Error | undefined;
  confirmError: Error | undefined;
  disposeError: Error | undefined;
  preparedFrame: RuntimeFrame | undefined;
  readonly seekFrames: RuntimeFrame[] = [];
  readonly confirmedFrames: RuntimeFrame[] = [];

  async load(plan: RuntimePlan, mediaMode: RuntimeMediaMode): Promise<void> {
    this.operations.push("load");
    this.loadedPlan = plan;
    this.mediaMode = mediaMode;
    if (this.loadError !== undefined) {
      throw this.loadError;
    }
    if (this.loadBarrier !== undefined) {
      await this.loadBarrier;
    }
  }

  async prepare(frame: RuntimeFrame): Promise<BrowserMediaLayout> {
    this.operations.push(`prepare:${frame.index}`);
    this.preparedFrame = frame;
    if (this.prepareError !== undefined) {
      throw this.prepareError;
    }
    return this.mediaLayout;
  }

  async seek(frame: RuntimeFrame): Promise<void> {
    this.operations.push(`seek:${frame.index}`);
    this.seekFrames.push(frame);
    if (this.seekError !== undefined) {
      throw this.seekError;
    }
  }

  async confirm(frame: RuntimeFrame): Promise<void> {
    this.operations.push(`confirm:${frame.index}`);
    this.confirmedFrames.push(frame);
    if (this.confirmError !== undefined) {
      throw this.confirmError;
    }
  }

  async dispose(): Promise<void> {
    this.operations.push("dispose");
    if (this.disposeError !== undefined) {
      throw this.disposeError;
    }
  }
}
