// Native HTML image freezing at the deterministic bundle boundary.
// Local bytes become opaque resources; projected documents retain exact uses.

import { createHash } from "node:crypto";
import { open } from "node:fs/promises";
import { extname, isAbsolute, resolve } from "node:path";

import { parse, type DefaultTreeAdapterTypes, type ParserError } from "parse5";

import { hasAmbientImageAnimation } from "./image_admission.js";

const IMAGE_EXTENSIONS = new Set([
  ".avif",
  ".gif",
  ".jpeg",
  ".jpg",
  ".png",
  ".svg",
  ".webp",
]);
const DATA_IMAGE_EXTENSIONS = new Map([
  ["image/avif", ".avif"],
  ["image/gif", ".gif"],
  ["image/jpeg", ".jpeg"],
  ["image/png", ".png"],
  ["image/svg+xml", ".svg"],
  ["image/webp", ".webp"],
]);
const RESOURCE_DIRECTORY = "resources";

/** One local image frozen into the presentation artifact. */
export interface HtmlImageResource {
  readonly contents: Uint8Array;
  readonly path: string;
}

/** Invalid or unreadable native image input. */
export class HtmlImageError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "HtmlImageError";
  }
}

/** Native image bytes exceed the bounded presentation artifact. */
export class HtmlImageLimitError extends HtmlImageError {
  constructor() {
    super("authored HTML images exceed the presentation output-byte limit");
    this.name = "HtmlImageLimitError";
  }
}

/** Freezes local `img[src]` bytes and rewrites their authored references. */
export async function freezeHtmlImages(
  source: string,
  directory: string,
  maxOutputBytes: number,
): Promise<{
  readonly document: string;
  readonly resources: readonly HtmlImageResource[];
}> {
  const references = imageReferences(source);
  const files = new Map<string, Uint8Array>();
  const sources = new Map<string, string>();
  const edits: SourceEdit[] = [];
  let retainedBytes = 0;

  for (const reference of references) {
    const inline = inlineImageSource(reference.source);
    if (inline !== undefined) {
      rejectAmbientAnimation("inline image", inline.extension, inline.contents);
      continue;
    }
    const local = localImageSource(reference.source);
    const absolute = resolve(directory, local.path);
    let path = sources.get(absolute);
    if (path === undefined) {
      const contents = await readBoundedImage(absolute, maxOutputBytes);
      rejectAmbientAnimation(local.path, local.extension, contents);
      path = resourcePath(contents, local.extension);
      sources.set(absolute, path);
      if (!files.has(path)) {
        retainedBytes = retainBytes(
          retainedBytes,
          contents.byteLength,
          maxOutputBytes,
        );
        files.set(path, contents);
      }
    }
    edits.push({
      ...reference.valueRange,
      replacement: `./${path}${local.suffix}`,
    });
  }

  const resources = [...files]
    .sort(([left], [right]) => comparePath(left, right))
    .map(([path, contents]) => Object.freeze({ contents, path }));
  return Object.freeze({
    document: applyEdits(source, edits),
    resources: Object.freeze(resources),
  });
}

/** Selects only frozen images referenced by one projected document. */
export function projectedImageResources(
  document: string,
  resources: readonly HtmlImageResource[],
): readonly HtmlImageResource[] {
  const referenced = new Set(
    imageReferences(document)
      .map(({ source }) => bundledResourcePath(source))
      .filter((path): path is string => path !== undefined),
  );
  return resources.filter(({ path }) => referenced.has(path));
}

// ── Authored references

interface ImageReference {
  readonly source: string;
  readonly valueRange: SourceRange;
}

interface SourceRange {
  readonly end: number;
  readonly start: number;
}

interface SourceEdit extends SourceRange {
  readonly replacement: string;
}

interface SourceOffsets {
  readonly endOffset: number;
  readonly startOffset: number;
}

function imageReferences(source: string): ImageReference[] {
  const references: ImageReference[] = [];
  visit(parseDocument(source), (element) => {
    rejectResponsiveImageSource(element);
    if (element.tagName !== "img") {
      return;
    }
    const attribute = element.attrs.find(({ name }) => name === "src");
    if (attribute === undefined) {
      return;
    }
    const location = element.sourceCodeLocation?.attrs?.["src"];
    if (location === undefined) {
      throw new HtmlImageError("cannot locate src on authored <img>");
    }
    references.push({
      source: attribute.value,
      valueRange: attributeValueRange(source, location),
    });
  });
  return references;
}

