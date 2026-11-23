//! Secret detection and placeholders.
//!
//! A detected value becomes `${HARYX_SECRET_<NAME>}`; the [`SecretBook`] keeps name -> value. On apply,
//! placeholders are resolved from the book, then from the environment variable `HARYX_SECRET_<NAME>`.
//! An unresolved placeholder stays as written: Claude Code expands `${VAR}` in MCP configs, and in a
//! shell script it is an ordinary variable reference.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value};

/// Prefix of every placeholder variable.
pub const VAR_PREFIX: &str = "HARYX_SECRET_";
/// What an exported history keeps of a redacted secret.
pub const HISTORY_REDACTION: &str = "[REDACTED:haryx]";

/// Known secret shapes: (pattern, name hint, capture group holding the secret).
fn patterns() -> &'static [(Regex, &'static str, usize)] {
    static PATTERNS: OnceLock<Vec<(Regex, &'static str, usize)>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        let table: [(&str, &str, usize); 8] = [
            (r"sk-ant-[A-Za-z0-9_\-]{20,}", "ANTHROPIC_KEY", 0),
            (r"sk-(?:proj-)?[A-Za-z0-9_\-]{32,}", "OPENAI_KEY", 0),
            (r"github_pat_[A-Za-z0-9_]{40,}", "GITHUB_TOKEN", 0),
            (r"gh[pousr]_[A-Za-z0-9]{30,}", "GITHUB_TOKEN", 0),
            (r"xox[abprs]-[A-Za-z0-9\-]{10,}", "SLACK_TOKEN", 0),
            (r"AKIA[0-9A-Z]{16}", "AWS_ACCESS_KEY", 0),
            (r"AIza[0-9A-Za-z_\-]{35}", "GOOGLE_API_KEY", 0),
            (
                r"(?i)bearer\s+([A-Za-z0-9._~+/=\-]{20,})",
                "BEARER_TOKEN",
                1,
            ),
        ];
        table
            .into_iter()
            .map(|(re, name, group)| (Regex::new(re).expect("static pattern"), name, group))
            .collect()
    })
}

/// Keys of an `env`/`headers` object whose values are secrets.
fn secret_key() -> &'static Regex {
    static KEY: OnceLock<Regex> = OnceLock::new();
    KEY.get_or_init(|| {
        Regex::new(
            r"(?i)(token|secret|passw(or)?d|api[_-]?key|access[_-]?key|private[_-]?key|credential|^auth(orization)?$|^x-api-key$)",
        )
        .expect("static pattern")
    })
}

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\$\{HARYX_SECRET_([A-Z0-9_]+)\}").expect("static pattern"))
}

/// The placeholder text for a name.
pub fn placeholder(name: &str) -> String {
    format!("${{{VAR_PREFIX}{name}}}")
}

/// Names of the placeholders in a text, in order, without duplicates.
pub fn placeholder_names(text: &str) -> Vec<String> {
    let mut seen = Vec::new();
    for cap in placeholder_re().captures_iter(text) {
        let name = cap[1].to_owned();
        if !seen.contains(&name) {
            seen.push(name);
        }
    }
    seen
}

/// Whether the text holds at least one placeholder.
pub fn has_placeholder(text: &str) -> bool {
    text.contains(VAR_PREFIX) && placeholder_re().is_match(text)
}

/// Replace placeholders with known values; unknown ones stay. Returns the text and the unresolved names.
pub fn resolve_text<'a>(
    text: &'a str,
    values: &BTreeMap<String, String>,
) -> (Cow<'a, str>, Vec<String>) {
    if !has_placeholder(text) {
        return (Cow::Borrowed(text), Vec::new());
    }
    let mut missing = Vec::new();
    let out = placeholder_re().replace_all(text, |cap: &regex::Captures<'_>| {
        let name = &cap[1];
        match values.get(name) {
            Some(value) => value.clone(),
            None => {
                if !missing.iter().any(|m| m == name) {
                    missing.push(name.to_owned());
                }
                cap[0].to_owned()
            }
        }
    });
    (Cow::Owned(out.into_owned()), missing)
}

/// Every placeholder becomes [`HISTORY_REDACTION`] (history exported with `--secrets redact`).
pub fn redact_placeholders(text: &str) -> Cow<'_, str> {
    placeholder_re().replace_all(text, HISTORY_REDACTION)
}

/// Whether a text holds something that looks like a secret.
pub fn looks_secret(text: &str) -> bool {
    patterns().iter().any(|(re, _, _)| re.is_match(text))
}

/// Detected secrets: name -> value, de-duplicated by value.
#[derive(Clone, Debug, Default)]
pub struct SecretBook {
    values: BTreeMap<String, String>,
    by_value: HashMap<String, String>,
}

impl SecretBook {
    pub fn from_values(values: BTreeMap<String, String>) -> Self {
        let by_value = values.iter().map(|(k, v)| (v.clone(), k.clone())).collect();
        SecretBook { values, by_value }
    }

