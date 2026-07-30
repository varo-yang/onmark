// GSAP adapter behavior under exact, non-monotonic runtime frame requests.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import type { PresentationExtensionContext } from "@onmark/authoring/types";
import { gsapMotion } from "../src/index.js";

test("emits a self-contained public timeline type", async () => {
  const declaration = await readFile(
    new URL("../src/index.d.ts", import.meta.url),
    "utf8",
  );

  assert.match(declaration, /import \{ gsap \} from "gsap"/);
  assert.match(declaration, /ReturnType<typeof gsap\.timeline>/);
});

test("seeks a paused local timeline from exact runtime frames", async () => {
  const state = { value: 0 };
  const motion = gsapMotion({
    shot({ durationSeconds, timeline }) {
      assert.equal(durationSeconds, 1);
      timeline.to(state, { duration: 1, ease: "none", value: 100 });
    },
  });
  const extension = await motion.bind(CONTEXT);
  const [effect] = extension.effects;
  assert.ok(effect);

  await effect.apply({ index: 45, timeSeconds: 1.5 });
  assert.equal(state.value, 50);
  await effect.apply({ index: 20, timeSeconds: 2 / 3 });
  assert.equal(state.value, 50);
  await effect.apply({ index: 30, timeSeconds: 1 });
  assert.equal(state.value, 0);
  await effect.apply({ index: 59, timeSeconds: 59 / 30 });
  assert.ok(state.value > 95 && state.value < 100);
  await effect.dispose();
});

test("renders timeline state when the first requested frame is local zero", async () => {
  const state = { value: 0 };
  const motion = gsapMotion({
    shot({ timeline }) {
      timeline.set(state, { value: 100 }, 0);
    },
  });
  const extension = await motion.bind(CONTEXT);
  const [effect] = extension.effects;
  assert.ok(effect);

  await effect.apply({ index: 30, timeSeconds: 1 });

  assert.equal(state.value, 100);
  state.value = 0;
  await effect.apply({ index: 30, timeSeconds: 1 });
  assert.equal(state.value, 100);
  await effect.dispose();
});

test("renders the completed transition on its final overlap frame", async () => {
  const outgoing = { value: 100 };
  const incoming = { value: 0 };
  const outgoingElement = outgoing as unknown as HTMLElement;
  const incomingElement = incoming as unknown as HTMLElement;
  const motion = gsapMotion({
    transition({ incomingElement, outgoingElement, timeline }) {
      timeline.to(outgoingElement, { duration: 0.5, value: 0 }, 0);
      timeline.to(incomingElement, { duration: 0.5, value: 100 }, 0);
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
  assert.equal(outgoing.value, 100);
  assert.equal(incoming.value, 0);
  await effect.apply({ index: 59, timeSeconds: 59 / 30 });
  assert.equal(outgoing.value, 0);
  assert.equal(incoming.value, 100);
  await effect.dispose();
});

test("renders a one-frame transition at its terminal playhead", async () => {
  const state = { value: 0 };
  const motion = gsapMotion({
    transition({ durationSeconds, timeline }) {
      timeline.to(state, { duration: durationSeconds, value: 100 }, 0);
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

  assert.equal(state.value, 100);
  await effect.dispose();
});

test("rejects motion that escapes its compiler-owned interval", async () => {
  const motion = gsapMotion({
    shot({ timeline }) {
      timeline.to({}, { duration: 2 }, 0);
    },
  });

  await assert.rejects(
    Promise.resolve().then(() => motion.bind(CONTEXT)),
    /shot motion exceeds/,
  );
});

test("accepts a timeline ending at a fractional-frame boundary", async () => {
  const motion = gsapMotion({
    caption({ durationSeconds, timeline }) {
      timeline.to({}, { duration: 0.25 }, Math.max(0, durationSeconds - 0.25));
    },
  });

  await motion.bind({
    frameRate: CONTEXT.frameRate,
    targets: [
      {
        element: {} as HTMLElement,
        interval: { start: 123, end: 218 },
        kind: "caption",
        node: { nodeId: 4 },
      },
    ],
  });
});

test("composes semantic and selector rules without author-owned dispatch", async () => {
  const calls: string[] = [];
  const element = {
    matches: (selector: string) => selector === "#hero",
  } as unknown as HTMLElement;
  const motion = gsapMotion({
    shot() {
      calls.push("kind");
    },
    selectors: {
      "#hero"() {
        calls.push("selector");
      },
    },
  });

  await motion.bind({
    frameRate: CONTEXT.frameRate,
    targets: [{ ...CONTEXT.targets[0]!, element }],
  });

  assert.deepEqual(calls, ["kind", "selector"]);
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
