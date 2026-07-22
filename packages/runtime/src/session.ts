// Sequential browser protocol session.
// Owns command ordering and state while a narrow adapter owns browser effects;
// this split keeps protocol behavior deterministic and directly testable.

import { decodeBrowserResponse } from "./generated/codec.js";
import type {
  BrowserPlan,
  BrowserRequest,
} from "./generated/browser-request.js";
import type { BrowserResponse } from "./generated/browser-response.js";
import {
  MAX_FAILURE_MESSAGE_CHARACTERS,
  MAX_PENDING_RESOURCE_CHARACTERS,
  MAX_PENDING_RESOURCES,
} from "./generated/runtime-contract.js";
import { runtimeFrameAt, type RuntimeFrame } from "./clock.js";

// ── Browser adapter boundary ──

type Immutable<T> = T extends object
  ? { readonly [Key in keyof T]: Immutable<T[Key]> }
  : T;

/** Immutable browser-plan view owned by one runtime session. */
export type RuntimePlan = Immutable<BrowserPlan>;

/**
 * Browser-owned operations sequenced by one runtime session.
 *
 * Implementations bound every asynchronous wait and report expected browser
 * failures through `RuntimeAdapterError`.
 */
export interface RuntimeAdapter {
  /** Installs one owned snapshot of the immutable browser plan. */
  load(plan: RuntimePlan): Promise<void>;
  /** Resolves only after resources at the evaluation start are stable. */
  prepare(frame: RuntimeFrame): Promise<void>;
  /** Stages one exact frame and registers media presentation observers. */
  seek(frame: RuntimeFrame): Promise<void>;
  /** Verifies media presentation after native compositor capture. */
  confirm(frame: RuntimeFrame): Promise<void>;
  /** Releases all resources owned by this adapter. */
  dispose(): Promise<void>;
}

export type RuntimeAdapterFailureKind = "operation" | "readinessTimeout";

/** Expected failure reported by a browser adapter. */
export class RuntimeAdapterError extends Error {
  readonly kind: RuntimeAdapterFailureKind;
  readonly pendingResources: readonly string[];

  constructor(
    kind: RuntimeAdapterFailureKind,
    message: string,
    pendingResources: readonly string[] = [],
  ) {
    const failureMessage = boundedText(
      message,
      "adapter failure message",
      MAX_FAILURE_MESSAGE_CHARACTERS,
    );
    super(failureMessage);
    this.name = "RuntimeAdapterError";
    this.kind = kind;
    this.pendingResources = boundedPendingResources(pendingResources);
  }

  /** Preserves typed failures and contains untyped browser exceptions. */
  static fromUnknown(error: unknown, message: string): RuntimeAdapterError {
    return error instanceof RuntimeAdapterError
      ? error
      : new RuntimeAdapterError("operation", message);
  }
}

// ── Protocol session ──

/** Sequential browser protocol session. */
export class RuntimeSession {
  readonly #adapter: RuntimeAdapter;
  #state: SessionState = { kind: "empty" };
  #busy = false;

  constructor(adapter: RuntimeAdapter) {
    this.#adapter = adapter;
  }

  /** Executes one decoded request without introducing an unbounded queue. */
  async dispatch(request: BrowserRequest): Promise<BrowserResponse> {
    if (this.#busy) {
      return invalidRequest(
        request.requestId,
        "another browser command is still in progress",
      );
    }

    this.#busy = true;
    try {
      return await this.#execute(request);
    } finally {
      this.#busy = false;
    }
  }

