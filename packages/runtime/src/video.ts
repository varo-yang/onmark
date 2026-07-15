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

  /** Seeks and resolves only after the selected source frame is presented. */
  async present(selection: VideoFrameSelection): Promise<void> {
    this.#requireState("loaded", "present");
    requireSelection(selection);
    if (selection.mediaTimeSeconds === this.#presentedMediaTime) {
      return;
    }

    await new FrameReadiness(
      this.#element,
      selection,
      this.#timeoutMilliseconds,
    ).wait();
    this.#presentedMediaTime = selection.mediaTimeSeconds;
  }

  /** Releases the media resource and makes this controller terminal. */
  dispose(): void {
    if (this.#state === "disposed") {
      return;
    }
    this.#state = "disposed";
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

class FrameReadiness {
  readonly #deadline: ReturnType<typeof setTimeout>;
  readonly #element: BrowserVideoElement;
  readonly #promise: Promise<void>;
  readonly #reject: (error: RuntimeAdapterError) => void;
  readonly #resolve: () => void;
  readonly #selection: VideoFrameSelection;
  #frameCallback: number | undefined;
  #framePresented = false;
  #seekFinished = false;
  #settled = false;

  constructor(
    element: BrowserVideoElement,
    selection: VideoFrameSelection,
    timeoutMilliseconds: number,
  ) {
    this.#element = element;
    this.#selection = selection;
    const pending = Promise.withResolvers<void>();
    this.#promise = pending.promise;
    this.#reject = pending.reject;
    this.#resolve = pending.resolve;
    this.#deadline = setTimeout(this.#timeout, timeoutMilliseconds);
  }

  wait(): Promise<void> {
    this.#element.addEventListener("seeked", this.#seeked);
    this.#element.addEventListener("error", this.#failed);
    try {
      this.#requestFrame();
      this.#element.currentTime = this.#selection.seekTimeSeconds;
    } catch (error) {
      this.#operationFailed(error);
    }
    return this.#promise;
  }

  readonly #seeked = (): void => {
    this.#seekFinished = true;
    this.#completeWhenReady();
  };

  readonly #inspectFrame: FrameCallback = (_now, metadata): void => {
    this.#frameCallback = undefined;
    this.#framePresented =
      Math.abs(metadata.mediaTime - this.#selection.mediaTimeSeconds) <=
      FRAME_TOLERANCE_SECONDS;
    if (this.#framePresented) {
      this.#completeWhenReady();
      return;
    }
    try {
      this.#requestFrame();
    } catch (error) {
      this.#operationFailed(error);
    }
  };

  readonly #failed = (): void => {
    this.#operationFailed(
      new RuntimeAdapterError("operation", "video seek failed"),
    );
  };

  readonly #timeout = (): void => {
    if (this.#settle()) {
      this.#reject(
        new RuntimeAdapterError(
          "readinessTimeout",
          "decoded video frame did not become ready",
          ["video-frame"],
        ),
      );
    }
  };

  #completeWhenReady(): void {
    if (this.#seekFinished && this.#framePresented && this.#settle()) {
      this.#resolve();
    }
  }

  #operationFailed(error: unknown): void {
    if (this.#settle()) {
      this.#reject(
        RuntimeAdapterError.fromUnknown(error, "video frame callback failed"),
      );
    }
  }

  #requestFrame(): void {
    this.#frameCallback = this.#element.requestVideoFrameCallback(
      this.#inspectFrame,
    );
  }

  #settle(): boolean {
    if (this.#settled) {
      return false;
    }
    this.#settled = true;
    clearTimeout(this.#deadline);
    this.#element.removeEventListener("seeked", this.#seeked);
    this.#element.removeEventListener("error", this.#failed);
    if (this.#frameCallback !== undefined) {
      this.#element.cancelVideoFrameCallback(this.#frameCallback);
    }
    return true;
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
