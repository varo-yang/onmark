// Browser-video lifecycle tests through one controllable media capability.

import assert from "node:assert/strict";
import test from "node:test";

import {
  DecodedVideo,
  RuntimeAdapterError,
  materializedVideoSource,
  type RuntimeVideo,
} from "../src/index.js";
import { FakeVideoElement } from "./fake-video-element.js";

const selection = {
  mediaTimeSeconds: 2 / 30,
  seekTimeSeconds: 2.5 / 30,
  readinessToleranceSeconds: 0.000_001,
};

// ── Decoded-video resource ──

test("stages a seek before confirming the compositor-presented frame", async () => {
  const element = new FakeVideoElement();
  const video = new DecodedVideo({
    element,
    nodeId: 0,
    readinessTimeoutMilliseconds: 100,
  });

  const loading = video.load("./assets/sha256/source");
  assert.equal(element.src, "./assets/sha256/source");
  assert.equal(element.loadCount, 1);
  element.emit("loadeddata");
  await loading;

  const staging = video.stage(selection);
  assert.equal(element.currentTime, selection.seekTimeSeconds);
  element.emit("seeked");
  await staging;

  const confirming = video.confirm(selection);
  element.present(selection.mediaTimeSeconds);
  await confirming;
});

test("reuses an already confirmed source frame without seeking again", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element);

  const first = video.stage(selection);
  element.emit("seeked");
  await first;
  const confirmation = video.confirm(selection);
  element.present(selection.mediaTimeSeconds);
  await confirmation;
  const seekCount = element.seekCount;

  await video.stage(selection);
  await video.confirm(selection);

  assert.equal(element.seekCount, seekCount);
  assert.equal(element.pendingFrameCallbacks, 0);
});

test("waits past unrelated decoded frames and removes every observer", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element);

  const staging = video.stage(selection);
  element.emit("seeked");
  await staging;
  const confirming = video.confirm(selection);
  element.present(0);
  assert.equal(element.pendingFrameCallbacks, 1);
  element.present(selection.mediaTimeSeconds);
  await confirming;

  assert.equal(element.listenerCount, 0);
  assert.equal(element.pendingFrameCallbacks, 0);
});

test("does not accept an adjacent short frame inside the old fixed tolerance", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element);
  const shortFrame = {
    mediaTimeSeconds: 1,
    seekTimeSeconds: 1.000_000_2,
    readinessToleranceSeconds: 0.000_000_1,
  };

  const staging = video.stage(shortFrame);
  element.emit("seeked");
  await staging;
  const confirming = video.confirm(shortFrame);
  element.present(1.000_000_3);
  assert.equal(element.pendingFrameCallbacks, 1);
  element.present(shortFrame.mediaTimeSeconds);
  await confirming;
});

test("reports bounded readiness failures and cleans the pending frame wait", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element, 5);

  await assert.rejects(
    async () => {
      const staging = video.stage(selection);
      element.emit("seeked");
      await staging;
      await video.confirm(selection);
    },
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.kind === "readinessTimeout" &&
      error.pendingResources.includes("video:0:frame"),
  );
  assert.equal(element.listenerCount, 0);
  assert.equal(element.pendingFrameCallbacks, 0);
});

test("identifies the video whose seek misses its readiness deadline", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element, 5, 7);

  await assert.rejects(
    video.stage(selection),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.kind === "readinessTimeout" &&
      error.pendingResources.includes("video:7:seeked"),
  );
});

