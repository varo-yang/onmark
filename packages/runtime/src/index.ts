// Public facade for the browser runtime package.
// Consumers never reach into generated codecs or session internals directly.

export {
  ProtocolDecodeError,
  decodeBrowserRequest,
  decodeBrowserResponse,
} from "./generated/codec.js";
export type { BrowserRequest } from "./generated/browser-request.js";
export type { BrowserResponse } from "./generated/browser-response.js";
export type { BrowserPlan } from "./generated/browser-request.js";
export { runtimeFrameAt, type RuntimeFrame } from "./clock.js";
export {
  RUNTIME_HOST_NAME,
  installRuntimeHost,
  type RuntimeHost,
} from "./host.js";
export {
  RuntimeAdapterError,
  RuntimeSession,
  type RuntimeAdapter,
  type RuntimeAdapterFailureKind,
  type RuntimePlan,
} from "./session.js";
