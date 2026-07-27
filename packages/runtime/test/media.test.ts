// Browser-media tests for placement-local source-frame selection.

import assert from "node:assert/strict";
import test from "node:test";

import {
  videoFrameSelection,
  type RuntimeFrame,
  type VideoFrameSelection,
} from "../src/index.js";

const outputRate = { numerator: 30, denominator: 1 };
const readinessToleranceSeconds = 0.000_001;
const video = {
  node: { nodeId: 3, authoredId: "video" },
  shotId: 2,
  assetId:
    "sha256:0101010101010101010101010101010101010101010101010101010101010101",
  interval: { start: 10, end: 310 },
  sourceTiming: {
    kind: "constant" as const,
    frameRate: { numerator: 30, denominator: 1 },
  },
  source: {
    startNanoseconds: "0",
    endNanoseconds: "10000000000",
    naturalEndNanoseconds: "10000000000",
    playbackRate: { numerator: 1, denominator: 1 },
    plays: 1,
    holdLastNanoseconds: "0",
  },
};

// ── Constant-rate selection ──

test("projects film frames into placement-local source frames", () => {
  const frame: RuntimeFrame = { index: 12, timeSeconds: 0.4 };

  assert.deepEqual(
    videoFrameSelection(frame, video, outputRate),
    selection(2 / 30, 2.5 / 30),
  );
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
      plays: 1,
      holdLastNanoseconds: "0",
    },
  };

  assert.deepEqual(
    videoFrameSelection(frame, edited, outputRate),
    selection(4 + 1 / 30, 4 + 1.5 / 30),
  );
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
      plays: 1,
      holdLastNanoseconds: "0",
    },
  };

  assert.deepEqual(
    videoFrameSelection(frame, subframe, outputRate),
    selection(0, 0.5 / 30),
  );
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
    sourceTiming: {
      kind: "constant" as const,
      frameRate: { numerator: 60, denominator: 1 },
    },
  };

  assert.deepEqual(
    videoFrameSelection(frame, fasterSource, outputRate),
    selection(1 / 60, 1.5 / 60),
  );
});

// ── Variable-rate selection ──

test("selects variable-rate frames from exact source timestamp boundaries", () => {
  const variable = {
    ...video,
    interval: { start: 10, end: 15 },
    sourceTiming: {
      kind: "variable" as const,
      timebase: { numerator: 1, denominator: 1_000 },
      boundaries: ["0", "40", "100", "140"] as [
        string,
        string,
        string,
        ...string[],
      ],
    },
    source: {
      startNanoseconds: "0",
      endNanoseconds: "140000000",
      naturalEndNanoseconds: "140000000",
      playbackRate: { numerator: 1, denominator: 1 },
      plays: 1,
      holdLastNanoseconds: "0",
    },
  };

  assert.deepEqual(
    [10, 11, 12, 13, 14].map((index) =>
      videoFrameSelection(
        { index, timeSeconds: index / 30 },
        variable,
        outputRate,
      ),
    ),
    [
      selection(0, 0.02),
      selection(0.04, 0.07),
      selection(0.04, 0.07),
      selection(0.1, 0.12),
      selection(0.1, 0.12),
    ],
  );
});

test("projects valid timestamp ticks beyond the exact-integer domain", () => {
  const firstLargeTick = 9_007_199_254_740_993n;
  const nextLargeTick = firstLargeTick + 40_000_000n;
  const variable = {
    ...video,
    interval: { start: 0, end: 1 },
    sourceTiming: {
      kind: "variable" as const,
      timebase: { numerator: 1, denominator: 1_000_000_000 },
      boundaries: ["0", String(firstLargeTick), String(nextLargeTick)] as [
        string,
        string,
        string,
        ...string[],
      ],
    },
    source: {
      startNanoseconds: String(firstLargeTick),
      endNanoseconds: String(nextLargeTick),
      naturalEndNanoseconds: String(nextLargeTick),
      playbackRate: { numerator: 1, denominator: 1 },
      plays: 1,
      holdLastNanoseconds: "0",
    },
  };

  const selection = videoFrameSelection(
    { index: 0, timeSeconds: 0 },
    variable,
    { numerator: 1_000_000_000, denominator: 1 },
  );

  assert.ok(selection !== undefined);
  assert.equal(selection.mediaTimeSeconds, seconds(firstLargeTick));
  assert.ok(selection.seekTimeSeconds > selection.mediaTimeSeconds);
  assert.ok(selection.seekTimeSeconds < seconds(nextLargeTick));
});

