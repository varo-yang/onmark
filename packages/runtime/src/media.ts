// Browser-media projection from exact Rust-owned frame facts.
// It selects source frames without becoming a second timeline solver.

import type { RuntimeFrame } from "./clock.js";
import type { RuntimePlan } from "./session.js";

/** Immutable browser placement projected from Timeline IR. */
export type RuntimeVideo = RuntimePlan["videos"][number];
type FrameRate = RuntimePlan["frameRate"];
const MAX_DURATION_NANOSECONDS = 18_446_744_073_709_551_615n;
const MAX_RATIO_PART = 4_294_967_295;

/** One source frame and an interior seek time that cannot hit its boundary. */
export interface VideoFrameSelection {
  readonly mediaTimeSeconds: number;
  readonly seekTimeSeconds: number;
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
  const sourceFrame = sourceFrameAtMidpoint(
    localFrame,
    outputFrameRate,
    video.sourceFrameRate,
    video.source,
  );
  const sourceFrameDuration =
    video.sourceFrameRate.denominator / video.sourceFrameRate.numerator;

  return Object.freeze({
    mediaTimeSeconds: sourceFrame * sourceFrameDuration,
    seekTimeSeconds: (sourceFrame + 0.5) * sourceFrameDuration,
  });
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
    !exactRatioIsCanonical(video.sourceFrameRate) ||
    !exactRatioIsCanonical(source.playbackRate)
  ) {
    return false;
  }

  const sourceDuration = end - start;
  const numerator =
    sourceDuration *
    BigInt(outputFrameRate.numerator) *
    BigInt(source.playbackRate.denominator);
  const denominator =
    1_000_000_000n *
    BigInt(outputFrameRate.denominator) *
    BigInt(source.playbackRate.numerator);
  const expectedFrames = (numerator + denominator - 1n) / denominator;
  const actualFrames = BigInt(video.interval.end - video.interval.start);

  return actualFrames === expectedFrames;
}

function sourceFrameAtMidpoint(
  localFrame: number,
  outputFrameRate: FrameRate,
  sourceFrameRate: FrameRate,
  source: RuntimeVideo["source"],
): number {
  const midpoint = 2n * BigInt(localFrame) + 1n;
  const outputNumerator = BigInt(outputFrameRate.numerator);
  const outputDenominator = BigInt(outputFrameRate.denominator);
  const sourceNumerator = BigInt(sourceFrameRate.numerator);
  const sourceDenominator = BigInt(sourceFrameRate.denominator);
  const speedNumerator = BigInt(source.playbackRate.numerator);
  const speedDenominator = BigInt(source.playbackRate.denominator);
  const startNanoseconds = BigInt(source.startNanoseconds);
  const endNanoseconds = BigInt(source.endNanoseconds);
  const nanosecondsPerSecond = 1_000_000_000n;

  // Rust owns the affine source-time mapping. This integer projection samples
  // the output-frame midpoint without reconstructing a browser timeline.
  const timeDenominator = 2n * outputNumerator * speedDenominator;
  const sourceTimeNumerator =
    startNanoseconds * timeDenominator +
    midpoint * outputDenominator * speedNumerator * nanosecondsPerSecond;
  const frameNumerator = sourceTimeNumerator * sourceNumerator;
  const denominator =
    timeDenominator * nanosecondsPerSecond * sourceDenominator;
  const selectedFrame = frameNumerator / denominator;
  const sourceEndNumerator = endNanoseconds * sourceNumerator;
  const sourceFrameDenominator = nanosecondsPerSecond * sourceDenominator;
  const exclusiveEndFrame =
    (sourceEndNumerator + sourceFrameDenominator - 1n) / sourceFrameDenominator;
  // Ceil-rounded output may expose one midpoint beyond the trim edge. Keep
  // that final sample inside the last source frame intersecting the interval.
  const sourceFrame = Number(
    selectedFrame < exclusiveEndFrame ? selectedFrame : exclusiveEndFrame - 1n,
  );

  if (!Number.isSafeInteger(sourceFrame)) {
    throw new RangeError(
      "selected source frame exceeds JavaScript's exact integer range",
    );
  }
  return sourceFrame;
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
