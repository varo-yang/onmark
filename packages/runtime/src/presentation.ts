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

/** Maximum exact-frame effects retained by one presentation adapter. */
export const MAX_PRESENTATION_EFFECTS = 10_000;

/** Immutable overlay placement projected from Timeline IR. */
export type RuntimeOverlay = RuntimePlan["overlays"][number];
/** Immutable scene container projected from Timeline IR. */
export type RuntimeScene = RuntimePlan["scenes"][number];
/** Immutable shot container projected from Timeline IR. */
export type RuntimeShot = RuntimePlan["shots"][number];
/** Immutable film identity projected from Timeline IR. */
export type RuntimeNode = RuntimePlan["film"];

/** Presentation-owned effects for one semantic container. */
export interface ContainerPresentation {
  readonly element: HTMLElement;
  setVisible(visible: boolean): void;
  dispose(): void;
}

/** Presentation-owned effects for one decoded video placement. */
export interface VideoPresentation {
  readonly element: BrowserVideoElement;
  readonly source: string;
  setVisible(visible: boolean): void;
  dispose(): void;
}

/** Presentation-owned effects for one title or call-to-action placement. */
export interface OverlayPresentation {
  readonly element: HTMLElement;
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

/** Browser resources and exact-frame effects produced by one extension program. */
export interface PresentationExtensions {
  readonly effects: readonly FrameEffect[];
  readonly resources: readonly PresentationResource[];
}

/** Browser effects supplied by one presentation entry point. */
export interface PresentationBindings {
  bindFilm(node: RuntimeNode): ContainerPresentation;
  bindScene(scene: RuntimeScene): ContainerPresentation;
  bindShot(shot: RuntimeShot): ContainerPresentation;
  bindVideo(placement: RuntimeVideo): VideoPresentation;
  bindOverlay(placement: RuntimeOverlay): OverlayPresentation;
  bindExtensions(plan: RuntimePlan): Promise<PresentationExtensions>;
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

interface BoundContainer<T> {
  readonly placement: T;
  readonly presentation: ContainerPresentation;
}

interface BoundStructure {
  readonly film: BoundContainer<RuntimeNode>;
  readonly scenes: readonly BoundContainer<RuntimeScene>[];
  readonly shots: readonly BoundContainer<RuntimeShot>[];
}

interface PendingStructure {
  film: BoundContainer<RuntimeNode> | undefined;
  readonly scenes: BoundContainer<RuntimeScene>[];
  readonly shots: BoundContainer<RuntimeShot>[];
}

interface LoadedPresentation {
  readonly kind: "loaded";
  readonly effects: readonly FrameEffect[];
  readonly frameRate: RuntimePlan["frameRate"];
  readonly resources: readonly PresentationResource[];
  readonly structure: BoundStructure;
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
    const structure: PendingStructure = {
      film: undefined,
      scenes: [],
      shots: [],
    };
    let boundStructure: BoundStructure;
    const videos: BoundVideo[] = [];
    const overlays: BoundOverlay[] = [];
    try {
      boundStructure = this.#bindStructure(plan, structure);
      this.#bindVideos(plan, videos);
      this.#bindOverlays(plan, overlays);
      const extensions = await this.#bindings.bindExtensions(plan);
      // Take both returned collections before either ownership projection or
      // validation can fail; cleanup must retain every transferred resource.
      effects = extensions.effects;
      resources = extensions.resources;
      effects = ownFrameEffects(effects);
      resources = ownPresentationResources(resources);
      validateFrameEffects(effects);
      validatePresentationResources(resources);
      await loadVideos(videos);
    } catch (error) {
      const cleanupFailure = await releasePresentation(
        effects,
        resources,
        structure,
        videos,
        overlays,
      );
      // A binding attempt may mutate author-owned state that generic cleanup
      // cannot prove reusable. A fresh page owns the only valid retry.
      this.#state = { kind: "disposed" };
      throw presentationLoadFailure(error, cleanupFailure);
    }

