//! Keys in settings.json that nothing reads, and the valid name each one most resembles.

use std::collections::HashMap;

use serde_json::{Map, Value};

use super::{SECTIONS, SETTING_SPECS, section_keys};
use crate::models::pyfmt::repr_str;

/// How alike two names must be (difflib ratio, compared without case) before one is offered as the
/// spelling the other meant. 0.9 admits one wrong, missing or extra character in all but the shortest
/// names, and another word separator (`weekly_threshold`). A name that only shares a word with a real
/// key (`maxThreshold`, `holdSeconds`) scores below it.
const TYPO_CUTOFF: f64 = 0.9;

/// A shortened name scores too low for that and is matched as a prefix: `interval` and `cooldown`, the
/// `haryx auto` flag names, for intervalSeconds and cooldownSeconds. Shorter than this is not a name.
const PREFIX_MIN_LENGTH: usize = 3;

/// A key in settings.json that nothing reads, so it changes nothing.
///
/// `dotted` is `section.key` for a key inside a known section and the bare name for a top-level key that
/// is not a section. `suggestion` is the valid name it most resembles, when one is close enough to be a
/// typo. The two are equal for a setting written flat at the top level (`"autoswitch.threshold": 80`):
/// the right name in the wrong place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnrecognizedKey {
    pub dotted: String,
    pub suggestion: Option<String>,
}

impl UnrecognizedKey {
    pub fn new(dotted: impl Into<String>, suggestion: Option<&str>) -> Self {
        UnrecognizedKey {
            dotted: dotted.into(),
            suggestion: suggestion.map(str::to_owned),
        }
    }

    /// One line for the user. The key is the file's own text, so it is quoted: a stray space or a
    /// newline in it stays visible and on one line.
    pub fn describe(&self) -> String {
        let hint = match &self.suggestion {
            None => String::new(),
            Some(suggestion) if *suggestion == self.dotted => {
                let (section, key) = self.dotted.split_once('.').unwrap_or((&self.dotted, ""));
                format!(
                    "; it belongs inside {} as {}",
                    repr_str(section),
                    repr_str(key)
                )
            }
            Some(suggestion) => format!("; did you mean {suggestion}?"),
        };
        format!(
            "{} is not a key this version reads, so it has no effect{hint}",
            repr_str(&self.dotted)
        )
    }
}

/// Keys in a parsed settings file that no loader reads, in file order.
pub(super) fn unrecognized(raw: &Map<String, Value>) -> Vec<UnrecognizedKey> {
    let mut found = Vec::new();
    for (name, value) in raw {
        if name == "schemaVersion" {
            continue;
        }
        if !SECTIONS.contains(&name.as_str()) {
            // A dotted setting name here is the spelling `haryx config` prints, written as one flat key.
            let flat = SETTING_SPECS.iter().any(|spec| spec.dotted() == *name);
            let suggestion = if flat {
                Some(name.clone())
            } else {
                nearest(name, &SECTIONS)
            };
            found.push(UnrecognizedKey {
                dotted: name.clone(),
                suggestion,
            });
            continue;
        }
        let Value::Object(section) = value else {
            continue;
        };
        let known = section_keys(name);
        for key in section.keys() {
            if !known.contains(&key.as_str()) {
                found.push(UnrecognizedKey {
                    dotted: format!("{name}.{key}"),
                    suggestion: nearest(key, &known).map(|near| format!("{name}.{near}")),
                });
            }
        }
    }
    found
}

/// The valid name `name` most resembles, or `None` when none is close: the only one it is a prefix of,
/// else the nearest spelling.
fn nearest(name: &str, valid: &[&str]) -> Option<String> {
    let by_lower: Vec<(String, &str)> = valid.iter().map(|v| (v.to_lowercase(), *v)).collect();
    let lowered = name.to_lowercase();
    if lowered.chars().count() >= PREFIX_MIN_LENGTH {
        let longer: Vec<&str> = by_lower
            .iter()
            .filter(|(low, _)| low.starts_with(&lowered))
            .map(|(_, v)| *v)
            .collect();
        if let [only] = longer[..] {
            return Some(only.to_owned());
        }
    }
    let lowers: Vec<&str> = by_lower.iter().map(|(low, _)| low.as_str()).collect();
    let best = closest_match(&lowered, &lowers, TYPO_CUTOFF)?;
    by_lower
        .iter()
        .find(|(low, _)| low == best)
        .map(|(_, v)| (*v).to_owned())
}

