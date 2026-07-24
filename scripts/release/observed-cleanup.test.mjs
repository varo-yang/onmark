// Cleanup arbitration preserves every terminal action and cleanup outcome.

import assert from "node:assert/strict";
import test from "node:test";

import { withObservedCleanup } from "./observed-cleanup.mjs";

test("returns the action value after successful cleanup", async () => {
  const value = await withObservedCleanup(
    async () => "artifact",
    async () => {},
    "unused",
  );

  assert.equal(value, "artifact");
});

test("preserves an action failure after successful cleanup", async () => {
  const failure = new Error("action failed");

  await assert.rejects(
    withObservedCleanup(
      async () => {
        throw failure;
      },
      async () => {},
      "unused",
    ),
    (error) => error === failure,
  );
});

test("reports a cleanup failure after a successful action", async () => {
  const failure = new Error("cleanup failed");

  await assert.rejects(
    withObservedCleanup(
      async () => "artifact",
      async () => {
        throw failure;
      },
      "unused",
    ),
    (error) => error === failure,
  );
});

test("retains action and cleanup failures in observation order", async () => {
  const actionFailure = new Error("action failed");
  const cleanupFailure = new Error("cleanup failed");

  await assert.rejects(
    withObservedCleanup(
      async () => {
        throw actionFailure;
      },
      async () => {
        throw cleanupFailure;
      },
      "action and cleanup failed",
    ),
    (error) =>
      error instanceof AggregateError &&
      error.message === "action and cleanup failed" &&
      error.errors[0] === actionFailure &&
      error.errors[1] === cleanupFailure,
  );
});
