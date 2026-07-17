// Bounded browser-video lifecycle over a presentation-owned media element.
// Timeline selection remains Rust-owned; this module only proves decode readiness.

import { BUNDLE_ASSET_DIRECTORY } from "./generated/bundle-layout.js";
import type { RuntimeVideo, VideoFrameSelection } from "./media.js";
import { RuntimeAdapterError } from "./session.js";

const FRAME_TOLERANCE_SECONDS = 0.000_001;
const MAX_READINESS_TIMEOUT_MILLISECONDS = 24 * 60 * 60 * 1_000;

type VideoEvent = "error" | "loadeddata" | "seeked";
type FrameCallback = (
  now: number,
  metadata: { readonly mediaTime: number },
) => void;

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

    const readiness = new LoadedDataReadiness(
      this.#element,
      this.#timeoutMilliseconds,
    );
    try {
      this.#element.src = source;
      this.#element.load();
      await readiness.wait();
      this.#state = "loaded";
    } catch (error) {
      readiness.cancel();
      releaseElement(this.#element);
      throw RuntimeAdapterError.fromUnknown(error, "video data failed to load");
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

    const readiness = new SeekReadiness(
      this.#element,
      this.#timeoutMilliseconds,
    );
    let pending: StagedFrame | undefined;
    try {
      const seeked = readiness.seek(selection.seekTimeSeconds);
      // Observe the seek now in flight. Registering against the loaded frame
      // can miss the first presentation when rendering begins out of order.
      pending = StagedFrame.observe(this.#element, selection);
      this.#pendingFrame = pending;
      await seeked;
    } catch (error) {
      readiness.cancel();
      pending?.cancel();
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

// ── Readiness waits ──

class LoadedDataReadiness {
  readonly #deadline: ReturnType<typeof setTimeout>;
  readonly #element: BrowserVideoElement;
  readonly #promise: Promise<void>;
  readonly #reject: (error: RuntimeAdapterError) => void;
  readonly #resolve: () => void;
  #settled = false;

  constructor(element: BrowserVideoElement, timeoutMilliseconds: number) {
    this.#element = element;
    const pending = Promise.withResolvers<void>();
    this.#promise = pending.promise;
    this.#reject = pending.reject;
    this.#resolve = pending.resolve;
    this.#deadline = setTimeout(this.#timeout, timeoutMilliseconds);
    element.addEventListener("loadeddata", this.#complete);
    element.addEventListener("error", this.#fail);
  }

  wait(): Promise<void> {
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
    if (this.#settle()) {
      this.#reject(
        new RuntimeAdapterError("operation", "video data failed to load"),
      );
    }
  };

  readonly #timeout = (): void => {
    if (this.#settle()) {
      this.#reject(
        new RuntimeAdapterError(
          "readinessTimeout",
          "video data did not load before its readiness deadline",
          ["video:loadeddata"],
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
    this.#element.removeEventListener("loadeddata", this.#complete);
    this.#element.removeEventListener("error", this.#fail);
    return true;
  }
}

class SeekReadiness {
  readonly #deadline: ReturnType<typeof setTimeout>;
  readonly #element: BrowserVideoElement;
  readonly #promise: Promise<void>;
  readonly #reject: (error: RuntimeAdapterError) => void;
  readonly #resolve: () => void;
  #settled = false;

  constructor(element: BrowserVideoElement, timeoutMilliseconds: number) {
    this.#element = element;
    const pending = Promise.withResolvers<void>();
    this.#promise = pending.promise;
    this.#reject = pending.reject;
    this.#resolve = pending.resolve;
    this.#deadline = setTimeout(this.#timeout, timeoutMilliseconds);
    element.addEventListener("seeked", this.#complete);
    element.addEventListener("error", this.#failed);
  }

  seek(timeSeconds: number): Promise<void> {
    try {
      this.#element.currentTime = timeSeconds;
    } catch (error) {
      this.#fail(error);
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

  readonly #failed = (): void => {
    if (this.#settle()) {
      this.#reject(new RuntimeAdapterError("operation", "video seek failed"));
    }
  };

  readonly #timeout = (): void => {
    if (this.#settle()) {
      this.#reject(
        new RuntimeAdapterError(
          "readinessTimeout",
          "video seek did not finish before its readiness deadline",
          ["video:seeked"],
        ),
      );
    }
  };

  #fail(error: unknown): void {
    if (this.#settle()) {
      this.#reject(RuntimeAdapterError.fromUnknown(error, "video seek failed"));
    }
  }

  #settle(): boolean {
    if (this.#settled) {
      return false;
    }
    this.#settled = true;
    clearTimeout(this.#deadline);
    this.#element.removeEventListener("seeked", this.#complete);
    this.#element.removeEventListener("error", this.#failed);
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

function requireReadinessTimeout(timeoutMilliseconds: number): void {
  if (
    !Number.isSafeInteger(timeoutMilliseconds) ||
    timeoutMilliseconds <= 0 ||
    timeoutMilliseconds > MAX_READINESS_TIMEOUT_MILLISECONDS
  ) {
    throw new TypeError(
      "video readiness timeout must be a positive integer no greater than one day",
    );
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
