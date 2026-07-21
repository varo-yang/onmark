// Bounded readiness and terminal cleanup for presentation-owned resources.
// Resources carry browser effects only; Rust-owned plan facts remain authoritative.

import {
  MAX_PENDING_RESOURCE_CHARACTERS,
  MAX_PENDING_RESOURCES,
} from "./generated/runtime-contract.js";
import { RuntimeAdapterError } from "./session.js";

// ── Public contract ──

const MAX_READINESS_TIMEOUT_MILLISECONDS = 24 * 60 * 60 * 1_000;
const MAX_RESOURCE_ID_CHARACTERS = 256;

export const PRESENTATION_RESOURCE_KINDS = Object.freeze([
  "image",
  "font",
  "texture",
  "custom",
] as const);

export type PresentationResourceKind =
  (typeof PRESENTATION_RESOURCE_KINDS)[number];

/** Maximum resources retained by one presentation adapter. */
export const MAX_PRESENTATION_RESOURCES = MAX_PENDING_RESOURCES;

/** One browser resource whose readiness is required before frame capture. */
export interface PresentationResource {
  readonly id: string;
  readonly kind: PresentationResourceKind;
  /** Resolves only after the resource is stable for deterministic capture. */
  prepare(): void | Promise<void>;
  /** Cancels pending work and releases every browser resource it owns. */
  dispose(): void | Promise<void>;
}

type ReadinessOutcome =
  | { readonly kind: "ready" }
  | { readonly kind: "failed"; readonly error: RuntimeAdapterError }
  | { readonly kind: "timedOut"; readonly pendingResource: string };

// ── Collection lifecycle ──

/** Takes an immutable snapshot before validation or asynchronous work begins. */
export function ownPresentationResources(
  resources: readonly PresentationResource[],
): readonly PresentationResource[] {
  return Object.freeze(
    resources.map((resource) => {
      const dispose = resource.dispose.bind(resource);
      const prepare = resource.prepare.bind(resource);
      return Object.freeze({
        id: resource.id,
        kind: resource.kind,
        dispose,
        prepare,
      });
    }),
  );
}

/** Rejects resource collections that cannot fit the runtime failure contract. */
export function validatePresentationResources(
  resources: readonly PresentationResource[],
): void {
  if (resources.length > MAX_PRESENTATION_RESOURCES) {
    throw new RuntimeAdapterError(
      "operation",
      "presentation resource count exceeds its limit",
    );
  }

  const identities = new Set<string>();
  for (const resource of resources) {
    const identity = resourceIdentity(resource);
    if (identities.has(identity)) {
      throw new RuntimeAdapterError(
        "operation",
        "presentation resource identity is duplicated",
      );
    }
    identities.add(identity);
  }
}

/** Waits for every resource under one bounded, deterministic failure policy. */
export async function preparePresentationResources(
  resources: readonly PresentationResource[],
  timeoutMilliseconds: number,
): Promise<void> {
  const outcomes = await Promise.all(
    resources.map((resource) => prepareResource(resource, timeoutMilliseconds)),
  );
  let failure: RuntimeAdapterError | undefined;
  const pendingResources: string[] = [];
  for (const outcome of outcomes) {
    if (outcome.kind === "failed") {
      failure ??= outcome.error;
    } else if (outcome.kind === "timedOut") {
      pendingResources.push(outcome.pendingResource);
    }
  }
  if (failure !== undefined) {
    throw withPendingResources(failure, pendingResources);
  }
  if (pendingResources.length > 0) {
    throw new RuntimeAdapterError(
      "readinessTimeout",
      "presentation resources did not become ready",
      pendingResources,
    );
  }
}

function withPendingResources(
  failure: RuntimeAdapterError,
  pendingResources: readonly string[],
): RuntimeAdapterError {
  if (pendingResources.length === 0) {
    return failure;
  }
  // Collection-owned identities take precedence when a custom failure already
  // carries its own pending details; this keeps every declared resource within
  // the protocol's fixed collection budget.
  return new RuntimeAdapterError(
    failure.kind,
    failure.message,
    pendingResources,
  );
}

/** Releases all owned resources while retaining the first cleanup failure. */
export async function releasePresentationResources(
  resources: readonly PresentationResource[],
): Promise<unknown | undefined> {
  let failure: unknown;
  for (const resource of resources) {
    try {
      await resource.dispose();
    } catch (error) {
      failure ??= error;
    }
  }
  return failure;
}

/** Validates the shared browser-readiness deadline before resources are bound. */
export function requireReadinessTimeout(timeoutMilliseconds: number): void {
  if (
    !Number.isSafeInteger(timeoutMilliseconds) ||
    timeoutMilliseconds <= 0 ||
    timeoutMilliseconds > MAX_READINESS_TIMEOUT_MILLISECONDS
  ) {
    throw new TypeError(
      "readiness timeout must be a positive integer no greater than one day",
    );
  }
}

// ── Individual readiness ──

async function prepareResource(
  resource: PresentationResource,
  timeoutMilliseconds: number,
): Promise<ReadinessOutcome> {
  const pendingResource = `${resourceIdentity(resource)}:prepare`;
  let deadline: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<ReadinessOutcome>((resolve) => {
    deadline = setTimeout(
      () => resolve({ kind: "timedOut", pendingResource }),
      timeoutMilliseconds,
    );
  });
  const preparation = observePreparation(resource, pendingResource);

  try {
    return await Promise.race([preparation, timeout]);
  } finally {
    clearTimeout(deadline);
  }
}

async function observePreparation(
  resource: PresentationResource,
  pendingResource: string,
): Promise<ReadinessOutcome> {
  try {
    // Install every resource deadline before author code can run.
    await Promise.resolve();
    await resource.prepare();
    return { kind: "ready" };
  } catch (error) {
    return {
      kind: "failed",
      error: resourceFailure(error, pendingResource),
    };
  }
}

function resourceIdentity(resource: PresentationResource): string {
  if (typeof resource.id !== "string") {
    throw new RuntimeAdapterError(
      "operation",
      "presentation resource id must be a string",
    );
  }
  if (!PRESENTATION_RESOURCE_KINDS.includes(resource.kind)) {
    throw new RuntimeAdapterError(
      "operation",
      "presentation resource kind is invalid",
    );
  }
  if (resource.id.length === 0 || resource.id !== resource.id.trim()) {
    throw new RuntimeAdapterError(
      "operation",
      "presentation resource id must be nonblank and trimmed",
    );
  }

  let characters = 0;
  for (const _character of resource.id) {
    characters += 1;
    if (characters > MAX_RESOURCE_ID_CHARACTERS) {
      throw new RuntimeAdapterError(
        "operation",
        "presentation resource id exceeds its character limit",
      );
    }
  }

  const identity = `${resource.kind}:${resource.id}`;
  if ([...`${identity}:prepare`].length > MAX_PENDING_RESOURCE_CHARACTERS) {
    throw new RuntimeAdapterError(
      "operation",
      "presentation resource identity exceeds the protocol limit",
    );
  }
  return identity;
}

function resourceFailure(
  error: unknown,
  pendingResource: string,
): RuntimeAdapterError {
  if (error instanceof RuntimeAdapterError) {
    return error;
  }
  return new RuntimeAdapterError(
    "operation",
    `${pendingResource} failed to prepare`,
  );
}
