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
  materializedVideoSource,
  PresentationRuntimeAdapter,
  type FrameEffect,
  type RuntimeFrame,
} from "@onmark/runtime";
import { createDomPresentationBindings } from "@onmark/authoring";

import "./temporal-experiment.css";

const ANIMATION_SECONDS = 1;
const MARKER_TRAVEL_PIXELS = 160;
const VIDEO_READINESS_TIMEOUT_MILLISECONDS = 1_000;

const bindings = createDomPresentationBindings({
  document,
  frameEffects: temporalEffects,
  videoSource: materializedVideoSource,
});
installRuntimeHost(
  new PresentationRuntimeAdapter(
    bindings,
    VIDEO_READINESS_TIMEOUT_MILLISECONDS,
  ),
);

function temporalEffects(): readonly FrameEffect[] {
  const stage = experimentStage();
  const stageLifecycle: FrameEffect = {
    apply(frame: RuntimeFrame): void {
      stage.dataset["frame"] = String(frame.index);
    },
    dispose(): void {
      stage.remove();
    },
  };

  return [
    waapiEffect(stage),
    gsapEffect(stage),
    threeEffect(stage),
    stageLifecycle,
  ];
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

function waapiEffect(stage: HTMLElement): FrameEffect {
  const element = marker(stage, "experiment-waapi");
  const [animation] = element.getAnimations();
  if (animation === undefined) {
    throw new TypeError("the CSS animation did not produce a WAAPI playhead");
  }
  animation.pause();
  return {
    apply(frame): void {
      animation.currentTime = effectTime(frame) * 1_000;
    },
    dispose(): void {
      animation.cancel();
      element.remove();
    },
  };
}

function gsapEffect(stage: HTMLElement): FrameEffect {
  const element = marker(stage, "experiment-gsap");
  const timeline = gsap.timeline({ paused: true });
  timeline.to(element, {
    duration: ANIMATION_SECONDS,
    ease: "none",
    rotation: 180,
    x: MARKER_TRAVEL_PIXELS,
  });

  return {
    apply(frame): void {
      timeline.seek(effectTime(frame), true);
    },
    dispose(): void {
      timeline.kill();
      element.remove();
    },
  };
}

function threeEffect(stage: HTMLElement): FrameEffect {
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
    apply(frame): void {
      mixer.setTime(effectTime(frame));
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

function effectTime(frame: RuntimeFrame): number {
  return Math.min(frame.timeSeconds, ANIMATION_SECONDS);
}