  #execute(request: BrowserRequest): Promise<BrowserResponse> {
    switch (request.command.type) {
      case "load":
        return this.#load(request.requestId, request.command.plan);
      case "prepare":
        return this.#prepare(
          request.requestId,
          request.command.evaluationStart,
        );
      case "seek":
        return this.#seek(request.requestId, request.command.frame);
      case "confirm":
        return this.#confirm(request.requestId, request.command.frame);
      case "dispose":
        return this.#dispose(request.requestId);
    }
  }

  async #load(requestId: number, plan: BrowserPlan): Promise<BrowserResponse> {
    if (this.#state.kind !== "empty") {
      return invalidRequest(
        requestId,
        "load requires an empty browser session",
      );
    }

    const violation = planViolation(plan);
    if (violation !== undefined) {
      return invalidRequest(requestId, violation);
    }

    const ownedPlan = snapshotPlan(plan);
    const nextState = loadedState(ownedPlan);
    try {
      await this.#adapter.load(ownedPlan);
    } catch (error) {
      return operationFailure(requestId, "loadFailed", error);
    }

    this.#state = nextState;
    return response(requestId, { type: "loaded" });
  }

  async #prepare(
    requestId: number,
    evaluationStart: number,
  ): Promise<BrowserResponse> {
    if (this.#state.kind !== "loaded") {
      return invalidRequest(
        requestId,
        "prepare requires a loaded browser plan",
      );
    }
    if (evaluationStart !== this.#state.evaluationStart) {
      return invalidRequest(
        requestId,
        "prepare must use the plan evaluation start",
      );
    }

    try {
      await this.#adapter.prepare(
        runtimeFrameAt(evaluationStart, this.#state.frameRate),
      );
    } catch (error) {
      // Preparation may have started author-owned asynchronous work that the
      // generic adapter cannot cancel. Make the session terminal so a retry
      // cannot overlap that work with a second preparation phase.
      this.#state = { kind: "failed" };
      return readinessFailure(requestId, "prepareFailed", error);
    }

    this.#state = { ...this.#state, kind: "ready" };
    return response(requestId, { type: "prepared", evaluationStart });
  }

  async #seek(requestId: number, frame: number): Promise<BrowserResponse> {
    if (this.#state.kind !== "ready") {
      return invalidRequest(
        requestId,
        "seek requires a prepared browser session",
      );
    }
    if (
      frame < this.#state.evaluationStart ||
      frame >= this.#state.evaluationEnd
    ) {
      return invalidRequest(
        requestId,
        "seek frame falls outside the evaluation interval",
      );
    }

    try {
      await this.#adapter.seek(runtimeFrameAt(frame, this.#state.frameRate));
    } catch (error) {
      return readinessFailure(requestId, "seekFailed", error);
    }

    this.#state = { ...this.#state, kind: "staged", frame };
    return response(requestId, { type: "frameStaged", frame });
  }

  async #confirm(requestId: number, frame: number): Promise<BrowserResponse> {
    if (this.#state.kind !== "staged") {
      return invalidRequest(
        requestId,
        "confirm requires a staged browser frame",
      );
    }
    if (frame !== this.#state.frame) {
      return invalidRequest(requestId, "confirm must use the staged frame");
    }

    try {
      await this.#adapter.confirm(runtimeFrameAt(frame, this.#state.frameRate));
    } catch (error) {
      return readinessFailure(requestId, "confirmFailed", error);
    }

    this.#state = { ...this.#state, kind: "ready" };
    return response(requestId, { type: "frameReady", frame });
  }

  async #dispose(requestId: number): Promise<BrowserResponse> {
    if (this.#state.kind === "disposed") {
      return invalidRequest(requestId, "browser session is already disposed");
    }

    // A disposal attempt is terminal even when browser cleanup reports an
    // error; partially released resources must never be used again.
    this.#state = { kind: "disposed" };
    try {
      await this.#adapter.dispose();
      return response(requestId, { type: "disposed" });
    } catch (error) {
      return operationFailure(requestId, "internal", error);
    }
  }
}

// ── Session state and response construction ──

type SessionState =
  | { readonly kind: "empty" }
  | LoadedState
  | ReadyState
  | StagedState
  | { readonly kind: "failed" }
  | { readonly kind: "disposed" };

interface LoadedState {
  readonly kind: "loaded";
  readonly evaluationStart: number;
  readonly evaluationEnd: number;
  readonly frameRate: RuntimePlan["frameRate"];
}

interface ReadyState {
  readonly kind: "ready";
  readonly evaluationStart: number;
  readonly evaluationEnd: number;
  readonly frameRate: RuntimePlan["frameRate"];
}

interface StagedState {
  readonly kind: "staged";
  readonly evaluationStart: number;
  readonly evaluationEnd: number;
  readonly frameRate: RuntimePlan["frameRate"];
  readonly frame: number;
}

type BrowserEvent = BrowserResponse["event"];
type FailureCode = Extract<BrowserEvent, { type: "failed" }>["code"];
type OperationFailureCode = Exclude<
  FailureCode,
  "protocolMismatch" | "invalidRequest" | "readinessTimeout"
>;

function response(requestId: number, event: BrowserEvent): BrowserResponse {
  return decodeBrowserResponse({ version: 1, requestId, event });
}

function invalidRequest(requestId: number, message: string): BrowserResponse {
  return response(requestId, {
    type: "failed",
    code: "invalidRequest",
    message,
    pendingResources: [],
  });
}

