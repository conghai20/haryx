// The tide demo (components/TideDemo.astro), driven by scroll: the visitor's scroll raises the
// active account's 5-hour usage, and crossing the threshold fires the switch. Scrolling back
// undoes it. Replay plays the same rise on its own. Sample data only.

import { gsap } from "gsap";
import { ScrollTrigger } from "gsap/ScrollTrigger";

gsap.registerPlugin(ScrollTrigger);

/** Pin the demo only where the whole figure fits on screen while pinned. */
const PIN_QUERY = "(min-width: 761px) and (min-height: 700px)";
const FREE_QUERY = "(max-width: 760px), (max-height: 699px)";

export function setupTideDemo(fig: HTMLElement, animate: boolean): void {
  const threshold = Number(fig.dataset.threshold ?? 90);
  const rows = Array.from(fig.querySelectorAll<HTMLElement>("[data-row]"));
  const event = fig.querySelector<HTMLElement>("[data-event]");
  const replay = fig.querySelector<HTMLButtonElement>("[data-replay]");
  const first = rows[0];
  const next = rows[1];
  if (!first || !next || !event) return;

  const pctEl = first.querySelector<HTMLElement>("[data-pct]");
  const staff = first.querySelector<HTMLElement>(".staff");
  const from = Number(first.dataset.from);
  const to = Number(first.dataset.to);
  const pace = Number(first.dataset.pace);
  let isSwitched: boolean | null = null;

  // Three-Step Fill Rule: amber at 85% or once 5 points ahead of pace.
  const level = (v: number): string => (v >= 85 || v > pace + 5 ? "amber" : "live");

  function announceSwitch(): void {
    event!.replaceChildren(
      "work reached ",
      Object.assign(document.createElement("strong"), { textContent: `${to}%` }),
      `, past its ${threshold}% threshold. Claude Code now runs as `,
      Object.assign(document.createElement("strong"), { textContent: "personal" }),
      ".",
    );
  }

  function setSwitched(on: boolean): void {
    if (on === isSwitched) return;
    isSwitched = on;
    fig.dataset.switched = String(on);
    first!.dataset.active = String(!on);
    next!.dataset.active = String(on);
    if (on) announceSwitch();
    else event!.textContent = `Auto-switch is watching work. Threshold ${threshold}%.`;
  }

  function render(v: number): void {
    const rounded = Math.round(v);
    if (pctEl) pctEl.textContent = String(rounded);
    staff?.style.setProperty("--v", String(v / 100));
    first!.dataset.level = level(rounded);
    setSwitched(rounded >= threshold);
  }

  if (!animate) {
    // Reduced motion and ?still show the outcome: the switch has happened.
    render(to);
    return;
  }

  const state = { v: from };
  const draw = (): void => render(state.v);
  render(from);

  // The rise takes most of the scroll; the last stretch holds the switched state before release.
  const rise = (): gsap.core.Timeline =>
    gsap
      .timeline({ paused: true })
      .to(state, { v: to, duration: 0.78, ease: "power1.in", onUpdate: draw })
      .to({}, { duration: 0.22 });

  const mm = gsap.matchMedia();
  mm.add(PIN_QUERY, () => {
    ScrollTrigger.create({
      trigger: fig,
      start: "center 55%",
      end: "+=70%",
      pin: true,
      scrub: 0.6,
      animation: rise(),
    });
  });
  mm.add(FREE_QUERY, () => {
    ScrollTrigger.create({
      trigger: fig,
      start: "top 85%",
      end: "bottom 40%",
      scrub: 0.6,
      animation: rise(),
    });
  });

  replay?.addEventListener("click", () => {
    gsap.fromTo(state, { v: from }, { v: to, duration: 1.6, delay: 0.2, ease: "power2.out", onUpdate: draw });
  });
}
