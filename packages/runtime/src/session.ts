// Sequential Gate-one browser protocol session.
// Owns command ordering and state while a narrow adapter owns browser effects;
// this split keeps protocol behavior deterministic and directly testable.

import { decodeBrowserResponse } from "./generated/codec.js";
import type {
  BrowserPlan,
  BrowserRequest,
} from "./generated/browser-request.js";
import type { BrowserResponse } from "./generated/browser-response.js";

// ── Browser adapter boundary ──

/**
 * Browser-owned operations sequenced by one runtime session.
 *
 * Implementations bound every asynchronous wait and report expected browser
 * failures through `RuntimeAdapterError`.
 */
export interface RuntimeAdapter {
  /** Installs one owned snapshot of the immutable browser plan. */
  load(plan: BrowserPlan): Promise<void>;
  /** Resolves only after resources at the evaluation start are stable. */
  prepare(evaluationStart: number): Promise<void>;
  /** Resolves only after one exact frame is stable for native capture. */
  seek(frame: number): Promise<void>;
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
    super(nonBlank(message, "adapter failure message"));
    this.name = "RuntimeAdapterError";
    this.kind = kind;
    this.pendingResources = pendingResources.map((resource) =>
      nonBlank(resource, "pending resource"),
    );
  }
}

// ── Protocol session ──

/** Sequential Gate-one browser protocol session. */
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

    const ownedPlan = copyPlan(plan);
    const { start: evaluationStart, end: evaluationEnd } = ownedPlan.evaluation;
    try {
      await this.#adapter.load(ownedPlan);
    } catch (error) {
      return operationFailure(requestId, "loadFailed", error);
    }

    this.#state = {
      kind: "loaded",
      evaluationStart,
      evaluationEnd,
    };
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
      await this.#adapter.prepare(evaluationStart);
    } catch (error) {
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
      await this.#adapter.seek(frame);
      return response(requestId, { type: "frameReady", frame });
    } catch (error) {
      return readinessFailure(requestId, "seekFailed", error);
    }
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
  | { readonly kind: "disposed" };

interface LoadedState {
  readonly kind: "loaded";
  readonly evaluationStart: number;
  readonly evaluationEnd: number;
}

interface ReadyState {
  readonly kind: "ready";
  readonly evaluationStart: number;
  readonly evaluationEnd: number;
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

function copyPlan(plan: BrowserPlan): BrowserPlan {
  // Listing every generated field makes a schema addition fail compilation
  // instead of silently falling outside the owned snapshot.
  return {
    timelineVersion: plan.timelineVersion,
    frameRate: { ...plan.frameRate },
    evaluation: { ...plan.evaluation },
    output: { ...plan.output },
  };
}

function nonBlank(value: string, role: string): string {
  if (value.trim().length === 0) {
    throw new TypeError(`${role} cannot be blank`);
  }
  return value;
}
