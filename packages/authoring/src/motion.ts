// Vendor-neutral motion composition over compiler-owned semantic targets.
// Each extension owns the resources and exact-frame effects it creates.

import type {
  FrameEffect,
  PresentationExtensions,
  PresentationResource,
  RuntimeNode,
  RuntimePlan,
} from "@onmark/runtime/types";

export type PresentationElementTargetKind =
  "film" | "scene" | "shot" | "video" | "title" | "callToAction" | "caption";

export type PresentationTargetKind =
  PresentationElementTargetKind | "transition";

interface PresentationTargetBase {
  readonly element: HTMLElement;
  readonly interval: RuntimePlan["evaluation"];
  readonly node: RuntimeNode;
}

/** One semantic element and its compiler-owned active interval. */
export interface PresentationElementTarget extends PresentationTargetBase {
  readonly kind: PresentationElementTargetKind;
}

/** One transition marker and the exact adjacent elements it relates. */
export interface PresentationTransitionTarget extends PresentationTargetBase {
  readonly kind: "transition";
  readonly incomingElement: HTMLElement;
  readonly outgoingElement: HTMLElement;
}

/** Closed browser-owned view of one compiler-projected semantic target. */
export type PresentationTarget =
  PresentationElementTarget | PresentationTransitionTarget;

/** Complete immutable DOM view delivered after all solved facts are bound. */
export interface PresentationExtensionContext {
  readonly frameRate: RuntimePlan["frameRate"];
  readonly targets: readonly PresentationTarget[];
}

/** One optional browser integration over the fully bound semantic document. */
export interface PresentationExtension {
  bind(
    context: PresentationExtensionContext,
  ): PresentationExtensions | Promise<PresentationExtensions>;
}

/** Combines extensions in declaration order with terminal partial cleanup. */
export function combineMotion(
  ...extensions: readonly PresentationExtension[]
): PresentationExtension {
  const owned = extensions.map(ownExtension);
  return Object.freeze({
    bind(context: PresentationExtensionContext) {
      return bindExtensions(owned, context);
    },
  });
}

export const EMPTY_PRESENTATION_EXTENSIONS: PresentationExtensions =
  Object.freeze({
    effects: Object.freeze([]),
    resources: Object.freeze([]),
  });

export function ownExtension(
  extension: PresentationExtension,
): PresentationExtension {
  return Object.freeze({ bind: extension.bind.bind(extension) });
}

export function ownExtensions(
  extensions: PresentationExtensions,
): PresentationExtensions {
  return Object.freeze({
    effects: Object.freeze(extensions.effects.map(ownEffect)),
    resources: Object.freeze(extensions.resources.map(ownResource)),
  });
}

function ownEffect(effect: FrameEffect): FrameEffect {
  return Object.freeze({
    apply: effect.apply.bind(effect),
    dispose: effect.dispose.bind(effect),
  });
}

function ownResource(resource: PresentationResource): PresentationResource {
  return Object.freeze({
    id: resource.id,
    kind: resource.kind,
    prepare: resource.prepare.bind(resource),
    dispose: resource.dispose.bind(resource),
  });
}

async function bindExtensions(
  extensions: readonly PresentationExtension[],
  context: PresentationExtensionContext,
): Promise<PresentationExtensions> {
  const bound: PresentationExtensions[] = [];
  try {
    for (const extension of extensions) {
      const result = await extension.bind(context);
      bound.push(ownExtensions(result));
    }
  } catch (error) {
    const cleanupFailure = await releaseExtensions(bound);
    if (cleanupFailure !== undefined) {
      throw new AggregateError(
        [error, cleanupFailure],
        "presentation extension binding failed and cleanup was incomplete",
      );
    }
    throw error;
  }

  return ownExtensions({
    effects: bound.flatMap((extension) => extension.effects),
    resources: bound.flatMap((extension) => extension.resources),
  });
}

async function releaseExtensions(
  extensions: readonly PresentationExtensions[],
): Promise<unknown | undefined> {
  let failure: unknown;
  for (const extension of extensions.toReversed()) {
    for (const effect of extension.effects.toReversed()) {
      const releaseFailure = await release(effect.dispose.bind(effect));
      failure ??= releaseFailure;
    }
    for (const resource of extension.resources.toReversed()) {
      const releaseFailure = await release(resource.dispose.bind(resource));
      failure ??= releaseFailure;
    }
  }
  return failure;
}

async function release(
  operation: () => void | Promise<void>,
): Promise<unknown | undefined> {
  try {
    await operation();
    return undefined;
  } catch (error) {
    return error;
  }
}
