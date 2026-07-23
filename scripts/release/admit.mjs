// Desktop release admission exercises only the two npm artifacts a user installs.

import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import {
  copyFile,
  mkdir,
  mkdtemp,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const MAX_CAPTURED_BYTES = 1024 * 1024;
const RENDER_TIMEOUT_MILLISECONDS = 10 * 60_000;
const TOOL_TIMEOUT_MILLISECONDS = 2 * 60_000;
const FRAME_RATE = 24;
const EXPECTED_OUTPUT_FRAMES = 45;
const WIDTH = 320;
const HEIGHT = 180;

async function main() {
  const request = parseArguments(process.argv);
  const root = await mkdtemp(join(tmpdir(), "onmark-desktop-admission-"));
  try {
    await admitRelease(request, root);
  } finally {
    await rm(root, { force: true, recursive: true });
  }
}

function parseArguments(argv) {
  const [npmCli, product, sidecar] = argv.slice(2);
  if (
    npmCli === undefined ||
    product === undefined ||
    sidecar === undefined ||
    argv.length !== 5
  ) {
    throw new Error(
      "expected `node scripts/release/admit.mjs <npm-cli.js> <product-directory> <sidecar-directory>`",
    );
  }
  return Object.freeze({
    npmCli: resolve(npmCli),
    product: resolve(product),
    sidecar: resolve(sidecar),
  });
}

// ── Installed-product boundary

async function admitRelease(request, root) {
  const archives = join(root, "archives");
  const consumer = join(root, "consumer");
  const film = join(consumer, "film");
  await mkdir(archives);
  await mkdir(film, { recursive: true });

  const packages = await packRelease(request, archives);
  await installRelease(request.npmCli, consumer, packages);

  const tools = releaseTools(consumer);
  await materializeFilm(film);
  await generateMedia(tools.ffmpeg, film);
  await admitRender(tools, film);
}

async function packRelease(request, destination) {
  const archives = await Promise.all(
    [request.product, request.sidecar].map((directory) =>
      pack(request.npmCli, directory, destination),
    ),
  );
  return Object.freeze(archives);
}

async function pack(npmCli, directory, destination) {
  const result = await captureInvocation(
    npmInvocation(npmCli, [
      "pack",
      directory,
      "--pack-destination",
      destination,
      "--json",
    ]),
    process.cwd(),
    TOOL_TIMEOUT_MILLISECONDS,
  );
  const records = JSON.parse(result.stdout);
  const record = Array.isArray(records) ? records[0] : undefined;
  if (!isObject(record) || typeof record.filename !== "string") {
    throw new Error(`npm pack returned no archive for ${directory}`);
  }
  return join(destination, basename(record.filename));
}

async function installRelease(npmCli, consumer, packages) {
  await writeFile(
    join(consumer, "package.json"),
    '{"name":"onmark-release-admission","private":true,"type":"module"}\n',
  );
  await runInvocation(
    npmInvocation(npmCli, ["install", "--no-audit", "--no-fund", ...packages]),
    consumer,
    TOOL_TIMEOUT_MILLISECONDS,
  );
}

function releaseTools(consumer) {
  const target = releaseTarget();
  const extension = process.platform === "win32" ? ".exe" : "";
  const sidecar = join(
    consumer,
    "node_modules",
    "@onmark",
    `cli-${target}`,
    "bin",
  );
  return Object.freeze({
    ffmpeg: join(sidecar, `ffmpeg${extension}`),
    ffprobe: join(sidecar, `ffprobe${extension}`),
    onmark: Object.freeze({
      arguments: Object.freeze([
        join(
          consumer,
          "node_modules",
          "onmark",
          "packages",
          "launcher",
          "dist",
          "src",
          "command.js",
        ),
      ]),
      command: process.execPath,
    }),
  });
}

function releaseTarget() {
  const target = `${process.platform}-${process.arch}`;
  switch (target) {
    case "darwin-arm64":
    case "linux-x64":
    case "win32-x64":
      return target;
    default:
      throw new Error(`desktop release admission does not support ${target}`);
  }
}

// ── Real render admission

async function materializeFilm(directory) {
  const repository = fileURLToPath(new URL("../../", import.meta.url));
  for (const [source, destination] of [
    ["conformance/cli/gate-one.onmark", "film.onmark"],
    ["conformance/browser/semantic-presentation.css", "film.css"],
    ["conformance/browser/semantic-presentation.motion.ts", "film.motion.ts"],
  ]) {
    await copyFile(join(repository, source), join(directory, destination));
  }
}

async function generateMedia(ffmpeg, directory) {
  const rawVideo = join(directory, "source.rgba");
  const rawAudio = join(directory, "voice.s16le");
  // Raw inputs keep test-only lavfi devices out of the shipped FFmpeg build.
  await writeFile(rawVideo, rawVideoSequence());
  await writeFile(rawAudio, Buffer.alloc(48_000 * 2));

  try {
    await encodeVideo(ffmpeg, directory, rawVideo);
    await encodeAudio(ffmpeg, directory, rawAudio);
  } finally {
    await Promise.all([
      rm(rawVideo, { force: true }),
      rm(rawAudio, { force: true }),
    ]);
  }
}

function rawVideoSequence() {
  const frame = Buffer.alloc(WIDTH * HEIGHT * 4);
  for (let pixel = 0; pixel < WIDTH * HEIGHT; pixel += 1) {
    const offset = pixel * 4;
    const x = pixel % WIDTH;
    const y = Math.floor(pixel / WIDTH);
    frame[offset] = (x * 5 + y * 3) % 256;
    frame[offset + 1] = (x * 2 + y * 7) % 256;
    frame[offset + 2] = (x + y * 11) % 256;
    frame[offset + 3] = 255;
  }

  const sequence = Buffer.alloc(frame.length * FRAME_RATE);
  for (let index = 0; index < FRAME_RATE; index += 1) {
    frame.copy(sequence, index * frame.length);
  }
  return sequence;
}

async function encodeVideo(ffmpeg, directory, input) {
  await run(
    ffmpeg,
    [
      "-nostdin",
      "-v",
      "error",
      "-f",
      "rawvideo",
      "-pixel_format",
      "rgba",
      "-video_size",
      `${WIDTH}x${HEIGHT}`,
      "-framerate",
      String(FRAME_RATE),
      "-i",
      input,
      "-an",
      "-c:v",
      "libx264",
      "-pix_fmt",
      "yuv420p",
      "-g",
      String(FRAME_RATE),
      "-bf",
      "3",
      "-movflags",
      "+faststart",
      "-y",
      join(directory, "source.mp4"),
    ],
    directory,
    TOOL_TIMEOUT_MILLISECONDS,
  );
}

async function encodeAudio(ffmpeg, directory, input) {
  await run(
    ffmpeg,
    [
      "-nostdin",
      "-v",
      "error",
      "-f",
      "s16le",
      "-ar",
      "48000",
      "-ac",
      "1",
      "-i",
      input,
      "-c:a",
      "aac",
      "-b:a",
      "128k",
      "-y",
      join(directory, "voice.m4a"),
    ],
    directory,
    TOOL_TIMEOUT_MILLISECONDS,
  );
}

async function admitRender(tools, directory) {
  const screenplay = join(directory, "film.onmark");
  const first = join(directory, "first.mp4");
  const second = join(directory, "second.mp4");
  // Separate CLI invocations guarantee independent native and browser sessions.
  for (const output of [first, second]) {
    await runInvocation(
      appendArguments(tools.onmark, [
        "render",
        screenplay,
        "--output",
        output,
        "--width",
        String(WIDTH),
        "--height",
        String(HEIGHT),
      ]),
      directory,
      RENDER_TIMEOUT_MILLISECONDS,
    );
  }

  const [firstVideo, secondVideo, firstAudio, secondAudio] = await Promise.all([
    decodedVideoHash(tools.ffmpeg, first, directory),
    decodedVideoHash(tools.ffmpeg, second, directory),
    decodedAudioHash(tools.ffmpeg, first, directory),
    decodedAudioHash(tools.ffmpeg, second, directory),
  ]);
  if (firstVideo !== secondVideo) {
    throw new Error("independent release renders differ after RGBA decoding");
  }
  if (firstAudio !== secondAudio) {
    throw new Error("independent release renders differ after audio decoding");
  }

  const [firstFrames, secondFrames] = await Promise.all([
    verifyStreams(tools.ffprobe, first, directory),
    verifyStreams(tools.ffprobe, second, directory),
  ]);
  if (
    firstFrames !== EXPECTED_OUTPUT_FRAMES ||
    secondFrames !== EXPECTED_OUTPUT_FRAMES
  ) {
    throw new Error(
      `release output has an unexpected frame count: ${firstFrames}, ${secondFrames}`,
    );
  }

  const before = await fileIdentity(first);
  const refusal = await runInvocationStatus(
    appendArguments(tools.onmark, ["render", screenplay, "--output", first]),
    directory,
    RENDER_TIMEOUT_MILLISECONDS,
  );
  if (refusal === 0) {
    throw new Error("release CLI replaced an existing output");
  }
  const after = await fileIdentity(first);
  if (before !== after) {
    throw new Error("failed no-clobber render changed the existing output");
  }
}

function decodedVideoHash(ffmpeg, input, directory) {
  return decodedHash(
    ffmpeg,
    input,
    ["-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"],
    directory,
    "video",
  );
}

function decodedAudioHash(ffmpeg, input, directory) {
  return decodedHash(
    ffmpeg,
    input,
    ["-map", "0:a:0", "-f", "s16le", "-acodec", "pcm_s16le", "pipe:1"],
    directory,
    "audio",
  );
}

async function decodedHash(ffmpeg, input, outputArguments, directory, kind) {
  const child = spawnProcess(
    ffmpeg,
    ["-nostdin", "-v", "error", "-i", input, ...outputArguments],
    directory,
  );
  const digest = createHash("sha256");
  let bytes = 0;
  child.stdout.on("data", (chunk) => {
    bytes += chunk.byteLength;
    digest.update(chunk);
  });
  const stderr = retainBounded(child.stderr, () => child.kill());
  const status = await waitForChild(child, TOOL_TIMEOUT_MILLISECONDS);
  if (status !== 0) {
    throw new Error(`cannot decode release ${kind}: ${await stderr}`);
  }
  if (bytes === 0) {
    throw new Error(`release ${kind} decodes to no samples`);
  }
  return digest.digest("hex");
}

async function verifyStreams(ffprobe, input, directory) {
  const result = await capture(
    ffprobe,
    [
      "-v",
      "error",
      "-count_frames",
      "-show_entries",
      "stream=codec_type,codec_name,nb_read_frames",
      "-of",
      "json",
      input,
    ],
    directory,
    TOOL_TIMEOUT_MILLISECONDS,
  );
  const value = JSON.parse(result.stdout);
  const streams =
    isObject(value) && Array.isArray(value.streams) ? value.streams : [];
  const video = streams.find(
    (stream) => isObject(stream) && stream.codec_type === "video",
  );
  const audio = streams.find(
    (stream) => isObject(stream) && stream.codec_type === "audio",
  );
  const frames = isObject(video) ? Number(video.nb_read_frames) : Number.NaN;
  if (
    !isObject(video) ||
    video.codec_name !== "h264" ||
    !Number.isSafeInteger(frames) ||
    frames <= 0
  ) {
    throw new Error("release output has no decoded H.264 frame sequence");
  }
  if (!isObject(audio) || audio.codec_name !== "aac") {
    throw new Error("release output has no decoded AAC audio stream");
  }
  return frames;
}

// ── Bounded child processes

async function fileIdentity(path) {
  const metadata = await stat(path);
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(path)) {
    digest.update(chunk);
  }
  return `${metadata.size}:${digest.digest("hex")}`;
}