test("cleans media observers after synchronous browser failures", async () => {
  const loadingElement = new FakeVideoElement();
  loadingElement.loadError = new Error("browser load failed");
  const loadingVideo = new DecodedVideo({
    element: loadingElement,
    nodeId: 0,
    readinessTimeoutMilliseconds: 100,
  });

  await assert.rejects(
    loadingVideo.load("./assets/sha256/source"),
    RuntimeAdapterError,
  );
  assert.equal(loadingElement.listenerCount, 0);
  assert.equal(loadingElement.hasSource, false);

  const eventThenErrorElement = new FakeVideoElement();
  eventThenErrorElement.loadErrorAfterReadiness = new Error("load rejected");
  const eventThenErrorVideo = new DecodedVideo({
    element: eventThenErrorElement,
    nodeId: 0,
    readinessTimeoutMilliseconds: 100,
  });

  await assert.rejects(
    eventThenErrorVideo.load("./assets/sha256/source"),
    RuntimeAdapterError,
  );
  assert.equal(eventThenErrorElement.listenerCount, 0);
  assert.equal(eventThenErrorElement.hasSource, false);

  const cleanupErrorElement = new FakeVideoElement();
  cleanupErrorElement.loadError = new Error("load rejected");
  cleanupErrorElement.releaseError = new Error("release rejected");
  const cleanupErrorVideo = new DecodedVideo({
    element: cleanupErrorElement,
    nodeId: 0,
    readinessTimeoutMilliseconds: 100,
  });

  await assert.rejects(
    cleanupErrorVideo.load("./assets/sha256/source"),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "video load failed and cleanup was incomplete",
  );
  assert.equal(cleanupErrorElement.listenerCount, 0);
  cleanupErrorElement.releaseError = undefined;
  await assert.rejects(
    cleanupErrorVideo.load("./assets/sha256/source"),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "video load requires the empty state",
  );

  const seekingElement = new FakeVideoElement();
  const seekingVideo = await loadedVideo(seekingElement);
  seekingElement.frameCallbackError = new Error("callback unavailable");

  await assert.rejects(seekingVideo.stage(selection), RuntimeAdapterError);
  assert.equal(seekingElement.listenerCount, 0);
  assert.equal(seekingElement.pendingFrameCallbacks, 0);

  const rejectedSeekElement = new FakeVideoElement();
  const rejectedSeekVideo = await loadedVideo(rejectedSeekElement);
  rejectedSeekElement.seekError = new Error("seek rejected");

  await assert.rejects(rejectedSeekVideo.stage(selection), RuntimeAdapterError);
  assert.equal(rejectedSeekElement.listenerCount, 0);
  assert.equal(rejectedSeekElement.pendingFrameCallbacks, 0);
});

test("releases media bytes and makes disposal terminal", async () => {
  const element = new FakeVideoElement();
  const video = await loadedVideo(element);

  video.dispose();

  assert.equal(element.hasSource, false);
  assert.equal(element.loadCount, 2);
  await assert.rejects(video.stage(selection), RuntimeAdapterError);
});

test("rejects invalid video identity and readiness policy", () => {
  assert.throws(
    () =>
      new DecodedVideo({
        element: new FakeVideoElement(),
        nodeId: -1,
        readinessTimeoutMilliseconds: 100,
      }),
    TypeError,
  );
  assert.throws(
    () =>
      new DecodedVideo({
        element: new FakeVideoElement(),
        nodeId: 0,
        readinessTimeoutMilliseconds: 0,
      }),
    TypeError,
  );
  assert.throws(
    () =>
      new DecodedVideo({
        element: new FakeVideoElement(),
        nodeId: 0,
        readinessTimeoutMilliseconds: 86_400_001,
      }),
    TypeError,
  );
});

test("derives the materialized source from the Rust-owned bundle layout", () => {
  const placement: RuntimeVideo = {
    node: { nodeId: 3, authoredId: "video" },
    shotId: 2,
    assetId:
      "sha256:0101010101010101010101010101010101010101010101010101010101010101",
    interval: { start: 10, end: 20 },
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

  assert.equal(
    materializedVideoSource(placement),
    "./assets/sha256/0101010101010101010101010101010101010101010101010101010101010101",
  );
});

// ── Test support ──

async function loadedVideo(
  element: FakeVideoElement,
  timeoutMilliseconds = 100,
  nodeId = 0,
): Promise<DecodedVideo> {
  const video = new DecodedVideo({
    element,
    nodeId,
    readinessTimeoutMilliseconds: timeoutMilliseconds,
  });
  const loading = video.load("./assets/sha256/source");
  element.emit("loadeddata");
  await loading;
  return video;
}
