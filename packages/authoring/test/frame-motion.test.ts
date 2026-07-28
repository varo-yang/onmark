// Exact-frame motion behavior over compiler-owned element intervals.
// Tests request frames out of order because distributed reuse may do the same.

import assert from "node:assert/strict";
import test from "node:test";

import type { PresentationExtensionContext } from "../src/index.js";
import { frameMotion } from "../src/index.js";

test("projects arbitrary runtime frames into one local motion domain", async () => {
  const samples: Array<{
    readonly durationFrames: number;
    readonly localFrame: number;
    readonly progress: number;
  }> = [];
  const motion = frameMotion({
    shot({ durationFrames, localFrame, progress }) {
      samples.push({ durationFrames, localFrame, progress });
    },
  });
  const extension = await motion.bind(CONTEXT);
  const [effect] = extension.effects;
  assert.ok(effect);

  await effect.apply({ index: 45, timeSeconds: 1.5 });
  await effect.apply({ index: 30, timeSeconds: 1 });
  await effect.apply({ index: 59, timeSeconds: 59 / 30 });

  assert.deepEqual(samples, [
    { durationFrames: 30, localFrame: 15, progress: 0.5 },
    { durationFrames: 30, localFrame: 0, progress: 0 },
    { durationFrames: 30, localFrame: 29, progress: 29 / 30 },
  ]);
});

test("does not evaluate local motion outside its solved interval", async () => {
  const frames: number[] = [];
  const motion = frameMotion({
    shot({ localFrame }) {
      frames.push(localFrame);
    },
  });
  const extension = await motion.bind(CONTEXT);
  const [effect] = extension.effects;
  assert.ok(effect);

  await effect.apply({ index: 29, timeSeconds: 29 / 30 });
  await effect.apply({ index: 30, timeSeconds: 1 });
  await effect.apply({ index: 60, timeSeconds: 2 });

  assert.deepEqual(frames, [0]);
});

test("samples a transition from zero through its final overlap frame", async () => {
  const outgoingElement = {} as HTMLElement;
  const incomingElement = {} as HTMLElement;
  const progress: number[] = [];
  const motion = frameMotion({
    transition(context) {
      assert.equal(context.outgoingElement, outgoingElement);
      assert.equal(context.incomingElement, incomingElement);
      progress.push(context.progress);
    },
  });
  const extension = await motion.bind({
    frameRate: CONTEXT.frameRate,
    targets: [
      {
        element: {} as HTMLElement,
        incomingElement,
        interval: { start: 45, end: 60 },
        kind: "transition",
        node: { nodeId: 3, authoredId: "reveal" },
        outgoingElement,
      },
    ],
  });
  const [effect] = extension.effects;
  assert.ok(effect);

  await effect.apply({ index: 45, timeSeconds: 1.5 });
  await effect.apply({ index: 59, timeSeconds: 59 / 30 });

  assert.deepEqual(progress, [0, 1]);
});

test("treats a one-frame transition as its terminal sample", async () => {
  let progress: number | undefined;
  const motion = frameMotion({
    transition(context) {
      progress = context.progress;
    },
  });
  const extension = await motion.bind({
    frameRate: CONTEXT.frameRate,
    targets: [
      {
        element: {} as HTMLElement,
        incomingElement: {} as HTMLElement,
        interval: { start: 45, end: 46 },
        kind: "transition",
        node: { nodeId: 3, authoredId: "reveal" },
        outgoingElement: {} as HTMLElement,
      },
    ],
  });
  const [effect] = extension.effects;
  assert.ok(effect);

  await effect.apply({ index: 45, timeSeconds: 1.5 });

  assert.equal(progress, 1);
});

test("composes semantic and selector motion in declaration order", async () => {
  const calls: string[] = [];
  const element = {
    matches: (selector: string) => selector === "#hero",
  } as unknown as HTMLElement;
  const motion = frameMotion({
    shot() {
      calls.push("kind");
    },
    selectors: {
      "#hero"() {
        calls.push("selector");
      },
    },
  });
  const extension = await motion.bind({
    frameRate: CONTEXT.frameRate,
    targets: [{ ...CONTEXT.targets[0]!, element }],
  });
  const [effect] = extension.effects;
  assert.ok(effect);

  await effect.apply({ index: 30, timeSeconds: 1 });

  assert.deepEqual(calls, ["kind", "selector"]);
});

test("rejects blank selectors at the authoring boundary", () => {
  assert.throws(
    () =>
      frameMotion({
        selectors: {
          "  ": () => undefined,
        },
      }),
    /selector cannot be blank/,
  );
});

const CONTEXT: PresentationExtensionContext = {
  frameRate: { numerator: 30, denominator: 1 },
  targets: [
    {
      kind: "shot",
      element: {} as HTMLElement,
      interval: { start: 30, end: 60 },
      node: { nodeId: 2, authoredId: "hero" },
    },
  ],
};
