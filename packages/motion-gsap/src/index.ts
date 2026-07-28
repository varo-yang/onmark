// Exact-frame GSAP integration over Onmark's vendor-neutral extension contract.
// Authors describe local motion; this adapter owns paused playheads and cleanup.

import { gsap } from "gsap";
import type {
  PresentationExtension,
  PresentationExtensionContext,
  PresentationElementTargetKind,
  PresentationTarget,
  PresentationTransitionTarget,
} from "@onmark/authoring/types";

// ── Public contract ──

/** Local GSAP authoring facts for one semantic screenplay element. */
export interface GsapMotionContext {
  readonly durationSeconds: number;
  readonly element: HTMLElement;
  readonly timeline: ReturnType<typeof gsap.timeline>;
}

/** Adds local animation to one Onmark-owned paused timeline. */
export type GsapMotionHandler = (context: GsapMotionContext) => void;

/** Local GSAP transition facts with both adjacent shot elements. */
export interface GsapTransitionContext extends GsapMotionContext {
  readonly incomingElement: HTMLElement;
  readonly outgoingElement: HTMLElement;
}

/** Adds local animation across one compiler-owned shot overlap. */
export type GsapTransitionHandler = (context: GsapTransitionContext) => void;

/** Semantic handlers and optional selector handlers for local motion. */
export type GsapMotionDefinition = Readonly<
  Partial<Record<PresentationElementTargetKind, GsapMotionHandler>> & {
    readonly selectors?: Readonly<Record<string, GsapMotionHandler>>;
    readonly transition?: GsapTransitionHandler;
  }
>;

interface GsapMotionRules {
  readonly kinds: Readonly<
    Record<PresentationElementTargetKind, GsapMotionHandler | undefined>
  >;
  readonly selectors: readonly GsapSelectorRule[];
  readonly transition: GsapTransitionHandler | undefined;
}

interface GsapSelectorRule {
  readonly animate: GsapMotionHandler;
  readonly selector: string;
}

interface GsapFrame {
  readonly index: number;
}

interface GsapFrameEffect {
  apply(frame: GsapFrame): void;
  dispose(): void;
}

interface MatchingGsapMotion {
  readonly elements: readonly GsapMotionHandler[];
  readonly transition: GsapTransitionHandler | undefined;
}

/** Creates exact-frame GSAP effects without exposing runtime lifecycle code. */
export function gsapMotion(
  definition: GsapMotionDefinition,
): PresentationExtension {
  const rules = ownRules(definition);
  return Object.freeze({
    bind(context: PresentationExtensionContext) {
      return {
        effects: bindGsapEffects(rules, context),
        resources: [],
      };
    },
  });
}

// ── Element motion ──

function bindGsapEffects(
  rules: GsapMotionRules,
  context: PresentationExtensionContext,
): readonly GsapFrameEffect[] {
  const effects: GsapFrameEffect[] = [];
  try {
    for (const target of context.targets) {
      const handlers = matchingHandlers(rules, target);
      if (handlers.elements.length === 0 && handlers.transition === undefined) {
        continue;
      }
      effects.push(bindGsapEffect(handlers, target, context));
    }
  } catch (error) {
    const cleanupFailure = releaseGsapEffects(effects);
    if (cleanupFailure !== undefined) {
      throw new AggregateError(
        [error, cleanupFailure],
        "GSAP motion binding failed and cleanup was incomplete",
      );
    }
    throw error;
  }
  return Object.freeze(effects);
}

