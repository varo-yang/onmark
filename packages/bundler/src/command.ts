#!/usr/bin/env node
// Executable boundary for one bounded presentation build.

import process from "node:process";
import { parseArgs } from "node:util";

import {
  BundleError,
  bundlePresentation,
  type BundleOptions,
} from "./presentation.js";
import {
  BUNDLE_TEMPORAL_CAPABILITIES,
  BUNDLE_VISUAL_CAPABILITIES,
} from "./generated/bundle-manifest.js";

const USAGE = [
  "Usage: onmark-bundle",
  "  --entry <path>",
  "  --output <directory>",
  "  --max-output-bytes <bytes>",
  "  --temporal-capability <sequential|randomAccess>",
  "  --visual-capability <browserComposite|separableOverlay>",
  "",
].join("\n");

type Command =
  | { readonly kind: "bundle"; readonly options: BundleOptions }
  | { readonly kind: "help" };

try {
  const command = parseArguments(process.argv.slice(2));
  if (command.kind === "help") {
    process.stdout.write(USAGE);
  } else {
    await bundlePresentation(command.options);
  }
} catch (error) {
  const failure = commandFailure(error);
  process.stderr.write(`${failure.kind}: ${failure.message}\n`);
  process.exitCode = failure.kind === "configuration" ? 2 : 1;
}

function parseArguments(arguments_: readonly string[]): Command {
  const values = commandValues(arguments_);
  if (values.help !== undefined) {
    if (arguments_.length !== 1) {
      throw configuration("--help cannot be combined with bundle options");
    }
    return { kind: "help" };
  }

  const entryPoint = oneValue(values.entry, "--entry");
  const outputDirectory = oneValue(values.output, "--output");
  const maxOutputBytes = parseByteLimit(
    oneValue(values["max-output-bytes"], "--max-output-bytes"),
  );
  const temporalCapability = parseTemporalCapability(
    oneValue(values["temporal-capability"], "--temporal-capability"),
  );
  const visualCapability = parseVisualCapability(
    oneValue(values["visual-capability"], "--visual-capability"),
  );

  return {
    kind: "bundle",
    options: {
      entryPoint,
      maxOutputBytes,
      outputDirectory,
      temporalCapability,
      visualCapability,
    },
  };
}

function commandValues(arguments_: readonly string[]) {
  try {
    return parseArgs({
      args: arguments_,
      allowPositionals: false,
      options: {
        entry: { type: "string", multiple: true },
        help: { type: "boolean" },
        "max-output-bytes": { type: "string", multiple: true },
        output: { type: "string", multiple: true },
        "temporal-capability": { type: "string", multiple: true },
        "visual-capability": { type: "string", multiple: true },
      },
      strict: true,
    }).values;
  } catch (error) {
    throw configuration("arguments are invalid", error);
  }
}

function parseTemporalCapability(
  value: string,
): BundleOptions["temporalCapability"] {
  const capability = BUNDLE_TEMPORAL_CAPABILITIES.find(
    (candidate) => candidate === value,
  );
  if (capability !== undefined) {
    return capability;
  }
  throw configuration(
    "--temporal-capability must be sequential or randomAccess",
  );
}

function parseVisualCapability(
  value: string,
): BundleOptions["visualCapability"] {
  const capability = BUNDLE_VISUAL_CAPABILITIES.find(
    (candidate) => candidate === value,
  );
  if (capability !== undefined) {
    return capability;
  }
  throw configuration(
    "--visual-capability must be browserComposite or separableOverlay",
  );
}

function oneValue(values: readonly string[] | undefined, name: string): string {
  const value = values?.[0];
  if (value === undefined) {
    throw configuration(`${name} is required`);
  }
  if (values !== undefined && values.length > 1) {
    throw configuration(`${name} cannot be repeated`);
  }
  return value;
}

function parseByteLimit(value: string): number {
  const bytes = Number(value);
  if (!Number.isSafeInteger(bytes) || bytes <= 0) {
    throw configuration("--max-output-bytes must be a positive safe integer");
  }
  return bytes;
}

function configuration(message: string, cause?: unknown): BundleError {
  return new BundleError("configuration", message, cause);
}

function commandFailure(error: unknown): BundleError {
  if (error instanceof BundleError) {
    return error;
  }
  return new BundleError("output", "unexpected bundler failure", error);
}
