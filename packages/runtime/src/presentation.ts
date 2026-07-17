// Browser presentation lifecycle for solved video and overlay placements.
// Rust owns every interval; presentation callbacks own DOM and layout effects.

import type { RuntimeFrame } from "./clock.js";
import {
  videoFrameSelection,
  type RuntimeVideo,
  type VideoFrameSelection,
} from "./media.js";
import {
  RuntimeAdapterError,
  type RuntimeAdapter,
  type RuntimePlan,
} from "./session.js";
import {
  DecodedVideo,
  requireVideoReadinessTimeout,
  type BrowserVideoElement,
} from "./video.js";

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

interface StagedVideo {
  readonly video: BoundVideo;
  readonly selection: VideoFrameSelection;
}

interface StagedPresentation {
  readonly frame: RuntimeFrame;
  readonly videos: readonly StagedVideo[];
}

type PresentationState =
  | { readonly kind: "empty" }
  | LoadedPresentation
  | { readonly kind: "disposed" };

/** Gate-one runtime adapter for presentation-owned browser effects. */
export class PresentationRuntimeAdapter implements RuntimeAdapter {
  readonly #bindings: PresentationBindings;
  readonly #videoTimeoutMilliseconds: number;
  #staged: StagedPresentation | undefined;
  #state: PresentationState = { kind: "empty" };

  constructor(
    bindings: PresentationBindings,
    videoTimeoutMilliseconds: number,
  ) {
    requireVideoReadinessTimeout(videoTimeoutMilliseconds);
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
      const cleanupFailure = releasePresentation(videos, overlays);
      if (cleanupFailure !== undefined) {
        // Incomplete release makes the adapter terminal; retrying would bind
        // new effects beside browser state that no longer has one owner.
        this.#state = { kind: "disposed" };
      }
      throw presentationLoadFailure(error, cleanupFailure);
    }

    this.#state = {
      kind: "loaded",
      frameRate: plan.frameRate,
      videos,
      overlays,
    };
  }

  async prepare(_frame: RuntimeFrame): Promise<void> {
    this.#loadedState("prepare");
  }

  async seek(frame: RuntimeFrame): Promise<void> {
    const state = this.#loadedState("seek");
    if (this.#staged !== undefined) {
      throw new RuntimeAdapterError(
        "operation",
        "presentation cannot stage another frame before confirmation",
      );
    }

    try {
      hideVideos(state.videos);
      const videos = await stageVideos(frame, state);
      presentOverlays(frame, state.overlays);
      this.#staged = { frame, videos };
    } catch (error) {
      discardStagedVideos(state.videos);
      throw RuntimeAdapterError.fromUnknown(error, "frame staging failed");
    }
  }

  async confirm(frame: RuntimeFrame): Promise<void> {
    this.#loadedState("confirm");
    const staged = this.#staged;
    if (staged === undefined || staged.frame.index !== frame.index) {
      throw new RuntimeAdapterError(
        "operation",
        "presentation confirmation requires the staged frame",
      );
    }

    try {
      await confirmVideos(staged.videos);
      this.#staged = undefined;
    } catch (error) {
      throw RuntimeAdapterError.fromUnknown(error, "frame confirmation failed");
    }
  }

  async dispose(): Promise<void> {
    if (this.#state.kind === "disposed") {
      return;
    }
    const loaded = this.#state.kind === "loaded" ? this.#state : undefined;
    this.#state = { kind: "disposed" };
    this.#staged = undefined;

    const failure = releasePresentation(
      loaded?.videos ?? [],
      loaded?.overlays ?? [],
    );
    if (failure !== undefined) {
      throw RuntimeAdapterError.fromUnknown(
        failure,
        "presentation cleanup failed",
      );
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

  #loadedState(operation: string): LoadedPresentation {
    if (this.#state.kind !== "loaded") {
      throw new RuntimeAdapterError(
        "operation",
        `presentation ${operation} requires the loaded state`,
      );
    }
    return this.#state;
  }
}

// ── Frame application ──

async function stageVideos(
  frame: RuntimeFrame,
  state: LoadedPresentation,
): Promise<readonly StagedVideo[]> {
  const staged: StagedVideo[] = [];
  for (const video of state.videos) {
    const selection = videoFrameSelection(
      frame,
      video.placement,
      state.frameRate,
    );
    if (selection === undefined) {
      continue;
    }
    await video.resource.stage(selection);
    staged.push({ video, selection });
  }
  for (const { video } of staged) {
    video.presentation.setVisible(true);
  }
  return staged;
}

async function confirmVideos(videos: readonly StagedVideo[]): Promise<void> {
  for (const { video, selection } of videos) {
    await video.resource.confirm(selection);
  }
}

function discardStagedVideos(videos: readonly BoundVideo[]): void {
  for (const video of videos) {
    video.resource.discardStagedFrame();
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

function presentationLoadFailure(
  error: unknown,
  cleanupFailure: unknown | undefined,
): RuntimeAdapterError {
  if (cleanupFailure !== undefined) {
    return new RuntimeAdapterError(
      "operation",
      "presentation load failed and cleanup was incomplete",
    );
  }
  return RuntimeAdapterError.fromUnknown(error, "presentation failed to load");
}

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
  return releaseAll([
    () => video.presentation.setVisible(false),
    () => video.resource.dispose(),
    () => video.presentation.dispose(),
  ]);
}

function releaseOverlay(overlay: BoundOverlay): unknown | undefined {
  return releaseAll([
    () => overlay.presentation.setVisible(false),
    () => overlay.presentation.dispose(),
  ]);
}

function releaseAll(operations: readonly (() => void)[]): unknown | undefined {
  let failure: unknown;
  for (const operation of operations) {
    try {
      operation();
    } catch (error) {
      failure ??= error;
    }
  }
  return failure;
}
