// Real-process fixture for the public element-local GSAP adapter surface.

import { gsapMotion } from "onmark/motion/gsap";

export const motion = gsapMotion({
  title({ element, timeline }) {
    timeline.from(element, {
      duration: 0.35,
      ease: "power2.out",
      opacity: 0,
      x: -24,
    });
  },
  callToAction({ element, timeline }) {
    timeline.from(element, {
      duration: 0.25,
      ease: "power2.out",
      opacity: 0,
      y: 12,
    });
  },
});
