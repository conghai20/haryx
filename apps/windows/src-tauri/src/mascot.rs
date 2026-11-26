//! The pixel mascots, drawn from the shared sprite data (`packages/mascots/sprites.json`, exported
//! from the macOS app). Same species choice, palette, moods and poses as `PixelArt.swift` and
//! `Mascots.swift`, rendered to RGBA for the tray. The UI draws the same sprites in
//! `src/lib/mascot.ts`; `src/lib/mascot.fixture.json` holds results both must reproduce.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::models::Account;

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

/// A per-account mascot choice (macOS `MascotStyle`), saved in the app's preferences by identity key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MascotStyle {
    /// A species id from the sprite file (`cat`, `robot`, …).
    pub species: String,
    /// The body hue, 0–1; absent keeps the species' own colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hue: Option<f64>,
}

/// A resolved style: the species as an index into `species()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub species: usize,
    pub hue: Option<f64>,
}

/// Every account's style (macOS `MascotStyles.resolve`): a saved choice wins; the rest, in
/// identity-key order, start at their hashed default and take the next species no saved or earlier
/// account has, so a handful of accounts all look different. An unknown saved species counts as unset.
pub fn resolve(
    saved: &BTreeMap<String, MascotStyle>,
    identity_keys: &[String],
) -> HashMap<String, Style> {
    let all = species();
    let count = all.len();
    let known: BTreeMap<&String, Style> = saved
        .iter()
        .filter_map(|(key, o)| {
            let species = all.iter().position(|s| s.id == o.species)?;
            Some((
                key,
                Style {
                    species,
                    hue: o.hue,
                },
            ))
        })
        .collect();
    let mut used: HashSet<usize> = known.values().map(|s| s.species).collect();
    let mut result: HashMap<String, Style> = identity_keys
        .iter()
        .filter_map(|k| known.get(k).map(|s| (k.clone(), *s)))
        .collect();
    let mut unset: Vec<&String> = identity_keys
        .iter()
        .filter(|k| !known.contains_key(k))
        .collect();
    unset.sort();
    for key in unset {
        let start = usize::try_from(stable_hash(key) % count as u64).unwrap_or(0);
        let pick = (0..count)
            .map(|i| (start + i) % count)
            .find(|i| !used.contains(i))
            .unwrap_or(start);
        used.insert(pick);
        result.insert(
            key.clone(),
            Style {
                species: pick,
                hue: None,
            },
        );
    }
    result
}

/// What an account's mascot is doing (macOS `Mood`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mood {
    #[default]
    Idle,
    Running,
    Angry,
    Tired,
    Sleeping,
    Error,
}

