#!/usr/bin/env node
// Executable boundary for one bounded presentation build.

import process from "node:process";
import { parseArgs } from "node:util";

import {
  BundleError,
  bundleDomPresentation,
  bundlePresentation,
  type BundleOptions,
  type DomBundleOptions,
} from "./presentation.js";
import {
  BUNDLE_FRAME_BEHAVIORS,
  BUNDLE_TEMPORAL_CAPABILITIES,
  BUNDLE_VISUAL_CAPABILITIES,
} from "./generated/bundle-manifest.js";

const USAGE = [
  "Usage: onmark-bundle",
  "  (--entry <path> | --semantic-dom [--stylesheet <path>] [--motion <path>])",
  "  --output <directory>",
  "  --max-output-bytes <bytes>",
  "  --frame-behavior <perFrame|placementBounded>",
  "  --temporal-capability <sequential|randomAccess>",
  "  --visual-capability <browserComposite|separableOverlay>",
  "",
].join("\n");

type Command =
  | { readonly kind: "custom"; readonly options: BundleOptions }
  | { readonly kind: "semanticDom"; readonly options: DomBundleOptions }
  | { readonly kind: "help" };

// ── Execution ──

try {
  const command = parseArguments(process.argv.slice(2));
  if (command.kind === "help") {
    process.stdout.write(USAGE);
  } else if (command.kind === "custom") {
    await bundlePresentation(command.options);
  } else {
    await bundleDomPresentation(command.options);
  }
} catch (error) {
  const failure = commandFailure(error);
  process.stderr.write(`${failure.kind}: ${failure.message}\n`);
  process.exitCode = failure.kind === "configuration" ? 2 : 1;
}

// ── Arguments ──

function parseArguments(arguments_: readonly string[]): Command {
  const values = commandValues(arguments_);
  if (values.help !== undefined) {
    if (arguments_.length !== 1) {
      throw configuration("--help cannot be combined with bundle options");
    }
    return { kind: "help" };
  }

  const outputDirectory = oneValue(values.output, "--output");
  const maxOutputBytes = parseByteLimit(
    oneValue(values["max-output-bytes"], "--max-output-bytes"),
  );
  const frameBehavior = parseFrameBehavior(
    oneValue(values["frame-behavior"], "--frame-behavior"),
  );
  const temporalCapability = parseTemporalCapability(
    oneValue(values["temporal-capability"], "--temporal-capability"),
  );
  const visualCapability = parseVisualCapability(
    oneValue(values["visual-capability"], "--visual-capability"),
  );

  const controls = {
    frameBehavior,
    maxOutputBytes,
    outputDirectory,
    temporalCapability,
    visualCapability,
  };
  return presentationCommand(values, controls);
}

type BundleControls = Omit<BundleOptions, "entryPoint">;

function presentationCommand(
  values: ReturnType<typeof commandValues>,
  controls: BundleControls,
): Exclude<Command, { readonly kind: "help" }> {
  const entryPoint = optionalOneValue(values.entry, "--entry");
  const semanticDom = values["semantic-dom"] === true;
  const motion = optionalOneValue(values.motion, "--motion");
  const stylesheet = optionalOneValue(values.stylesheet, "--stylesheet");

  if (entryPoint !== undefined) {
    if (semanticDom || stylesheet !== undefined || motion !== undefined) {
      throw invalidPresentationSource();
    }
    return { kind: "custom", options: { ...controls, entryPoint } };
  }
  if (!semanticDom) {
    throw invalidPresentationSource();
  }
  const options: DomBundleOptions = {
    ...controls,
    ...(motion === undefined ? {} : { motion }),
    ...(stylesheet === undefined ? {} : { stylesheet }),
  };
  return { kind: "semanticDom", options };
}

function invalidPresentationSource(): BundleError {
  return configuration(
    "use --entry alone or --semantic-dom with optional style and motion files",
  );
}

function commandValues(arguments_: readonly string[]) {
  try {
    return parseArgs({
      args: arguments_,
      allowPositionals: false,
      options: {
        entry: { type: "string", multiple: true },
        "frame-behavior": { type: "string", multiple: true },
        help: { type: "boolean" },
        "max-output-bytes": { type: "string", multiple: true },
        motion: { type: "string", multiple: true },
        output: { type: "string", multiple: true },
        "semantic-dom": { type: "boolean" },
        stylesheet: { type: "string", multiple: true },
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

function parseFrameBehavior(value: string): BundleOptions["frameBehavior"] {
  const behavior = BUNDLE_FRAME_BEHAVIORS.find(
    (candidate) => candidate === value,
  );
  if (behavior !== undefined) {
    return behavior;
  }
  throw configuration("--frame-behavior must be perFrame or placementBounded");
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

function optionalOneValue(
  values: readonly string[] | undefined,
  name: string,
): string | undefined {
  if (values !== undefined && values.length > 1) {
    throw configuration(`${name} cannot be repeated`);
  }
  return values?.[0];
}

function parseByteLimit(value: string): number {
  const bytes = Number(value);
  if (!Number.isSafeInteger(bytes) || bytes <= 0) {
    throw configuration("--max-output-bytes must be a positive safe integer");
  }
  return bytes;
}

// ── Failures ──

function configuration(message: string, cause?: unknown): BundleError {
  return new BundleError("configuration", message, cause);
}

function commandFailure(error: unknown): BundleError {
  if (error instanceof BundleError) {
    return error;
  }
  return new BundleError("output", "unexpected bundler failure", error);
}
