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