/// macOS `Mood.of`: held out of rotation → sleeping, signed out → error, at its limit → sleeping, a
/// Claude session on it → running, 90% or more of a quota → tired, 75% or more → angry.
pub fn mood(account: &Account, running: bool) -> Mood {
    let peak = account
        .usage
        .iter()
        .flat_map(|u| [&u.five_hour, &u.seven_day])
        .filter_map(|w| w.as_ref().and_then(|w| w.pct))
        .fold(0.0_f64, f64::max);
    if account.disabled == Some(true) {
        Mood::Sleeping
    } else if matches!(
        account.usage_status.as_str(),
        "relogin_required" | "no_credentials" | "foreign_credential"
    ) {
        Mood::Error
    } else if peak >= 100.0 {
        Mood::Sleeping
    } else if running {
        Mood::Running
    } else if peak >= TIRED_PCT {
        Mood::Tired
    } else if peak >= ANGRY_PCT {
        Mood::Angry
    } else {
        Mood::Idle
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pose {
    pub alt_legs: bool,
    pub squat: bool,
    pub blink: bool,
    pub overlay_phase: u64,
    pub mood: Mood,
}

impl Pose {
    /// Running: legs alternate and the body bobs every frame (the macOS menu bar's walk).
    pub fn running(tick: u64) -> Self {
        Pose {
            alt_legs: tick % 2 == 1,
            squat: tick.is_multiple_of(2),
            mood: Mood::Running,
            ..Pose::default()
        }
    }

    /// Standing, with the mood's mark: the macOS menu bar's mascot when it isn't walking.
    pub fn still(mood: Mood) -> Self {
        Pose {
            mood: if mood == Mood::Running {
                Mood::Idle
            } else {
                mood
            },
            ..Pose::default()
        }
    }
}

type Rgb = [f64; 3];

const SWEAT: Rgb = [0.45, 0.78, 1.0];
const ALERT: Rgb = [1.0, 0.27, 0.23];
/// The running dust: white dimmed to 60%.
const DUST: Rgb = [0.6, 0.6, 0.6];
const ANGRY_PCT: f64 = 75.0;
const TIRED_PCT: f64 = 90.0;
/// The anger mark, big then small as it throbs: (x, dy) from its top row.
const ANGER_BIG: [(usize, usize); 12] = [
    (13, 0),
    (15, 0),
    (12, 1),
    (13, 1),
    (15, 1),
    (16, 1),
    (12, 3),
    (13, 3),
    (15, 3),
    (16, 3),
    (13, 4),
    (15, 4),
];
const ANGER_SMALL: [(usize, usize); 4] = [(13, 1), (15, 1), (13, 3), (15, 3)];

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

/// The `canvas`×`canvas` grid of colours (`None` = clear) for a style in a pose.
pub fn pixels(style: Style, pose: Pose) -> Vec<Vec<Option<Rgb>>> {
    let sheet = sheet();
    let sp = &sheet.species[style.species % sheet.species.len()];
    let n = sheet.canvas;
    let [base_h, s, b] = sp.base_hsb;
    let h = style.hue.unwrap_or(base_h);
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
    overlay(&mut grid, pose, top, sheet.colors.white);
    grid
}

/// The mood's mark beside the sprite (macOS `PixelArt.overlay`).
fn overlay(grid: &mut [Vec<Option<Rgb>>], pose: Pose, top: usize, white: Rgb) {
    let n = grid.len();
    let mut put = |x: usize, y: usize, c: Rgb| {
        if x < n && y < n {
            grid[y][x] = Some(c);
        }
    };
    let phase = usize::try_from(pose.overlay_phase).unwrap_or(0);
    match pose.mood {
        // A sweat drop sliding down beside the head.
        Mood::Tired => {
            let y = top + 1 + phase % 3;
            put(1, y, SWEAT);
            put(1, y + 1, SWEAT);
            put(0, y + 1, SWEAT);
        }
        // A drifting "z".
        Mood::Sleeping => {
            let y = top.saturating_sub(4) + phase % 2;
            for dx in 0..4 {
                put(13 + dx, y, white);
                put(13 + dx, y + 3, white);
            }
            put(15, y + 1, white);
            put(14, y + 2, white);
        }
        // A throbbing anger mark.
        Mood::Angry => {
            let y = top.saturating_sub(5);
            let marks: &[(usize, usize)] = if phase.is_multiple_of(2) {
                &ANGER_BIG
            } else {
                &ANGER_SMALL
            };
            for &(x, dy) in marks {
                put(x, y + dy, ALERT);
            }
        }
        // A blinking "!".
        Mood::Error => {
            if phase.is_multiple_of(2) {
                let y = top.saturating_sub(4);
                put(16, y, ALERT);
                put(16, y + 1, ALERT);
                put(16, y + 3, ALERT);
            }
        }
        // Speed dust behind the feet.
        Mood::Running => {
            if pose.alt_legs {
                put(0, n - 1, DUST);
            } else {
                put(1, n - 2, DUST);
            }
        }
        Mood::Idle => {}
    }
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

    fn style(species: usize) -> Style {
        Style { species, hue: None }
    }

    fn keys(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("u{i}@x.com|")).collect()
    }

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
        let keys = keys(8);
        let resolved = resolve(&BTreeMap::new(), &keys);
        let mut picked: Vec<usize> = resolved.values().map(|s| s.species).collect();
        picked.sort_unstable();
        picked.dedup();
        assert_eq!(picked.len(), 8);
        assert_eq!(
            resolve(&BTreeMap::new(), &keys),
            resolved,
            "stable across calls"
        );
        let mut reversed = keys.clone();
        reversed.reverse();
        assert_eq!(
            resolve(&BTreeMap::new(), &reversed),
            resolved,
            "independent of list order"
        );
    }

    #[test]
    fn a_saved_choice_wins_and_others_avoid_it() {
        let keys = keys(2);
        let free = resolve(&BTreeMap::new(), &keys);
        let taken = &species()[free[&keys[1]].species].id;
        let saved = BTreeMap::from([(
            keys[0].clone(),
            MascotStyle {
                species: taken.clone(),
                hue: Some(0.3),
            },
        )]);
        let resolved = resolve(&saved, &keys);
        assert_eq!(&species()[resolved[&keys[0]].species].id, taken);
        assert_eq!(resolved[&keys[0]].hue, Some(0.3));
        assert_ne!(resolved[&keys[1]].species, resolved[&keys[0]].species);
        let unknown = BTreeMap::from([(
            keys[0].clone(),
            MascotStyle {
                species: "dragon".into(),
                hue: None,
            },
        )]);
        assert_eq!(resolve(&unknown, &keys), free, "unknown species = unset");
    }

    fn account(json: &str) -> Account {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn mood_follows_the_macos_rules_in_order() {
        let a = |extra: &str| account(&format!(r#"{{"number":1,"email":"a@x.com"{extra}}}"#));
        let tired = r#","usage":{"fiveHour":{"pct":10},"sevenDay":{"pct":90}}"#;
        assert_eq!(mood(&a(""), false), Mood::Idle);
        assert_eq!(mood(&a(tired), false), Mood::Tired);
        assert_eq!(mood(&a(tired), true), Mood::Running);
        assert_eq!(
            mood(&a(r#","usageStatus":"no_credentials""#), true),
            Mood::Error
        );
        assert_eq!(
            mood(
                &a(r#","usageStatus":"relogin_required","disabled":true"#),
                true
            ),
            Mood::Sleeping
        );
        let angry = r#","usage":{"fiveHour":{"pct":10},"sevenDay":{"pct":75}}"#;
        let calm = r#","usage":{"fiveHour":{"pct":74}}"#;
        assert_eq!(mood(&a(angry), false), Mood::Angry);
        assert_eq!(mood(&a(angry), true), Mood::Running);
        assert_eq!(mood(&a(calm), false), Mood::Idle);
        let full = r#","usage":{"fiveHour":{"pct":100},"sevenDay":{"pct":40}}"#;
        assert_eq!(mood(&a(full), true), Mood::Sleeping);
        assert_eq!(
            mood(
                &a(&format!(r#"{full},"usageStatus":"relogin_required""#)),
                false
            ),
            Mood::Error
        );
    }

    #[test]
    fn renders_a_filled_sprite_and_running_moves_it() {
        let still = pixels(style(0), Pose::default());
        assert_eq!(still.len(), 18);
        assert!(still.iter().flatten().filter(|c| c.is_some()).count() > 60);
        assert_ne!(
            still,
            pixels(style(0), Pose::running(0)),
            "the squat moves the body"
        );
        assert_ne!(
            still,
            pixels(style(0), Pose::still(Mood::Error)),
            "the mood adds its mark"
        );
        assert_eq!(Pose::still(Mood::Running), Pose::default());
        let (bytes, side) = rgba(&still, 2);
        assert_eq!(side, 36);
        assert_eq!(bytes.len(), 36 * 36 * 4);
    }

    /// The fixture the UI's `mascot.test.ts` checks too, so tray and cards agree.
    #[derive(Deserialize)]
    struct Fixture {
        resolve: Vec<ResolveCase>,
        pixels: Vec<PixelCase>,
    }

    #[derive(Deserialize)]
    struct ResolveCase {
        saved: BTreeMap<String, MascotStyle>,
        keys: Vec<String>,
        expect: BTreeMap<String, String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PixelCase {
        species: String,
        hue: Option<f64>,
        alt_legs: bool,
        squat: bool,
        blink: bool,
        overlay_phase: u64,
        mood: String,
        rows: Vec<String>,
    }

    fn hex(grid: &[Vec<Option<Rgb>>]) -> Vec<String> {
        let byte = |v: f64| (v * 255.0).round() as u8;
        grid.iter()
            .map(|row| {
                row.iter()
                    .map(|c| match c {
                        Some([r, g, b]) => {
                            format!("{:02x}{:02x}{:02x}", byte(*r), byte(*g), byte(*b))
                        }
                        None => "------".into(),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn matches_the_shared_fixture() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../src/lib/mascot.fixture.json")).unwrap();
        for case in &fixture.resolve {
            let got: BTreeMap<String, String> = resolve(&case.saved, &case.keys)
                .into_iter()
                .map(|(k, s)| (k, species()[s.species].id.clone()))
                .collect();
            assert_eq!(got, case.expect);
        }
        for case in &fixture.pixels {
            let species = species().iter().position(|s| s.id == case.species).unwrap();
            let mood = match case.mood.as_str() {
                "running" => Mood::Running,
                "angry" => Mood::Angry,
                "tired" => Mood::Tired,
                "sleeping" => Mood::Sleeping,
                "error" => Mood::Error,
                _ => Mood::Idle,
            };
            let pose = Pose {
                alt_legs: case.alt_legs,
                squat: case.squat,
                blink: case.blink,
                overlay_phase: case.overlay_phase,
                mood,
            };
            let grid = pixels(
                Style {
                    species,
                    hue: case.hue,
                },
                pose,
            );
            assert_eq!(hex(&grid), case.rows, "{} {}", case.species, case.mood);
        }
    }
}
