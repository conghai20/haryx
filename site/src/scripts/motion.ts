// Page motion: scroll reveals, the hero mark's routing drift, and the roadmap tide.
// Content is visible by default; motion only runs when the visitor allows it.

const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const still = document.documentElement.hasAttribute("data-still");
const animate = !reduce && !still;

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

/** The mark's halves drift apart as the page scrolls and lean toward the pointer. */
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

  let queued = false;
  const paint = (): void => {
    queued = false;
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
  const queue = (): void => {
    if (queued) return;
    queued = true;
    requestAnimationFrame(paint);
  };
  window.addEventListener("scroll", queue, { passive: true });
  window.addEventListener("resize", queue);
  paint();
}

setupReveals();
setupMarkDrift();
setupRoadmap();
