// Cross-language wire compatibility tests for generated runtime codecs.
// Checked-in JSONL examples must decode identically in Rust and TypeScript.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

import {
  MAX_BROWSER_VIDEOS,
  MAX_FAILURE_MESSAGE_CHARACTERS,
  MAX_PENDING_RESOURCE_CHARACTERS,
  MAX_PENDING_RESOURCES,
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
    {
      version: 1,
      requestId: 1,
      command: {
        type: "load",
        plan: {
          timelineVersion: 1,
          frameRate: { numerator: 30, denominator: 1 },
          evaluation: { start: 0, end: 1 },
          output: { start: 0, end: 1 },
          videos: [
            {
              assetId: "opening.mp4",
              interval: { start: 0, end: 1 },
              sourceFrameRate: { numerator: 30, denominator: 1 },
            },
          ],
        },
      },
    },
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

test("rejects protocol payloads outside generated resource budgets", () => {
  const video = {
    assetId:
      "sha256:0101010101010101010101010101010101010101010101010101010101010101",
    interval: { start: 0, end: 1 },
    sourceFrameRate: { numerator: 30, denominator: 1 },
  };
  const request = {
    version: 1,
    requestId: 1,
    command: {
      type: "load",
      plan: {
        timelineVersion: 1,
        frameRate: { numerator: 30, denominator: 1 },
        evaluation: { start: 0, end: 1 },
        output: { start: 0, end: 1 },
        videos: Array.from({ length: MAX_BROWSER_VIDEOS + 1 }, () => video),
      },
    },
  };
  assert.throws(() => decodeBrowserRequest(request), ProtocolDecodeError);

  const oversizedFailures = [
    failure("x".repeat(MAX_FAILURE_MESSAGE_CHARACTERS + 1), []),
    failure(
      "rendering failed",
      Array.from({ length: MAX_PENDING_RESOURCES + 1 }, () => "resource"),
    ),
    failure("rendering failed", [
      "x".repeat(MAX_PENDING_RESOURCE_CHARACTERS + 1),
    ]),
  ];
  for (const response of oversizedFailures) {
    assert.throws(() => decodeBrowserResponse(response), ProtocolDecodeError);
  }
});

async function fixture(filename: string): Promise<unknown[]> {
  const lines = (await readFile(new URL(filename, PROTOCOL_FIXTURES), "utf8"))
    .trim()
    .split("\n");
  return lines.map((line) => JSON.parse(line) as unknown);
}

function failure(message: string, pendingResources: string[]): unknown {
  return {
    version: 1,
    requestId: 1,
    event: {
      type: "failed",
      code: "internal",
      message,
      pendingResources,
    },
  };
}
