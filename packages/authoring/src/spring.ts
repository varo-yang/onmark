// Closed-form spring sampling over exact local frame facts.
// Direct evaluation stays independent of DOM state and prior seek order.

import type { PresentationExtensionContext } from "./motion.js";

export interface SpringFrame {
  readonly frameRate: PresentationExtensionContext["frameRate"];
  readonly localFrame: number;
}

export interface SpringOptions {
  /** Damping coefficient. Defaults to 10. */
  readonly damping?: number;
  /** Initial progress velocity per second toward the target. Defaults to zero. */
  readonly initialVelocity?: number;
  /** Moving mass. Defaults to one. */
  readonly mass?: number;
  /** Whether progress may cross the zero-to-one visual range. */
  readonly overshoot?: "allow" | "clamp";
  /** Restoring-force stiffness. Defaults to 100. */
  readonly stiffness?: number;
}

interface SpringParameters {
  readonly damping: number;
  readonly initialVelocity: number;
  readonly mass: number;
  readonly overshoot: "allow" | "clamp";
  readonly stiffness: number;
}

const DEFAULT_SPRING = Object.freeze({
  damping: 10,
  initialVelocity: 0,
  mass: 1,
  overshoot: "allow" as const,
  stiffness: 100,
});

/** Samples a damped physical spring directly at one exact local frame. */
export function spring(
  frame: SpringFrame,
  options: SpringOptions = {},
): number {
  validateFrame(frame);
  const parameters = springParameters(options);
  const seconds =
    (frame.localFrame / frame.frameRate.numerator) *
    frame.frameRate.denominator;
  const displacement = springDisplacement(seconds, parameters);
  const progress = 1 - displacement;
  requireFinite(progress, "spring progress must be finite");

  if (parameters.overshoot === "allow") {
    return progress;
  }
  return Math.max(0, Math.min(progress, 1));
}

function springDisplacement(
  seconds: number,
  parameters: SpringParameters,
): number {
  const frequency = Math.sqrt(parameters.stiffness / parameters.mass);
  const decay = parameters.damping / (2 * parameters.mass);
  const criticalTolerance = frequency * 1e-7;

  if (decay < frequency - criticalTolerance) {
    return underDampedDisplacement(seconds, frequency, decay, parameters);
  }
  if (decay > frequency + criticalTolerance) {
    return overDampedDisplacement(seconds, frequency, decay, parameters);
  }
  // The general solutions divide by the vanishing distance between roots.
  // Their critical limit is both equivalent and numerically stable here.
  return criticallyDampedDisplacement(seconds, frequency, parameters);
}

function underDampedDisplacement(
  seconds: number,
  frequency: number,
  decay: number,
  parameters: SpringParameters,
): number {
  const dampedFrequency = Math.sqrt(frequency ** 2 - decay ** 2);
  const velocityTerm = (decay - parameters.initialVelocity) / dampedFrequency;
  return (
    Math.exp(-decay * seconds) *
    (Math.cos(dampedFrequency * seconds) +
      velocityTerm * Math.sin(dampedFrequency * seconds))
  );
}

function criticallyDampedDisplacement(
  seconds: number,
  decay: number,
  parameters: SpringParameters,
): number {
  const velocityTerm = decay - parameters.initialVelocity;
  return Math.exp(-decay * seconds) * (1 + velocityTerm * seconds);
}

function overDampedDisplacement(
  seconds: number,
  frequency: number,
  decay: number,
  parameters: SpringParameters,
): number {
  const root = Math.sqrt(decay ** 2 - frequency ** 2);
  const slowRoot = -decay + root;
  const fastRoot = -decay - root;
  const slowWeight =
    (-parameters.initialVelocity - fastRoot) / (slowRoot - fastRoot);
  const fastWeight = 1 - slowWeight;
  return (
    slowWeight * Math.exp(slowRoot * seconds) +
    fastWeight * Math.exp(fastRoot * seconds)
  );
}

function springParameters(options: SpringOptions): SpringParameters {
  const parameters = {
    damping: options.damping ?? DEFAULT_SPRING.damping,
    initialVelocity: options.initialVelocity ?? DEFAULT_SPRING.initialVelocity,
    mass: options.mass ?? DEFAULT_SPRING.mass,
    overshoot: options.overshoot ?? DEFAULT_SPRING.overshoot,
    stiffness: options.stiffness ?? DEFAULT_SPRING.stiffness,
  };
  requirePositive(parameters.damping, "spring damping");
  requireFinite(
    parameters.initialVelocity,
    "spring initial velocity must be finite",
  );
  requirePositive(parameters.mass, "spring mass");
  requirePositive(parameters.stiffness, "spring stiffness");
  if (parameters.overshoot !== "allow" && parameters.overshoot !== "clamp") {
    throw new RangeError("spring overshoot must be allow or clamp");
  }
  return parameters;
}

function validateFrame(frame: SpringFrame): void {
  if (!Number.isSafeInteger(frame.localFrame) || frame.localFrame < 0) {
    throw new RangeError(
      "spring local frame must be a non-negative safe integer",
    );
  }
  if (
    !Number.isSafeInteger(frame.frameRate.numerator) ||
    !Number.isSafeInteger(frame.frameRate.denominator) ||
    frame.frameRate.numerator <= 0 ||
    frame.frameRate.denominator <= 0
  ) {
    throw new RangeError(
      "spring frame rate must contain positive safe integers",
    );
  }
}

function requirePositive(value: number, label: string): void {
  if (!Number.isFinite(value) || value <= 0) {
    throw new RangeError(`${label} must be positive and finite`);
  }
}

function requireFinite(value: number, message: string): void {
  if (!Number.isFinite(value)) {
    throw new RangeError(message);
  }
}
