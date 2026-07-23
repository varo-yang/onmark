// Type-only public contracts consumed by higher browser product layers.

export type { RuntimeFrame } from "./clock.js";
export type { RuntimeVideo } from "./media.js";
export type {
  PresentationTemporalCapability,
  PresentationVisualCapability,
} from "./generated/bundle-layout.js";
export type { RuntimePlan } from "./session.js";
export type {
  ContainerPresentation,
  FrameEffect,
  OverlayPresentation,
  PresentationBindings,
  PresentationExtensions,
  RuntimeOverlay,
  RuntimeNode,
  RuntimeScene,
  RuntimeShot,
  VideoPresentation,
} from "./presentation.js";
export type {
  PresentationResource,
  PresentationResourceKind,
} from "./resource.js";
