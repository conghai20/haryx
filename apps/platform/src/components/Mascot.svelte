<script lang="ts">
  // An account's pixel mascot, still: the Windows app's sprite renderer drawn once on a canvas.
  // The web has no live usage per account, so it always stands idle.
  import { CANVAS, pixels, speciesTitle, stillPose, type Style } from "@win/lib/mascot";

  let { style }: { style: Style } = $props();
  let canvas = $state<HTMLCanvasElement>();
  const grid = $derived(pixels(style, stillPose("idle")));

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

<span class="mascot" role="img" aria-label="{speciesTitle(style.species)} mascot">
  <canvas bind:this={canvas} width={CANVAS} height={CANVAS}></canvas>
</span>

<style>
  .mascot { display: block; flex-shrink: 0; }
  canvas { display: block; width: 2.5rem; height: 2.5rem; flex-shrink: 0; image-rendering: pixelated; }
</style>
