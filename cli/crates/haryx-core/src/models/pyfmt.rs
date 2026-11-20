//! Python-compatible text formatting.
//!
//! Upstream messages and files are produced by Python's `repr()`, `format(x, "g")` and
//! `json.dumps(..., indent=2)`; these helpers reproduce them so haryx prints and writes the same text.

use serde_json::Value;

/// Python's `repr()` of a string: single quotes unless the text holds a `'` and no `"`, with
/// backslashes, quotes and control characters escaped, so stray whitespace or a newline in a key stays
/// visible and on one line.
pub fn repr_str(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(text.len() + 2);
    out.push(quote);
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => {
                let code = u32::from(c);
                if code < 0x100 {
                    out.push_str(&format!("\\x{code:02x}"));
                } else if code < 0x10000 {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    out.push_str(&format!("\\U{code:08x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Python's `repr()` of a parsed JSON value, as upstream's `%r` shows a value read from settings.json.
pub fn repr_json(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => repr_str(s),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

/// Python's `format(x, "g")`: six significant digits, trailing zeros dropped, scientific notation below
/// 1e-4 and from 1e6 on (`20` → `20`, `99.91` → `99.91`, `1e6` → `1e+06`).
pub fn format_g(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "inf" } else { "-inf" }.to_owned();
    }
    if value == 0.0 {
        return if value.is_sign_negative() { "-0" } else { "0" }.to_owned();
    }
    // `{:.5e}` rounds to six significant digits; its exponent decides the notation, as in C's %g.
    let sci = format!("{value:.5e}");
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if !(-4..6).contains(&exp) {
        let mantissa = strip_fraction_zeros(mantissa);
        let sign = if exp < 0 { '-' } else { '+' };
        return format!("{mantissa}e{sign}{:02}", exp.abs());
    }
    let decimals = usize::try_from(5 - exp).unwrap_or(0);
    strip_fraction_zeros(&format!("{value:.decimals$}")).to_owned()
}

fn strip_fraction_zeros(text: &str) -> &str {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        text
    }
}

/// Python's `json.dumps(value, indent=2)`: two-space indent and `ensure_ascii` (every non-ASCII character
/// written as a `\uXXXX` escape, astral ones as a surrogate pair).
pub fn json_dumps_pretty(value: &Value) -> String {
    // serde_json cannot fail to serialize a `Value` (its keys are strings); the fallback is unreachable.
    let pretty = serde_json::to_string_pretty(value).unwrap_or_default();
    ascii_escape(&pretty)
}

/// Escape non-ASCII characters. Safe on serialized JSON: such characters only occur inside strings.
fn ascii_escape(json: &str) -> String {
    if json.is_ascii() {
        return json.to_owned();
    }
    let mut out = String::with_capacity(json.len() + 16);
    for ch in json.chars() {
        if ch.is_ascii() {
            out.push(ch);
        } else {
            let mut units = [0u16; 2];
            for unit in ch.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repr_quotes_like_python() {
        assert_eq!(repr_str("abc"), "'abc'");
        assert_eq!(repr_str("it's"), "\"it's\"");
        assert_eq!(repr_str("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(repr_str("a\nb"), "'a\\nb'");
        assert_eq!(repr_str("threshold "), "'threshold '");
        assert_eq!(repr_str("\u{1}"), "'\\x01'");
    }

    #[test]
    fn repr_of_json_values() {
        assert_eq!(repr_json(&json!(null)), "None");
        assert_eq!(repr_json(&json!(true)), "True");
        assert_eq!(repr_json(&json!("x")), "'x'");
        assert_eq!(repr_json(&json!(5)), "5");
    }

    #[test]
    fn format_g_matches_python() {
        for (value, expected) in [
            (50.0, "50"),
            (99.9, "99.9"),
            (99.91, "99.91"),
            (100.0, "100"),
            (1000.0, "1000"),
            (-85.0, "-85"),
            (0.0, "0"),
            (1e6, "1e+06"),
            (123_456_789.0, "1.23457e+08"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (99.999_999_9, "100"),
            (f64::NAN, "nan"),
            (f64::INFINITY, "inf"),
        ] {
            assert_eq!(format_g(value), expected, "{value}");
        }
    }

    #[test]
    fn json_dumps_escapes_non_ascii_like_python() {
        let value = json!({"a": "é😀", "b": [1, 2.5], "c": {}});
        assert_eq!(
            json_dumps_pretty(&value),
            "{\n  \"a\": \"\\u00e9\\ud83d\\ude00\",\n  \"b\": [\n    1,\n    2.5\n  ],\n  \"c\": {}\n}"
        );
    }
}
