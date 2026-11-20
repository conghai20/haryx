//! What kind of credential a value is, and which of its fields are machine-shared.

use serde_json::{Map, Value};

/// Prefixes of Anthropic OAuth tokens: the access/setup-token and the refresh token. A bare value carrying
/// one is OAuth material, never a managed API key, however it arrived. Mirrors Claude Code's own
/// `isAnthropicOAuthToken`.
const OAUTH_TOKEN_PREFIXES: [&str; 2] = ["sk-ant-oat", "sk-ant-ort"];

/// Siblings of `claudeAiOauth` holding machine-shared OAuth integrations that rotate independently of any
/// account slot: on activation the live copy is authoritative. Everything else (known or unknown) stays
/// with the target slot: a stale restore of an unlisted shared field merely re-prompts for auth, while
/// carrying a live account-bound field across a switch would present one account's credential under
/// another.
pub const SHARED_CREDENTIAL_KEYS: [&str; 5] = [
    "mcpOAuth",
    "mcpOAuthClientConfig",
    "mcpXaaIdp",
    "mcpXaaIdpConfig",
    "pluginSecrets",
];

/// Account-scoped siblings haryx knows about: `claudeAiOauth` is the login itself, `trustedDeviceToken` is
/// enrolled per (device, account) at /login.
pub const ACCOUNT_CREDENTIAL_KEYS: [&str; 2] = ["claudeAiOauth", "trustedDeviceToken"];

/// Whether a stored active credential is a raw managed API key vs OAuth JSON.
///
/// A managed key is a bare `sk-ant-…` string, while every OAuth/setup-token credential is a JSON object.
/// The Console issues several key families and Claude Code never inspects the prefix of a key it stores, so
/// neither does this: any bare `sk-ant-` value is a key UNLESS it carries an OAuth token prefix. That
/// exclusion (rather than an allowlist of key prefixes) keeps a raw `sk-ant-oat…` setup token from ever
/// being activated as an API key (upstream #465).
pub fn looks_like_api_key(credentials: Option<&str>) -> bool {
    let Some(text) = credentials.map(str::trim).filter(|t| !t.is_empty()) else {
        return false;
    };
    if text.starts_with('{') {
        return false;
    }
    text.starts_with("sk-ant-") && !OAUTH_TOKEN_PREFIXES.iter().any(|p| text.starts_with(p))
}

/// Parse a JSON credential object, excluding managed API keys.
fn credential_object(credentials: Option<&str>) -> Option<Map<String, Value>> {
    let text = credentials.filter(|t| !t.is_empty())?;
    if looks_like_api_key(Some(text)) {
        return None;
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => Some(map),
        _ => None,
    }
}

/// The machine-shared fields of a Claude OAuth credential object.
///
/// `None` means the input is not a JSON credential object (missing, malformed, or a managed API key). A map,
/// including an empty one, is authoritative for every allowlisted key: a key absent here is absent from the
/// machine's current shared state.
pub fn shared_credential_fields(credentials: Option<&str>) -> Option<Map<String, Value>> {
    let data = credential_object(credentials)?;
    if data.contains_key("claudeAiOauth") {
        // An unknown sibling defaults to slot-owned (fails safe), but leave a trace: if Claude Code grows a new
        // *shared* key, that default quietly reintroduces the stale-restore papercut for it.
        let unrecognized: Vec<&str> = data
            .keys()
            .map(String::as_str)
            .filter(|k| !SHARED_CREDENTIAL_KEYS.contains(k) && !ACCOUNT_CREDENTIAL_KEYS.contains(k))
            .collect();
        if !unrecognized.is_empty() {
            let mut sorted = unrecognized;
            sorted.sort_unstable();
            log::debug!(
                "Live credential has sibling keys haryx does not recognize (a newer Claude Code?), \
                 treating them as slot-owned: {sorted:?}"
            );
        }
    }
    Some(
        SHARED_CREDENTIAL_KEYS
            .iter()
            .filter_map(|k| data.get(*k).map(|v| ((*k).to_string(), v.clone())))
            .collect(),
    )
}

/// Compose a target Claude login with the machine's shared fields.
///
/// The allowlisted keys are wholly live-owned, presence and absence alike: the target's copies are dropped
/// and `shared_fields` supplies the current generation. All other target fields pass through. Returns
/// `target_credentials` unchanged when it is not a JSON credential object carrying a Claude login (managed
/// API keys and opaque legacy shapes stay activatable verbatim).
pub fn merge_shared_credential_fields(
    target_credentials: &str,
    shared_fields: &Map<String, Value>,
) -> String {
    let Some(target) = credential_object(Some(target_credentials)) else {
        return target_credentials.to_string();
    };
    if !target.contains_key("claudeAiOauth") {
        return target_credentials.to_string();
    }
    let composed: Map<String, Value> = target
        .into_iter()
        .filter(|(k, _)| !SHARED_CREDENTIAL_KEYS.contains(&k.as_str()))
        .chain(shared_fields.iter().map(|(k, v)| (k.clone(), v.clone())))
        .collect();
    python_json_dumps(&Value::Object(composed))
}

