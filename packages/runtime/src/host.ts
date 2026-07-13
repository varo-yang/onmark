// Browser-global host for the native-to-runtime protocol boundary.
// It owns request decoding and exposes exactly one immutable dispatch capability.

import { decodeBrowserRequest } from "./generated/codec.js";
import type { BrowserResponse } from "./generated/browser-response.js";
import { RUNTIME_HOST_NAME } from "./generated/runtime-contract.js";
import { RuntimeSession, type RuntimeAdapter } from "./session.js";

export { RUNTIME_HOST_NAME };

/** Narrow browser capability invoked by the native executor through CDP. */
export interface RuntimeHost {
  /** Decodes and executes one browser request. */
  dispatch(request: unknown): Promise<BrowserResponse>;
}

/** Installs one immutable runtime host on the selected browser global. */
export function installRuntimeHost(
  adapter: RuntimeAdapter,
  scope: object = globalThis,
): RuntimeHost {
  if (Object.hasOwn(scope, RUNTIME_HOST_NAME)) {
    throw new TypeError("the Onmark runtime host is already installed");
  }

  const session = new RuntimeSession(adapter);
  const host: RuntimeHost = Object.freeze({
    async dispatch(value: unknown): Promise<BrowserResponse> {
      return session.dispatch(decodeBrowserRequest(value));
    },
  });
  Object.defineProperty(scope, RUNTIME_HOST_NAME, {
    configurable: false,
    enumerable: false,
    value: host,
    writable: false,
  });
  return host;
}
