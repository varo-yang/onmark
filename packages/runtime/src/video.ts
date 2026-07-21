// Bounded browser-video lifecycle over a presentation-owned media element.
// Timeline selection remains Rust-owned; this module only proves decode readiness.

import { BUNDLE_ASSET_DIRECTORY } from "./generated/bundle-layout.js";
import type { RuntimeVideo, VideoFrameSelection } from "./media.js";
import { requireReadinessTimeout } from "./resource.js";
import { RuntimeAdapterError } from "./session.js";

const FRAME_TOLERANCE_SECONDS = 0.000_001;

type VideoEvent = "error" | "loadeddata" | "seeked";
type ReadinessEvent = Exclude<VideoEvent, "error">;
type FrameCallback = (
  now: number,
  metadata: { readonly mediaTime: number },
) => void;

interface ReadinessContract {
  readonly event: ReadinessEvent;
  readonly failureMessage: string;
  readonly pendingResource: string;
  readonly timeoutMessage: string;
}

const LOAD_READINESS: ReadinessContract = {
  event: "loadeddata",
  failureMessage: "video data failed to load",
  pendingResource: "video:loadeddata",
  timeoutMessage: "video data did not load before its readiness deadline",
};

const SEEK_READINESS: ReadinessContract = {
  event: "seeked",
  failureMessage: "video seek failed",
  pendingResource: "video:seeked",
  timeoutMessage: "video seek did not finish before its readiness deadline",
};

/** Minimal browser-media capability required from a presentation. */
export interface BrowserVideoElement {
  currentTime: number;
  src: string;
  addEventListener(type: VideoEvent, listener: () => void): void;
  cancelVideoFrameCallback(handle: number): void;
  load(): void;
  removeAttribute(name: "src"): void;
  removeEventListener(type: VideoEvent, listener: () => void): void;
  requestVideoFrameCallback(callback: FrameCallback): number;
}

type VideoState = "empty" | "loaded" | "disposed";

/** One decoded-video resource with exact-frame readiness and terminal cleanup. */
export class DecodedVideo {
  readonly #element: BrowserVideoElement;
  readonly #timeoutMilliseconds: number;
  #pendingFrame: StagedFrame | undefined;
  #presentedMediaTime: number | undefined;
  #state: VideoState = "empty";

  constructor(element: BrowserVideoElement, timeoutMilliseconds: number) {
    requireReadinessTimeout(timeoutMilliseconds);
    this.#element = element;
    this.#timeoutMilliseconds = timeoutMilliseconds;
  }

