// Browser-media projection from exact Rust-owned frame facts.
// It selects source frames without becoming a second timeline solver.

import type { RuntimeFrame } from "./clock.js";
import type { RuntimePlan } from "./session.js";

type Video = RuntimePlan["videos"][number];
type FrameRate = RuntimePlan["frameRate"];

/** One source frame and an interior seek time that cannot hit its boundary. */
export interface VideoFrameSelection {
  readonly mediaTimeSeconds: number;
  readonly seekTimeSeconds: number;
}

/** Selects the source frame visible at one output-frame midpoint. */
export function videoFrameSelection(
  frame: RuntimeFrame,
  video: Video,
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
  );
  const sourceFrameDuration =
    video.sourceFrameRate.denominator / video.sourceFrameRate.numerator;

  return Object.freeze({
    mediaTimeSeconds: sourceFrame * sourceFrameDuration,
    seekTimeSeconds: (sourceFrame + 0.5) * sourceFrameDuration,
  });
}

function sourceFrameAtMidpoint(
  localFrame: number,
  outputFrameRate: FrameRate,
  sourceFrameRate: FrameRate,
): number {
  const numerator =
    (2n * BigInt(localFrame) + 1n) *
    BigInt(outputFrameRate.denominator) *
    BigInt(sourceFrameRate.numerator);
  const denominator =
    2n *
    BigInt(outputFrameRate.numerator) *
    BigInt(sourceFrameRate.denominator);
  const sourceFrame = Number(numerator / denominator);

  if (!Number.isSafeInteger(sourceFrame)) {
    throw new RangeError(
      "selected source frame exceeds JavaScript's exact integer range",
    );
  }
  return sourceFrame;
}
