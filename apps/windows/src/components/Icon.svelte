<script lang="ts" module>
  // HaryX's icon set for Windows: one 16px grid, 1.5 stroke, round joins, drawn in currentColor so an
  // icon takes its control's colour. Stands in for the SF Symbols the macOS app uses, one for one.
  const PATHS = {
    accounts:
      "M5.5 7.25a2.25 2.25 0 1 0 0-4.5 2.25 2.25 0 0 0 0 4.5ZM1.75 13.25c0-2.07 1.68-3.5 3.75-3.5s3.75 1.43 3.75 3.5M10.75 7.25a2 2 0 1 0 0-4M11.75 9.9c1.45.35 2.5 1.6 2.5 3.35",
    usage: "M2.25 13.75h11.5M4 11.25v-3.5M7 11.25v-8M10 11.25v-5.5M13 11.25v-2",
    sessions: "M2.25 3.25h11.5v9.5H2.25zM4.75 6.25l2 1.75-2 1.75M8.5 10h2.75",
    autoSwitch:
      "M12.75 6.25A5 5 0 0 0 3.6 5M3.25 2.75V5.1h2.35M3.25 9.75A5 5 0 0 0 12.4 11M12.75 13.25V10.9H10.4",
    wake: "M8 14.25a5 5 0 1 0 0-10 5 5 0 0 0 0 10ZM8 6.5v2.75l1.75 1.25M2.5 3.75 4.25 2.25M13.5 3.75l-1.75-1.5",
    mappings:
      "M1.75 4.25c0-.55.45-1 1-1h3.1l1.4 1.5h5.99c.55 0 1 .45 1 1v6.5c0 .55-.45 1-1 1H2.75c-.55 0-1-.45-1-1zM8 10.75h3.25M9.75 9.25l1.5 1.5-1.5 1.5",
    profiles: "M8 1.75 13.75 4.75v6.5L8 14.25 2.25 11.25v-6.5zM2.25 4.75 8 7.75l5.75-3M8 7.75v6.5",
    settings:
      "M2.25 4.25h6.5M11.75 4.25h2M2.25 8h2M7.25 8h6.5M2.25 11.75h5.5M10.75 11.75h3M10.25 2.75v3M5.75 6.5v3M9.25 10.25v3",
    backup:
      "M2.25 9.25h11.5v3.5c0 .55-.45 1-1 1h-9.5c-.55 0-1-.45-1-1zM2.25 9.25l1.6-5.1c.13-.42.52-.65.95-.65h6.4c.43 0 .82.23.95.65l1.6 5.1M11.25 11.5h.01",
    preferences:
      "M8 10.25a2.25 2.25 0 1 0 0-4.5 2.25 2.25 0 0 0 0 4.5ZM6.9 1.75h2.2l.35 1.65 1.2.7 1.6-.53 1.1 1.9-1.25 1.13v1.4l1.25 1.13-1.1 1.9-1.6-.53-1.2.7-.35 1.65H6.9l-.35-1.65-1.2-.7-1.6.53-1.1-1.9L3.9 8.7V7.3L2.65 6.17l1.1-1.9 1.6.53 1.2-.7z",
    about: "M8 14.25a6.25 6.25 0 1 0 0-12.5 6.25 6.25 0 0 0 0 12.5ZM8 7.25v4M8 4.75h.01",
    account: "M8 14.25a6.25 6.25 0 1 0 0-12.5 6.25 6.25 0 0 0 0 12.5ZM8 8.25a2 2 0 1 0 0-4 2 2 0 0 0 0 4ZM4.1 12.5c.75-1.4 2.2-2.25 3.9-2.25s3.15.85 3.9 2.25",
    refresh: "M13.25 8A5.25 5.25 0 1 1 11.6 4.2M13.25 2.25v2.5h-2.5",
    terminal: "M2.25 3.25h11.5v9.5H2.25zM4.75 6.25l2 1.75-2 1.75M8.5 10h2.75",
    undo: "M5.25 2.75 2.25 5.75l3 3M2.25 5.75h7a4.5 4.5 0 0 1 0 9H6",
    close: "M4 4l8 8M12 4l-8 8",
    more: "M3.5 8h.01M8 8h.01M12.5 8h.01",
    plus: "M8 3v10M3 8h10",
    switch: "M2.75 5.25h10.5M10.25 2.25l3 3-3 3M13.25 10.75H2.75M5.75 7.75l-3 3 3 3",
    chevronRight: "M6 3.5 10.5 8 6 12.5",
    chevronUp: "M3.5 10 8 5.5 12.5 10",
    external: "M9.25 2.75h4v4M13.25 2.75 7.5 8.5M11.75 9.5v3.25c0 .55-.45 1-1 1h-7.5c-.55 0-1-.45-1-1v-7.5c0-.55.45-1 1-1H6.5",
    check: "M3.25 8.25 6.5 11.5l6.25-7",
    warning: "M8 2.25 14.25 13.25H1.75zM8 6.5v3M8 11.25h.01",
  } as const;

  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg
  class="icon"
  width={size}
  height={size}
  viewBox="0 0 16 16"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  <path d={PATHS[name]} />
</svg>

<style>
  .icon {
    flex: none;
    display: block;
  }
</style>
