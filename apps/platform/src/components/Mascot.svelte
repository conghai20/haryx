<script lang="ts">
  // An account's pixel mascot, still: the Windows app's sprite renderer drawn once on a canvas.
  // The web has no live usage per account, so it always stands idle. Decorative: the account's
  // name is announced beside it, so screen readers skip the mascot.
  import { CANVAS, pixels, stillPose, type Style } from "@win/lib/mascot";

  let { style, class: className = "size-10" }: { style: Style; class?: string } = $props();
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

<canvas bind:this={canvas} width={CANVAS} height={CANVAS} class="block shrink-0 [image-rendering:pixelated] {className}" aria-hidden="true"></canvas>
