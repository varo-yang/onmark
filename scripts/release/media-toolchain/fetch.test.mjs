// Transport-boundary tests for bounded release-source retries.

import assert from "node:assert/strict";
import test from "node:test";

import { fetchSource } from "./fetch.mjs";

const SOURCE = Object.freeze({
  name: "fixture.tar.xz",
  url: "https://example.invalid/fixture.tar.xz",
});

test("retries transient transport failures", async () => {
  const attempts = [];
  const delays = [];
  const response = { body: null, status: 200 };

  const downloaded = await fetchSource(
    SOURCE,
    async (url) => {
      attempts.push(url.href);
      if (attempts.length < 3) {
        throw new TypeError("fetch failed");
      }
      return response;
    },
    async (milliseconds) => {
      delays.push(milliseconds);
    },
  );

  assert.equal(downloaded, response);
  assert.equal(attempts.length, 3);
  assert.deepEqual(delays, [1_000, 4_000]);
});

test("retries only transient HTTP responses", async () => {
  let cancellations = 0;
  let attempts = 0;
  const unavailable = {
    body: {
      async cancel() {
        cancellations += 1;
      },
    },
    status: 503,
  };
  const available = { body: null, status: 200 };

  const downloaded = await fetchSource(
    SOURCE,
    async () => {
      attempts += 1;
      return attempts === 1 ? unavailable : available;
    },
    async () => {},
  );

  assert.equal(downloaded, available);
  assert.equal(attempts, 2);
  assert.equal(cancellations, 1);
});

test("returns permanent HTTP responses without retrying", async () => {
  let attempts = 0;
  const missing = { body: null, status: 404 };

  const downloaded = await fetchSource(
    SOURCE,
    async () => {
      attempts += 1;
      return missing;
    },
    async () => {},
  );

  assert.equal(downloaded, missing);
  assert.equal(attempts, 1);
});

test("does not retry redirect-contract failures", async () => {
  let attempts = 0;
  const redirect = {
    body: { async cancel() {} },
    headers: { get: () => null },
    status: 302,
  };

  await assert.rejects(
    fetchSource(
      SOURCE,
      async () => {
        attempts += 1;
        return redirect;
      },
      async () => {},
    ),
    (error) =>
      error instanceof Error &&
      error.message === "fixture.tar.xz redirects without a location",
  );
  assert.equal(attempts, 1);
});

test("reports the admitted source after bounded retries", async () => {
  let attempts = 0;

  await assert.rejects(
    fetchSource(
      SOURCE,
      async () => {
        attempts += 1;
        throw new TypeError("connection reset");
      },
      async () => {},
    ),
    (error) =>
      error instanceof Error &&
      error.message === "cannot download fixture.tar.xz: connection reset",
  );
  assert.equal(attempts, 4);
});