function operationFailure(
  requestId: number,
  operationCode: OperationFailureCode,
  error: unknown,
): BrowserResponse {
  if (!(error instanceof RuntimeAdapterError)) {
    return response(requestId, {
      type: "failed",
      code: "internal",
      message: "runtime adapter threw an untyped error",
      pendingResources: [],
    });
  }

  return response(requestId, {
    type: "failed",
    code: operationCode,
    message: error.message,
    pendingResources: [...error.pendingResources],
  });
}

function readinessFailure(
  requestId: number,
  operationCode: OperationFailureCode,
  error: unknown,
): BrowserResponse {
  if (
    error instanceof RuntimeAdapterError &&
    error.kind === "readinessTimeout"
  ) {
    return response(requestId, {
      type: "failed",
      code: "readinessTimeout",
      message: error.message,
      pendingResources: [...error.pendingResources],
    });
  }
  return operationFailure(requestId, operationCode, error);
}

function planViolation(plan: BrowserPlan): string | undefined {
  if (plan.evaluation.start > plan.evaluation.end) {
    return "plan evaluation interval is reversed";
  }
  if (plan.output.start >= plan.output.end) {
    return "plan output interval is empty or reversed";
  }
  if (
    plan.output.start < plan.evaluation.start ||
    plan.output.end > plan.evaluation.end
  ) {
    return "plan output interval falls outside evaluation";
  }

  for (const video of plan.videos) {
    if (video.interval.start >= video.interval.end) {
      return "plan video interval is empty or reversed";
    }
    if (
      video.interval.start < plan.evaluation.start ||
      video.interval.end > plan.evaluation.end
    ) {
      return "plan video interval falls outside evaluation";
    }
  }
  const componentIds = new Set<number>();
  for (const overlay of plan.overlays) {
    if (componentIds.has(overlay.componentId)) {
      return "plan overlay component identity is duplicated";
    }
    componentIds.add(overlay.componentId);
    if (overlay.interval.start >= overlay.interval.end) {
      return "plan overlay interval is empty or reversed";
    }
    if (
      overlay.interval.start < plan.evaluation.start ||
      overlay.interval.end > plan.evaluation.end
    ) {
      return "plan overlay interval falls outside evaluation";
    }
  }
  return undefined;
}

function snapshotPlan(plan: BrowserPlan): RuntimePlan {
  // Listing every generated field makes a schema addition fail compilation
  // instead of silently falling outside the immutable adapter snapshot.
  const frameRate = Object.freeze({ ...plan.frameRate });
  const evaluation = Object.freeze({ ...plan.evaluation });
  const output = Object.freeze({ ...plan.output });
  const videos = Object.freeze(
    plan.videos.map((video) =>
      Object.freeze({
        assetId: video.assetId,
        interval: Object.freeze({ ...video.interval }),
        sourceFrameRate: Object.freeze({ ...video.sourceFrameRate }),
      }),
    ),
  );
  const overlays = Object.freeze(
    plan.overlays.map((overlay) =>
      Object.freeze({
        componentId: overlay.componentId,
        kind: overlay.kind,
        text: overlay.text,
        interval: Object.freeze({ ...overlay.interval }),
      }),
    ),
  );

  return Object.freeze({
    timelineVersion: plan.timelineVersion,
    frameRate,
    evaluation,
    output,
    videos,
    overlays,
  });
}

function loadedState(plan: RuntimePlan): LoadedState {
  return {
    kind: "loaded",
    evaluationStart: plan.evaluation.start,
    evaluationEnd: plan.evaluation.end,
    frameRate: plan.frameRate,
  };
}

function boundedPendingResources(resources: readonly string[]): string[] {
  if (resources.length > MAX_PENDING_RESOURCES) {
    throw new TypeError("adapter failure has too many pending resources");
  }
  return resources.map((resource) =>
    boundedText(resource, "pending resource", MAX_PENDING_RESOURCE_CHARACTERS),
  );
}

function boundedText(value: string, role: string, limit: number): string {
  const text = nonBlank(value, role);
  let characters = 0;
  for (const _character of text) {
    characters += 1;
    if (characters > limit) {
      throw new TypeError(`${role} exceeds the protocol character limit`);
    }
  }
  return text;
}

function nonBlank(value: string, role: string): string {
  if (value.trim().length === 0) {
    throw new TypeError(`${role} cannot be blank`);
  }
  return value;
}
