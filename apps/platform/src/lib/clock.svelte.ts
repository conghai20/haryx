// Unix seconds, refreshed every 30 s, so "2 min ago" stays true while the tab is open.
class Clock {
  now = $state(Math.floor(Date.now() / 1000));
  constructor() {
    if (typeof window !== "undefined") setInterval(() => (this.now = Math.floor(Date.now() / 1000)), 30_000);
  }
}
export const clock = new Clock();
