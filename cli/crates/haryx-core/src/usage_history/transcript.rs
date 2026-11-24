//! One model reply's usage from a Claude Code transcript line. Only these fields are read; the
//! conversation text never leaves the line.

use serde_json::Value;

use super::Tokens;

/// The usage a transcript line records for one model reply.
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    /// The API request id, or the message id: Claude Code writes one line per content block, each
    /// with the same usage, so replies are counted once per id.
    pub id: String,
    /// Unix seconds.
    pub at: i64,
    pub model: String,
    pub session: String,
    pub cwd: String,
    pub tokens: Tokens,
}

/// Parse one line; `None` for anything that is not an assistant reply with usage.
pub fn parse_line(line: &str) -> Option<Reply> {
    // Most lines are user turns, tool results and metadata: skip them before parsing JSON.
    if !line.contains("\"usage\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let message = v.get("message")?;
    let usage = message.get("usage")?;
    let model = message.get("model").and_then(Value::as_str).unwrap_or("");
    if model.is_empty() || model.starts_with('<') {
        return None; // "<synthetic>" replies are Claude Code's own, not a model call.
    }
    let id = v
        .get("requestId")
        .and_then(Value::as_str)
        .or_else(|| message.get("id").and_then(Value::as_str))?
        .to_owned();
    let at = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|t| t.parse::<jiff::Timestamp>().ok())?
        .as_second();
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
    let written = n(usage, "cache_creation_input_tokens");
    let (w5, w1) = match usage.get("cache_creation") {
        Some(c) => (
            n(c, "ephemeral_5m_input_tokens"),
            n(c, "ephemeral_1h_input_tokens"),
        ),
        None => (written, 0),
    };
    Some(Reply {
        id,
        at,
        model: model.to_owned(),
        session: v
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        cwd: v
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        tokens: Tokens {
            input: n(usage, "input_tokens"),
            output: n(usage, "output_tokens"),
            cache_read: n(usage, "cache_read_input_tokens"),
            cache_write_5m: w5,
            cache_write_1h: w1,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const REPLY: &str = r#"{"type":"assistant","timestamp":"2026-11-24T08:05:24.123Z","sessionId":"s1","cwd":"/w/api","requestId":"req_1","message":{"id":"msg_1","model":"claude-opus-5-5","content":[{"type":"text","text":"secret words"}],"usage":{"input_tokens":2,"cache_creation_input_tokens":45692,"cache_read_input_tokens":29203,"output_tokens":263,"cache_creation":{"ephemeral_1h_input_tokens":45692,"ephemeral_5m_input_tokens":0}}}}"#;

    #[test]
    fn reads_a_reply() {
        let r = parse_line(REPLY).unwrap();
        assert_eq!(r.id, "req_1");
        assert_eq!(r.model, "claude-opus-5-5");
        assert_eq!(r.session, "s1");
        assert_eq!(r.cwd, "/w/api");
        assert_eq!(r.at, 1_795_507_524);
        assert_eq!(
            r.tokens,
            Tokens {
                input: 2,
                output: 263,
                cache_read: 29203,
                cache_write_5m: 0,
                cache_write_1h: 45692
            }
        );
    }

    #[test]
    fn skips_everything_else() {
        assert_eq!(
            parse_line(r#"{"type":"user","message":{"content":"hi"}}"#),
            None
        );
        assert_eq!(parse_line("not json \"usage\""), None);
        let synthetic = REPLY.replace("claude-opus-5-5", "<synthetic>");
        assert_eq!(parse_line(&synthetic), None);
        let user = REPLY.replace("\"type\":\"assistant\"", "\"type\":\"user\"");
        assert_eq!(parse_line(&user), None);
    }

    #[test]
    fn without_a_ttl_breakdown_writes_count_as_five_minute() {
        let line = REPLY.replace(
            r#","cache_creation":{"ephemeral_1h_input_tokens":45692,"ephemeral_5m_input_tokens":0}"#,
            "",
        );
        let r = parse_line(&line).unwrap();
        assert_eq!(
            (r.tokens.cache_write_5m, r.tokens.cache_write_1h),
            (45692, 0)
        );
    }

    #[test]
    fn falls_back_to_the_message_id() {
        let line = REPLY.replace(r#""requestId":"req_1","#, "");
        assert_eq!(parse_line(&line).unwrap().id, "msg_1");
    }
}