/// The value Claude Code stores in `customApiKeyResponses.approved`: the last 20 chars (Claude Code's
/// `normalizeApiKeyForConfig`). Anything else makes Claude Code re-prompt for approval.
pub fn approved_form(api_key: &str) -> String {
    let trimmed = api_key.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    chars[chars.len().saturating_sub(20)..].iter().collect()
}

/// Python's compact `json.dumps` separators (`", "` and `": "`), with non-ASCII kept as UTF-8.
pub(crate) fn python_json_dumps(value: &Value) -> String {
    let mut out = String::new();
    write_compact(value, &mut out);
    out
}

fn write_compact(value: &Value, out: &mut String) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_compact(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write_compact(v, out);
            }
            out.push('}');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const API_KEY: &str = "sk-ant-api03-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn api_keys_are_detected_by_excluding_oauth_prefixes() {
        for value in [
            API_KEY,
            "sk-ant-usr-pppppppppppppppppppppppppppppppp",
            "sk-ant-new99-kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk",
            "sk-ant-admin01-dddddddddddddddddddddddddddddd",
        ] {
            assert!(looks_like_api_key(Some(value)), "{value}");
            assert!(looks_like_api_key(Some(&format!("  {value}\n"))), "{value}");
        }
    }

    #[test]
    fn non_keys_are_not_api_keys() {
        for value in [
            Some(""),
            None,
            Some("sk-ant-oat01-ssssssssssssssssssss"),
            Some("  sk-ant-oat01-ssssssssssssssssssss\n"),
            Some("sk-ant-ort01-rrrrrrrrrrrrrrrrrrrr"),
            Some("sk-ant-oat"),
            Some("not-a-key"),
            Some(r#"{"claudeAiOauth": {"accessToken": "x"}}"#),
            Some(r#"{"x": "sk-ant-api03-inside-json"}"#),
            Some(r#"{"x": "sk-ant-usr-inside-json"}"#),
        ] {
            assert!(!looks_like_api_key(value), "{value:?}");
        }
    }

    #[test]
    fn approved_form_is_the_last_20_chars() {
        assert_eq!(approved_form(API_KEY), &API_KEY[API_KEY.len() - 20..]);
        assert_eq!(approved_form(API_KEY).len(), 20);
        assert_eq!(approved_form(" short "), "short");
    }

    #[test]
    fn shared_fields_take_only_the_allowlist() {
        let live = json!({
            "claudeAiOauth": {"accessToken": "a"},
            "trustedDeviceToken": "t",
            "mcpOAuth": {"s": 1},
            "pluginSecrets": {"p": 2},
            "somethingNew": 3,
        })
        .to_string();
        let shared = shared_credential_fields(Some(&live)).unwrap();
        assert_eq!(
            Value::Object(shared),
            json!({"mcpOAuth": {"s": 1}, "pluginSecrets": {"p": 2}})
        );
        assert!(shared_credential_fields(Some(API_KEY)).is_none());
        assert!(shared_credential_fields(Some("not json")).is_none());
        assert!(shared_credential_fields(None).is_none());
        assert_eq!(shared_credential_fields(Some("{}")), Some(Map::new()));
    }

    #[test]
    fn merge_replaces_shared_keys_wholesale() {
        let target = json!({
            "claudeAiOauth": {"accessToken": "target"},
            "trustedDeviceToken": "target-device",
            "mcpOAuth": {"stale": true},
            "mcpXaaIdp": {"gone": true},
        })
        .to_string();
        let shared = json!({"mcpOAuth": {"fresh": true}});
        let merged = merge_shared_credential_fields(&target, shared.as_object().unwrap());
        let parsed: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(
            parsed,
            json!({
                "claudeAiOauth": {"accessToken": "target"},
                "trustedDeviceToken": "target-device",
                "mcpOAuth": {"fresh": true},
            })
        );
        assert!(merged.starts_with(r#"{"claudeAiOauth": {"accessToken": "target"}, "#));
    }

    #[test]
    fn merge_leaves_keys_and_opaque_shapes_verbatim() {
        let shared = Map::new();
        assert_eq!(merge_shared_credential_fields(API_KEY, &shared), API_KEY);
        assert_eq!(
            merge_shared_credential_fields(r#"{"x":1}"#, &shared),
            r#"{"x":1}"#
        );
        assert_eq!(
            merge_shared_credential_fields("garbage", &shared),
            "garbage"
        );
    }
}