function rejectResponsiveImageSource(
  element: DefaultTreeAdapterTypes.Element,
): void {
  if (
    (element.tagName === "img" || element.tagName === "source") &&
    element.attrs.some(({ name }) => name === "srcset")
  ) {
    throw new HtmlImageError(
      "authored HTML images must use src; srcset is not yet supported",
    );
  }
}

function attributeValueRange(
  source: string,
  location: SourceOffsets,
): SourceRange {
  const attribute = source.slice(location.startOffset, location.endOffset);
  const equals = attribute.indexOf("=");
  if (equals === -1) {
    throw new HtmlImageError("local image src must have a value");
  }
  let start = equals + 1;
  while (start < attribute.length && isHtmlSpace(attribute[start]!)) {
    start += 1;
  }
  const quote = attribute[start];
  return quote === '"' || quote === "'"
    ? {
        end: location.endOffset - 1,
        start: location.startOffset + start + 1,
      }
    : {
        end: location.endOffset,
        start: location.startOffset + start,
      };
}

interface LocalImageSource {
  readonly extension: string;
  readonly path: string;
  readonly suffix: string;
}

function localImageSource(source: string): LocalImageSource {
  if (source.length === 0 || source !== source.trim()) {
    throw new HtmlImageError("local image src must be nonempty and trimmed");
  }
  if (source.startsWith("//") || /^[a-z][a-z0-9+.-]*:/iu.test(source)) {
    throw new HtmlImageError(
      "authored HTML images must use a local relative path or data URL",
    );
  }

  const suffixOffset = source.search(/[?#]/u);
  const encodedPath =
    suffixOffset === -1 ? source : source.slice(0, suffixOffset);
  const suffix = suffixOffset === -1 ? "" : source.slice(suffixOffset);
  const path = decodeImagePath(encodedPath);
  const extension = extname(path).toLowerCase();
  if (!IMAGE_EXTENSIONS.has(extension)) {
    throw new HtmlImageError(
      `authored HTML image uses unsupported extension ${extension || "(none)"}`,
    );
  }
  return { extension, path, suffix };
}

interface InlineImageSource {
  readonly contents: Uint8Array;
  readonly extension: string;
}

function inlineImageSource(source: string): InlineImageSource | undefined {
  if (!/^data:/iu.test(source)) {
    return undefined;
  }
  const separator = source.indexOf(",");
  if (separator === -1) {
    throw new HtmlImageError("inline image src is not a valid data URL");
  }
  const metadata = source.slice("data:".length, separator).split(";");
  const mime = metadata.shift()?.toLowerCase() ?? "";
  const extension = DATA_IMAGE_EXTENSIONS.get(mime);
  if (extension === undefined) {
    throw new HtmlImageError(
      `inline image uses unsupported media type ${mime || "(none)"}`,
    );
  }
  const base64 = metadata.some((value) => value.toLowerCase() === "base64");
  const unsupported = metadata.filter(
    (value) =>
      value.length > 0 &&
      value.toLowerCase() !== "base64" &&
      !/^charset=/iu.test(value),
  );
  if (unsupported.length > 0) {
    throw new HtmlImageError("inline image data URL has unsupported metadata");
  }

  const payload = source.slice(separator + 1);
  const contents = base64
    ? decodeBase64Image(payload)
    : decodeTextImage(payload, extension);
  return { contents, extension };
}

function decodeBase64Image(source: string): Uint8Array {
  if (
    !/^(?:[a-z0-9+/]{4})*(?:[a-z0-9+/]{2}==|[a-z0-9+/]{3}=)?$/iu.test(source)
  ) {
    throw new HtmlImageError("inline image data URL has invalid base64");
  }
  return Buffer.from(source, "base64");
}

function decodeTextImage(source: string, extension: string): Uint8Array {
  if (extension !== ".svg") {
    throw new HtmlImageError("inline binary images must use base64 data URLs");
  }
  try {
    return new TextEncoder().encode(decodeURIComponent(source));
  } catch (error) {
    throw new HtmlImageError("inline image data URL is not valid UTF-8", {
      cause: error,
    });
  }
}

function rejectAmbientAnimation(
  source: string,
  extension: string,
  contents: Uint8Array,
): void {
  if (hasAmbientImageAnimation(extension, contents)) {
    throw new HtmlImageError(
      `${source} contains ambient animation; use an Onmark frame effect`,
    );
  }
}

function decodeImagePath(source: string): string {
  let path;
  try {
    path = decodeURIComponent(source);
  } catch (error) {
    throw new HtmlImageError("local image src is not a valid URL", {
      cause: error,
    });
  }
  if (path.includes("\\") || isAbsolute(path)) {
    throw new HtmlImageError(
      "authored HTML images must use a portable relative path",
    );
  }
  return path;
}

function bundledResourcePath(source: string): string | undefined {
  const suffixOffset = source.search(/[?#]/u);
  const path = suffixOffset === -1 ? source : source.slice(0, suffixOffset);
  const prefix = `./${RESOURCE_DIRECTORY}/`;
  return path.startsWith(prefix) ? path.slice(2) : undefined;
}

// ── Frozen bytes

async function readBoundedImage(
  path: string,
  limit: number,
): Promise<Uint8Array> {
  let file;
  try {
    file = await open(path, "r");
  } catch (error) {
    throw new HtmlImageError(`cannot open local image ${path}`, {
      cause: error,
    });
  }

  try {
    const chunks: Uint8Array[] = [];
    let length = 0;
    while (length <= limit) {
      const capacity = Math.min(64 * 1024, limit - length + 1);
      const chunk = Buffer.allocUnsafe(capacity);
      const { bytesRead } = await file.read(chunk, 0, capacity, length);
      if (bytesRead === 0) {
        return Buffer.concat(chunks, length);
      }
      chunks.push(chunk.subarray(0, bytesRead));
      length += bytesRead;
    }
    throw new HtmlImageLimitError();
  } catch (error) {
    if (error instanceof HtmlImageError) {
      throw error;
    }
    throw new HtmlImageError(`cannot read local image ${path}`, {
      cause: error,
    });
  } finally {
    await file.close();
  }
}

function retainBytes(retained: number, added: number, limit: number): number {
  if (added > limit - retained) {
    throw new HtmlImageLimitError();
  }
  return retained + added;
}

function resourcePath(contents: Uint8Array, extension: string): string {
  const digest = createHash("sha256").update(contents).digest("hex");
  return `${RESOURCE_DIRECTORY}/${digest}${extension}`;
}

// ── Parsed HTML and edits

function parseDocument(source: string): DefaultTreeAdapterTypes.Document {
  const errors: ParserError[] = [];
  const document = parse(source, {
    onParseError(error) {
      if (error.code !== "missing-doctype") {
        errors.push(error);
      }
    },
    sourceCodeLocationInfo: true,
  });
  if (errors.length > 0) {
    throw new HtmlImageError(
      `cannot freeze images after parse error ${errors[0]!.code}`,
    );
  }
  return document;
}

function visit(
  node: DefaultTreeAdapterTypes.Node,
  visitor: (element: DefaultTreeAdapterTypes.Element) => void,
): void {
  const pending = [node];
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined) {
      break;
    }
    if ("tagName" in current) {
      visitor(current);
    }
    if ("childNodes" in current) {
      for (let index = current.childNodes.length - 1; index >= 0; index -= 1) {
        const child = current.childNodes[index];
        if (child !== undefined) {
          pending.push(child);
        }
      }
    }
  }
}

function applyEdits(source: string, edits: readonly SourceEdit[]): string {
  const ordered = edits.toSorted((left, right) => left.start - right.start);
  const parts: string[] = [];
  let cursor = 0;

  for (const edit of ordered) {
    if (edit.start < cursor || edit.end < edit.start) {
      throw new HtmlImageError("authored image edits overlap");
    }
    parts.push(source.slice(cursor, edit.start), edit.replacement);
    cursor = edit.end;
  }
  parts.push(source.slice(cursor));
  return parts.join("");
}

function comparePath(left: string, right: string): number {
  if (left < right) {
    return -1;
  }
  if (left > right) {
    return 1;
  }
  return 0;
}

function isHtmlSpace(character: string): boolean {
  return (
    character === "\t" ||
    character === "\n" ||
    character === "\f" ||
    character === "\r" ||
    character === " "
  );
}
