// Closed-form spring behavior across exact and non-monotonic frame samples.
// The same sample must never depend on an earlier call or worker partition.

import assert from "node:assert/strict";
import test from "node:test";

import { spring } from "../src/index.js";

test("samples one spring directly at arbitrary exact frames", () => {
  const frame = { frameRate: { numerator: 30, denominator: 1 }, localFrame: 0 };
  assert.equal(spring(frame), 0);

  const later = spring({ ...frame, localFrame: 15 });
  const earlier = spring({ ...frame, localFrame: 6 });
  const repeated = spring({ ...frame, localFrame: 15 });

  assert.ok(earlier > 0);
  assert.ok(later > earlier);
  assert.equal(repeated, later);
});

test("uses the exact rational frame rate projection", () => {
  const ntsc = spring({
    frameRate: { numerator: 30_000, denominator: 1_001 },
    localFrame: 30_000,
  });
  const seconds = spring({
    frameRate: { numerator: 1, denominator: 1 },
    localFrame: 1_001,
  });

  assert.equal(ntsc, seconds);
});

test("can clamp physical overshoot without changing spring time", () => {
  const sample = {
    frameRate: { numerator: 30, denominator: 1 },
    localFrame: 10,
  };
  const options = { damping: 1, stiffness: 180 } as const;

  assert.ok(spring(sample, options) > 1);
  assert.equal(spring(sample, { ...options, overshoot: "clamp" }), 1);
});

test("covers underdamped critical and overdamped spring regimes", () => {
  const sample = {
    frameRate: { numerator: 30, denominator: 1 },
    localFrame: 15,
  };
  const underdamped = spring(sample, { damping: 10 });
  const critical = spring(sample, { damping: 20 });
  const overdamped = spring(sample, { damping: 30 });

  assert.ok(underdamped > critical);
  assert.ok(critical > overdamped);
  assert.ok(overdamped > 0);
});

test("uses the stable critical solution near a repeated spring root", () => {
  const sample = {
    frameRate: { numerator: 60, denominator: 1 },
    localFrame: 30,
  };
  const critical = spring(sample, { damping: 20 });
  const nearlyCritical = spring(sample, { damping: 20 + 1e-8 });

  assert.equal(nearlyCritical, critical);
});

test("rejects invalid spring facts before evaluating physics", () => {
  assert.throws(
    () =>
      spring({
        frameRate: { numerator: 0, denominator: 1 },
        localFrame: 0,
      }),
    /frame rate/,
  );
  assert.throws(
    () =>
      spring(
        {
          frameRate: { numerator: 30, denominator: 1 },
          localFrame: 0,
        },
        { damping: 0 },
      ),
    /damping/,
  );
  assert.throws(
    () =>
      Reflect.apply(spring, undefined, [
        {
          frameRate: { numerator: 30, denominator: 1 },
          localFrame: 0,
        },
        { overshoot: "invalid" },
      ]),
    /overshoot/,
  );
});