  /** Loads one materialized source and waits for decoded data. */
  async load(source: string): Promise<void> {
    this.#requireState("empty", "load");
    if (source.length === 0) {
      throw new RuntimeAdapterError("operation", "video source is empty");
    }

    const readiness = new MediaEventReadiness(
      this.#element,
      this.#timeoutMilliseconds,
      LOAD_READINESS,
    );
    try {
      await readiness.waitAfter(() => {
        this.#element.src = source;
        this.#element.load();
      });
      this.#state = "loaded";
    } catch (error) {
      readiness.cancel();
      const cleanupFailure = releaseElement(this.#element);
      if (cleanupFailure !== undefined) {
        // A source that could not be fully released must never be reused.
        this.#state = "disposed";
      }
      throw videoLoadFailure(error, cleanupFailure);
    }
  }

  /** Seeks while registering the callback that will observe compositor output. */
  async stage(selection: VideoFrameSelection): Promise<void> {
    this.#requireState("loaded", "stage");
    requireSelection(selection);
    if (selection.mediaTimeSeconds === this.#presentedMediaTime) {
      return;
    }

    if (this.#pendingFrame !== undefined) {
      throw new RuntimeAdapterError(
        "operation",
        "video cannot stage another frame before confirmation",
      );
    }

    const pending = StagedFrame.observe(this.#element, selection);
    const readiness = new MediaEventReadiness(
      this.#element,
      this.#timeoutMilliseconds,
      SEEK_READINESS,
    );
    this.#pendingFrame = pending;
    try {
      await readiness.waitAfter(() => {
        this.#element.currentTime = selection.seekTimeSeconds;
      });
    } catch (error) {
      readiness.cancel();
      pending.cancel();
      this.#pendingFrame = undefined;
      throw error;
    }
  }

  /** Confirms staged media reached the compositor before capture is accepted. */
  async confirm(selection: VideoFrameSelection): Promise<void> {
    this.#requireState("loaded", "confirm");
    requireSelection(selection);
    if (selection.mediaTimeSeconds === this.#presentedMediaTime) {
      return;
    }

    const pending = this.#pendingFrame;
    if (pending === undefined || !pending.matches(selection)) {
      throw new RuntimeAdapterError(
        "operation",
        "video confirmation requires the staged frame",
      );
    }

    try {
      await pending.confirm(this.#timeoutMilliseconds);
    } catch (error) {
      throw RuntimeAdapterError.fromUnknown(
        error,
        "video frame confirmation failed",
      );
    } finally {
      pending.cancel();
      this.#pendingFrame = undefined;
    }
    this.#presentedMediaTime = selection.mediaTimeSeconds;
  }

  /** Cancels one staged frame that will not be captured. */
  discardStagedFrame(): void {
    this.#pendingFrame?.cancel();
    this.#pendingFrame = undefined;
  }

  /** Releases the media resource and makes this controller terminal. */
  dispose(): void {
    if (this.#state === "disposed") {
      return;
    }
    this.#state = "disposed";
    this.discardStagedFrame();
    this.#presentedMediaTime = undefined;
    const failure = releaseElement(this.#element);
    if (failure !== undefined) {
      throw RuntimeAdapterError.fromUnknown(
        failure,
        "video resource cleanup failed",
      );
    }
  }

  #requireState(expected: VideoState, operation: string): void {
    if (this.#state !== expected) {
      throw new RuntimeAdapterError(
        "operation",
        `video ${operation} requires the ${expected} state`,
      );
    }
  }
}

/** Returns the unit-root source for one already-validated video placement. */
export function materializedVideoSource(placement: RuntimeVideo): string {
  const digest = placement.assetId.slice("sha256:".length);
  return `./${BUNDLE_ASSET_DIRECTORY}/${digest}`;
}

function releaseElement(element: BrowserVideoElement): unknown | undefined {
  let failure: unknown;
  for (const release of [
    () => element.removeAttribute("src"),
    () => element.load(),
  ]) {
    try {
      release();
    } catch (error) {
      failure ??= error;
    }
  }
  return failure;
}

function videoLoadFailure(
  error: unknown,
  cleanupFailure: unknown | undefined,
): RuntimeAdapterError {
  if (cleanupFailure !== undefined) {
    return new RuntimeAdapterError(
      "operation",
      "video load failed and cleanup was incomplete",
    );
  }
  return RuntimeAdapterError.fromUnknown(error, "video data failed to load");
}

// ── Readiness waits ──

/** One listener-first media event barrier with a bounded terminal state. */
class MediaEventReadiness {
  readonly #deadline: ReturnType<typeof setTimeout>;
  readonly #contract: ReadinessContract;
  readonly #element: BrowserVideoElement;
  readonly #promise: Promise<void>;
  readonly #reject: (error: RuntimeAdapterError) => void;
  readonly #resolve: () => void;
  #settled = false;

  constructor(
    element: BrowserVideoElement,
    timeoutMilliseconds: number,
    contract: ReadinessContract,
  ) {
    this.#element = element;
    this.#contract = contract;
    const pending = Promise.withResolvers<void>();
    this.#promise = pending.promise;
    this.#reject = pending.reject;
    this.#resolve = pending.resolve;
    this.#deadline = setTimeout(this.#timeout, timeoutMilliseconds);
    element.addEventListener(contract.event, this.#complete);
    element.addEventListener("error", this.#fail);
  }

  waitAfter(action: () => void): Promise<void> {
    try {
      action();
    } catch (error) {
      this.#settle();
      throw RuntimeAdapterError.fromUnknown(
        error,
        this.#contract.failureMessage,
      );
    }
    return this.#promise;
  }

  cancel(): void {
    if (this.#settle()) {
      this.#resolve();
    }
  }

  readonly #complete = (): void => {
    if (this.#settle()) {
      this.#resolve();
    }
  };

  readonly #fail = (): void => {
    if (!this.#settle()) {
      return;
    }
    this.#reject(
      new RuntimeAdapterError("operation", this.#contract.failureMessage),
    );
  };

  readonly #timeout = (): void => {
    if (this.#settle()) {
      this.#reject(
        new RuntimeAdapterError(
          "readinessTimeout",
          this.#contract.timeoutMessage,
          [this.#contract.pendingResource],
        ),
      );
    }
  };

  #settle(): boolean {
    if (this.#settled) {
      return false;
    }
    this.#settled = true;
    clearTimeout(this.#deadline);
    this.#element.removeEventListener(this.#contract.event, this.#complete);
    this.#element.removeEventListener("error", this.#fail);
    return true;
  }
}

type FrameObservation =
  | { readonly kind: "presented" }
  | { readonly kind: "failed"; readonly error: RuntimeAdapterError };

class StagedFrame {
  readonly #element: BrowserVideoElement;
  readonly #observation: Promise<FrameObservation>;
  readonly #resolve: (observation: FrameObservation) => void;
  readonly #selection: VideoFrameSelection;
  #frameCallback: number | undefined;
  #settled = false;

  private constructor(
    element: BrowserVideoElement,
    selection: VideoFrameSelection,
  ) {
    this.#element = element;
    this.#selection = selection;
    const pending = Promise.withResolvers<FrameObservation>();
    this.#observation = pending.promise;
    this.#resolve = pending.resolve;
    element.addEventListener("error", this.#failed);
  }

  static observe(
    element: BrowserVideoElement,
    selection: VideoFrameSelection,
  ): StagedFrame {
    const staged = new StagedFrame(element, selection);
    try {
      staged.#requestFrame();
      return staged;
    } catch (error) {
      staged.cancel();
      throw RuntimeAdapterError.fromUnknown(
        error,
        "video frame callback failed",
      );
    }
  }

  matches(selection: VideoFrameSelection): boolean {
    return (
      selection.mediaTimeSeconds === this.#selection.mediaTimeSeconds &&
      selection.seekTimeSeconds === this.#selection.seekTimeSeconds
    );
  }

  async confirm(timeoutMilliseconds: number): Promise<void> {
    const observation = await observedBeforeDeadline(
      this.#observation,
      timeoutMilliseconds,
    );
    if (observation.kind === "failed") {
      throw observation.error;
    }
  }

  cancel(): void {
    if (!this.#settle()) {
      return;
    }
    this.#resolve({
      kind: "failed",
      error: new RuntimeAdapterError(
        "operation",
        "staged video frame was discarded",
      ),
    });
  }

  readonly #inspectFrame: FrameCallback = (_now, metadata): void => {
    this.#frameCallback = undefined;
    const exactFrame =
      Math.abs(metadata.mediaTime - this.#selection.mediaTimeSeconds) <=
      FRAME_TOLERANCE_SECONDS;
    if (exactFrame) {
      this.#finish({ kind: "presented" });
      return;
    }

    try {
      this.#requestFrame();
    } catch (error) {
      this.#finish({
        kind: "failed",
        error: RuntimeAdapterError.fromUnknown(
          error,
          "video frame callback failed",
        ),
      });
    }
  };

  readonly #failed = (): void => {
    this.#finish({
      kind: "failed",
      error: new RuntimeAdapterError("operation", "video seek failed"),
    });
  };

  #requestFrame(): void {
    this.#frameCallback = this.#element.requestVideoFrameCallback(
      this.#inspectFrame,
    );
  }

  #finish(observation: FrameObservation): void {
    if (this.#settle()) {
      this.#resolve(observation);
    }
  }

  #settle(): boolean {
    if (this.#settled) {
      return false;
    }
    this.#settled = true;
    this.#element.removeEventListener("error", this.#failed);
    if (this.#frameCallback !== undefined) {
      this.#element.cancelVideoFrameCallback(this.#frameCallback);
    }
    return true;
  }
}

async function observedBeforeDeadline(
  observation: Promise<FrameObservation>,
  timeoutMilliseconds: number,
): Promise<FrameObservation> {
  let deadline: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<FrameObservation>((resolve) => {
    deadline = setTimeout(() => {
      resolve({
        kind: "failed",
        error: new RuntimeAdapterError(
          "readinessTimeout",
          "decoded video frame did not become ready",
          ["video-frame"],
        ),
      });
    }, timeoutMilliseconds);
  });

  try {
    return await Promise.race([observation, timeout]);
  } finally {
    clearTimeout(deadline);
  }
}

function requireSelection(selection: VideoFrameSelection): void {
  if (
    !Number.isFinite(selection.mediaTimeSeconds) ||
    selection.mediaTimeSeconds < 0 ||
    !Number.isFinite(selection.seekTimeSeconds) ||
    selection.seekTimeSeconds < selection.mediaTimeSeconds
  ) {
    throw new RuntimeAdapterError(
      "operation",
      "video frame selection is invalid",
    );
  }
}