test("rejects a variable frame with no representable interior second", () => {
  const firstLargeTick = 9_007_199_254_740_993n;
  const variable = {
    ...video,
    interval: { start: 0, end: 1 },
    sourceTiming: {
      kind: "variable" as const,
      timebase: { numerator: 1, denominator: 1_000_000_000 },
      boundaries: [
        "0",
        String(firstLargeTick),
        String(firstLargeTick + 1n),
      ] as [string, string, string, ...string[]],
    },
    source: {
      startNanoseconds: String(firstLargeTick),
      endNanoseconds: String(firstLargeTick + 1n),
      naturalEndNanoseconds: String(firstLargeTick + 1n),
      playbackRate: { numerator: 1, denominator: 1 },
      plays: 1,
      holdLastNanoseconds: "0",
    },
  };

  assert.throws(
    () =>
      videoFrameSelection({ index: 0, timeSeconds: 0 }, variable, {
        numerator: 1_000_000_000,
        denominator: 1,
      }),
    /no representable interior seek time/u,
  );
});

test("rejects a constant frame with no representable interior second", () => {
  const firstLargeTimestamp = 18_446_744_073_709_551_614n;
  const constant = {
    ...video,
    interval: { start: 0, end: 1 },
    sourceTiming: {
      kind: "constant" as const,
      frameRate: { numerator: 4_294_967_295, denominator: 1 },
    },
    source: {
      startNanoseconds: String(firstLargeTimestamp),
      endNanoseconds: String(firstLargeTimestamp + 1n),
      naturalEndNanoseconds: String(firstLargeTimestamp + 1n),
      playbackRate: { numerator: 1, denominator: 1 },
      plays: 1,
      holdLastNanoseconds: "0",
    },
  };

  assert.throws(
    () =>
      videoFrameSelection({ index: 0, timeSeconds: 0 }, constant, {
        numerator: 1_000_000_000,
        denominator: 1,
      }),
    /no representable interior seek time/u,
  );
});

// ── Source continuity ──

test("restarts exact source passes before holding the final frame", () => {
  const repeated = {
    ...video,
    interval: { start: 0, end: 6 },
    sourceTiming: {
      kind: "constant" as const,
      frameRate: { numerator: 2, denominator: 1 },
    },
    source: {
      startNanoseconds: "0",
      endNanoseconds: "1000000000",
      naturalEndNanoseconds: "1000000000",
      playbackRate: { numerator: 1, denominator: 1 },
      plays: 2,
      holdLastNanoseconds: "1000000000",
    },
  };

  assert.deepEqual(
    Array.from({ length: 6 }, (_, index) =>
      videoFrameSelection({ index, timeSeconds: index / 2 }, repeated, {
        numerator: 2,
        denominator: 1,
      }),
    ),
    [
      selection(0, 0.25),
      selection(0.5, 0.75),
      selection(0, 0.25),
      selection(0.5, 0.75),
      selection(0.5, 0.75),
      selection(0.5, 0.75),
    ],
  );
});

function selection(
  mediaTimeSeconds: number,
  seekTimeSeconds: number,
): VideoFrameSelection {
  return { mediaTimeSeconds, seekTimeSeconds, readinessToleranceSeconds };
}

function seconds(nanoseconds: bigint): number {
  const whole = nanoseconds / 1_000_000_000n;
  const remainder = nanoseconds % 1_000_000_000n;
  return Number(whole) + Number(remainder) / 1_000_000_000;
}
