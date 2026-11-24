//! API prices per model (`data/anthropic-pricing.json`, embedded at build), to show what usage
//! would cost at Anthropic's published API rates. Plans are flat-priced: this is a value, not a bill.

use std::sync::OnceLock;

use serde_json::Value;

use super::Tokens;

const TABLE: &str = include_str!("../../data/anthropic-pricing.json");

/// USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
}

struct Table {
    as_of: String,
    models: Vec<(String, Price)>,
}

fn table() -> &'static Table {
    static TABLE_CELL: OnceLock<Table> = OnceLock::new();
    TABLE_CELL.get_or_init(|| parse(TABLE))
}

fn parse(text: &str) -> Table {
    let data: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    let mut models: Vec<(String, Price)> = data
        .get("models")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .map(|(id, p)| {
                    let price = Price {
                        input: num(p, "input"),
                        output: num(p, "output"),
                        cache_read: num(p, "cacheRead"),
                        cache_write_5m: num(p, "cacheWrite5m"),
                        cache_write_1h: num(p, "cacheWrite1h"),
                    };
                    (id.clone(), price)
                })
                .collect()
        })
        .unwrap_or_default();
    // Longest prefix first, so `claude-opus-5-5` wins over `claude-opus-5`.
    models.sort_by_key(|m| std::cmp::Reverse(m.0.len()));
    Table {
        as_of: data
            .get("asOf")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        models,
    }
}

/// The date the embedded prices were taken.
pub fn as_of() -> &'static str {
    &table().as_of
}

/// The price for a model id (`claude-opus-5-5`, `claude-opus-5-5[1m]`, ...); `None` when unknown.
pub fn price(model: &str) -> Option<Price> {
    table()
        .models
        .iter()
        .find(|(id, _)| model.starts_with(id.as_str()))
        .map(|(_, p)| *p)
}

/// What `tokens` on `model` would cost at API prices, in USD; `None` for an unknown model.
pub fn cost(model: &str, tokens: &Tokens) -> Option<f64> {
    let p = price(model)?;
    let per = |n: u64, usd: f64| n as f64 * usd / 1_000_000.0;
    Some(
        per(tokens.input, p.input)
            + per(tokens.output, p.output)
            + per(tokens.cache_read, p.cache_read)
            + per(tokens.cache_write_5m, p.cache_write_5m)
            + per(tokens.cache_write_1h, p.cache_write_1h),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_table_reads() {
        assert!(!as_of().is_empty());
        let opus = price("claude-opus-5-5").unwrap();
        assert_eq!(
            (opus.input, opus.output, opus.cache_read),
            (4.0, 20.0, 0.20)
        );
    }

    #[test]
    fn longest_prefix_wins_and_suffixes_match() {
        assert_eq!(price("claude-opus-5").unwrap().input, 5.0);
        assert_eq!(price("claude-opus-5-5").unwrap().input, 4.0);
        assert_eq!(price("claude-opus-5-5[1m]").unwrap().input, 4.0);
        assert_eq!(price("claude-sonnet-5-5-preview").unwrap().input, 2.0);
        assert_eq!(price("gpt-something"), None);
    }

    #[test]
    fn cost_adds_every_kind_of_token() {
        let t = Tokens {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write_5m: 1_000_000,
            cache_write_1h: 1_000_000,
        };
        let c = cost("claude-opus-5-5", &t).unwrap();
        assert!((c - (4.0 + 20.0 + 0.20 + 5.0 + 8.0)).abs() < 1e-9);
        assert_eq!(cost("unknown", &t), None);
    }

    #[test]
    fn the_table_is_not_too_old() {
        // A reminder, not a correctness check: refresh data/anthropic-pricing.json from the
        // published pricing page when this fails.
        let as_of = jiff::civil::Date::strptime("%Y-%m-%d", as_of()).unwrap();
        let today = jiff::Zoned::now().date();
        let age = as_of.until(today).unwrap().get_days();
        assert!(
            age < 120,
            "the price table is {age} days old; update data/anthropic-pricing.json"
        );
    }
}
