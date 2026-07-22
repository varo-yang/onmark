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
import { DecodedVideo, type BrowserVideoElement } from "./video.js";
import {
  ownPresentationResources,
  preparePresentationResources,
  releasePresentationResources,
  requireReadinessTimeout,
  validatePresentationResources,
  type PresentationResource,
} from "./resource.js";

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

/** One paused browser effect driven exclusively by the authored frame. */
export interface FrameEffect {
  /** Applies the exact frame before the runtime reports it as staged. */
  apply(frame: RuntimeFrame): void | Promise<void>;
  /** Releases resources retained by this effect. */
  dispose(): void | Promise<void>;
}

/** Browser effects supplied by one presentation entry point. */
export interface PresentationBindings {
  bindVideo(placement: RuntimeVideo, index: number): VideoPresentation;
  bindOverlay(placement: RuntimeOverlay): OverlayPresentation;
  bindResources(plan: RuntimePlan): readonly PresentationResource[];
  bindFrameEffects(plan: RuntimePlan): readonly FrameEffect[];
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
  readonly effects: readonly FrameEffect[];
  readonly frameRate: RuntimePlan["frameRate"];
  readonly resources: readonly PresentationResource[];
  readonly videos: readonly BoundVideo[];
  readonly overlays: readonly BoundOverlay[];
}

type FailedPresentation = Omit<LoadedPresentation, "kind"> & {
  readonly kind: "failed";
};

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
  | FailedPresentation
  | { readonly kind: "disposed" };

/** Runtime adapter for presentation-owned browser effects. */
export class PresentationRuntimeAdapter implements RuntimeAdapter {
  readonly #bindings: PresentationBindings;
  readonly #readinessTimeoutMilliseconds: number;
  #staged: StagedPresentation | undefined;
  #state: PresentationState = { kind: "empty" };

  constructor(
    bindings: PresentationBindings,
    readinessTimeoutMilliseconds: number,
  ) {
    requireReadinessTimeout(readinessTimeoutMilliseconds);
    this.#bindings = bindings;
    this.#readinessTimeoutMilliseconds = readinessTimeoutMilliseconds;
  }

  async load(plan: RuntimePlan): Promise<void> {
    if (this.#state.kind !== "empty") {
      throw new RuntimeAdapterError(
        "operation",
        "presentation load requires the empty state",
      );
    }

    let effects: readonly FrameEffect[] = [];
    let resources: readonly PresentationResource[] = [];
    const videos: BoundVideo[] = [];
    const overlays: BoundOverlay[] = [];
    try {
      this.#bindVideos(plan, videos);
      this.#bindOverlays(plan, overlays);
      resources = ownPresentationResources(this.#bindings.bindResources(plan));
      validatePresentationResources(resources);
      effects = this.#bindings.bindFrameEffects(plan);
      effects = ownFrameEffects(effects);
      await loadVideos(videos);
    } catch (error) {
      const cleanupFailure = await releasePresentation(
        effects,
        resources,
        videos,
        overlays,
      );
      if (cleanupFailure !== undefined) {
        // Incomplete release makes the adapter terminal; retrying would bind
        // new effects beside browser state that no longer has one owner.
        this.#state = { kind: "disposed" };
      }
      throw presentationLoadFailure(error, cleanupFailure);
    }

    this.#state = {
      kind: "loaded",
      effects,
      frameRate: plan.frameRate,
      resources,
      videos,
      overlays,
    };
  }

  async prepare(_frame: RuntimeFrame): Promise<void> {
    const state = this.#loadedState("prepare");
    try {
      await preparePresentationResources(
        state.resources,
        this.#readinessTimeoutMilliseconds,
      );
    } catch (error) {
      this.#state = { ...state, kind: "failed" };
      throw error;
    }
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
      await applyFrameEffects(frame, state.effects);
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
    const loaded =
      this.#state.kind === "loaded" || this.#state.kind === "failed"
        ? this.#state
        : undefined;
    this.#state = { kind: "disposed" };
    this.#staged = undefined;

    const failure = await releasePresentation(
      loaded?.effects ?? [],
      loaded?.resources ?? [],
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

  #bindVideos(plan: RuntimePlan, videos: BoundVideo[]): void {
    for (const [index, placement] of plan.videos.entries()) {
      const presentation = this.#bindings.bindVideo(placement, index);
      const resource = new DecodedVideo(
        presentation.element,
        this.#readinessTimeoutMilliseconds,
      );
      const video = { placement, presentation, resource };
      videos.push(video);
      presentation.setVisible(false);
    }
  }

  #bindOverlays(plan: RuntimePlan, overlays: BoundOverlay[]): void {
    for (const placement of plan.overlays) {
      const presentation = this.#bindings.bindOverlay(placement);
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

function ownFrameEffects(
  effects: readonly FrameEffect[],
): readonly FrameEffect[] {
  return Object.freeze(
    effects.map((effect) =>
      Object.freeze({
        apply: effect.apply.bind(effect),
        dispose: effect.dispose.bind(effect),
      }),
    ),
  );
}

async function loadVideos(videos: readonly BoundVideo[]): Promise<void> {
  for (const video of videos) {
    await video.resource.load(video.presentation.source);
  }
}

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

async function applyFrameEffects(
  frame: RuntimeFrame,
  effects: readonly FrameEffect[],
): Promise<void> {
  for (const effect of effects) {
    await effect.apply(frame);
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

async function releasePresentation(
  effects: readonly FrameEffect[],
  resources: readonly PresentationResource[],
  videos: readonly BoundVideo[],
  overlays: readonly BoundOverlay[],
): Promise<unknown | undefined> {
  let failure = await releaseFrameEffects(effects);
  const resourceFailure = await releasePresentationResources(resources);
  failure ??= resourceFailure;
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

async function releaseFrameEffects(
  effects: readonly FrameEffect[],
): Promise<unknown | undefined> {
  let failure: unknown;
  for (const effect of effects) {
    try {
      await effect.dispose();
    } catch (error) {
      failure ??= error;
    }
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
