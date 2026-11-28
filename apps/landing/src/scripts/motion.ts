// Page motion, in page order (ScrollTriggers refresh in creation order, and the tide demo pins):
//   1. the hero, the page's one authored moment: the headline rises, the X is traced and
//      flooded, the water is drawn under the words;
//   2. the tide demo, driven by scroll (scripts/tide-demo.ts);
//   3. the roadmap tide, filled by scroll;
//   4. the close: the mark's two halves join as "Install HaryX." arrives.
// Content is visible by default. Reduced motion and ?still render every end state, and
// html.no-motion (set in <head> if this script never starts) does the same.

import { gsap } from "gsap";
import { ScrollTrigger } from "gsap/ScrollTrigger";
import { SplitText } from "gsap/SplitText";
import { setupTideDemo } from "./tide-demo";

gsap.registerPlugin(ScrollTrigger, SplitText);

const root = document.documentElement;
const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const still = root.hasAttribute("data-still");
const animate = !reduce && !still && !root.classList.contains("no-motion");
root.classList.add("motion-ready");

const EXPO_OUT = "expo.out";
const TRACE_WIDTH_PX = 1.25;

/**
 * Ready a mark half's trace to be drawn with pathLength="1" dashes. Under non-scaling-stroke
 * Chrome ignores pathLength and dashes in screen pixels, so the trace draws in user units
 * instead, with a stroke width that comes out at TRACE_WIDTH_PX on screen.
 */
function primeTrace(path: SVGPathElement | null): void {
  const m = path?.getScreenCTM();
  if (!path || !m) return;
  const scale = Math.hypot(m.a, m.b) || 1;
  path.style.setProperty("vector-effect", "none");
  // Set directly: GSAP rounds px values, and this width is a fraction of a user unit.
  path.style.setProperty("stroke-width", String(TRACE_WIDTH_PX / scale));
  gsap.set(path, { strokeDasharray: 1, strokeDashoffset: 1, opacity: 1 });
}

function revealAll(): void {
  document.querySelectorAll<HTMLElement>("[data-reveal]").forEach((el) => el.classList.add("is-in"));
}

function setupReveals(): void {
  if (!animate || !("IntersectionObserver" in window)) {
    revealAll();
    return;
  }
  const io = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.add("is-in");
        io.unobserve(entry.target);
      }
    },
    { rootMargin: "0px 0px -12% 0px", threshold: 0.12 },
  );
  document.querySelectorAll<HTMLElement>("[data-reveal]").forEach((el) => io.observe(el));
}

/** The hero entrance as one timeline. Runs after the fonts load, so the lines split where they wrap. */
function playHero(): void {
  const title = document.querySelector<HTMLElement>("[data-hero-title]");
  const tide = document.querySelector<HTMLElement>("[data-hero-tide]");
  const water = tide?.querySelector<SVGSVGElement>(".tideline");
  const follow = gsap.utils.toArray<HTMLElement>("[data-hero-follow]");
  const traceA = document.querySelector<SVGPathElement>(".hero-mark .half--a .trace");
  const traceB = document.querySelector<SVGPathElement>(".hero-mark .half--b .trace");
  const fillA = document.querySelector(".hero-mark .half--a .fill");
  const fillB = document.querySelector(".hero-mark .half--b .fill");
  if (!title || !tide || !water) return;

  // The swell is a loop: it rests whenever the hero is off screen.
  ScrollTrigger.create({
    trigger: tide,
    start: "top bottom",
    end: "bottom top",
    onToggle: (self) => tide.classList.toggle("is-resting", !self.isActive),
  });
  if (!animate) return;

  const split = SplitText.create(title, { type: "lines", mask: "lines" });
  gsap.set(title, { visibility: "visible" });
  primeTrace(traceA);
  primeTrace(traceB);
  const flood = { opacity: 1, clipPath: "inset(0% 0% 0% 0%)", duration: 1, ease: EXPO_OUT };
  // autoRound off: the dash runs 1 → 0 in pathLength units, which whole-pixel rounding would skip.
  const trace = { strokeDashoffset: 0, duration: 1.4, ease: "power2.inOut", autoRound: false };

  gsap
    .timeline()
    // The words first: each line rises out of its own mask.
    .from(split.lines, { yPercent: 108, duration: 1, stagger: 0.09, ease: EXPO_OUT }, 0.1)
    .add(() => split.revert(), 1.4)
    // Then the X, as two strokes: the cyan half, then its counterpart, each flooded once traced.
    .to(traceA, trace, 0.25)
    .to(traceB, trace, 0.5)
    .to(fillA, flood, 1.35)
    .to(fillB, flood, 1.6)
    .to([traceA, traceB], { opacity: 0, duration: 0.6, ease: "power1.out" }, 2.2)
    // The water is drawn under the landed headline, and the lede and install follow it in.
    .to(water, { clipPath: "inset(-24px 0% -24px 0px)", duration: 2, ease: "power2.inOut" }, 0.85)
    .to(follow, { opacity: 1, duration: 0.8, stagger: 0.1, ease: "power2.out" }, 0.6)
    .add(() => tide.classList.add("is-swelling"));
}

