// Public contract for installing the browser-global runtime host.
// A no-op adapter isolates decoding, immutability, and single-install ownership.

import assert from "node:assert/strict";
import test from "node:test";

import {
  ProtocolDecodeError,
  RUNTIME_HOST_NAME,
  installRuntimeHost,
  type RuntimeAdapter,
  type RuntimeFrame,
  type RuntimePlan,
} from "../src/index.js";

const adapter: RuntimeAdapter = {
  async load(_plan: RuntimePlan): Promise<void> {},
  async prepare(_frame: RuntimeFrame): Promise<void> {},
  async seek(_frame: RuntimeFrame): Promise<void> {},
  async confirm(_frame: RuntimeFrame): Promise<void> {},
  async dispose(): Promise<void> {},
};

test("installs one immutable host that decodes before dispatch", async () => {
  const scope = {};
  const host = installRuntimeHost(adapter, scope);

  assert.equal(Object.isFrozen(host), true);
  assert.equal(
    Object.getOwnPropertyDescriptor(scope, RUNTIME_HOST_NAME)?.writable,
    false,
  );
  assert.deepEqual(
    await host.dispatch({
      version: 1,
      requestId: 1,
      command: {
        type: "load",
        plan: {
          timelineVersion: 1,
          frameRate: { numerator: 30, denominator: 1 },
          evaluation: { start: 0, end: 1 },
          output: { start: 0, end: 1 },
          videos: [],
          overlays: [],
        },
      },
    }),
    { version: 1, requestId: 1, event: { type: "loaded" } },
  );
  await assert.rejects(
    host.dispatch({ version: 2, requestId: 2, command: { type: "dispose" } }),
    ProtocolDecodeError,
  );
});

test("rejects a second owner for the same browser global", () => {
  const scope = {};
  installRuntimeHost(adapter, scope);

  assert.throws(() => installRuntimeHost(adapter, scope), TypeError);
});
