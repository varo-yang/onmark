// Browser-media projection from exact Rust-owned frame facts.
// It selects source frames without becoming a second timeline solver.

import type { RuntimeFrame } from "./clock.js";
import type { RuntimePlan } from "./session.js";

/** Immutable browser placement projected from Timeline IR. */
export type RuntimeVideo = RuntimePlan["videos"][number];
type FrameRate = RuntimePlan["frameRate"];
const MAX_DURATION_NANOSECONDS = 18_446_744_073_709_551_615n;
const MAX_RATIO_PART = 4_294_967_295;

// ── Source mapping boundary ──

/** One source frame and an interior seek time that cannot hit its boundary. */
export interface VideoFrameSelection {
  readonly mediaTimeSeconds: number;
  readonly seekTimeSeconds: number;
  /** Largest callback error that cannot identify an adjacent source frame. */
  readonly readinessToleranceSeconds: number;
}

/** Selects the source frame visible at one output-frame midpoint. */
export function videoFrameSelection(
  frame: RuntimeFrame,
  video: RuntimeVideo,
  outputFrameRate: FrameRate,
): VideoFrameSelection | undefined {
  if (frame.index < video.interval.start || frame.index >= video.interval.end) {
    return undefined;
  }

  const localFrame = frame.index - video.interval.start;
  const sourceTime = sourceTimeAtMidpoint(
    localFrame,
    outputFrameRate,
    video.source,
  );
  return selectSourceFrame(sourceTime, video.sourceTiming, video.source);
}

/** Checks protocol source facts without deriving any authored timeline. */
export function videoSourceMappingIsValid(
  video: RuntimeVideo,
  outputFrameRate: FrameRate,
): boolean {
  const source = video.source;
  const start = BigInt(source.startNanoseconds);
  const end = BigInt(source.endNanoseconds);
  const naturalEnd = BigInt(source.naturalEndNanoseconds);
  if (
    start >= end ||
    end > naturalEnd ||
    naturalEnd > MAX_DURATION_NANOSECONDS
  ) {
    return false;
  }
  if (
    !exactRatioIsCanonical(source.playbackRate) ||
    !Number.isInteger(source.plays) ||
    source.plays <= 0 ||
    source.plays > 4_294_967_295 ||
    !canonicalUnsignedInteger(source.holdLastNanoseconds) ||
    !sourceTimingIsValid(video.sourceTiming, naturalEnd)
  ) {
    return false;
  }

  const sourceDuration = end - start;
  const playbackNumerator =
    sourceDuration *
    BigInt(outputFrameRate.numerator) *
    BigInt(source.playbackRate.denominator) *
    BigInt(source.plays);
  const holdNumerator =
    BigInt(source.holdLastNanoseconds) *
    BigInt(outputFrameRate.numerator) *
    BigInt(source.playbackRate.numerator);
  const numerator = playbackNumerator + holdNumerator;
  const denominator =
    1_000_000_000n *
    BigInt(outputFrameRate.denominator) *
    BigInt(source.playbackRate.numerator);
  const expectedFrames = (numerator + denominator - 1n) / denominator;
  const actualFrames = BigInt(video.interval.end - video.interval.start);

  return actualFrames === expectedFrames;
}

// ── Exact placement time ──

interface ExactValue {
  readonly numerator: bigint;
  readonly denominator: bigint;
}

function sourceTimeAtMidpoint(
  localFrame: number,
  outputFrameRate: FrameRate,
  source: RuntimeVideo["source"],
): ExactValue {
  const midpoint = 2n * BigInt(localFrame) + 1n;
  const outputNumerator = BigInt(outputFrameRate.numerator);
  const outputDenominator = BigInt(outputFrameRate.denominator);
  const speedNumerator = BigInt(source.playbackRate.numerator);
  const speedDenominator = BigInt(source.playbackRate.denominator);
  const startNanoseconds = BigInt(source.startNanoseconds);
  const endNanoseconds = BigInt(source.endNanoseconds);
  const plays = BigInt(source.plays);
  const nanosecondsPerSecond = 1_000_000_000n;

  // Rust owns the source treatment. This integer projection samples one exact
  // pass without reconstructing a browser timeline.
  const timeDenominator = 2n * outputNumerator * speedDenominator;
  const elapsedNumerator =
    midpoint * outputDenominator * speedNumerator * nanosecondsPerSecond;
  const passNumerator = (endNanoseconds - startNanoseconds) * timeDenominator;
  const playbackNumerator = passNumerator * plays;
  const sourceTimeNumerator =
    elapsedNumerator < playbackNumerator
      ? startNanoseconds * timeDenominator + (elapsedNumerator % passNumerator)
      : endNanoseconds * timeDenominator - 1n;
  return {
    numerator: sourceTimeNumerator,
    denominator: timeDenominator,
  };
}

// ── CFR and VFR projection ──

function selectSourceFrame(
  time: ExactValue,
  timing: RuntimeVideo["sourceTiming"],
  source: RuntimeVideo["source"],
): VideoFrameSelection {
  switch (timing.kind) {
    case "constant":
      return selectConstantFrame(time, timing.frameRate, source);
    case "variable":
      return selectVariableFrame(time, timing);
  }
}

function selectConstantFrame(
  time: ExactValue,
  rate: FrameRate,
  source: RuntimeVideo["source"],
): VideoFrameSelection {
  const rateNumerator = BigInt(rate.numerator);
  const frameDenominator =
    time.denominator * 1_000_000_000n * BigInt(rate.denominator);
  const selected = (time.numerator * rateNumerator) / frameDenominator;
  const sourceEnd = BigInt(source.endNanoseconds) * rateNumerator;
  const exclusiveEnd = divideCeil(
    sourceEnd,
    1_000_000_000n * BigInt(rate.denominator),
  );
  const frame = selected < exclusiveEnd ? selected : exclusiveEnd - 1n;
  const rateDenominator = BigInt(rate.denominator);
  return projectFrameInterval(
    frame * rateDenominator,
    (frame + 1n) * rateDenominator,
    rateNumerator,
    rateDenominator,
  );
}