function bindGsapEffect(
  handlers: MatchingGsapMotion,
  target: PresentationTarget,
  context: PresentationExtensionContext,
): GsapFrameEffect {
  const durationSeconds = intervalSeconds(target, context);
  const timeline = gsap.timeline({ paused: true });

  try {
    const motion: GsapMotionContext = Object.freeze({
      durationSeconds,
      element: target.element,
      timeline,
    });
    if (target.kind === "transition" && handlers.transition !== undefined) {
      handlers.transition(transitionContext(motion, target));
    }
    for (const handler of handlers.elements) {
      handler(motion);
    }
    requireLocalTimeline(timeline, durationSeconds, target.kind);
    // Add the full compiler-owned domain only after author construction. A
    // leading sentinel would move GSAP's default append position to the end.
    const sentinel = { value: 0 };
    timeline.to(
      sentinel,
      { duration: durationSeconds, ease: "none", value: 0 },
      0,
    );
  } catch (error) {
    try {
      timeline.kill();
    } catch (cleanupFailure) {
      throw new AggregateError(
        [error, cleanupFailure],
        "GSAP motion binding failed and cleanup was incomplete",
      );
    }
    throw error;
  }

  return {
    apply(frame): void {
      if (
        frame.index < target.interval.start ||
        frame.index >= target.interval.end
      ) {
        return;
      }
      // A fresh GSAP timeline already reports time zero, and `time(0)` is a
      // no-op. Force rendering so a first-frame `.set()` and a repeated exact
      // frame both restore the authored state.
      timeline.render(localSeconds(frame, target, context), true, true);
    },
    dispose(): void {
      timeline.kill();
    },
  };
}

function ownRules(definition: GsapMotionDefinition): GsapMotionRules {
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

function ownSelector([selector, animate]: [
  string,
  GsapMotionHandler,
]): GsapSelectorRule {
  if (selector.trim().length === 0) {
    throw new RangeError("GSAP motion selector cannot be blank");
  }
  return Object.freeze({ animate, selector });
}

function matchingHandlers(
  rules: GsapMotionRules,
  target: PresentationTarget,
): MatchingGsapMotion {
  const handlers: GsapMotionHandler[] = [];
  const kind =
    target.kind === "transition" ? undefined : rules.kinds[target.kind];
  if (kind !== undefined) {
    handlers.push(kind);
  }
  for (const rule of rules.selectors) {
    if (target.element.matches(rule.selector)) {
      handlers.push(rule.animate);
    }
  }
  return Object.freeze({
    elements: Object.freeze(handlers),
    transition: target.kind === "transition" ? rules.transition : undefined,
  });
}

// ── Exact-frame playheads ──

function intervalSeconds(
  target: PresentationTarget,
  context: PresentationExtensionContext,
): number {
  const frames = target.interval.end - target.interval.start;
  return (frames / context.frameRate.numerator) * context.frameRate.denominator;
}

function localSeconds(
  frame: GsapFrame,
  target: PresentationTarget,
  context: PresentationExtensionContext,
): number {
  const durationFrames = target.interval.end - target.interval.start;
  const localFrame = Math.max(
    0,
    Math.min(frame.index - target.interval.start, durationFrames),
  );
  if (target.kind === "transition") {
    const finalFrame = durationFrames - 1;
    const progress = finalFrame === 0 ? 1 : localFrame / finalFrame;
    return intervalSeconds(target, context) * progress;
  }
  return (
    (localFrame / context.frameRate.numerator) * context.frameRate.denominator
  );
}

function transitionContext(
  context: GsapMotionContext,
  target: PresentationTransitionTarget,
): GsapTransitionContext {
  return Object.freeze({
    ...context,
    incomingElement: target.incomingElement,
    outgoingElement: target.outgoingElement,
  });
}

function requireLocalTimeline(
  timeline: gsap.core.Timeline,
  durationSeconds: number,
  label: string,
): void {
  if (timeline.duration() <= durationSeconds) {
    return;
  }
  throw new RangeError(
    `${label} motion exceeds its compiler-owned interval of ${durationSeconds} seconds`,
  );
}

function releaseGsapEffects(
  effects: readonly GsapFrameEffect[],
): unknown | undefined {
  let failure: unknown;
  for (const effect of effects.toReversed()) {
    try {
      effect.dispose();
    } catch (error) {
      failure ??= error;
    }
  }
  return failure;
}