/// `difflib.get_close_matches(word, possibilities, n=1, cutoff)`: the possibility with the highest ratio
/// at or above `cutoff`, ties going to the greater string.
fn closest_match<'a>(word: &str, possibilities: &[&'a str], cutoff: f64) -> Option<&'a str> {
    let b: Vec<char> = word.chars().collect();
    possibilities
        .iter()
        .map(|x| (ratio(&x.chars().collect::<Vec<_>>(), &b), *x))
        .filter(|(score, _)| *score >= cutoff)
        .max_by(|(sa, xa), (sb, xb)| sa.total_cmp(sb).then_with(|| xa.cmp(xb)))
        .map(|(_, x)| x)
}

/// `difflib.SequenceMatcher(None, a, b).ratio()`: `2 * matches / (len(a) + len(b))`.
fn ratio(a: &[char], b: &[char]) -> f64 {
    let total = a.len() + b.len();
    if total == 0 {
        return 1.0;
    }
    2.0 * matching_chars(a, b) as f64 / total as f64
}

/// Sum of `SequenceMatcher.get_matching_blocks()` sizes, with difflib's autojunk heuristic (characters
/// of `b` more frequent than 1% + 1 are not indexed once `b` has 200 or more).
fn matching_chars(a: &[char], b: &[char]) -> usize {
    let b2j = index_b(b);
    let mut queue = vec![(0, a.len(), 0, b.len())];
    let mut total = 0;
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = find_longest_match(a, b, &b2j, (alo, ahi, blo, bhi));
        if k == 0 {
            continue;
        }
        total += k;
        if alo < i && blo < j {
            queue.push((alo, i, blo, j));
        }
        if i + k < ahi && j + k < bhi {
            queue.push((i + k, ahi, j + k, bhi));
        }
    }
    total
}

fn index_b(b: &[char]) -> HashMap<char, Vec<usize>> {
    let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
    for (j, ch) in b.iter().enumerate() {
        b2j.entry(*ch).or_default().push(j);
    }
    if b.len() >= 200 {
        let ntest = b.len() / 100 + 1;
        b2j.retain(|_, indices| indices.len() <= ntest);
    }
    b2j
}

/// `SequenceMatcher.find_longest_match` without a junk function: the longest matching block in
/// `a[alo:ahi]` / `b[blo:bhi]`, earliest in `a` then in `b`, extended over unindexed (popular) characters.
fn find_longest_match(
    a: &[char],
    b: &[char],
    b2j: &HashMap<char, Vec<usize>>,
    (alo, ahi, blo, bhi): (usize, usize, usize, usize),
) -> (usize, usize, usize) {
    let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0);
    let mut j2len: HashMap<usize, usize> = HashMap::new();
    for (i, ch) in a.iter().enumerate().take(ahi).skip(alo) {
        let mut next: HashMap<usize, usize> = HashMap::new();
        for &j in b2j.get(ch).map(Vec::as_slice).unwrap_or_default() {
            if j < blo {
                continue;
            }
            if j >= bhi {
                break;
            }
            let k = j
                .checked_sub(1)
                .and_then(|prev| j2len.get(&prev))
                .copied()
                .unwrap_or(0)
                + 1;
            next.insert(j, k);
            if k > bestsize {
                (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
            }
        }
        j2len = next;
    }
    while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
        besti -= 1;
        bestj -= 1;
        bestsize += 1;
    }
    while besti + bestsize < ahi
        && bestj + bestsize < bhi
        && a[besti + bestsize] == b[bestj + bestsize]
    {
        bestsize += 1;
    }
    (besti, bestj, bestsize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn ratio_matches_difflib() {
        // Values computed with Python's difflib.SequenceMatcher.
        assert_eq!(ratio(&chars("abcd"), &chars("bcde")), 0.75);
        assert_eq!(ratio(&chars(""), &chars("")), 1.0);
        assert_eq!(ratio(&chars("abc"), &chars("xyz")), 0.0);
        let r = ratio(&chars("hysteresispct"), &chars("hysteresispct"));
        assert_eq!(r, 1.0);
        let r = ratio(&chars("cooldownseconds"), &chars("cooldownsecond"));
        assert!((r - 28.0 / 29.0).abs() < 1e-12);
    }

    #[test]
    fn nearest_prefers_a_unique_prefix() {
        let valid = ["intervalSeconds", "cooldownSeconds", "threshold"];
        assert_eq!(
            nearest("interval", &valid).as_deref(),
            Some("intervalSeconds")
        );
        assert_eq!(nearest("th", &valid), None);
        assert_eq!(nearest("treshold", &valid).as_deref(), Some("threshold"));
    }
}
