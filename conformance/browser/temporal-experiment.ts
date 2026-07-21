// Gate-five evidence fixture for exact-frame browser animation playheads.
// Every library is paused; RuntimeFrame is the only authority that advances it.

import { gsap } from "gsap";
import {
  AnimationClip,
  AnimationMixer,
  BoxGeometry,
  Mesh,
  MeshBasicMaterial,
  NumberKeyframeTrack,
  OrthographicCamera,
  Scene,
  WebGLRenderer,
} from "three";

import {
  installRuntimeHost,
  RuntimeAdapterError,
  type RuntimeAdapter,
  type RuntimeFrame,
  type RuntimePlan,
} from "@onmark/runtime";

import "./temporal-experiment.css";

const ANIMATION_SECONDS = 1;
const MARKER_TRAVEL_PIXELS = 160;

class TemporalExperiment implements RuntimeAdapter {
  readonly #stage = experimentStage();
  readonly #waapi = waapiEffect(this.#stage);
  readonly #gsap = gsapEffect(this.#stage);
  readonly #three = threeEffect(this.#stage);
  #state: "disposed" | "empty" | "loaded" = "empty";

  async load(_plan: RuntimePlan): Promise<void> {
    if (this.#state !== "empty") {
      throw new RuntimeAdapterError(
        "operation",
        "temporal experiment requires the empty state",
      );
    }
    this.#state = "loaded";
  }

  async prepare(frame: RuntimeFrame): Promise<void> {
    this.#apply(frame);
  }

  async seek(frame: RuntimeFrame): Promise<void> {
    this.#apply(frame);
  }

  async confirm(_frame: RuntimeFrame): Promise<void> {}

  async dispose(): Promise<void> {
    if (this.#state === "disposed") {
      return;
    }
    this.#state = "disposed";
    this.#waapi.cancel();
    this.#gsap.dispose();
    this.#three.dispose();
    this.#stage.remove();
  }

  #apply(frame: RuntimeFrame): void {
    if (this.#state !== "loaded") {
      throw new RuntimeAdapterError(
        "operation",
        "temporal experiment requires a loaded plan",
      );
    }

    const time = Math.min(frame.timeSeconds, ANIMATION_SECONDS);
    this.#waapi.currentTime = time * 1_000;
    this.#gsap.seek(time);
    this.#three.seek(time);
    this.#stage.dataset["frame"] = String(frame.index);
  }
}

installRuntimeHost(new TemporalExperiment());

interface SeekableEffect {
  seek(timeSeconds: number): void;
  dispose(): void;
}

function experimentStage(): HTMLElement {
  const stage = document.createElement("main");
  stage.className = "experiment-stage";
  document.body.append(stage);
  return stage;
}

function marker(stage: HTMLElement, className: string): HTMLElement {
  const element = document.createElement("div");
  element.className = `experiment-marker ${className}`;
  stage.append(element);
  return element;
}

function waapiEffect(stage: HTMLElement): Animation {
  const element = marker(stage, "experiment-waapi");
  const [animation] = element.getAnimations();
  if (animation === undefined) {
    throw new TypeError("the CSS animation did not produce a WAAPI playhead");
  }
  animation.pause();
  return animation;
}

function gsapEffect(stage: HTMLElement): SeekableEffect {
  const element = marker(stage, "experiment-gsap");
  const timeline = gsap.timeline({ paused: true });
  timeline.to(element, {
    duration: ANIMATION_SECONDS,
    ease: "none",
    rotation: 180,
    x: MARKER_TRAVEL_PIXELS,
  });

  return {
    seek(timeSeconds): void {
      timeline.seek(timeSeconds, true);
    },
    dispose(): void {
      timeline.kill();
    },
  };
}

function threeEffect(stage: HTMLElement): SeekableEffect {
  const canvas = document.createElement("canvas");
  canvas.className = "experiment-webgl";
  stage.append(canvas);

  const renderer = new WebGLRenderer({ alpha: true, antialias: false, canvas });
  renderer.setPixelRatio(1);
  renderer.setSize(96, 72, false);

  const scene = new Scene();
  const camera = new OrthographicCamera(-1.5, 1.5, 1.125, -1.125, 0.1, 10);
  camera.position.z = 4;
  const geometry = new BoxGeometry(1.4, 1.4, 1.4);
  const material = new MeshBasicMaterial({ color: 0x5aa7ff });
  const cube = new Mesh(geometry, material);
  scene.add(cube);

  const rotation = new NumberKeyframeTrack(
    ".rotation[y]",
    [0, ANIMATION_SECONDS],
    [0, Math.PI * 2],
  );
  const clip = new AnimationClip("turn", ANIMATION_SECONDS, [rotation]);
  const mixer = new AnimationMixer(cube);
  mixer.clipAction(clip).play();

  return {
    seek(timeSeconds): void {
      mixer.setTime(timeSeconds);
      renderer.render(scene, camera);
    },
    dispose(): void {
      mixer.stopAllAction();
      geometry.dispose();
      material.dispose();
      renderer.dispose();
      canvas.remove();
    },
  };
}
