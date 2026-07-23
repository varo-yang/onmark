// Native process ownership preserves Rust CLI semantics and terminal behavior.

import { spawn, type SpawnOptions } from "node:child_process";

export interface ProductTools {
  readonly browserProvisioner: string;
  readonly bundler: string;
  readonly ffmpeg: string;
  readonly ffprobe: string;
  readonly nativeCli: string;
  readonly node: string;
}

export interface NativeResult {
  readonly code: number | null;
  readonly signal: NodeJS.Signals | null;
}

export interface NativeInvocation {
  readonly arguments: readonly string[];
  readonly command: string;
  readonly options: SpawnOptions;
}

export function nativeInvocation(
  arguments_: readonly string[],
  tools: ProductTools,
  environment: NodeJS.ProcessEnv,
): NativeInvocation {
  const options: SpawnOptions = {
    env: {
      ...environment,
      ONMARK_BROWSER_PROVISIONER: tools.node,
      ONMARK_BROWSER_PROVISIONER_ENTRY: tools.browserProvisioner,
      ONMARK_BUNDLER: tools.node,
      ONMARK_BUNDLER_ENTRY: tools.bundler,
      ONMARK_FFMPEG: tools.ffmpeg,
      ONMARK_FFPROBE: tools.ffprobe,
    },
    stdio: "inherit",
  };
  return Object.freeze({
    arguments: Object.freeze([...arguments_]),
    command: tools.nativeCli,
    options,
  });
}

export async function runNative(
  arguments_: readonly string[],
  tools: ProductTools,
  environment: NodeJS.ProcessEnv,
): Promise<NativeResult> {
  const invocation = nativeInvocation(arguments_, tools, environment);
  const child = spawn(
    invocation.command,
    invocation.arguments,
    invocation.options,
  );

  return new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => {
      resolve(Object.freeze({ code, signal }));
    });
  });
}
