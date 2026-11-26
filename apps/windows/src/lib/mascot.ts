// The pixel mascots, from the shared sprite data (packages/mascots/sprites.json). A port of the macOS
// app's PixelArt.swift and MascotStyles, and of src-tauri/src/mascot.rs, which draws the tray icon:
// mascot.fixture.json holds results both this file and the Rust one are tested against, so the tray
// and the cards always show the same mascot.

import sheet from "../../../../packages/mascots/sprites.json";
import { exhausted, peakPct } from "./handoff";
import { needsRelogin } from "./tide";
import type { Account, MascotStyle } from "./types";

type Rgb = readonly [number, number, number];

export type Mood = "idle" | "running" | "angry" | "tired" | "sleeping" | "error";

export interface Pose {
  altLegs: boolean;
  squat: boolean;
  blink: boolean;
  overlayPhase: number;
  mood: Mood;
}

/** A resolved style: a species id from the sprite file, and a hue override (0–1). */
export interface Style {
  species: string;
  hue: number | null;
}

export const CANVAS = sheet.canvas;
export const SPECIES = sheet.species.map((s) => s.id);
const IDLE_FPS = 4;
const RUNNING_FPS = 8;
const BLINK_EVERY = 14;
/** Phases spread over this many ticks so mascots don't move in lockstep. */
const PHASES = 13n;
const SWEAT: Rgb = [0.45, 0.78, 1.0];
const ALERT: Rgb = [1.0, 0.27, 0.23];
const DUST: Rgb = [0.6, 0.6, 0.6];
const ANGRY_PCT = 75;
const TIRED_PCT = 90;
/** The anger mark, big then small as it throbs: [x, dy] from its top row. */
const ANGER_BIG: [number, number][] = [
  [13, 0], [15, 0], [12, 1], [13, 1], [15, 1], [16, 1], [12, 3], [13, 3], [15, 3], [16, 3], [13, 4], [15, 4],
];
const ANGER_SMALL: [number, number][] = [[13, 1], [15, 1], [13, 3], [15, 3]];

export const speciesTitle = (id: string) => id.charAt(0).toUpperCase() + id.slice(1);

/** The species' own hue, 0–1: where the colour slider sits before a colour is chosen. */
export const baseHue = (id: string) => sheet.species.find((s) => s.id === id)?.baseHSB[0] ?? 0;

/** Accounts are identified by (email, org), stable across slot moves. */
export const identityKey = (a: Account) => `${a.email}|${a.organizationUuid ?? ""}`;

/** FNV-1a 64 over the UTF-8 bytes (macOS `stableHash`). */
export function stableHash(text: string): bigint {
  let h = 14_695_981_039_346_656_037n;
  for (const b of new TextEncoder().encode(text)) h = ((h ^ BigInt(b)) * 1_099_511_628_211n) & 0xffff_ffff_ffff_ffffn;
  return h;
}

export const phase = (key: string) => Number(stableHash(key) % PHASES);

/**
 * Every account's style (macOS `MascotStyles.resolve`): a saved choice wins; the rest, in identity-key
 * order, start at their hashed default and take the next species no saved or earlier account has.
 * An unknown saved species counts as unset.
 */
export function resolve(saved: Record<string, MascotStyle>, keys: string[]): Record<string, Style> {
  const count = SPECIES.length;
  const known = Object.entries(saved).filter(([, s]) => SPECIES.includes(s.species));
  const used = new Set(known.map(([, s]) => s.species));
  const result: Record<string, Style> = {};
  for (const [key, s] of known) if (keys.includes(key)) result[key] = { species: s.species, hue: s.hue ?? null };
  const unset = keys.filter((k) => !(k in result)).sort();
  for (const key of unset) {
    const start = Number(stableHash(key) % BigInt(count));
    const offsets = Array.from({ length: count }, (_, i) => SPECIES[(start + i) % count]!);
    const species = offsets.find((s) => !used.has(s)) ?? SPECIES[start]!;
    used.add(species);
    result[key] = { species, hue: null };
  }
  return result;
}

/**
 * macOS `Mood.of`: held out → sleeping, signed out → error, at its limit → sleeping, running a
 * session → running, ≥ 90% → tired, ≥ 75% → angry.
 */
export function mood(a: Account, running: boolean): Mood {
  if (a.disabled) return "sleeping";
  if (needsRelogin(a)) return "error";
  if (exhausted(a)) return "sleeping";
  if (running) return "running";
  const peak = peakPct(a) ?? 0;
  if (peak >= TIRED_PCT) return "tired";
  if (peak >= ANGRY_PCT) return "angry";
  return "idle";
}

export const fps = (m: Mood) => (m === "running" ? RUNNING_FPS : IDLE_FPS);

