import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import test from "node:test";

import {
  ProtocolDecodeError,
  decodeBrowserRequest,
  decodeBrowserResponse,
} from "../src/index.js";

test("decodes every checked-in Gate-one protocol example", async () => {
  for (const request of await fixture("browser-requests-v1.jsonl")) {
    assert.deepEqual(decodeBrowserRequest(request), request);
  }
  for (const response of await fixture("browser-responses-v1.jsonl")) {
    assert.deepEqual(decodeBrowserResponse(response), response);
  }
});

test("rejects values outside the versioned browser contract", () => {
  const invalidRequests = [
    { version: 2, requestId: 1, command: { type: "dispose" } },
    { version: 1, requestId: 1, command: { type: "seek", frame: 2 ** 53 } },
    { version: 1, requestId: 2 ** 32, command: { type: "dispose" } },
    { version: 1, requestId: 1, command: { type: "dispose", surprise: true } },
  ];

  for (const request of invalidRequests) {
    assert.throws(() => decodeBrowserRequest(request), ProtocolDecodeError);
  }

  const invalidResponses = [
    {
      version: 1,
      requestId: 1,
      event: { type: "failed", code: "internal", message: "", pendingResources: [] },
    },
    {
      version: 1,
      requestId: 1,
      event: { type: "frameReady", frame: 0, stateHash: "A".repeat(64) },
    },
  ];

  for (const response of invalidResponses) {
    assert.throws(() => decodeBrowserResponse(response), ProtocolDecodeError);
  }
});

async function fixture(filename: string): Promise<unknown[]> {
  const path = resolve(process.cwd(), "../../conformance/protocol", filename);
  const lines = (await readFile(path, "utf8")).trim().split("\n");
  return lines.map((line) => JSON.parse(line) as unknown);
}
