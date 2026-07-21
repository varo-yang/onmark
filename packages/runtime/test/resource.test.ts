// Presentation-resource readiness, identity, and terminal cleanup.

import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_PRESENTATION_RESOURCES,
  PresentationRuntimeAdapter,
  RuntimeAdapterError,
  runtimeFrameAt,
  type BrowserPlan,
  type PresentationBindings,
  type PresentationResource,
  type PresentationResourceKind,
} from "../src/index.js";

const PLAN: BrowserPlan = {
  timelineVersion: 1,
  frameRate: { numerator: 30, denominator: 1 },
  evaluation: { start: 0, end: 1 },
  output: { start: 0, end: 1 },
  videos: [],
  overlays: [],
};

test("prepares independent presentation resources concurrently", async () => {
  const image = new RecordedResource("image", "poster");
  const font = new RecordedResource("font", "Inter");
  const adapter = resourceAdapter([image, font], 100);

  await adapter.load(PLAN);
  const preparing = adapter.prepare(runtimeFrameAt(0, PLAN.frameRate));
  await Promise.resolve();
  assert.equal(image.preparing, true);
  assert.equal(font.preparing, true);

  image.ready();
  font.ready();
  await preparing;
  await adapter.dispose();
  assert.equal(image.disposed, true);
  assert.equal(font.disposed, true);
});

test("names every presentation resource that misses readiness", async () => {
  const image = new RecordedResource("image", "poster");
  const font = new RecordedResource("font", "Inter");
  const adapter = resourceAdapter([image, font], 5);
  await adapter.load(PLAN);

  await assert.rejects(
    adapter.prepare(runtimeFrameAt(0, PLAN.frameRate)),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.kind === "readinessTimeout" &&
      error.message === "presentation resources did not become ready" &&
      JSON.stringify(error.pendingResources) ===
        JSON.stringify(["image:poster:prepare", "font:Inter:prepare"]),
  );

  await adapter.dispose();
  assert.equal(image.disposed, true);
  assert.equal(font.disposed, true);
});

test("retains timed-out identities beside a preparation failure", async () => {
  const image = new RecordedResource("image", "poster");
  const font = new RecordedResource("font", "Inter");
  const adapter = resourceAdapter([image, font], 5);
  await adapter.load(PLAN);
  image.fail();

  await assert.rejects(
    adapter.prepare(runtimeFrameAt(0, PLAN.frameRate)),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.kind === "operation" &&
      error.message === "image:poster:prepare failed to prepare" &&
      JSON.stringify(error.pendingResources) ===
        JSON.stringify(["font:Inter:prepare"]),
  );

  await adapter.dispose();
});

test("owns resource identity before author objects can mutate", async () => {
  const image = new RecordedResource("image", "poster");
  const adapter = resourceAdapter([image], 5);
  await adapter.load(PLAN);
  Object.defineProperty(image, "id", { value: "changed" });

  await assert.rejects(
    adapter.prepare(runtimeFrameAt(0, PLAN.frameRate)),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      JSON.stringify(error.pendingResources) ===
        JSON.stringify(["image:poster:prepare"]),
  );
  await adapter.dispose();
});

test("rejects duplicate and unbounded presentation resources", async () => {
  const duplicate = [
    new RecordedResource("image", "poster"),
    new RecordedResource("image", "poster"),
  ];
  const duplicateAdapter = resourceAdapter(duplicate, 100);
  await assert.rejects(
    duplicateAdapter.load(PLAN),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "presentation resource identity is duplicated",
  );
  assert.equal(
    duplicate.every((resource) => resource.disposed),
    true,
  );

  const excessive = Array.from(
    { length: MAX_PRESENTATION_RESOURCES + 1 },
    (_, index) => new RecordedResource("custom", `resource-${index}`),
  );
  const excessiveAdapter = resourceAdapter(excessive, 100);
  await assert.rejects(
    excessiveAdapter.load(PLAN),
    (error: unknown) =>
      error instanceof RuntimeAdapterError &&
      error.message === "presentation resource count exceeds its limit",
  );
  assert.equal(
    excessive.every((resource) => resource.disposed),
    true,
  );
});

test("releases every presentation resource after one cleanup failure", async () => {
  const first = new RecordedResource("texture", "hero");
  const second = new RecordedResource("custom", "layout");
  first.rejectDisposal();
  first.ready();
  second.ready();
  const adapter = resourceAdapter([first, second], 100);
  await adapter.load(PLAN);
  await adapter.prepare(runtimeFrameAt(0, PLAN.frameRate));

  await assert.rejects(adapter.dispose(), RuntimeAdapterError);
  assert.equal(first.disposed, true);
  assert.equal(second.disposed, true);
});

// ── Test resources ──

class RecordedResource implements PresentationResource {
  readonly id: string;
  readonly kind: PresentationResourceKind;
  readonly #readiness = Promise.withResolvers<void>();
  #rejectDisposal = false;
  disposed = false;
  preparing = false;

  constructor(kind: PresentationResourceKind, id: string) {
    this.kind = kind;
    this.id = id;
  }

  prepare(): Promise<void> {
    this.preparing = true;
    return this.#readiness.promise;
  }

  dispose(): void {
    this.disposed = true;
    if (this.#rejectDisposal) {
      throw new Error("resource cleanup failed");
    }
  }

  ready(): void {
    this.#readiness.resolve();
  }

  fail(): void {
    this.#readiness.reject(new Error("resource preparation failed"));
  }

  rejectDisposal(): void {
    this.#rejectDisposal = true;
  }
}

function resourceAdapter(
  resources: readonly PresentationResource[],
  timeoutMilliseconds: number,
): PresentationRuntimeAdapter {
  const bindings: PresentationBindings = {
    bindVideo(): never {
      throw new Error("the resource fixture contains no video");
    },
    bindOverlay(): never {
      throw new Error("the resource fixture contains no overlay");
    },
    bindFrameEffects() {
      return [];
    },
    bindResources() {
      return resources;
    },
  };
  return new PresentationRuntimeAdapter(bindings, timeoutMilliseconds);
}
