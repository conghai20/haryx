// Small text helpers shared by the panes: home folders as ~, long names cut in the middle, relative
// times, wake results in words and hook commands shortened. The macOS app has the same rules
// (HaryXCore `WakeStatus.resultText`, `HookSummary`).

/** `C:\Users\name\…` (or `/Users/name/…`, `/home/name/…`) with the home folder shown as ~. */
export function tildePath(path: string): string {
  return path.replace(/^([A-Za-z]:[\\/]Users[\\/][^\\/]+|\/Users\/[^/]+|\/home\/[^/]+)(?=[\\/]|$)/, "~");
}

/** Cut `text` to `max` characters in the middle, so emails that share a start still differ: "Dep1pj.c…@ml.vn". */
export function middleTruncate(text: string, max: number): string {
  if (text.length <= max || max < 5) return text;
  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  return `${text.slice(0, head)}…${text.slice(text.length - (keep - head))}`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "just now", "2 hr ago", "in 34 min", "in 3 days" (macOS's abbreviated relative style). */
export function relativeTime(at: number, now: number): string {
  const diff = at - now;
  const abs = Math.abs(diff);
  if (abs < MINUTE) return diff >= 0 ? "in under a minute" : "just now";
  const [n, unit] =
    abs < HOUR ? [Math.round(abs / MINUTE), "min"] : abs < DAY ? [Math.round(abs / HOUR), "hr"] : [Math.round(abs / DAY), "day"];
  const label = unit === "day" && n !== 1 ? "days" : unit;
  return diff >= 0 ? `in ${n} ${label}` : `${n} ${label} ago`;
}

/**
 * A wake result code in words: "Woke", "Not due", "Skipped: weekly cap". The CLI's `skip:disabled`
 * means the account is held out of rotation (Off), not that wake is off for it.
 */
export function wakeResultText(code: string | null | undefined): string {
  if (!code) return "—";
  if (code === "woke") return "Woke";
  if (code === "not-due") return "Not due";
  const i = code.indexOf(":");
  const kind = i === -1 ? code : code.slice(0, i);
  const why = i === -1 ? "" : code.slice(i + 1);
  const reason = why === "disabled" ? "account is off" : why.replaceAll("-", " ");
  if (kind === "skip") return `Skipped: ${reason}`;
  if (kind === "failed") return `Failed: ${reason}`;
  return code;
}

/**
 * A hook command for display: the home folder as ~, and a long command reduced to the name of the
 * script it runs plus its arguments up to the first flag. The full command belongs in the tooltip.
 */
export function hookSummary(command: string, limit = 60): string {
  const text = command.replace(/[A-Za-z]:[\\/]Users[\\/][^\\/\s'"]+|\/Users\/[^/\s'"]+|\/home\/[^/\s'"]+/g, "~");
  if (text.length <= limit) return text;
  const tokens = text.split(" ").map((t) => t.replace(/^['"]+|['"]+$/g, ""));
  const isPath = (t: string) => t.includes("/") || t.includes("\\");
  const lastName = (t: string) => t.split(/[\\/]/).pop() ?? "";
  // The script the command runs: the last path that is not a flag's value (`--node /x/node`).
  let pathIndex = -1;
  let anyPath = -1;
  tokens.forEach((t, i) => {
    if (!isPath(t)) return;
    anyPath = i;
    if (!(tokens[i - 1] ?? "").startsWith("-")) pathIndex = i;
  });
  if (pathIndex === -1) pathIndex = anyPath;
  if (pathIndex === -1) return `${text.slice(0, limit)}…`;
  // Its arguments up to the first flag, paths by their last part.
  const rest = tokens.slice(pathIndex + 1);
  const flag = rest.findIndex((t) => t.startsWith("-"));
  const args = (flag === -1 ? rest : rest.slice(0, flag)).map((t) => (isPath(t) ? lastName(t) : t)).join(" ");
  const name = lastName(tokens[pathIndex]!);
  return args ? `${name} ${args}` : name;
}
