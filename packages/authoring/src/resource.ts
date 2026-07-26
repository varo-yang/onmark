// Standard image and font resources for deterministic browser preparation.
// Each adapter owns the platform object it releases at terminal disposal.

import type { PresentationResource } from "@onmark/runtime/types";

/** Inputs required to create one decoded image resource. */
export interface ImageResourceOptions {
  readonly document: Document;
  readonly id: string;
  readonly source: string;
}

/** An owned image resource whose element remains available for authored layout. */
export interface ImageResource extends PresentationResource {
  readonly element: HTMLImageElement;
  readonly kind: "image";
  prepare(): Promise<void>;
  dispose(): void;
}

/** Inputs required to load one exact font face into a document font set. */
export interface FontResourceOptions {
  readonly face: FontFace;
  readonly fonts: FontFaceSet;
  readonly id: string;
}

/** An owned font face installed into one document font set. */
export interface FontResource extends PresentationResource {
  readonly kind: "font";
  prepare(): Promise<void>;
  dispose(): void;
}

/** Creates one image whose decoded pixels gate browser preparation. */
export function createImageResource(
  options: ImageResourceOptions,
): ImageResource {
  const element = options.document.createElement("img");
  element.src = options.source;
  return new DecodedImageResource(element, options.id, () => element.remove());
}

/** Creates one font resource installed only after its exact face loads. */
export function createFontResource(options: FontResourceOptions): FontResource {
  return new OwnedFontResource(options);
}

/** Collects static authored images under the runtime readiness boundary. */
export function authoredImageResources(
  elements: readonly HTMLImageElement[],
  reserved: readonly PresentationResource[],
): readonly PresentationResource[] {
  const identities = new Set(
    reserved.filter(({ kind }) => kind === "image").map(({ id }) => id),
  );
  let candidate = 0;
  return elements.map((element) => {
    while (identities.has(`authored-image-${candidate}`)) {
      candidate += 1;
    }
    const id = `authored-image-${candidate}`;
    identities.add(id);
    candidate += 1;
    return new DecodedImageResource(element, id);
  });
}

class DecodedImageResource implements ImageResource {
  readonly element: HTMLImageElement;
  readonly id: string;
  readonly kind = "image" as const;
  readonly #release: (() => void) | undefined;
  #disposed = false;

  constructor(element: HTMLImageElement, id: string, release?: () => void) {
    this.element = element;
    this.id = id;
    this.#release = release;
  }

  async prepare(): Promise<void> {
    this.#requireActive();
    await this.element.decode();
    this.#requireActive();
  }

  dispose(): void {
    if (this.#disposed) {
      return;
    }
    this.#disposed = true;
    this.element.removeAttribute("src");
    this.#release?.();
  }

  #requireActive(): void {
    if (this.#disposed) {
      throw new TypeError("image resource is disposed");
    }
  }
}

class OwnedFontResource implements FontResource {
  readonly id: string;
  readonly kind = "font" as const;
  readonly #face: FontFace;
  readonly #fonts: FontFaceSet;
  #disposed = false;

  constructor({ face, fonts, id }: FontResourceOptions) {
    this.#face = face;
    this.#fonts = fonts;
    this.id = id;
  }

  async prepare(): Promise<void> {
    this.#requireActive();
    const face = await this.#face.load();
    this.#requireActive();
    this.#fonts.add(face);
  }

  dispose(): void {
    if (this.#disposed) {
      return;
    }
    this.#disposed = true;
    this.#fonts.delete(this.#face);
  }

  #requireActive(): void {
    if (this.#disposed) {
      throw new TypeError("font resource is disposed");
    }
  }
}
