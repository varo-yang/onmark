// Controllable browser-video capability shared by runtime behavior tests.

import assert from "node:assert/strict";

import type { BrowserVideoElement } from "../src/index.js";

type VideoEvent = "error" | "loadeddata" | "seeked";

export class FakeVideoElement implements BrowserVideoElement {
  readonly #listeners = new Map<VideoEvent, Set<() => void>>();
  readonly #frameCallbacks = new Map<
    number,
    (now: number, metadata: { readonly mediaTime: number }) => void
  >();
  #nextFrameCallback = 1;
  #currentTime = 0;
  #hasSource = false;
  #src = "";
  readonly #loadAutomatically: boolean;
  readonly #frameCallbackTimes: number[] = [];
  frameCallbackError: Error | undefined;
  loadCount = 0;
  loadError: Error | undefined;
  seekCount = 0;

  constructor(loadAutomatically = false) {
    this.#loadAutomatically = loadAutomatically;
  }

  get currentTime(): number {
    return this.#currentTime;
  }

  get frameCallbackTimes(): readonly number[] {
    return this.#frameCallbackTimes;
  }

  set currentTime(value: number) {
    this.#currentTime = value;
    this.seekCount += 1;
  }

  get hasSource(): boolean {
    return this.#hasSource;
  }

  get listenerCount(): number {
    let count = 0;
    for (const listeners of this.#listeners.values()) {
      count += listeners.size;
    }
    return count;
  }

  get pendingFrameCallbacks(): number {
    return this.#frameCallbacks.size;
  }

  get src(): string {
    return this.#src;
  }

  set src(value: string) {
    this.#src = value;
    this.#hasSource = true;
  }

  addEventListener(type: VideoEvent, listener: () => void): void {
    const listeners = this.#listeners.get(type) ?? new Set();
    listeners.add(listener);
    this.#listeners.set(type, listeners);
  }

  cancelVideoFrameCallback(handle: number): void {
    this.#frameCallbacks.delete(handle);
  }

  load(): void {
    this.loadCount += 1;
    if (this.loadError !== undefined) {
      throw this.loadError;
    }
    if (this.#loadAutomatically && this.#hasSource) {
      queueMicrotask(() => this.emit("loadeddata"));
    }
  }

  removeAttribute(name: "src"): void {
    assert.equal(name, "src");
    this.#src = "";
    this.#hasSource = false;
  }

  removeEventListener(type: VideoEvent, listener: () => void): void {
    this.#listeners.get(type)?.delete(listener);
  }

  requestVideoFrameCallback(
    callback: (now: number, metadata: { readonly mediaTime: number }) => void,
  ): number {
    if (this.frameCallbackError !== undefined) {
      throw this.frameCallbackError;
    }
    const handle = this.#nextFrameCallback;
    this.#nextFrameCallback += 1;
    this.#frameCallbackTimes.push(this.#currentTime);
    this.#frameCallbacks.set(handle, callback);
    return handle;
  }

  emit(type: VideoEvent): void {
    for (const listener of this.#listeners.get(type) ?? []) {
      listener();
    }
  }

  present(mediaTime: number): void {
    const callbacks = [...this.#frameCallbacks.values()];
    this.#frameCallbacks.clear();
    for (const callback of callbacks) {
      callback(0, { mediaTime });
    }
  }
}
