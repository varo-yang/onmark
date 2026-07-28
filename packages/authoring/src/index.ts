// Public facade for semantic browser authoring and optional motion adapters.

export {
  frameMotion,
  type FrameMotionContext,
  type FrameMotionDefinition,
  type FrameMotionHandler,
  type TransitionFrameMotionContext,
  type TransitionFrameMotionHandler,
} from "./frame-motion.js";
export {
  easing,
  interpolate,
  type EasingFunction,
  type Extrapolation,
  type InterpolationOptions,
} from "./interpolation.js";
export { spring, type SpringFrame, type SpringOptions } from "./spring.js";
export {
  combineMotion,
  type PresentationExtension,
  type PresentationExtensionContext,
  type PresentationElementTarget,
  type PresentationElementTargetKind,
  type PresentationTarget,
  type PresentationTargetKind,
  type PresentationTransitionTarget,
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
