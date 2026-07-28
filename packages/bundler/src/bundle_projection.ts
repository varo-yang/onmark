// Bounded reader for the Rust-owned render-region projection contract.
// It validates process input before authored HTML is projected or published.

import { open } from "node:fs/promises";
import { resolve } from "node:path";

import type { BundleProjection } from "./generated/bundle-projection.js";
import { validateBundleProjection } from "./generated/bundle-projection-validator.js";

const MAX_PROJECTION_BYTES = 1024 * 1024;

/** Invalid or unreadable projection data at the bundler process boundary. */
export class BundleProjectionError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "BundleProjectionError";
  }
}

/** Reads and validates one bounded Rust-owned projection snapshot. */
export async function readBundleProjection(
  path: string,
): Promise<BundleProjection> {
  const absolute = resolve(path);
  let file;
  try {
    file = await open(absolute, "r");
  } catch (error) {
    throw new BundleProjectionError(
      `cannot open bundle projection ${absolute}`,
      { cause: error },
    );
  }

  try {
    const bytes = await readBoundedProjection(file);
    const value: unknown = JSON.parse(
      new TextDecoder("utf-8", { fatal: true }).decode(bytes),
    );
    return decodeBundleProjection(value);
  } catch (error) {
    if (error instanceof BundleProjectionError) {
      throw error;
    }
    throw new BundleProjectionError(
      `cannot read bundle projection ${absolute}`,
      { cause: error },
    );
  } finally {
    await file.close();
  }
}

/** Decodes one checked projection at an in-process or file boundary. */
export function decodeBundleProjection(value: unknown): BundleProjection {
  if (!validateBundleProjection(value)) {
    throw invalidProjection();
  }
  return value;
}

async function readBoundedProjection(
  file: Awaited<ReturnType<typeof open>>,
): Promise<Uint8Array> {
  const bytes = Buffer.allocUnsafe(MAX_PROJECTION_BYTES + 1);
  let length = 0;
  while (length < bytes.length) {
    const { bytesRead } = await file.read(
      bytes,
      length,
      bytes.length - length,
      length,
    );
    if (bytesRead === 0) {
      break;
    }
    length += bytesRead;
  }
  if (length > MAX_PROJECTION_BYTES) {
    throw new BundleProjectionError(
      `bundle projection exceeds the ${MAX_PROJECTION_BYTES}-byte limit`,
    );
  }
  return bytes.subarray(0, length);
}

function invalidProjection(): BundleProjectionError {
  const error = validateBundleProjection.errors?.[0];
  const path = error?.instancePath ?? "";
  const detail = error?.message ?? "does not match its schema";
  return new BundleProjectionError(`bundle projection${path} ${detail}`.trim());
}
