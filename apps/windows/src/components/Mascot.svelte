<script lang="ts">
  // An account's pixel mascot (macOS MascotView): its species and colour, its mood's pose and mark,
  // animated at the mood's frame rate with a per-account phase. Still under reduced motion, and
  // paused while the window is hidden.
  import { store } from "../lib/store.svelte";
  import {
    CANVAS,
    fps,
    identityKey,
    phase,
    pixels,
    poseAt,
    speciesTitle,
    stillPose,
    type Style,
  } from "../lib/mascot";
  import type { Account } from "../lib/types";

  // `style` previews a choice before it is saved (the inspector's colour slider).
  let { account, size, style }: { account: Account; size: number; style?: Style } = $props();

  const motion = matchMedia("(prefers-reduced-motion: reduce)");
  let reduced = $state(motion.matches);
  let seconds = $state(Date.now() / 1000);
  let canvas = $state<HTMLCanvasElement>();

  let shown = $derived(style ?? store.styleFor(account));
  let mood = $derived(store.moodFor(account));
  let pose = $derived(reduced ? stillPose(mood) : poseAt(seconds, mood, phase(identityKey(account))));
  let grid = $derived(pixels(shown, pose));

  $effect(() => {
    const onChange = () => (reduced = motion.matches);
    motion.addEventListener("change", onChange);
    return () => motion.removeEventListener("change", onChange);
  });

  $effect(() => {
    if (reduced) return;
    const timer = setInterval(() => {
      if (!document.hidden) seconds = Date.now() / 1000;
    }, 1000 / fps(mood));
    return () => clearInterval(timer);
  });

  $effect(() => {
    const ctx = canvas?.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, CANVAS, CANVAS);
    grid.forEach((row, y) =>
      row.forEach((color, x) => {
        if (!color) return;
        ctx.fillStyle = color;
        ctx.fillRect(x, y, 1, 1);
      }),
    );
  });
</script>

<span class="mascot" role="img" aria-label="{speciesTitle(shown.species)} mascot, {mood}">
  <canvas bind:this={canvas} width={CANVAS} height={CANVAS} style:width="{size}px" style:height="{size}px"></canvas>
</span>

<style>
  .mascot {
    display: block;
    flex-shrink: 0;
  }
  canvas {
    display: block;
    image-rendering: pixelated;
  }
</style>
