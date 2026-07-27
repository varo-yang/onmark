// Browser-media tests for placement-local source-frame selection.

import assert from "node:assert/strict";
import test from "node:test";

import { videoFrameSelection, type RuntimeFrame } from "../src/index.js";

const outputRate = { numerator: 30, denominator: 1 };
const video = {
  node: { nodeId: 3, authoredId: "video" },
  shotId: 2,
  assetId:
    "sha256:0101010101010101010101010101010101010101010101010101010101010101",
  interval: { start: 10, end: 310 },
  sourceFrameRate: { numerator: 30, denominator: 1 },
  source: {
    startNanoseconds: "0",
    endNanoseconds: "10000000000",
    naturalEndNanoseconds: "10000000000",
    playbackRate: { numerator: 1, denominator: 1 },
  },
};

test("projects film frames into placement-local source frames", () => {
  const frame: RuntimeFrame = { index: 12, timeSeconds: 0.4 };

  assert.deepEqual(videoFrameSelection(frame, video, outputRate), {
    mediaTimeSeconds: 2 / 30,
    seekTimeSeconds: 2.5 / 30,
  });
});

test("applies exact source-local trim and playback rate", () => {
  const frame: RuntimeFrame = { index: 10, timeSeconds: 10 / 30 };
  const edited = {
    ...video,
    interval: { start: 10, end: 100 },
    source: {
      startNanoseconds: "4000000000",
      endNanoseconds: "10000000000",
      naturalEndNanoseconds: "10000000000",
      playbackRate: { numerator: 2, denominator: 1 },
    },
  };

  assert.deepEqual(videoFrameSelection(frame, edited, outputRate), {
    mediaTimeSeconds: 4 + 1 / 30,
    seekTimeSeconds: 4 + 1.5 / 30,
  });
});

test("keeps a ceil-rounded final sample inside the source interval", () => {
  const frame: RuntimeFrame = { index: 10, timeSeconds: 10 / 30 };
  const subframe = {
    ...video,
    interval: { start: 10, end: 11 },
    source: {
      startNanoseconds: "0",
      endNanoseconds: "10000000",
      naturalEndNanoseconds: "10000000000",
      playbackRate: { numerator: 1, denominator: 1 },
    },
  };

  assert.deepEqual(videoFrameSelection(frame, subframe, outputRate), {
    mediaTimeSeconds: 0,
    seekTimeSeconds: 0.5 / 30,
  });
});

test("returns no selection outside the video placement", () => {
  for (const index of [9, 310]) {
    const frame: RuntimeFrame = { index, timeSeconds: index / 30 };
    assert.equal(videoFrameSelection(frame, video, outputRate), undefined);
  }
});

test("moves an exact source boundary into the selected frame interior", () => {
  const frame: RuntimeFrame = { index: 10, timeSeconds: 10 / 30 };
  const fasterSource = {
    ...video,
    sourceFrameRate: { numerator: 60, denominator: 1 },
  };

  assert.deepEqual(videoFrameSelection(frame, fasterSource, outputRate), {
    mediaTimeSeconds: 1 / 60,
    seekTimeSeconds: 1.5 / 60,
  });
});
