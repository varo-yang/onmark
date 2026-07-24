// Public facade for semantic browser authoring and optional motion adapters.

export {
  combineMotion,
  type PresentationExtension,
  type PresentationExtensionContext,
  type PresentationTarget,
  type PresentationTargetKind,
} from "./motion.js";
export {
  createDomPresentationBindings,
  type DomPresentationOptions,
  type VideoSource,
} from "./presentation.js";
export {
  createFontResource,
  createImageResource,
  type FontResource,
  type FontResourceOptions,
  type ImageResource,
  type ImageResourceOptions,
} from "./resource.js";
