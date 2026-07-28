// Vendor-neutral exact-frame motion over compiler-owned semantic intervals.
// Each sample is local to one target, so presentation code cannot reschedule it.

import type { FrameEffect, RuntimeFrame } from "@onmark/runtime/types";
import type {
  PresentationExtension,
  PresentationExtensionContext,
  PresentationElementTargetKind,
  PresentationTarget,
  PresentationTransitionTarget,
} from "./motion.js";

/** One immutable sample inside a semantic element's solved interval. */
export interface FrameMotionContext {
  readonly durationFrames: number;
  readonly element: HTMLElement;
  readonly frameRate: PresentationExtensionContext["frameRate"];
  readonly localFrame: number;
  readonly progress: number;
}

/** Applies browser-owned visual state for one exact local frame. */
export type FrameMotionHandler = (context: FrameMotionContext) => void;

/** Exact-frame transition sample with both related shot elements. */
export interface TransitionFrameMotionContext extends FrameMotionContext {
  readonly incomingElement: HTMLElement;
  readonly outgoingElement: HTMLElement;
}

/** Applies browser-owned transition state for one exact overlap frame. */
export type TransitionFrameMotionHandler = (
  context: TransitionFrameMotionContext,
) => void;

/** Semantic handlers and optional selector handlers for exact-frame motion. */
export type FrameMotionDefinition = Readonly<
  Partial<Record<PresentationElementTargetKind, FrameMotionHandler>> & {
    readonly selectors?: Readonly<Record<string, FrameMotionHandler>>;
    readonly transition?: TransitionFrameMotionHandler;
  }
>;

interface FrameMotionRules {
  readonly kinds: Readonly<
    Record<PresentationElementTargetKind, FrameMotionHandler | undefined>
  >;
  readonly selectors: readonly FrameMotionSelector[];
  readonly transition: TransitionFrameMotionHandler | undefined;
}

interface FrameMotionSelector {
  readonly apply: FrameMotionHandler;
  readonly selector: string;
}

interface MatchingFrameMotion {
  readonly elements: readonly FrameMotionHandler[];
  readonly transition: TransitionFrameMotionHandler | undefined;
}

/** Creates local browser effects driven directly by exact runtime frames. */
export function frameMotion(
  definition: FrameMotionDefinition,
): PresentationExtension {
  const rules = ownRules(definition);
  return Object.freeze({
    bind(context: PresentationExtensionContext) {
      return {
        effects: bindEffects(rules, context),
        resources: [],
      };
    },
  });
}

function bindEffects(
  rules: FrameMotionRules,
  context: PresentationExtensionContext,
): readonly FrameEffect[] {
  const effects: FrameEffect[] = [];
  for (const target of context.targets) {
    const handlers = matchingHandlers(rules, target);
    if (handlers.elements.length > 0 || handlers.transition !== undefined) {
      effects.push(bindEffect(target, handlers, context.frameRate));
    }
  }
  return Object.freeze(effects);
}

function bindEffect(
  target: PresentationTarget,
  handlers: MatchingFrameMotion,
  frameRate: PresentationExtensionContext["frameRate"],
): FrameEffect {
  const durationFrames = target.interval.end - target.interval.start;
  return Object.freeze({
    apply(frame: RuntimeFrame): void {
      const localFrame = frame.index - target.interval.start;
      if (localFrame < 0 || localFrame >= durationFrames) {
        return;
      }
      const sample: FrameMotionContext = Object.freeze({
        durationFrames,
        element: target.element,
        frameRate,
        localFrame,
        progress: targetProgress(target, localFrame, durationFrames),
      });
      if (target.kind === "transition" && handlers.transition !== undefined) {
        handlers.transition(transitionSample(sample, target));
      }
      for (const handler of handlers.elements) {
        handler(sample);
      }
    },
    dispose(): void {
      // Direct frame motion retains no vendor playhead or browser resource.
    },
  });
}

function ownRules(definition: FrameMotionDefinition): FrameMotionRules {
  const kinds = Object.freeze({
    film: definition.film,
    scene: definition.scene,
    shot: definition.shot,
    video: definition.video,
    title: definition.title,
    callToAction: definition.callToAction,
    caption: definition.caption,
  });
  const selectors = Object.freeze(
    Object.entries(definition.selectors ?? {}).map(ownSelector),
  );
  return Object.freeze({
    kinds,
    selectors,
    transition: definition.transition,
  });
}

function ownSelector([selector, apply]: [
  string,
  FrameMotionHandler,
]): FrameMotionSelector {
  if (selector.trim().length === 0) {
    throw new RangeError("exact-frame motion selector cannot be blank");
  }
  return Object.freeze({ apply, selector });
}

function matchingHandlers(
  rules: FrameMotionRules,
  target: PresentationTarget,
): MatchingFrameMotion {
  const handlers: FrameMotionHandler[] = [];
  const kind =
    target.kind === "transition" ? undefined : rules.kinds[target.kind];
  if (kind !== undefined) {
    handlers.push(kind);
  }
  for (const rule of rules.selectors) {
    if (target.element.matches(rule.selector)) {
      handlers.push(rule.apply);
    }
  }
  return Object.freeze({
    elements: Object.freeze(handlers),
    transition: target.kind === "transition" ? rules.transition : undefined,
  });
}

function targetProgress(
  target: PresentationTarget,
  localFrame: number,
  durationFrames: number,
): number {
  if (target.kind !== "transition") {
    return localFrame / durationFrames;
  }
  return durationFrames === 1 ? 1 : localFrame / (durationFrames - 1);
}

function transitionSample(
  sample: FrameMotionContext,
  target: PresentationTransitionTarget,
): TransitionFrameMotionContext {
  return Object.freeze({
    ...sample,
    incomingElement: target.incomingElement,
    outgoingElement: target.outgoingElement,
  });
}