    pub fn values(&self) -> &BTreeMap<String, String> {
        &self.values
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The name of a value, registering it under `hint` (made unique) when new.
    fn name_for(&mut self, value: &str, hint: &str) -> String {
        if let Some(name) = self.by_value.get(value) {
            return name.clone();
        }
        let base = sanitize(hint);
        let name = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base}_{n}")
                }
            })
            .find(|candidate| !self.values.contains_key(candidate))
            .unwrap_or(base);
        self.values.insert(name.clone(), value.to_owned());
        self.by_value.insert(value.to_owned(), name.clone());
        name
    }

    /// Replace known secret shapes in a text with placeholders.
    pub fn redact_text<'a>(&mut self, text: &'a str) -> Cow<'a, str> {
        if !looks_secret(text) {
            return Cow::Borrowed(text);
        }
        let mut current = text.to_owned();
        for (re, hint, group) in patterns() {
            let mut replaced = String::with_capacity(current.len());
            let mut last = 0;
            for cap in re.captures_iter(&current) {
                let Some(m) = cap.get(*group) else { continue };
                replaced.push_str(&current[last..m.start()]);
                let name = self.name_for(m.as_str(), hint);
                replaced.push_str(&placeholder(&name));
                last = m.end();
            }
            replaced.push_str(&current[last..]);
            current = replaced;
        }
        Cow::Owned(current)
    }

    /// Redact a JSON value: values of `env`/`headers` objects whose key names a secret, and every string
    /// that matches a known secret shape. Returns a new value.
    pub fn redact_json(&mut self, value: &Value) -> Value {
        self.redact_json_in(value, None)
    }

    fn redact_json_in(&mut self, value: &Value, parent_key: Option<&str>) -> Value {
        match value {
            Value::Object(map) => {
                let secret_container = matches!(parent_key, Some("env" | "headers"));
                let out: Map<String, Value> = map
                    .iter()
                    .map(|(k, v)| {
                        let redacted = match v {
                            Value::String(s)
                                if secret_container
                                    && secret_key().is_match(k)
                                    && is_literal_secret(s) =>
                            {
                                Value::String(placeholder(&self.name_for(s, k)))
                            }
                            other => self.redact_json_in(other, Some(k)),
                        };
                        (k.clone(), redacted)
                    })
                    .collect();
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|v| self.redact_json_in(v, parent_key))
                    .collect(),
            ),
            Value::String(s) => Value::String(self.redact_text(s).into_owned()),
            other => other.clone(),
        }
    }
}

/// A value worth hiding: not empty, not already a placeholder or a `${VAR}` reference.
fn is_literal_secret(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && !(trimmed.starts_with("${") && trimmed.ends_with('}'))
}

/// An uppercase `[A-Z0-9_]` name.
fn sanitize(hint: &str) -> String {
    let name: String = hint
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = name.trim_matches('_');
    if trimmed.is_empty() {
        "SECRET".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Fill missing names from the environment (`HARYX_SECRET_<NAME>`). Returns the merged map.
pub fn with_env(
    values: &BTreeMap<String, String>,
    names: &[String],
    lookup: &dyn Fn(&str) -> Option<String>,
) -> BTreeMap<String, String> {
    let from_env = names.iter().filter_map(|name| {
        lookup(&format!("{VAR_PREFIX}{name}"))
            .filter(|v| !v.is_empty())
            .map(|v| (name.clone(), v))
    });
    values
        .clone()
        .into_iter()
        .chain(from_env)
        .collect::<BTreeMap<_, _>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_known_shapes_and_reuses_names() {
        let mut book = SecretBook::default();
        let key = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz";
        let text = format!("export K={key}\necho {key}");
        let out = book.redact_text(&text).into_owned();
        assert_eq!(
            out,
            "export K=${HARYX_SECRET_ANTHROPIC_KEY}\necho ${HARYX_SECRET_ANTHROPIC_KEY}"
        );
        assert_eq!(book.values()["ANTHROPIC_KEY"], key);
    }

    #[test]
    fn redacts_bearer_value_only() {
        let mut book = SecretBook::default();
        let out = book.redact_text("Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123");
        assert_eq!(out, "Authorization: Bearer ${HARYX_SECRET_BEARER_TOKEN}");
    }

    #[test]
    fn redacts_env_keys_structurally() {
        let mut book = SecretBook::default();
        let value = json!({"github": {"command": "npx", "env": {
            "GITHUB_TOKEN": "plain-value", "NODE_ENV": "production", "API_KEY": "${FROM_ENV}"}}});
        let out = book.redact_json(&value);
        assert_eq!(
            out["github"]["env"]["GITHUB_TOKEN"],
            "${HARYX_SECRET_GITHUB_TOKEN}"
        );
        assert_eq!(out["github"]["env"]["NODE_ENV"], "production");
        assert_eq!(out["github"]["env"]["API_KEY"], "${FROM_ENV}");
        assert_eq!(book.values()["GITHUB_TOKEN"], "plain-value");
    }

    #[test]
    fn distinct_values_get_distinct_names() {
        let mut book = SecretBook::default();
        let a = book.name_for("one", "TOKEN");
        let b = book.name_for("two", "TOKEN");
        assert_eq!((a.as_str(), b.as_str()), ("TOKEN", "TOKEN_2"));
    }

    #[test]
    fn resolves_known_and_reports_missing() {
        let values: BTreeMap<_, _> = [("A".to_owned(), "1".to_owned())].into_iter().collect();
        let (out, missing) = resolve_text("${HARYX_SECRET_A}-${HARYX_SECRET_B}", &values);
        assert_eq!(out, "1-${HARYX_SECRET_B}");
        assert_eq!(missing, vec!["B"]);
    }

    #[test]
    fn env_fills_missing_names() {
        let lookup = |k: &str| (k == "HARYX_SECRET_B").then(|| "2".to_owned());
        let merged = with_env(&BTreeMap::new(), &["B".to_owned()], &lookup);
        assert_eq!(merged["B"], "2");
    }

    #[test]
    fn history_redaction_drops_placeholders() {
        assert_eq!(
            redact_placeholders("key ${HARYX_SECRET_X} end"),
            "key [REDACTED:haryx] end"
        );
    }
}
