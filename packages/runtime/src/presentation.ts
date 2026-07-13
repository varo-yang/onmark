// Browser presentation lifecycle for solved video and overlay placements.
// Rust owns every interval; presentation callbacks own DOM and layout effects.

import type { RuntimeFrame } from "./clock.js";
import { videoFrameSelection, type RuntimeVideo } from "./media.js";
import {
  RuntimeAdapterError,
  type RuntimeAdapter,
  type RuntimePlan,
} from "./session.js";
import { DecodedVideo, type BrowserVideoElement } from "./video.js";

// ── Presentation boundary ──

/** Immutable overlay placement projected from Timeline IR. */
export type RuntimeOverlay = RuntimePlan["overlays"][number];

/** Presentation-owned effects for one decoded video placement. */
export interface VideoPresentation {
  readonly element: BrowserVideoElement;
  readonly source: string;
  setVisible(visible: boolean): void;
  dispose(): void;
}

/** Presentation-owned effects for one title or call-to-action placement. */
export interface OverlayPresentation {
  setVisible(visible: boolean): void;
  dispose(): void;
}

/** Browser effects supplied by one presentation entry point. */
export interface PresentationBindings {
  bindVideo(placement: RuntimeVideo, index: number): VideoPresentation;
  bindOverlay(placement: RuntimeOverlay, index: number): OverlayPresentation;
}

interface BoundVideo {
  readonly placement: RuntimeVideo;
  readonly presentation: VideoPresentation;
  readonly resource: DecodedVideo;
}

interface BoundOverlay {
  readonly placement: RuntimeOverlay;
  readonly presentation: OverlayPresentation;
}

interface LoadedPresentation {
  readonly kind: "loaded";
  readonly frameRate: RuntimePlan["frameRate"];
  readonly videos: readonly BoundVideo[];
  readonly overlays: readonly BoundOverlay[];
}

type PresentationState =
  | { readonly kind: "empty" }
  | LoadedPresentation
  | { readonly kind: "disposed" };

/** Gate-one runtime adapter for presentation-owned browser effects. */
export class PresentationRuntimeAdapter implements RuntimeAdapter {
  readonly #bindings: PresentationBindings;
  readonly #videoTimeoutMilliseconds: number;
  #state: PresentationState = { kind: "empty" };

  constructor(
    bindings: PresentationBindings,
    videoTimeoutMilliseconds: number,
  ) {
    this.#bindings = bindings;
    this.#videoTimeoutMilliseconds = videoTimeoutMilliseconds;
  }

  async load(plan: RuntimePlan): Promise<void> {
    if (this.#state.kind !== "empty") {
      throw new RuntimeAdapterError(
        "operation",
        "presentation load requires the empty state",
      );
    }

    const videos: BoundVideo[] = [];
    const overlays: BoundOverlay[] = [];
    try {
      await this.#loadVideos(plan, videos);
      this.#bindOverlays(plan, overlays);
    } catch (error) {
      releasePresentation(videos, overlays);
      throw adapterFailure(error, "presentation failed to load");
    }

    this.#state = {
      kind: "loaded",
      frameRate: plan.frameRate,
      videos,
      overlays,
    };
  }

  prepare(frame: RuntimeFrame): Promise<void> {
    return this.#present(frame);
  }

  seek(frame: RuntimeFrame): Promise<void> {
    return this.#present(frame);
  }

  async dispose(): Promise<void> {
    if (this.#state.kind === "disposed") {
      return;
    }
    const loaded = this.#state.kind === "loaded" ? this.#state : undefined;
    this.#state = { kind: "disposed" };

    const failure = releasePresentation(
      loaded?.videos ?? [],
      loaded?.overlays ?? [],
    );
    if (failure !== undefined) {
      throw adapterFailure(failure, "presentation cleanup failed");
    }
  }

  async #loadVideos(plan: RuntimePlan, videos: BoundVideo[]): Promise<void> {
    for (const [index, placement] of plan.videos.entries()) {
      const presentation = this.#bindings.bindVideo(placement, index);
      const resource = new DecodedVideo(
        presentation.element,
        this.#videoTimeoutMilliseconds,
      );
      const video = { placement, presentation, resource };
      videos.push(video);
      presentation.setVisible(false);
      await resource.load(presentation.source);
    }
  }

  #bindOverlays(plan: RuntimePlan, overlays: BoundOverlay[]): void {
    for (const [index, placement] of plan.overlays.entries()) {
      const presentation = this.#bindings.bindOverlay(placement, index);
      overlays.push({ placement, presentation });
      presentation.setVisible(false);
    }
  }

  async #present(frame: RuntimeFrame): Promise<void> {
    if (this.#state.kind !== "loaded") {
      throw new RuntimeAdapterError(
        "operation",
        "frame presentation requires the loaded state",
      );
    }

    try {
      hideVideos(this.#state.videos);
      await presentVideos(frame, this.#state);
      presentOverlays(frame, this.#state.overlays);
    } catch (error) {
      throw adapterFailure(error, "frame presentation failed");
    }
  }
}

// ── Frame application ──

async function presentVideos(
  frame: RuntimeFrame,
  state: LoadedPresentation,
): Promise<void> {
  const visible: BoundVideo[] = [];
  for (const video of state.videos) {
    const selection = videoFrameSelection(
      frame,
      video.placement,
      state.frameRate,
    );
    if (selection === undefined) {
      continue;
    }
    await video.resource.present(selection);
    visible.push(video);
  }
  for (const video of visible) {
    video.presentation.setVisible(true);
  }
}

function presentOverlays(
  frame: RuntimeFrame,
  overlays: readonly BoundOverlay[],
): void {
  for (const overlay of overlays) {
    const { interval } = overlay.placement;
    const visible = frame.index >= interval.start && frame.index < interval.end;
    overlay.presentation.setVisible(visible);
  }
}

function hideVideos(videos: readonly BoundVideo[]): void {
  for (const video of videos) {
    video.presentation.setVisible(false);
  }
}

// ── Terminal cleanup ──

function releasePresentation(
  videos: readonly BoundVideo[],
  overlays: readonly BoundOverlay[],
): unknown | undefined {
  let failure: unknown;
  for (const video of videos) {
    const releaseFailure = releaseVideo(video);
    failure ??= releaseFailure;
  }
  for (const overlay of overlays) {
    const releaseFailure = releaseOverlay(overlay);
    failure ??= releaseFailure;
  }
  return failure;
}

function releaseVideo(video: BoundVideo): unknown | undefined {
  let failure: unknown;
  try {
    video.presentation.setVisible(false);
  } catch (error) {
    failure = error;
  }
  try {
    video.resource.dispose();
  } catch (error) {
    failure ??= error;
  }
  try {
    video.presentation.dispose();
  } catch (error) {
    failure ??= error;
  }
  return failure;
}

function releaseOverlay(overlay: BoundOverlay): unknown | undefined {
  let failure: unknown;
  try {
    overlay.presentation.setVisible(false);
  } catch (error) {
    failure = error;
  }
  try {
    overlay.presentation.dispose();
  } catch (error) {
    failure ??= error;
  }
  return failure;
}

function adapterFailure(error: unknown, message: string): RuntimeAdapterError {
  return error instanceof RuntimeAdapterError
    ? error
    : new RuntimeAdapterError("operation", message);
}