/** The hero mark's halves drift apart as the page scrolls and lean toward the pointer. */
function setupMarkDrift(): void {
  const art = document.querySelector<HTMLElement>("[data-drift]");
  if (!art || !animate) return;
  let pointerX = 0;
  let pointerY = 0;
  let queued = false;

  const paint = (): void => {
    queued = false;
    const scroll = Math.min(1, window.scrollY / (window.innerHeight * 0.9));
    art.style.setProperty("--split", scroll.toFixed(3));
    art.style.setProperty("--px", pointerX.toFixed(3));
    art.style.setProperty("--py", pointerY.toFixed(3));
  };
  const queue = (): void => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(paint);
  };

  window.addEventListener("scroll", queue, { passive: true });
  if (window.matchMedia("(pointer: fine)").matches) {
    window.addEventListener(
      "pointermove",
      (e) => {
        pointerX = e.clientX / window.innerWidth - 0.5;
        pointerY = e.clientY / window.innerHeight - 0.5;
        queue();
      },
      { passive: true },
    );
  }
  paint();
}

/** The roadmap's tide fills with scroll, up to "now", and lights each milestone it reaches. */
function setupRoadmap(): void {
  const map = document.querySelector<HTMLElement>("[data-roadmap]");
  if (!map) return;
  const fill = map.querySelector<HTMLElement>("[data-roadmap-fill]");
  const now = map.querySelector<HTMLElement>("[data-roadmap-now]");
  const stops = Array.from(map.querySelectorAll<HTMLElement>("[data-stop]"));
  if (!fill || !now) return;

  const paint = (): void => {
    const rect = map.getBoundingClientRect();
    // The rail starts 10px down the road; the tide stops at the "now" marker's centre.
    const nowAt = Math.max(0, now.offsetTop + now.offsetHeight / 2 - 10);
    map.style.setProperty("--now-at", `${nowAt}px`);
    // The tide's edge follows a line 62% down the viewport.
    const reach = animate ? window.innerHeight * 0.62 - rect.top - 10 : Number.POSITIVE_INFINITY;
    const level = Math.max(0, Math.min(nowAt, reach));
    fill.style.transform = `scaleY(${nowAt > 0 ? level / nowAt : 1})`;
    for (const stop of stops) {
      const at = stop.offsetTop + 16;
      stop.classList.toggle("is-lit", animate ? reach >= at : true);
    }
    map.classList.toggle("is-now", level >= nowAt - 1);
  };

  ScrollTrigger.create({ trigger: map, start: "top bottom", end: "bottom top", onUpdate: paint, onRefresh: paint });
  paint();
}

/** The close resolves the page: the mark's two halves start apart and join. */
function setupJoin(): void {
  const art = document.querySelector<HTMLElement>("[data-join]");
  if (!art || !animate) return;
  gsap.fromTo(
    art,
    { "--split": 3.2, opacity: 0.35 },
    {
      "--split": 0,
      opacity: 1,
      duration: 1.6,
      ease: EXPO_OUT,
      scrollTrigger: { trigger: art, start: "top 82%", toggleActions: "play none none reverse" },
    },
  );
}

setupReveals();
setupMarkDrift();
document.fonts.ready.then(() => {
  playHero();
  document.querySelectorAll<HTMLElement>("[data-tide]").forEach((fig) => setupTideDemo(fig, animate));
  setupRoadmap();
  setupJoin();
  ScrollTrigger.refresh();
});