async function run(command, arguments_, cwd, timeout) {
  const status = await runStatus(command, arguments_, cwd, timeout);
  if (status !== 0) {
    throw new Error(`${basename(command)} exited with status ${status}`);
  }
}

async function runInvocation(invocation, cwd, timeout) {
  return run(invocation.command, invocation.arguments, cwd, timeout);
}

async function runStatus(command, arguments_, cwd, timeout) {
  const child = spawnProcess(command, arguments_, cwd, "inherit");
  return waitForChild(child, timeout);
}

async function runInvocationStatus(invocation, cwd, timeout) {
  return runStatus(invocation.command, invocation.arguments, cwd, timeout);
}

async function capture(command, arguments_, cwd, timeout) {
  const child = spawnProcess(command, arguments_, cwd);
  const stdout = retainBounded(child.stdout, () => child.kill());
  const stderr = retainBounded(child.stderr, () => child.kill());
  const status = await waitForChild(child, timeout);
  const result = { stderr: await stderr, stdout: await stdout };
  if (status !== 0) {
    throw new Error(
      `${basename(command)} exited with status ${status}: ${result.stderr}`,
    );
  }
  return result;
}

async function captureInvocation(invocation, cwd, timeout) {
  return capture(invocation.command, invocation.arguments, cwd, timeout);
}

