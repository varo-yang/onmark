// Cache tests preserve host conventions without touching ambient environment.

import assert from "node:assert/strict";
import test from "node:test";

import { browserCacheDirectory } from "../src/cache.js";

test("uses native cache roots on release platforms", () => {
  assert.equal(
    browserCacheDirectory(
      { arch: "arm64", platform: "darwin" },
      "/Users/author",
      {},
    ),
    "/Users/author/Library/Caches/onmark/browser",
  );
  assert.equal(
    browserCacheDirectory({ arch: "x64", platform: "linux" }, "/home/author", {
      xdgCacheHome: "/cache",
    }),
    "/cache/onmark/browser",
  );
  assert.equal(
    browserCacheDirectory(
      { arch: "x64", platform: "win32" },
      "C:\\Users\\author",
      { localAppData: "D:\\Cache" },
    ),
    "D:\\Cache\\onmark\\browser",
  );
});
