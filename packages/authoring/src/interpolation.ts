// Piecewise visual-value interpolation for native exact-frame motion.
// The closed edge policy prevents implicit extrapolation beyond authored ranges.

export type EasingFunction = (progress: number) => number;
export type Extrapolation = "clamp" | "extend";

export interface InterpolationOptions {
  /** Maps normalized segment progress before output interpolation. */
  readonly easing?: EasingFunction;
  /** Defaults to clamp. */
  readonly extrapolateLeft?: Extrapolation;
  /** Defaults to clamp. */
  readonly extrapolateRight?: Extrapolation;
}

/** Stateless easing functions for native exact-frame motion. */
export const easing = Object.freeze({
  inCubic(progress: number): number {
    return progress ** 3;
  },
  inOutCubic(progress: number): number {
    if (progress < 0.5) {
      return 4 * progress ** 3;
    }
    return 1 - (-2 * progress + 2) ** 3 / 2;
  },
  linear(progress: number): number {
    return progress;
  },
  outCubic(progress: number): number {
    return 1 - (1 - progress) ** 3;
  },
});

/** Maps a scalar through one piecewise-linear visual domain. */
export function interpolate(
  input: number,
  inputRange: readonly number[],
  outputRange: readonly number[],
  options: InterpolationOptions = {},
): number {
  validateInterpolation(input, inputRange, outputRange);
  validateOptions(options);
  const value = applyExtrapolation(input, inputRange, options);
  const segment = segmentFor(value, inputRange);
  const inputStart = rangeValue(inputRange, segment);
  const inputEnd = rangeValue(inputRange, segment + 1);
  const outputStart = rangeValue(outputRange, segment);
  const outputEnd = rangeValue(outputRange, segment + 1);
  const progress = (value - inputStart) / (inputEnd - inputStart);
  const eased = (options.easing ?? easing.linear)(progress);
  requireFinite(eased, "interpolation easing must return finite progress");

  const output = outputStart + (outputEnd - outputStart) * eased;
  requireFinite(output, "interpolation output must be finite");
  return output;
}

function validateOptions(options: InterpolationOptions): void {
  if (options.easing !== undefined && typeof options.easing !== "function") {
    throw new RangeError("interpolation easing must be a function");
  }
  validateExtrapolation(options.extrapolateLeft, "left");
  validateExtrapolation(options.extrapolateRight, "right");
}

function validateExtrapolation(
  extrapolation: Extrapolation | undefined,
  edge: string,
): void {
  if (
    extrapolation !== undefined &&
    extrapolation !== "clamp" &&
    extrapolation !== "extend"
  ) {
    throw new RangeError(
      `interpolation ${edge} extrapolation must be clamp or extend`,
    );
  }
}

function validateInterpolation(
  input: number,
  inputRange: readonly number[],
  outputRange: readonly number[],
): void {
  requireFinite(input, "interpolation input must be finite");
  if (inputRange.length < 2) {
    throw new RangeError("interpolation ranges need at least two values");
  }
  if (inputRange.length !== outputRange.length) {
    throw new RangeError("interpolation ranges must have the same length");
  }

  for (let index = 0; index < inputRange.length; index += 1) {
    const inputValue = rangeValue(inputRange, index);
    const outputValue = rangeValue(outputRange, index);
    requireFinite(inputValue, "interpolation input range must be finite");
    requireFinite(outputValue, "interpolation output range must be finite");
    if (index > 0 && inputValue <= rangeValue(inputRange, index - 1)) {
      throw new RangeError(
        "interpolation input range must be strictly increasing",
      );
    }
  }
}

function applyExtrapolation(
  input: number,
  inputRange: readonly number[],
  options: InterpolationOptions,
): number {
  const first = rangeValue(inputRange, 0);
  const last = rangeValue(inputRange, inputRange.length - 1);
  if (input < first && (options.extrapolateLeft ?? "clamp") === "clamp") {
    return first;
  }
  if (input > last && (options.extrapolateRight ?? "clamp") === "clamp") {
    return last;
  }
  return input;
}

function segmentFor(input: number, inputRange: readonly number[]): number {
  for (let index = 1; index < inputRange.length; index += 1) {
    if (input <= rangeValue(inputRange, index)) {
      return index - 1;
    }
  }
  return inputRange.length - 2;
}

function requireFinite(value: number, message: string): void {
  if (!Number.isFinite(value)) {
    throw new RangeError(message);
  }
}

function rangeValue(range: readonly number[], index: number): number {
  const value = range[index];
  if (value === undefined) {
    throw new RangeError("interpolation range is incomplete");
  }
  return value;
}
