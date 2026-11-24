//! The pixel mascots, drawn from the shared sprite data (`packages/mascots/sprites.json`, exported
//! from the macOS app). Same species choice, palette and poses as `PixelArt.swift`, rendered to RGBA
//! for the tray.

use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sheet {
    canvas: usize,
    colors: Colors,
    species: Vec<Species>,
}

#[derive(Debug, Deserialize)]
struct Colors {
    white: [f64; 3],
    blush: [f64; 3],
    dark: [f64; 3],
    eye: [f64; 3],
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Species {
    /// Read by tests and kept for the sprite file's readers.
    #[allow(dead_code)]
    pub id: String,
    #[serde(rename = "baseHSB")]
    base_hsb: [f64; 3],
    accent: [f64; 3],
    eye_is_accent: bool,
    body: Vec<String>,
    legs: [Vec<String>; 2],
}

fn sheet() -> &'static Sheet {
    static SHEET: OnceLock<Sheet> = OnceLock::new();
    SHEET.get_or_init(|| {
        serde_json::from_str(include_str!("../../../../packages/mascots/sprites.json"))
            .expect("packages/mascots/sprites.json is valid")
    })
}

pub fn species() -> &'static [Species] {
    &sheet().species
}

/// FNV-1a over the bytes, as the macOS app's `stableHash` (a per-launch random hash would reshuffle).
pub fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(14_695_981_039_346_656_037, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(1_099_511_628_211)
    })
}

/// Species index per account (in switch order): each starts at its hashed default and takes the next
/// one no earlier account has, so a handful of accounts get different mascots.
pub fn assign(identity_keys: &[String]) -> Vec<usize> {
    let count = species().len();
    let mut used = vec![false; count];
    identity_keys
        .iter()
        .map(|key| {
            let start = usize::try_from(stable_hash(key) % count as u64).unwrap_or(0);
            let pick = (0..count)
                .map(|i| (start + i) % count)
                .find(|i| !used[*i])
                .unwrap_or(start);
            used[pick] = true;
            pick
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pose {
    pub alt_legs: bool,
    pub squat: bool,
    pub blink: bool,
}

impl Pose {
    /// Running: legs alternate and the body bobs every frame.
    pub fn running(tick: u64) -> Self {
        Pose {
            alt_legs: tick % 2 == 1,
            squat: tick.is_multiple_of(2),
            blink: false,
        }
    }
}

type Rgb = [f64; 3];

fn hsb(h: f64, s: f64, v: f64) -> Rgb {
    let h6 = (h - h.floor()) * 6.0;
    let c = v * s;
    let x = c * (1.0 - ((h6 % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

/// The `canvas`×`canvas` grid of colours (`None` = clear) for a species in a pose.
pub fn pixels(index: usize, pose: Pose) -> Vec<Vec<Option<Rgb>>> {
    let sheet = sheet();
    let sp = &sheet.species[index % sheet.species.len()];
    let n = sheet.canvas;
    let [h, s, b] = sp.base_hsb;
    let body = hsb(h, s, b);
    let shade = hsb(h, (s + 0.15).min(1.0), b * 0.68);
    let light = hsb(h, s * 0.35, (b + 0.12).min(1.0));
    let eye = if sp.eye_is_accent {
        sp.accent
    } else {
        sheet.colors.eye
    };
    let legs = &sp.legs[usize::from(pose.alt_legs)];
    let top = n.saturating_sub(sp.body.len() + legs.len());
    let body_top = top + usize::from(pose.squat);
    let mut grid = vec![vec![None; n]; n];
    let mut stamp = |rows: &[String], y0: usize| {
        let chars: Vec<Vec<char>> = rows.iter().map(|r| r.chars().collect()).collect();
        for (r, row) in chars.iter().enumerate() {
            let y = y0 + r;
            if y >= n {
                continue;
            }
            for (c, ch) in row.iter().enumerate() {
                let below_is_eye = chars.get(r + 1).and_then(|b| b.get(c)) == Some(&'E');
                let color = match ch {
                    'B' => Some(body),
                    'D' => Some(shade),
                    'L' => Some(light),
                    'W' => Some(sheet.colors.white),
                    'P' => Some(sheet.colors.blush),
                    'K' => Some(sheet.colors.dark),
                    'A' => Some(sp.accent),
                    // A closed eye is body colour, with a lash line on its lowest row.
                    'E' if pose.blink => Some(if below_is_eye { body } else { shade }),
                    'E' => Some(eye),
                    _ => None,
                };
                if let (Some(color), Some(cell)) = (color, grid[y].get_mut(c + 1)) {
                    *cell = Some(color);
                }
            }
        }
    };
    stamp(&sp.body, body_top);
    stamp(legs, n - legs.len());
    grid
}

/// The grid as RGBA, each pixel `scale`×`scale`.
pub fn rgba(grid: &[Vec<Option<Rgb>>], scale: usize) -> (Vec<u8>, u32) {
    let n = grid.len();
    let side = n * scale;
    let mut out = vec![0u8; side * side * 4];
    for (y, row) in grid.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            let Some([r, g, b]) = cell else { continue };
            let px = [
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8,
                255,
            ];
            for dy in 0..scale {
                for dx in 0..scale {
                    let i = ((y * scale + dy) * side + x * scale + dx) * 4;
                    out[i..i + 4].copy_from_slice(&px);
                }
            }
        }
    }
    (out, u32::try_from(side).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_species_load_with_both_leg_frames() {
        let all = species();
        assert_eq!(all.len(), 8);
        assert!(
            all.iter()
                .all(|s| !s.body.is_empty() && !s.legs[0].is_empty() && !s.legs[1].is_empty())
        );
        assert!(
            all.iter()
                .all(|s| s.body.iter().all(|r| r.chars().count() == 16))
        );
    }

    #[test]
    fn hash_matches_fnv1a() {
        // FNV-1a 64 of "a" is the published test vector.
        assert_eq!(stable_hash("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn accounts_get_different_mascots_until_they_run_out() {
        let keys: Vec<String> = (0..8).map(|i| format!("u{i}@x.com|")).collect();
        let mut picked = assign(&keys);
        picked.sort_unstable();
        picked.dedup();
        assert_eq!(picked.len(), 8);
        assert_eq!(
            assign(&keys[..1]),
            assign(&keys[..1]),
            "stable across calls"
        );
    }

    #[test]
    fn renders_a_filled_sprite_and_running_moves_it() {
        let still = pixels(0, Pose::default());
        assert_eq!(still.len(), 18);
        assert!(still.iter().flatten().filter(|c| c.is_some()).count() > 60);
        assert_ne!(
            still,
            pixels(0, Pose::running(0)),
            "the squat moves the body"
        );
        let (bytes, side) = rgba(&still, 2);
        assert_eq!(side, 36);
        assert_eq!(bytes.len(), 36 * 36 * 4);
    }
}
