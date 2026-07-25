// Release publication tests pin completeness, ordering, and retry behavior.

import assert from "node:assert/strict";
import test from "node:test";

import { admitPublication, publishAdmittedRelease } from "../publication.mjs";

const VERSION = "1.2.3";

test("admits one complete version with the public package last", () => {
  const release = admitPublication(`v${VERSION}`, archives().toReversed());

  assert.deepEqual(
    release.packages.map((archive) => archive.name),
    [
      "@onmark/cli-darwin-arm64",
      "@onmark/cli-linux-x64",
      "@onmark/cli-win32-x64",
      "@onmark/cli",
    ],
  );
  assert.equal(release.distributionTag, "latest");
});

test("routes prereleases through the next distribution tag", () => {
  const version = "1.2.3-rc.1";
  const release = admitPublication(
    `v${version}`,
    archives().map((archive) => ({ ...archive, version })),
  );

  assert.equal(release.distributionTag, "next");
});

test("rejects incomplete, duplicate, or differently versioned sets", () => {
  const complete = archives();

  assert.throws(() => admitPublication(`v${VERSION}`, complete.slice(1)));
  assert.throws(() =>
    admitPublication(`v${VERSION}`, [...complete.slice(1), complete[1]]),
  );
  assert.throws(() =>
    admitPublication(`v${VERSION}`, [
      { ...complete[0], version: "2.0.0" },
      ...complete.slice(1),
    ]),
  );
  assert.throws(() => admitPublication("v9.9.9", complete));
});

test("reuses matching versions and publishes only missing packages", async () => {
  const release = admitPublication(`v${VERSION}`, archives());
  const published = [];
  const registry = {
    async integrity(name) {
      return name === release.packages[0].name
        ? release.packages[0].integrity
        : undefined;
    },
    async publish(path, tag) {
      published.push({ path, tag });
    },
  };

  const outcomes = await publishAdmittedRelease(release, registry);

  assert.equal(outcomes[0].outcome, "reused");
  assert.equal(published.length, 3);
  assert.ok(published.every(({ tag }) => tag === "latest"));
});

test("accepts a matching winner after a publication race", async () => {
  const [archive] = archives();
  const release = admitPublication(`v${VERSION}`, archives());
  let lookups = 0;
  const registry = {
    async integrity(name) {
      lookups += 1;
      return lookups === 1 || name !== archive.name
        ? undefined
        : archive.integrity;
    },
    async publish(path) {
      if (path === archive.path) {
        throw new Error("concurrent publication");
      }
    },
  };

  const outcomes = await publishAdmittedRelease(release, registry);

  assert.equal(outcomes[0].outcome, "reused");
});

test("rejects an occupied version with different bytes", async () => {
  const release = admitPublication(`v${VERSION}`, archives());
  const registry = {
    async integrity() {
      return "sha512-different";
    },
    async publish() {
      assert.fail("an occupied version must not be published");
    },
  };

  await assert.rejects(
    publishAdmittedRelease(release, registry),
    /already exists with different bytes/,
  );
});

function archives() {
  return [
    "@onmark/cli-darwin-arm64",
    "@onmark/cli-linux-x64",
    "@onmark/cli-win32-x64",
    "@onmark/cli",
  ].map((name) =>
    Object.freeze({
      integrity: `sha512-${name}`,
      name,
      path: `/release/${name.replaceAll("/", "-")}.tgz`,
      version: VERSION,
    }),
  );
}
