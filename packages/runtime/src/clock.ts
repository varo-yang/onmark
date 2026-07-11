// Deterministic projection from exact protocol frames to browser API time.
// Integral frame identity remains authoritative; floating seconds are an edge value only.

import type { WireFrameRate } from "./generated/browser-request.js";

/** One exact frame together with its browser-facing time projection. */
export interface RuntimeFrame {
  /** Absolute frame identity selected by the native executor. */
  readonly index: number;
  /** Seconds derived from the exact rational rate for browser APIs. */
  readonly timeSeconds: number;
}

/** Projects one exact frame through the Rust-owned rational frame rate. */
export function runtimeFrameAt(
  index: number,
  frameRate: WireFrameRate,
): RuntimeFrame {
  safeInteger(index, "frame index", { allowZero: true });
  safeInteger(frameRate.numerator, "frame-rate numerator");
  safeInteger(frameRate.denominator, "frame-rate denominator");

  // Divide before multiplying so exact wire integers are not first combined
  // into an integer beyond JavaScript's safe range. The resulting Number is
  // used only at browser APIs; scheduling and identity continue to use index.
  const timeSeconds = (index / frameRate.numerator) * frameRate.denominator;
  return Object.freeze({ index, timeSeconds });
}

interface IntegerOptions {
  readonly allowZero?: boolean;
}

function safeInteger(
  value: number,
  role: string,
  options: IntegerOptions = {},
): void {
  const minimum = options.allowZero === true ? 0 : 1;
  if (!Number.isSafeInteger(value) || value < minimum) {
    throw new TypeError(`${role} must be a safe integer at least ${minimum}`);
  }
}
