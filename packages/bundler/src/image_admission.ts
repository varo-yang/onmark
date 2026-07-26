// Deterministic admission for image bytes consumed by browser presentations.
// Static pixels may cross the bundle boundary; self-advancing formats may not.

import { parse, type DefaultTreeAdapterTypes } from "parse5";

const PNG_SIGNATURE = Uint8Array.from([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a,
]);
const GIF_HEADERS = ["GIF87a", "GIF89a"];
const SVG_ANIMATION_ELEMENTS = new Set([
  "animate",
  "animatemotion",
  "animatetransform",
  "foreignobject",
  "image",
  "script",
  "set",
]);
const CSS_ANIMATION =
  /(?:@keyframes|\banimation(?:-[a-z-]+)?\s*:|\btransition(?:-[a-z-]+)?\s*:)/iu;

/** Reports whether browser wall time could change the decoded image pixels. */
export function hasAmbientImageAnimation(
  extension: string,
  contents: Uint8Array,
): boolean {
  if (
    gifFrameCount(contents) > 1 ||
    pngHasAnimationControl(contents) ||
    webpHasAnimation(contents) ||
    avifIsImageSequence(contents)
  ) {
    return true;
  }
  return extension === ".svg" || looksLikeSvg(contents)
    ? svgHasAnimation(contents)
    : false;
}

// ── Raster containers

function pngHasAnimationControl(contents: Uint8Array): boolean {
  if (!startsWith(contents, PNG_SIGNATURE)) {
    return false;
  }
  let offset = PNG_SIGNATURE.length;
  while (offset + 12 <= contents.length) {
    const length = readU32BigEndian(contents, offset);
    const type = ascii(contents, offset + 4, 4);
    const next = offset + 12 + length;
    if (!Number.isSafeInteger(next) || next > contents.length) {
      return false;
    }
    if (type === "acTL") {
      return true;
    }
    if (type === "IEND") {
      return false;
    }
    offset = next;
  }
  return false;
}

function gifFrameCount(contents: Uint8Array): number {
  if (contents.length < 13 || !GIF_HEADERS.includes(ascii(contents, 0, 6))) {
    return 0;
  }

  let offset = 13 + colorTableBytes(contents[10]!);
  let frames = 0;
  while (offset < contents.length) {
    switch (contents[offset]) {
      case 0x2c: {
        if (offset + 10 > contents.length) {
          return frames;
        }
        frames += 1;
        if (frames > 1) {
          return frames;
        }
        offset += 10 + colorTableBytes(contents[offset + 9]!);
        if (offset >= contents.length) {
          return frames;
        }
        offset = skipSubBlocks(contents, offset + 1);
        break;
      }
      case 0x21:
        offset = skipSubBlocks(contents, offset + 2);
        break;
      case 0x3b:
        return frames;
      default:
        return frames;
    }
  }
  return frames;
}

function colorTableBytes(flags: number): number {
  return (flags & 0x80) === 0 ? 0 : 3 * 2 ** ((flags & 0x07) + 1);
}

function skipSubBlocks(contents: Uint8Array, initialOffset: number): number {
  let offset = initialOffset;
  while (offset < contents.length) {
    const length = contents[offset]!;
    offset += 1;
    if (length === 0) {
      return offset;
    }
    offset += length;
  }
  return contents.length;
}

function webpHasAnimation(contents: Uint8Array): boolean {
  if (ascii(contents, 0, 4) !== "RIFF" || ascii(contents, 8, 4) !== "WEBP") {
    return false;
  }

  let offset = 12;
  while (offset + 8 <= contents.length) {
    const type = ascii(contents, offset, 4);
    const length = readU32LittleEndian(contents, offset + 4);
    const payload = offset + 8;
    const next = payload + length + (length % 2);
    if (!Number.isSafeInteger(next) || next > contents.length) {
      return false;
    }
    if (type === "ANIM" || type === "ANMF") {
      return true;
    }
    if (type === "VP8X" && length > 0 && (contents[payload]! & 0x02) !== 0) {
      return true;
    }
    offset = next;
  }
  return false;
}

function avifIsImageSequence(contents: Uint8Array): boolean {
  if (ascii(contents, 4, 4) !== "ftyp" || contents.length < 16) {
    return false;
  }
  const boxLength = readU32BigEndian(contents, 0);
  if (boxLength < 16 || boxLength > contents.length) {
    return false;
  }
  for (let offset = 8; offset + 4 <= boxLength; offset += 4) {
    const brand = ascii(contents, offset, 4);
    if (brand === "avis" || brand === "msf1") {
      return true;
    }
  }
  return false;
}

// ── SVG document

function svgHasAnimation(contents: Uint8Array): boolean {
  let source;
  try {
    source = new TextDecoder("utf-8", { fatal: true }).decode(contents);
  } catch {
    return false;
  }

  const pending: DefaultTreeAdapterTypes.Node[] = [parse(source)];
  while (pending.length > 0) {
    const current = pending.pop();
    if (current === undefined) {
      break;
    }
    if ("tagName" in current && svgElementHasAnimation(current)) {
      return true;
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
  return false;
}

function looksLikeSvg(contents: Uint8Array): boolean {
  const prefix = new TextDecoder()
    .decode(contents.subarray(0, 512))
    .trimStart();
  return prefix.startsWith("<svg") || prefix.startsWith("<?xml");
}

function svgElementHasAnimation(
  element: DefaultTreeAdapterTypes.Element,
): boolean {
  if (SVG_ANIMATION_ELEMENTS.has(element.tagName.toLowerCase())) {
    return true;
  }
  for (const attribute of element.attrs) {
    if (attribute.name.toLowerCase().startsWith("on")) {
      return true;
    }
    if (attribute.name === "style" && cssCanAnimate(attribute.value)) {
      return true;
    }
  }
  if (element.tagName === "style") {
    return element.childNodes.some(
      (child) => "value" in child && cssCanAnimate(child.value),
    );
  }
  return false;
}

function cssCanAnimate(source: string): boolean {
  return source.includes("\\") || CSS_ANIMATION.test(source);
}

// ── Byte reading

function startsWith(contents: Uint8Array, prefix: Uint8Array): boolean {
  return prefix.every((byte, index) => contents[index] === byte);
}

function ascii(contents: Uint8Array, offset: number, length: number): string {
  if (offset < 0 || offset + length > contents.length) {
    return "";
  }
  return String.fromCharCode(...contents.subarray(offset, offset + length));
}

function readU32BigEndian(contents: Uint8Array, offset: number): number {
  return (
    contents[offset]! * 0x1_00_00_00 +
    (contents[offset + 1]! << 16) +
    (contents[offset + 2]! << 8) +
    contents[offset + 3]!
  );
}

function readU32LittleEndian(contents: Uint8Array, offset: number): number {
  return (
    contents[offset]! +
    (contents[offset + 1]! << 8) +
    (contents[offset + 2]! << 16) +
    contents[offset + 3]! * 0x1_00_00_00
  );
}
