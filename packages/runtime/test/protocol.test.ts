// Cross-language wire compatibility tests for generated runtime codecs.
// Checked-in JSONL examples must decode identically in Rust and TypeScript.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  ProtocolDecodeError,
  decodeBrowserRequest,
  decodeBrowserResponse,
} from "../src/index.js";

// Package tests execute from `dist/test`; URL resolution avoids a caller-cwd
// assumption while preserving the checked-in conformance directory as owner.
const PROTOCOL_FIXTURES = new URL(
  "../../../../conformance/protocol/",
  import.meta.url,
);

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
      event: {
        type: "failed",
        code: "internal",
        message: "",
        pendingResources: [],
      },
    },
    {
      version: 1,
      requestId: 1,
      event: { type: "frameReady", frame: 0, stateHash: "0".repeat(64) },
    },
  ];

  for (const response of invalidResponses) {
    assert.throws(() => decodeBrowserResponse(response), ProtocolDecodeError);
  }
});

async function fixture(filename: string): Promise<unknown[]> {
  const lines = (await readFile(new URL(filename, PROTOCOL_FIXTURES), "utf8"))
    .trim()
    .split("\n");
  return lines.map((line) => JSON.parse(line) as unknown);
}
