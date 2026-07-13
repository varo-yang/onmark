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

test("derives the materialized source from the Rust-owned bundle layout", () => {
  const placement: RuntimeVideo = {
    assetId:
      "sha256:0101010101010101010101010101010101010101010101010101010101010101",
    interval: { start: 10, end: 20 },
    sourceFrameRate: { numerator: 30, denominator: 1 },
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
): Promise<DecodedVideo> {
  const video = new DecodedVideo(element, timeoutMilliseconds);
  const loading = video.load("./assets/sha256/source");
  element.emit("loadeddata");
  await loading;
  return video;
}
