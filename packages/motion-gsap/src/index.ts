// Exact-frame GSAP integration over Onmark's vendor-neutral extension contract.
// Authors describe local motion; this adapter owns paused playheads and cleanup.

import { gsap } from "gsap";
import type {
  PresentationExtension,
  PresentationExtensionContext,
  PresentationTarget,
  PresentationTargetKind,
} from "@onmark/authoring/types";

// ── Public contract ──

/** Local GSAP authoring facts for one semantic screenplay element. */
export interface GsapMotionContext {
  readonly durationSeconds: number;
  readonly element: HTMLElement;
  readonly timeline: gsap.core.Timeline;
}

/** Adds local animation to one Onmark-owned paused timeline. */
export type GsapMotionHandler = (context: GsapMotionContext) => void;

/** Semantic handlers and optional selector handlers for local motion. */
export type GsapMotionDefinition = Readonly<
  Partial<Record<PresentationTargetKind, GsapMotionHandler>> & {
    readonly selectors?: Readonly<Record<string, GsapMotionHandler>>;
  }
>;

interface GsapMotionRules {
  readonly kinds: Readonly<
    Record<PresentationTargetKind, GsapMotionHandler | undefined>
  >;
  readonly selectors: readonly GsapSelectorRule[];
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
      if (handlers.length === 0) {
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
  handlers: readonly GsapMotionHandler[],
  target: PresentationTarget,
  context: PresentationExtensionContext,
): GsapFrameEffect {
  const durationSeconds = intervalSeconds(target, context);
  const timeline = gsap.timeline({ paused: true });

  try {
    const motion = Object.freeze({
      durationSeconds,
      element: target.element,
      timeline,
    });
    for (const handler of handlers) {
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
      timeline.time(localSeconds(frame, target, context), true);
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
  return Object.freeze({ kinds, selectors });
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
): readonly GsapMotionHandler[] {
  const handlers: GsapMotionHandler[] = [];
  const kind = rules.kinds[target.kind];
  if (kind !== undefined) {
    handlers.push(kind);
  }
  for (const rule of rules.selectors) {
    if (target.element.matches(rule.selector)) {
      handlers.push(rule.animate);
    }
  }
  return handlers;
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
  return (
    (localFrame / context.frameRate.numerator) * context.frameRate.denominator
  );
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