function selectVariableFrame(
  time: ExactValue,
  timing: Extract<RuntimeVideo["sourceTiming"], { kind: "variable" }>,
): VideoFrameSelection {
  const selected = sourceFrameIndex(time, timing.timebase, timing.boundaries);
  const start = timing.boundaries[selected];
  const end = timing.boundaries[selected + 1];
  if (start === undefined || end === undefined) {
    throw new RangeError(
      "selected source frame lies outside its timestamp map",
    );
  }
  const startTick = BigInt(start);
  const endTick = BigInt(end);
  const timebaseNumerator = BigInt(timing.timebase.numerator);
  const previous = timing.boundaries[selected - 1];
  const previousDistance =
    previous === undefined ? endTick - startTick : startTick - BigInt(previous);
  const nearestDistance =
    previousDistance < endTick - startTick
      ? previousDistance
      : endTick - startTick;
  return projectFrameInterval(
    startTick * timebaseNumerator,
    endTick * timebaseNumerator,
    BigInt(timing.timebase.denominator),
    nearestDistance * timebaseNumerator,
  );
}

function projectFrameInterval(
  startNumerator: bigint,
  endNumerator: bigint,
  denominator: bigint,
  neighborDistanceNumerator: bigint,
): VideoFrameSelection {
  // Frame identity remains exact until this single projection into the
  // browser's floating-point media API. Reject intervals for which no
  // representable interior second can prove the selected frame.
  const mediaTimeSeconds = rationalSeconds(startNumerator, denominator);
  const seekTimeSeconds = rationalSeconds(
    startNumerator + endNumerator,
    2n * denominator,
  );
  const endTimeSeconds = rationalSeconds(endNumerator, denominator);
  const readinessToleranceSeconds = Math.min(
    0.000_001,
    rationalSeconds(neighborDistanceNumerator, denominator) / 4,
  );
  if (
    !Number.isFinite(mediaTimeSeconds) ||
    !(mediaTimeSeconds < seekTimeSeconds) ||
    !(seekTimeSeconds < endTimeSeconds) ||
    !(readinessToleranceSeconds > 0)
  ) {
    throw new RangeError(
      "selected source frame has no representable interior seek time",
    );
  }

  return Object.freeze({
    mediaTimeSeconds,
    seekTimeSeconds,
    readinessToleranceSeconds,
  });
}

function rationalSeconds(numerator: bigint, denominator: bigint): number {
  const whole = numerator / denominator;
  const remainder = numerator % denominator;
  return Number(whole) + Number(remainder) / Number(denominator);
}

function sourceFrameIndex(
  time: ExactValue,
  timebase: FrameRate,
  boundaries: readonly string[],
): number {
  const targetNumerator = time.numerator * BigInt(timebase.denominator);
  const targetDenominator =
    time.denominator * BigInt(timebase.numerator) * 1_000_000_000n;
  let low = 0;
  let high = boundaries.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    const boundary = boundaries[middle];
    if (
      boundary !== undefined &&
      BigInt(boundary) * targetDenominator <= targetNumerator
    ) {
      low = middle + 1;
    } else {
      high = middle;
    }
  }
  return Math.min(low - 1, boundaries.length - 2);
}

// ── Wire-fact validation ──

function sourceTimingIsValid(
  timing: RuntimeVideo["sourceTiming"],
  naturalEndNanoseconds: bigint,
): boolean {
  if (timing.kind === "constant") {
    return exactRatioIsCanonical(timing.frameRate);
  }
  if (
    !exactRatioIsCanonical(timing.timebase) ||
    timing.boundaries.length < 3 ||
    timing.boundaries.length > 100_000
  ) {
    return false;
  }

  let previous = -1n;
  for (const spelling of timing.boundaries) {
    if (!canonicalUnsignedInteger(spelling)) {
      return false;
    }
    const boundary = BigInt(spelling);
    if (boundary <= previous) {
      return false;
    }
    previous = boundary;
  }
  if (timing.boundaries[0] !== "0") {
    return false;
  }
  const duration = divideCeil(
    previous * BigInt(timing.timebase.numerator) * 1_000_000_000n,
    BigInt(timing.timebase.denominator),
  );
  return duration === naturalEndNanoseconds;
}

function canonicalUnsignedInteger(value: string): boolean {
  return (
    /^(0|[1-9][0-9]*)$/u.test(value) &&
    BigInt(value) <= MAX_DURATION_NANOSECONDS
  );
}

function divideCeil(numerator: bigint, denominator: bigint): bigint {
  return (numerator + denominator - 1n) / denominator;
}

/** Checks the canonical positive-rational spelling used by Rust wire values. */
export function exactRatioIsCanonical(rate: {
  readonly numerator: number;
  readonly denominator: number;
}): boolean {
  if (
    !Number.isInteger(rate.numerator) ||
    !Number.isInteger(rate.denominator) ||
    rate.numerator < 1 ||
    rate.denominator < 1 ||
    rate.numerator > MAX_RATIO_PART ||
    rate.denominator > MAX_RATIO_PART
  ) {
    return false;
  }

  let left = BigInt(rate.numerator);
  let right = BigInt(rate.denominator);
  while (right !== 0n) {
    [left, right] = [right, left % right];
  }
  return left === 1n;
}
