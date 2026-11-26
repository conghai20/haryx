import { describe, expect, it } from "vitest";
import fixture from "./mascot.fixture.json";
import { CANVAS, SPECIES, identityKey, mood, phase, pixels, poseAt, resolve, stableHash, stillPose, type Mood } from "./mascot";
import { resting } from "./handoff";
import type { Account, MascotStyle } from "./types";

const account = (extra: Partial<Account> = {}): Account => ({
  number: 1,
  email: "a@x.com",
  active: false,
  usageStatus: "ok",
  ...extra,
});

describe("agreement with the tray (src-tauri/src/mascot.rs, same fixture)", () => {
  it("resolves the same species for every account", () => {
    for (const c of fixture.resolve) {
      const got = resolve(c.saved as Record<string, MascotStyle>, c.keys);
      expect(Object.fromEntries(Object.entries(got).map(([k, s]) => [k, s.species]))).toEqual(c.expect);
    }
  });

  it("draws the same pixels", () => {
    for (const c of fixture.pixels) {
      const grid = pixels(
        { species: c.species, hue: c.hue },
        { altLegs: c.altLegs, squat: c.squat, blink: c.blink, overlayPhase: c.overlayPhase, mood: c.mood as Mood },
      );
      const rows = grid.map((row) => row.map((px) => px?.slice(1) ?? "------").join(" "));
      expect(rows, `${c.species} ${c.mood}`).toEqual(c.rows);
    }
  });
});

describe("species", () => {
  it("hashes with FNV-1a 64", () => {
    expect(stableHash("a")).toBe(0xaf63dc4c8601ec8cn);
  });

  it("gives up to eight accounts different species, whatever the order", () => {
    const keys = Array.from({ length: 8 }, (_, i) => `u${i}@x.com|`);
    const styles = resolve({}, keys);
    expect(new Set(Object.values(styles).map((s) => s.species)).size).toBe(SPECIES.length);
    expect(resolve({}, [...keys].reverse())).toEqual(styles);
  });

  it("keeps a saved choice and its colour, and treats an unknown species as unset", () => {
    expect(resolve({ "a@x.com|": { species: "frog", hue: 0.2 } }, ["a@x.com|"])).toEqual({
      "a@x.com|": { species: "frog", hue: 0.2 },
    });
    expect(resolve({ "a@x.com|": { species: "dragon" } }, ["a@x.com|"])).toEqual(resolve({}, ["a@x.com|"]));
  });

  it("keys accounts by email and organization", () => {
    expect(identityKey(account({ organizationUuid: "org" }))).toBe("a@x.com|org");
    expect(identityKey(account())).toBe("a@x.com|");
  });
});

describe("mood", () => {
  const tired = { usage: { fiveHour: { pct: 10 }, sevenDay: { pct: 90 } } };

  it("follows the macOS rules in order", () => {
    expect(mood(account(), false)).toBe("idle");
    expect(mood(account(tired), false)).toBe("tired");
    expect(mood(account(tired), true)).toBe("running");
    expect(mood(account({ usageStatus: "foreign_credential" }), true)).toBe("error");
    expect(mood(account({ usageStatus: "relogin_required", disabled: true }), true)).toBe("sleeping");
  });

  it("gets angry from 75% until it tires at 90%", () => {
    const at = (five: number, seven = 0) => account({ usage: { fiveHour: { pct: five }, sevenDay: { pct: seven } } });
    expect(mood(at(74), false)).toBe("idle");
    expect(mood(at(10, 75), false)).toBe("angry");
    expect(mood(at(89), false)).toBe("angry");
    expect(mood(at(89), true)).toBe("running");
    expect(mood(at(90), false)).toBe("tired");
  });

  it("stomps while angry", () => {
    expect([0, 0.25, 0.5].map((s) => poseAt(s, "angry", 0).squat)).toEqual([true, false, true]);
  });

  it("sleeps at the limit, unless a sign-in problem needs attention", () => {
    const full = { usage: { fiveHour: { pct: 40 }, sevenDay: { pct: 100 } } };
    expect(mood(account(full), true)).toBe("sleeping");
    expect(mood(account({ ...full, usageStatus: "relogin_required" }), false)).toBe("error");
    expect(resting(account(full))).toBe(true);
    expect(resting(account({ disabled: true }))).toBe(true);
    expect(resting(account(tired))).toBe(false);
  });
});

describe("poses", () => {
  it("walks at 8 fps, alternating legs and bobbing", () => {
    const a = poseAt(0, "running", 0);
    const b = poseAt(1 / 8, "running", 0);
    expect([a.altLegs, a.squat]).toEqual([false, true]);
    expect([b.altLegs, b.squat]).toEqual([true, false]);
  });

  it("sleeps with eyes shut, and blinks now and then otherwise", () => {
    expect(poseAt(5, "sleeping", 3).blink).toBe(true);
    const blinks = Array.from({ length: 28 }, (_, t) => poseAt(t / 4, "idle", 0).blink).filter(Boolean);
    expect(blinks).toHaveLength(2);
  });

  it("offsets each account's phase so mascots don't move in lockstep", () => {
    const phases = new Set(["a@x.com|", "b@x.com|", "c@x.com|", "d@x.com|"].map(phase));
    expect(phases.size).toBeGreaterThan(1);
    expect(Math.max(...phases)).toBeLessThan(13);
  });

  it("stands still with the mood's mark under reduced motion", () => {
    const style = { species: "critter", hue: null };
    expect(pixels(style, stillPose("error"))).not.toEqual(pixels(style, stillPose("idle")));
    expect(pixels(style, stillPose("idle"))).toHaveLength(CANVAS);
  });
});