    this.#state = {
      kind: "loaded",
      effects,
      frameRate: plan.frameRate,
      resources,
      structure: boundStructure,
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
      presentStructure(frame, state.structure);
      presentOverlays(frame, state.overlays);
      await applyFrameEffects(frame, state.effects);
      this.#staged = { frame, videos };
    } catch (error) {
      discardStagedVideos(state.videos);
      this.#staged = undefined;
      this.#state = { ...state, kind: "failed" };
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
      this.#staged = undefined;
      this.#state = { ...this.#loadedState("confirm"), kind: "failed" };
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
      loaded?.structure,
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

  #bindStructure(
    plan: RuntimePlan,
    structure: PendingStructure,
  ): BoundStructure {
    const film = this.#bindings.bindFilm(plan.film);
    structure.film = { placement: plan.film, presentation: film };
    film.setVisible(true);

    for (const placement of plan.scenes) {
      const presentation = this.#bindings.bindScene(placement);
      structure.scenes.push({ placement, presentation });
      presentation.setVisible(false);
    }
    for (const placement of plan.shots) {
      const presentation = this.#bindings.bindShot(placement);
      structure.shots.push({ placement, presentation });
      presentation.setVisible(false);
    }

    return {
      film: structure.film,
      scenes: structure.scenes,
      shots: structure.shots,
    };
  }

  #bindVideos(plan: RuntimePlan, videos: BoundVideo[]): void {
    for (const placement of plan.videos) {
      const presentation = this.#bindings.bindVideo(placement);
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
  return Object.freeze(effects.map(ownFrameEffect));
}

function ownFrameEffect(effect: FrameEffect): FrameEffect {
  return Object.freeze({
    apply: effect.apply.bind(effect),
    dispose: effect.dispose.bind(effect),
  });
}

function validateFrameEffects(effects: readonly FrameEffect[]): void {
  if (effects.length > MAX_PRESENTATION_EFFECTS) {
    throw new RuntimeAdapterError(
      "operation",
      "presentation frame-effect count exceeds its limit",
    );
  }
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

function presentStructure(
  frame: RuntimeFrame,
  structure: BoundStructure,
): void {
  presentContainers(frame, structure.scenes);
  presentContainers(frame, structure.shots);
}

function presentContainers<
  T extends { readonly interval: RuntimeOverlay["interval"] },
>(frame: RuntimeFrame, containers: readonly BoundContainer<T>[]): void {
  for (const container of containers) {
    const { interval } = container.placement;
    const visible = frame.index >= interval.start && frame.index < interval.end;
    container.presentation.setVisible(visible);
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
  structure: PendingStructure | BoundStructure | undefined,
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
  const structureFailure = releaseStructure(structure);
  failure ??= structureFailure;
  return failure;
}

function releaseStructure(
  structure: PendingStructure | BoundStructure | undefined,
): unknown | undefined {
  if (structure === undefined) {
    return undefined;
  }

  let failure = releaseContainers(structure.shots);
  const sceneFailure = releaseContainers(structure.scenes);
  failure ??= sceneFailure;
  if (structure.film !== undefined) {
    const filmFailure = releaseContainer(structure.film);
    failure ??= filmFailure;
  }
  return failure;
}

function releaseContainers<T>(
  containers: readonly BoundContainer<T>[],
): unknown | undefined {
  let failure: unknown;
  // Children release before parents so cleanup never relies on detached DOM.
  for (let index = containers.length - 1; index >= 0; index -= 1) {
    const container = containers[index];
    if (container !== undefined) {
      const releaseFailure = releaseContainer(container);
      failure ??= releaseFailure;
    }
  }
  return failure;
}

function releaseContainer<T>(
  container: BoundContainer<T>,
): unknown | undefined {
  return releaseAll([
    () => container.presentation.setVisible(false),
    () => container.presentation.dispose(),
  ]);
}

async function releaseFrameEffects(
  effects: readonly FrameEffect[],
): Promise<unknown | undefined> {
  let failure: unknown;
  for (const effect of effects.toReversed()) {
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
