// Public facade for deterministic presentation bundling.

export {
  BundleError,
  bundlePresentation,
  type BundleArtifact,
  type BundleErrorKind,
  type BundleFile,
  type BundleManifest,
  type BundleOptions,
} from "./presentation.js";

export {
  BUNDLE_ENTRY_POINT,
  BUNDLE_MANIFEST_FILE,
  BUNDLE_VERSION,
} from "./generated/bundle-manifest.js";
export { BUNDLE_ASSET_DIRECTORY } from "@onmark/runtime";