function spawnProcess(command, arguments_, cwd, stdio = "pipe") {
  return spawn(command, arguments_, { cwd, stdio });
}

function retainBounded(stream, abort = () => {}) {
  const chunks = [];
  let bytes = 0;
  let exceeded = false;
  return new Promise((resolvePromise, reject) => {
    stream.on("data", (chunk) => {
      if (exceeded) {
        return;
      }
      bytes += chunk.byteLength;
      if (bytes > MAX_CAPTURED_BYTES) {
        exceeded = true;
        abort();
        reject(new Error("release admission process output exceeds its limit"));
        return;
      }
      chunks.push(chunk);
    });
    stream.once("error", reject);
    stream.once("end", () =>
      resolvePromise(Buffer.concat(chunks).toString("utf8")),
    );
  });
}

async function waitForChild(child, timeout) {
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    child.kill();
  }, timeout);
  try {
    return await new Promise((resolvePromise, reject) => {
      child.once("error", reject);
      child.once("exit", (code, signal) => {
        if (timedOut) {
          reject(new Error(`release admission process exceeded ${timeout}ms`));
          return;
        }
        if (signal !== null) {
          reject(new Error(`release admission process exited with ${signal}`));
          return;
        }
        resolvePromise(code ?? 1);
      });
    });
  } finally {
    clearTimeout(timer);
  }
}

function npmInvocation(npmCli, arguments_) {
  return Object.freeze({
    arguments: Object.freeze([npmCli, ...arguments_]),
    command: process.execPath,
  });
}

function appendArguments(invocation, arguments_) {
  return Object.freeze({
    arguments: Object.freeze([...invocation.arguments, ...arguments_]),
    command: invocation.command,
  });
}

function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

main().catch((error) => {
  const message = error instanceof Error ? error.stack : String(error);
  process.stderr.write(`desktop-release-admission: ${message}\n`);
  process.exitCode = 1;
});
