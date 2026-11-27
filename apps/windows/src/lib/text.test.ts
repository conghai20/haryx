import { describe, expect, it } from "vitest";
import { hookSummary, middleTruncate, relativeTime, tildePath, wakeResultText } from "./text";

describe("text", () => {
  it("shows the home folder as ~", () => {
    expect(tildePath(String.raw`C:\Users\hai\code\haryx`)).toBe(String.raw`~\code\haryx`);
    expect(tildePath("/Users/hai/code")).toBe("~/code");
    expect(tildePath(String.raw`D:\work`)).toBe(String.raw`D:\work`);
  });

  it("cuts long names in the middle so shared prefixes stay apart", () => {
    expect(middleTruncate("Dep1pj.claude03@ml.vn-cubesystem.com", 20)).toBe("Dep1pj.cla…ystem.com");
    expect(middleTruncate("short@x.io", 20)).toBe("short@x.io");
  });

  it("says times relative to now", () => {
    const now = 1_000_000_000_000;
    expect(relativeTime(now - 2 * 3_600_000, now)).toBe("2 hr ago");
    expect(relativeTime(now + 34 * 60_000, now)).toBe("in 34 min");
    expect(relativeTime(now - 10_000, now)).toBe("just now");
    expect(relativeTime(now + 3 * 86_400_000, now)).toBe("in 3 days");
  });

  it("names wake results; disabled means the account is off", () => {
    expect(wakeResultText("skip:disabled")).toBe("Skipped: account is off");
    expect(wakeResultText("skip:weekly-cap")).toBe("Skipped: weekly cap");
    expect(wakeResultText("failed:http-429")).toBe("Failed: http 429");
    expect(wakeResultText("not-due")).toBe("Not due");
    expect(wakeResultText(null)).toBe("—");
  });

  it("shortens hook commands to the script and its arguments", () => {
    const cmd = "[ ! -f '/Users/hai/.claude/skills/impeccable/scripts/impeccable' ] || '/Users/hai/.claude/skills/impeccable/scripts/impeccable' hook";
    expect(hookSummary(cmd)).toBe("impeccable hook");
    expect(hookSummary("codegraph prompt-hook")).toBe("codegraph prompt-hook");
    expect(hookSummary(String.raw`node C:\Users\hai\x.js`)).toBe(String.raw`node ~\x.js`);
  });

  it("names the script a hook runs, not the paths given as flag values", () => {
    const cmd =
      "'/Users/hai/.caveman/bin/caveman-proxy' native-hook claude --adapter '/Users/hai/.nvm/lib/node_modules/cli/dist/native-hook-fast.js' --node '/Users/hai/.nvm/bin/node'";
    expect(hookSummary(cmd)).toBe("caveman-proxy native-hook claude");
  });
});