/** The frame at `seconds` (macOS `Pose.at`); `phase` de-synchronises mascots. */
export function poseAt(seconds: number, m: Mood, phase: number): Pose {
  const tick = Math.floor(seconds * fps(m)) + phase;
  const base = { altLegs: false, squat: false, blink: false, mood: m };
  if (m === "running") return { ...base, altLegs: tick % 2 === 1, squat: tick % 2 === 0, overlayPhase: tick };
  // Stomping, the mark throbbing in time.
  if (m === "angry") return { ...base, squat: tick % 2 === 0, overlayPhase: tick };
  if (m === "sleeping")
    return { ...base, squat: Math.floor(tick / 4) % 2 === 1, blink: true, overlayPhase: Math.floor(tick / 3) };
  const half = Math.floor(tick / 2);
  return { ...base, squat: half % 2 === 1, blink: tick % BLINK_EVERY === 0, overlayPhase: half };
}

/** Standing still with the mood's mark: what reduced motion shows (macOS `Pose(mood:)`). */
export const stillPose = (m: Mood): Pose => ({ altLegs: false, squat: false, blink: false, overlayPhase: 0, mood: m });

function hsb(h: number, s: number, v: number): Rgb {
  const h6 = (h - Math.floor(h)) * 6;
  const c = v * s;
  const x = c * (1 - Math.abs((h6 % 2) - 1));
  const m = v - c;
  const [r, g, b] = [[c, x, 0], [x, c, 0], [0, c, x], [0, x, c], [x, 0, c]][Math.trunc(h6)] ?? [c, 0, x];
  return [r! + m, g! + m, b! + m];
}

const hex = ([r, g, b]: Rgb) =>
  "#" + [r, g, b].map((v) => Math.round(v * 255).toString(16).padStart(2, "0")).join("");

/** The CANVAS×CANVAS grid of "#rrggbb" colours (null = clear) for a style in a pose. */
export function pixels(style: Style, pose: Pose): (string | null)[][] {
  const sp = sheet.species.find((s) => s.id === style.species) ?? sheet.species[0]!;
  const n = CANVAS;
  const [baseH, s, b] = sp.baseHSB as unknown as Rgb;
  const h = style.hue ?? baseH;
  const body = hsb(h, s, b);
  const shade = hsb(h, Math.min(1, s + 0.15), b * 0.68);
  const light = hsb(h, s * 0.35, Math.min(1, b + 0.12));
  const c = sheet.colors as unknown as Record<"white" | "blush" | "dark" | "eye", Rgb>;
  const accent = sp.accent as unknown as Rgb;
  const eye = sp.eyeIsAccent ? accent : c.eye;
  const legs = sp.legs[pose.altLegs ? 1 : 0]!;
  const top = Math.max(0, n - sp.body.length - legs.length);
  const grid: (Rgb | null)[][] = Array.from({ length: n }, () => Array<Rgb | null>(n).fill(null));
  const colors: Record<string, Rgb> = { B: body, D: shade, L: light, W: c.white, P: c.blush, K: c.dark, A: accent };

  const stamp = (rows: string[], y0: number) =>
    rows.forEach((row, r) => {
      const y = y0 + r;
      if (y >= n) return;
      [...row].forEach((ch, col) => {
        // A closed eye is body colour, with a lash line on its lowest row.
        const color =
          ch === "E" ? (pose.blink ? ([...(rows[r + 1] ?? "")][col] === "E" ? body : shade) : eye) : colors[ch];
        if (color && col + 1 < n) grid[y]![col + 1] = color;
      });
    });
  stamp(sp.body, top + (pose.squat ? 1 : 0));
  stamp(legs, n - legs.length);
  overlay(grid, pose, top, c.white);
  return grid.map((row) => row.map((px) => (px ? hex(px) : null)));
}

/** The mood's mark beside the sprite (macOS `PixelArt.overlay`). */
function overlay(grid: (Rgb | null)[][], pose: Pose, top: number, white: Rgb) {
  const n = grid.length;
  const put = (c: Rgb, ...points: [number, number][]) => {
    for (const [x, y] of points) if (x >= 0 && y >= 0 && x < n && y < n) grid[y]![x] = c;
  };
  const p = pose.overlayPhase;
  switch (pose.mood) {
    case "tired": {
      // A sweat drop sliding down beside the head.
      const y = top + 1 + (p % 3);
      put(SWEAT, [1, y], [1, y + 1], [0, y + 1]);
      break;
    }
    case "sleeping": {
      // A drifting "z".
      const y = Math.max(0, top - 4) + (p % 2);
      put(white, ...[0, 1, 2, 3].flatMap((dx): [number, number][] => [[13 + dx, y], [13 + dx, y + 3]]));
      put(white, [15, y + 1], [14, y + 2]);
      break;
    }
    case "angry": {
      // A throbbing anger mark.
      const y = Math.max(0, top - 5);
      put(ALERT, ...(p % 2 === 0 ? ANGER_BIG : ANGER_SMALL).map(([x, dy]): [number, number] => [x, y + dy]));
      break;
    }
    case "error":
      // A blinking "!".
      if (p % 2 === 0) {
        const y = Math.max(0, top - 4);
        put(ALERT, [16, y], [16, y + 1], [16, y + 3]);
      }
      break;
    case "running":
      // Speed dust behind the feet.
      put(DUST, pose.altLegs ? [0, n - 1] : [1, n - 2]);
      break;
    case "idle":
      break;
  }
}
