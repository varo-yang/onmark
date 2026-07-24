// Process tests verify transparent argument forwarding and private tool paths.

import assert from "node:assert/strict";
import test from "node:test";

import { nativeInvocation } from "../src/native.js";

test("forwards arguments unchanged and supplies release tools through environment", () => {
  const invocation = nativeInvocation(
    ["render", "film.html", "--fps", "30000/1001"],
    {
      browserProvisioner: "/product/browser-command.js",
      bundler: "/product/bundler-command.js",
      ffmpeg: "/tools/ffmpeg",
      ffprobe: "/tools/ffprobe",
      nativeCli: "/tools/onmark",
      node: "/tools/node",
    },
    { HOME: "/home/author" },
  );

  assert.equal(invocation.command, "/tools/onmark");
  assert.deepEqual(invocation.arguments, [
    "render",
    "film.html",
    "--fps",
    "30000/1001",
  ]);
  assert.deepEqual(invocation.options, {
    env: {
      HOME: "/home/author",
      ONMARK_BROWSER_PROVISIONER: "/tools/node",
      ONMARK_BROWSER_PROVISIONER_ENTRY: "/product/browser-command.js",
      ONMARK_BUNDLER: "/tools/node",
      ONMARK_BUNDLER_ENTRY: "/product/bundler-command.js",
      ONMARK_FFMPEG: "/tools/ffmpeg",
      ONMARK_FFPROBE: "/tools/ffprobe",
    },
    stdio: "inherit",
  });
});
