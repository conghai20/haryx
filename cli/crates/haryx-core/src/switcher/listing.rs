//! `list` and `status`: upstream `_usage_rows`, `_format_usage_lines`, `_login_row`, `SENTINEL_NOTES`,
//! `ERROR_NOTES`, `last_seen_note`, `_plan_marker`, `_usage_entry_lines`, `_label_token_status`,
//! `_token_status_lines`, `_duplicate_account_warnings`, `_lockstep_usage_warnings`,
//! `_build_list_payload`, `list_accounts`, `_active_account_usage`, `_build_status_payload`, `status`.

use std::collections::{BTreeMap, HashMap, HashSet};

use log::debug;
use serde_json::{Map, Value, json};

use super::{AccountInfo, Switcher, groups};
use crate::json_output::{
    AccountRowInput, SCHEMA_VERSION, USAGE_API_KEY, USAGE_FOREIGN_CREDENTIAL,
    USAGE_KEYCHAIN_UNAVAILABLE, USAGE_RELOGIN_REQUIRED, USAGE_TOKEN_EXPIRED, account_row,
    last_good_usage_fields, usage_failure_fields, usage_fields, usage_freshness_fields,
};
use crate::printer::{
    abbreviate_path, accent, bold_accent, bolded, dimmed, entrypoint_label, format_age,
    ide_short_name, muted, warning, yellowed,
};
use crate::usage_store::UsageEntry;
use crate::{
    AI_PROG, PROG, credentials, oauth, pace, paths, poll_policy, process, session, settings,
    subscription,
};

/// Slack between two slots' reset instants before the lockstep heuristic stops calling them the same
/// window (the stagger and latency spread of one collect pass).
const LOCKSTEP_RESET_TOLERANCE_S: f64 = 5.0;
/// Show a "· Xm ago" note on displayed usage older than the serve TTL.
const USAGE_AGE_NOTE_S: f64 = poll_policy::SERVE_TTL_S;

/// Human note for a sentinel usage state (the TUI uses the same wording).
pub fn sentinel_note(sentinel: &str) -> String {
    match sentinel {
        USAGE_TOKEN_EXPIRED => {
            "token expired — refresh deferred this pass; retries automatically".into()
        }
        USAGE_FOREIGN_CREDENTIAL => {
            "live credential belongs to another account — a switch repairs it".into()
        }
        USAGE_API_KEY => "API key (no quota)".into(),
        USAGE_KEYCHAIN_UNAVAILABLE => "keychain unavailable — locked or in use; try again".into(),
        USAGE_RELOGIN_REQUIRED => {
            format!("re-login needed — refresh token dead; sign in again: {AI_PROG} login <num>")
        }
        other => other.to_owned(),
    }
}

/// Friendly text for error kinds that deserve more than their identifier.
pub fn error_note(kind: &str) -> String {
    match kind {
        "store-unmirrored" => "CLAUDE_SECURESTORAGE_CONFIG_DIR set — unset it or run from a normal shell".into(),
        "invalid_client" => format!("{PROG}'s OAuth client was rejected — systemic, not this account"),
        "consume-busy" => format!("another {PROG} surface holds the slot — retries next pass"),
        "stash-unreadable" => format!(
            "this slot's stashed successor is unreadable — unlock the keychain or fix the file, then retry; \
             `{AI_PROG} unclaimed` inspects it"
        ),
        "live-login" => "the live login is this account's, so Claude Code renews it — switch to this account to realign".into(),
        "oauth_not_allowed_for_organization" => "this account's organization does not allow OAuth sign-in (a lapsed \
             plan or an org policy), so its usage cannot be read — renewing the plan restores it"
            .into(),
        "tls-cert" => "the TLS certificate was refused — if a proxy re-signs traffic, trust its CA in the OS \
             certificate store or via SSL_CERT_FILE (NODE_EXTRA_CA_CERTS is not read); otherwise check for a \
             captive portal or a wrong system clock"
            .into(),
        "setup-token" => format!(
            "setup-tokens cannot read the usage endpoint — `{AI_PROG} config set usage.headerProbe true` reads it \
             from a 1-token request"
        ),
        other => other.to_owned(),
    }
}

fn age_note(epoch_s: f64) -> String {
    format_age((epoch_s * 1000.0) as i64)
}

/// `f"{x:>3.0f}%"`.
fn pct3(x: f64) -> String {
    format!("{x:>3.0}%")
}

/// `f"{x:,.2f}"`: two decimals with thousands separators.
fn money(x: f64) -> String {
    let text = format!("{:.2}", x.abs());
    let (int, frac) = text.split_once('.').unwrap_or((&text, "00"));
    let grouped: Vec<String> = int
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    let sign = if x < 0.0 { "-" } else { "" };
    format!("{sign}{}.{frac}", grouped.join(","))
}

fn pace_marker(window: &Value, fetched_at: Option<f64>) -> &'static str {
    match pace::compute_pace(Some(window), fetched_at) {
        Some(result) if result.ahead => "  (ahead of pace)",
        _ => "",
    }
}

/// (label, body) rows for one usage measurement, unpadded.
fn usage_rows(usage: &Value, fetched_at: Option<f64>) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if let Some(spend) = usage.get("spend").filter(|s| s.is_object()) {
        let num = |k: &str| spend.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let amounts = format!("${} / ${}", money(num("used")), money(num("limit")));
        let body = match oauth::fresh_reset_strings(spend) {
            Some((_, clock)) => format!("{}   resets {clock:<12}  {amounts}", pct3(num("pct"))),
            None => format!("{}   {amounts}", pct3(num("pct"))),
        };
        rows.push(("$$".to_owned(), body));
    }
    for (label, key) in [("5h", "five_hour"), ("7d", "seven_day")] {
        let Some(w) = usage
            .get(key)
            .filter(|w| w.as_object().is_some_and(|o| !o.is_empty()))
        else {
            continue;
        };
        let marker = if label == "7d" {
            pace_marker(w, fetched_at)
        } else {
            ""
        };
        let pct = w
            .get("pct")
            .and_then(Value::as_f64)
            .map_or_else(|| "   ?".to_owned(), pct3);
        let body = match oauth::fresh_reset_strings(w) {
            Some((countdown, clock)) => {
                format!("{pct}   resets {clock:<12}  in {countdown}{marker}")
            }
            None => format!("{pct}{marker}"),
        };
        rows.push((label.to_owned(), body));
    }
    for w in usage
        .get("scoped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let pct = w.get("pct").and_then(Value::as_f64).unwrap_or(0.0);
        let marker = if pct >= 100.0 {
            "  (!)"
        } else {
            pace_marker(w, fetched_at)
        };
        let name = w
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let body = match oauth::fresh_reset_strings(w) {
            Some((countdown, clock)) => {
                format!("{}   resets {clock:<12}  in {countdown}{marker}", pct3(pct))
            }
            None => format!("{}{marker}", pct3(pct)),
        };
        rows.push((name, body));
    }
    if let Some(rejected) = usage.get("rejected").filter(|r| r.is_object()) {
        let body = match oauth::fresh_reset_strings(rejected) {
            Some((countdown, clock)) => format!("rejected   until {clock:<12}  in {countdown}"),
            None => "rejected".to_owned(),
        };
        rows.push(("probe".to_owned(), body));
    }
    rows
}

fn pad_row(label: &str, body: &str, width: usize) -> String {
    format!("{:<width$} {body}", format!("{label}:"))
}

fn format_usage_lines(
    usage: &Value,
    fetched_at: Option<f64>,
    extra: &[(String, String)],
) -> Vec<String> {
    let mut rows = usage_rows(usage, fetched_at);
    rows.extend(extra.iter().cloned());
    let width = rows
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0)
        + 1;
    rows.iter().map(|(l, b)| pad_row(l, b, width)).collect()
}

/// The login-expiry row, shaped like a window row, or `None` while there is nothing to say.
fn login_row(login_expires_at: Option<f64>, quarantined: bool) -> Option<(String, String)> {
    let left = oauth::login_time_left(login_expires_at, quarantined, None)?;
    let clock = oauth::login_expiry_clock(login_expires_at?, None);
    let body = if left <= 0.0 {
        format!("{:4}  expired {clock:<12}  re-login needed", "")
    } else {
        format!(
            "{:4}  expires {clock:<12}  in {}",
            "",
            oauth::format_countdown(left)
        )
    };
    Some(("login".to_owned(), body))
}

/// "last seen 53% used · 12m ago" from an entry's last-good measurement.
pub fn last_seen_note(entry: &UsageEntry) -> Option<String> {
    let (usage, fetched_at) = (entry.last_good.as_ref()?, entry.fetched_at?);
    let headroom = oauth::account_headroom(Some(usage), &[])?;
    Some(format!(
        "last seen {:.0}% used · {}",
        100.0 - headroom,
        age_note(fetched_at)
    ))
}

/// ` · Ultra` after a slot's header; a lapsed subscription is a highlighted `Free`.
fn plan_marker(record: Option<&Map<String, Value>>, now: f64) -> String {
    let Some(record) = record else {
        return String::new();
    };
    let plan = record.get("plan").and_then(Value::as_object);
    let label = subscription::plan_label(plan).unwrap_or_default();
    let painted = if subscription::is_expired(plan) {
        yellowed(&label)
    } else {
        muted(&label)
    };
    let mut marker = format!(" {} {painted}", muted("·"));
    if let Some(fetched) = record.get("fetchedAt").and_then(Value::as_f64)
        && now - fetched >= 2.0 * subscription::PLAN_TTL_S
    {
        marker.push_str(&format!(" {}", dimmed(&format!("({})", age_note(fetched)))));
    }
    marker
}

fn tree_prefix(j: usize, len: usize) -> String {
    dimmed(if j + 1 == len { "└" } else { "├" })
}

/// Styled usage lines (sans indent) for one account's entry.
pub fn usage_entry_lines(entry: &UsageEntry, login_expires_at: Option<f64>) -> Vec<String> {
    let quarantined = entry.sentinel.as_deref() == Some(USAGE_RELOGIN_REQUIRED);
    let login = login_row(login_expires_at, quarantined);
    let login_line = login
        .as_ref()
        .map(|(l, b)| muted(&pad_row(l, b, l.chars().count() + 1)));
    if let Some(sentinel) = entry.sentinel.as_deref() {
        let mut children = Vec::new();
        if sentinel != USAGE_API_KEY
            && let Some(seen) = last_seen_note(entry)
        {
            children.push(muted(&seen));
        }
        children.extend(login_line);
        let len = children.len();
        let mut out = vec![dimmed(&sentinel_note(sentinel))];
        out.extend(
            children
                .into_iter()
                .enumerate()
                .map(|(j, line)| format!("{} {line}", tree_prefix(j, len))),
        );
        return out;
    }
    if let Some(usage) = entry.last_good.as_ref() {
        let extra: Vec<(String, String)> = login.into_iter().collect();
        let mut lines = format_usage_lines(usage, entry.fetched_at, &extra);
        let usage_count = lines.len() - extra.len();
        if usage_count > 0
            && entry.age_s.is_some_and(|a| a > USAGE_AGE_NOTE_S)
            && let Some(fetched) = entry.fetched_at
        {
            lines[usage_count - 1].push_str(&format!(" · {}", age_note(fetched)));
        }
        let len = lines.len();
        return lines
            .iter()
            .enumerate()
            .map(|(j, line)| format!("{} {}", tree_prefix(j, len), muted(line)))
            .collect();
    }
    let mut detail = "usage unavailable".to_owned();
    if let Some(err) = entry.last_error.as_deref() {
        detail.push_str(&format!(" ({})", error_note(err)));
    }
    let mut out = vec![dimmed(&detail)];
    if let Some(line) = login_line {
        out.push(format!("{} {line}", dimmed("└")));
    }
    out
}

fn label_token_status(source: &str, credentials: &str) -> Option<String> {
    let status = oauth::build_token_status(credentials)?;
    Some(match status.strip_prefix("oauth: ") {
        Some(rest) => format!("{source}: {rest}"),
        None => format!("{source}: {status}"),
    })
}

impl Switcher {
    fn token_status_lines(&self, info: &AccountInfo) -> Vec<String> {
        if credentials::looks_like_api_key(Some(&info.credentials)) {
            return Vec::new();
        }
        if info.is_active {
            return label_token_status("active profile", &info.credentials)
                .into_iter()
                .collect();
        }
        let mut lines = Vec::new();
        let session_dir = self.session_dir(&info.num(), &info.email);
        if let Some(profile) =
            session::read_session_credentials(&self.profile_keychain(), &session_dir)
                .filter(|c| !c.is_empty())
        {
            if session::session_identity_drifted(&session_dir, &info.email, &info.org_uuid) {
                lines.push("session profile: ignored (different account)".to_owned());
            } else {
                lines.extend(label_token_status("session profile", &profile));
            }
        }
        lines.extend(label_token_status("stored backup", &info.credentials));
        lines
    }

    /// Two slots holding one credential, or authenticating as one identity.
    fn duplicate_account_warnings(&self, infos: &[AccountInfo]) -> Vec<String> {
        let data = self.get_sequence_data().ok().flatten().unwrap_or_default();
        let mut by_fp: HashMap<String, String> = HashMap::new();
        let mut by_identity: HashMap<(String, String), String> = HashMap::new();
        let mut out = Vec::new();
        for info in infos {
            let snum = info.num();
            if let Some(fp) = oauth::credential_fingerprint(&info.credentials) {
                match by_fp.get(&fp) {
                    Some(other) => out.push(format!(
                        "Account-{other} and Account-{snum} hold the same credential ({}) — one slot's backup \
                         was overwritten. Log in with the missing account and re-add it: {AI_PROG} add --slot N",
                        info.email
                    )),
                    None => {
                        by_fp.insert(fp, snum.clone());
                    }
                }
            }
            let uuid = data
                .get("accounts")
                .and_then(|a| a.get(&snum))
                .and_then(|a| a.get("uuid"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            if !uuid.is_empty() {
                let key = (uuid, info.org_uuid.clone());
                match by_identity.get(&key) {
                    Some(other) if *other != snum => out.push(format!(
                        "Account-{other} and Account-{snum} both authenticate as {} — remove or re-login one of them.",
                        info.email
                    )),
                    Some(_) => {}
                    None => {
                        by_identity.insert(key, snum);
                    }
                }
            }
        }
        out
    }

    /// Slots reporting identical usage and matching reset times (likely the same account).
    fn lockstep_usage_warnings(
        &self,
        infos: &[AccountInfo],
        entries: &BTreeMap<String, UsageEntry>,
    ) -> Vec<String> {
        let rows: Vec<(String, f64, f64, f64, f64)> = infos
            .iter()
            .filter_map(|info| {
                let snum = info.num();
                let usage = match entries.get(&snum)?.decision_value()? {
                    crate::usage_store::Decision::Usage(u) => u,
                    crate::usage_store::Decision::Sentinel(_) => return None,
                };
                let window = |k: &str| {
                    let w = usage.get(k)?;
                    let pct = w.get("pct")?.as_f64()?;
                    let ts =
                        poll_policy::parse_reset_ts(w.get("resets_at").and_then(Value::as_str))?;
                    Some((pct, ts))
                };
                let (h5, h5_ts) = window("five_hour")?;
                let (d7, d7_ts) = window("seven_day")?;
                Some((snum, h5, h5_ts, d7, d7_ts))
            })
            .collect();
        let mut out = Vec::new();
        for (i, (snum, h5, h5_ts, d7, d7_ts)) in rows.iter().enumerate() {
            let twin = rows[..i].iter().find(|(_, o_h5, o_h5_ts, o_d7, o_d7_ts)| {
                h5 == o_h5
                    && d7 == o_d7
                    && (h5_ts - o_h5_ts).abs() <= LOCKSTEP_RESET_TOLERANCE_S
                    && (d7_ts - o_d7_ts).abs() <= LOCKSTEP_RESET_TOLERANCE_S
            });
            if let Some((other, ..)) = twin {
                out.push(format!(
                    "Account-{other} and Account-{snum} report identical usage and matching reset times — they \
                     may be the same account (issue #117). If it persists, log in with the missing account and \
                     re-add it: {AI_PROG} add --slot N"
                ));
            }
        }
        out
    }

    fn build_list_payload(
        &self,
        infos: &[AccountInfo],
        entries: &BTreeMap<String, UsageEntry>,
        plans: &BTreeMap<String, Map<String, Value>>,
    ) -> Value {
        let seq = self.get_sequence_data().ok().flatten().unwrap_or_default();
        let now = self.usage_store.now();
        let mut active: Option<i64> = None;
        let default_entry = UsageEntry::default();
        let rows: Vec<Value> = infos
            .iter()
            .map(|info| {
                let n = info.num();
                if info.is_active {
                    active = Some(info.number);
                }
                let entry = entries.get(&n).unwrap_or(&default_entry);
                let login = oauth::login_expires_at_iso(&info.credentials);
                let plan = plans.get(&n).map(|p| Value::Object(p.clone()));
                account_row(&AccountRowInput {
                    number: info.number,
                    email: &info.email,
                    org_name: &info.org_name,
                    org_uuid: &info.org_uuid,
                    active: info.is_active,
                    usage_entry: entry.decision_value(),
                    usage_fetched_at: entry.fetched_at,
                    usage_age_s: entry.age_s,
                    last_good_usage: entry.last_good.as_ref(),
                    last_error: entry.last_error.as_deref(),
                    backoff_until: entry.backoff_until.filter(|_| entry.in_backoff(now)),
                    alias: &info.alias,
                    group: groups::group_of(&seq, &n),
                    disabled: Self::disabled_from_data(&seq, &n),
                    policy: Self::policy_from_data(&seq, &n),
                    login_expires_at: login.as_deref(),
                    subscription_plan: plan.as_ref(),
                })
            })
            .collect();
        let mut payload = Map::new();
        payload.insert("schemaVersion".into(), json!(SCHEMA_VERSION));
        payload.insert("activeAccountNumber".into(), json!(active));
        payload.insert("accounts".into(), Value::Array(rows));
        let dups = self.duplicate_account_warnings(infos);
        if !dups.is_empty() {
            payload.insert("duplicateAccountWarnings".into(), json!(dups));
        }
        let lockstep = self.lockstep_usage_warnings(infos, entries);
        if !lockstep.is_empty() {
            payload.insert("lockstepUsageWarnings".into(), json!(lockstep));
        }
        let mut unclaimed: Vec<String> = self
            .store
            .list_unclaimed_credentials()
            .keys()
            .cloned()
            .collect();
        if !unclaimed.is_empty() {
            unclaimed.sort();
            payload.insert("unclaimedCredentials".into(), json!(unclaimed));
        }
        Value::Object(payload)
    }

    /// `list`: the JSON payload with `json_output`, else the human view (printed; returns `None`).
    pub fn list_accounts(
        &self,
        show_token_status: bool,
        json_output: bool,
        fetch: Option<&HashSet<String>>,
    ) -> Option<Value> {
        if !self.sequence_file.exists() {
            if json_output {
                return Some(
                    json!({"schemaVersion": SCHEMA_VERSION, "activeAccountNumber": null, "accounts": []}),
                );
            }
            println!("{}", dimmed("No accounts are managed yet."));
            self.first_run_setup();
            return None;
        }
        let infos = self.build_accounts_info();
        let entries = self.collect_usage_entries(&infos, fetch, false);
        let plans = self.plans_for_pass(&infos, fetch);
        if json_output {
            return Some(self.build_list_payload(&infos, &entries, &plans));
        }
        self.print_accounts(&infos, &entries, &plans, show_token_status);
        None
    }

    fn print_accounts(
        &self,
        infos: &[AccountInfo],
        entries: &BTreeMap<String, UsageEntry>,
        plans: &BTreeMap<String, Map<String, Value>>,
        show_token_status: bool,
    ) {
        let seq = self.get_sequence_data().ok().flatten().unwrap_or_default();
        let tag_mode = settings::load_ui_settings(&self.backup_dir).org_tag;
        let emails: Vec<&str> = infos.iter().map(|i| i.email.as_str()).collect();
        let default_entry = UsageEntry::default();
        println!("{}", bolded("Accounts:"));
        for (i, info) in infos.iter().enumerate() {
            let n = info.num();
            let tag = Self::tag_suffix(
                &tag_mode,
                &emails,
                &info.email,
                &info.org_name,
                &info.org_uuid,
            );
            let label = if info.alias.is_empty() {
                info.email.clone()
            } else {
                format!("{} ({})", accent(&info.alias), info.email)
            };
            let mut markers = String::new();
            let group = groups::group_of(&seq, &n);
            if !group.is_empty() {
                markers.push_str(&format!(" {}", muted(&format!("[{group}]"))));
            }
            if info.is_active {
                markers.push_str(&format!(" {}", bold_accent("(active)")));
            }
            if Self::disabled_from_data(&seq, &n) {
                markers.push_str(&format!(" {}", muted("(disabled)")));
            }
            markers.push_str(&plan_marker(plans.get(&n), self.usage_store.now()));
            println!("  {n}: {label}{tag}{markers}");
            let login = oauth::login_expires_at_epoch(&info.credentials);
            for line in usage_entry_lines(entries.get(&n).unwrap_or(&default_entry), login) {
                println!("     {line}");
            }
            if show_token_status {
                for line in self.token_status_lines(info) {
                    println!("     {} {}", dimmed("•"), muted(&line));
                }
            }
            if i + 1 < infos.len() {
                println!();
            }
        }
        let dups = self.duplicate_account_warnings(infos);
        let lockstep = self.lockstep_usage_warnings(infos, entries);
        if !dups.is_empty() || !lockstep.is_empty() {
            println!();
            for msg in dups.iter().chain(&lockstep) {
                warning(msg);
            }
        }
        self.print_running_instances();
    }

    fn print_running_instances(&self) {
        let (sessions, ides) =
            process::get_running_instances(&paths::claude_config_home_in(&self.env));
        if sessions.is_empty() && ides.is_empty() {
            return;
        }
        let mut order: Vec<(String, String)> = Vec::new();
        let mut counts: HashMap<(String, String), (usize, usize)> = HashMap::new();
        let mut bump = |key: (String, String), session: bool| {
            if !counts.contains_key(&key) {
                order.push(key.clone());
            }
            let c = counts.entry(key).or_default();
            if session { c.0 += 1 } else { c.1 += 1 }
        };
        for s in &sessions {
            bump(
                (
                    entrypoint_label(&s.entrypoint).to_owned(),
                    abbreviate_path(&s.cwd),
                ),
                true,
            );
        }
        for ide in &ides {
            for folder in &ide.workspace_folders {
                bump(
                    (
                        ide_short_name(&ide.ide_name).to_owned(),
                        abbreviate_path(folder),
                    ),
                    false,
                );
            }
        }
        println!();
        println!("{}", bolded("Running instances:"));
        for key in order {
            let (s, ide) = counts[&key];
            let mut parts = Vec::new();
            if s > 0 {
                parts.push(format!("{s} session{}", if s > 1 { "s" } else { "" }));
            }
            if ide > 0 {
                parts.push("IDE".to_owned());
            }
            println!(
                "  {} {}   {}  {}",
                dimmed("●"),
                muted(&key.0),
                muted(&key.1),
                dimmed(&format!("({})", parts.join(", ")))
            );
        }
    }

    /// First run with nothing managed: offer to add the current login.
    fn first_run_setup(&self) {
        let Some((email, _)) = self.get_current_account() else {
            println!(
                "{}",
                dimmed("No active Claude account found. Please log in first.")
            );
            return;
        };
        let question = format!(
            "No managed accounts found. Add current account ({email}) to managed list? [Y/n] "
        );
        // No answer (end of input, no terminal) cancels: never add an account nobody confirmed.
        let Some(answer) = super::add::prompt(&question) else {
            println!();
            return;
        };
        if answer.to_lowercase() == "n" {
            println!(
                "{}",
                dimmed(&format!(
                    "Setup cancelled. You can run '{PROG} --add-account' later."
                ))
            );
            return;
        }
        if let Err(err) = self.add_account(None, None, false) {
            crate::printer::error(&format!("Error: {err}"));
        }
    }

    /// The active account's usage entry and login expiry (one collect pass over that slot only).
    fn active_account_usage(
        &self,
        num: &str,
        email: &str,
        org_uuid: &str,
    ) -> (UsageEntry, Option<f64>) {
        let active = self.read_active_credentials();
        let creds = active.value.clone().unwrap_or_default();
        self.record_active_verdict(Some(active));
        let info = AccountInfo {
            number: num.parse().unwrap_or_default(),
            email: email.to_owned(),
            org_uuid: org_uuid.to_owned(),
            is_active: true,
            credentials: creds.clone(),
            ..Default::default()
        };
        let entry = self
            .collect_usage_entries(std::slice::from_ref(&info), None, false)
            .remove(num)
            .unwrap_or_default();
        (entry, oauth::login_expires_at_epoch(&creds))
    }

    fn status_payload(&self) -> Value {
        let Some((email, org)) = self.get_current_account() else {
            return json!({"schemaVersion": SCHEMA_VERSION, "active": null});
        };
        let unmanaged = || json!({"schemaVersion": SCHEMA_VERSION, "active": {"email": email, "managed": false}});
        let Some(data) = self
            .get_sequence_data_migrated()
            .ok()
            .flatten()
            .filter(|d| !d.is_empty())
        else {
            return unmanaged();
        };
        let Some(num) = Self::find_account_slot(&data, &email, &org) else {
            return unmanaged();
        };
        let acct = data
            .get("accounts")
            .and_then(|a| a.get(&num))
            .cloned()
            .unwrap_or(Value::Null);
        let field = |k: &str| {
            acct.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let org_uuid = field("organizationUuid");
        let (entry, _) = self.active_account_usage(&num, &email, &org_uuid);
        let (status, usage) = usage_fields(entry.decision_value(), entry.fetched_at);
        let mut active = Map::new();
        active.insert(
            "number".into(),
            json!(num.parse::<i64>().unwrap_or_default()),
        );
        active.insert("email".into(), json!(email));
        active.insert("organizationName".into(), json!(field("organizationName")));
        active.insert("organizationUuid".into(), json!(org_uuid));
        active.insert("isOrganization".into(), json!(!org_uuid.is_empty()));
        active.insert("managed".into(), json!(true));
        active.insert("usageStatus".into(), json!(status));
        let has_usage = !usage.is_null();
        active.insert("usage".into(), usage);
        let alias = field("alias");
        if !alias.is_empty() {
            active.insert("alias".into(), json!(alias));
        }
        if has_usage {
            active.extend(usage_freshness_fields(entry.fetched_at, entry.age_s));
        } else {
            active.extend(last_good_usage_fields(
                entry.last_good.as_ref(),
                entry.fetched_at,
                entry.age_s,
            ));
            let now = self.usage_store.now();
            active.extend(usage_failure_fields(
                status,
                entry.last_error.as_deref(),
                entry.backoff_until.filter(|_| entry.in_backoff(now)),
            ));
        }
        let total = data
            .get("accounts")
            .and_then(Value::as_object)
            .map_or(0, Map::len);
        json!({"schemaVersion": SCHEMA_VERSION, "active": active, "totalManagedAccounts": total})
    }

    /// `status`: the JSON payload with `json_output`, else the human view (printed; returns `None`).
    pub fn status(&self, json_output: bool) -> Option<Value> {
        if json_output {
            return Some(self.status_payload());
        }
        let Some((email, org)) = self.get_current_account() else {
            println!(
                "{} {}",
                bolded("Status:"),
                dimmed("No active Claude account")
            );
            return None;
        };
        let not_managed = || println!("{} {email} {}", bolded("Status:"), dimmed("(not managed)"));
        let Some(data) = self
            .get_sequence_data_migrated()
            .ok()
            .flatten()
            .filter(|d| !d.is_empty())
        else {
            not_managed();
            return None;
        };
        let Some(num) = Self::find_account_slot(&data, &email, &org) else {
            not_managed();
            return None;
        };
        let accounts = data
            .get("accounts")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let org_name = accounts
            .get(&num)
            .and_then(|a| a.get("organizationName"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let emails: Vec<&str> = accounts
            .values()
            .map(|a| a.get("email").and_then(Value::as_str).unwrap_or_default())
            .collect();
        let tag_mode = settings::load_ui_settings(&self.backup_dir).org_tag;
        let tag = Self::tag_suffix(&tag_mode, &emails, &email, &org_name, &org);
        println!(
            "{} {} ({email}{tag})",
            bolded("Status:"),
            accent(&format!("Account-{num}"))
        );
        println!(
            "  {}",
            dimmed(&format!("Total managed accounts: {}", accounts.len()))
        );
        let (entry, login) = self.active_account_usage(&num, &email, &org);
        for line in usage_entry_lines(&entry, login) {
            println!("  {line}");
        }
        debug!("status for Account-{num} rendered");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_groups_thousands() {
        assert_eq!(money(12.5), "12.50");
        assert_eq!(money(1234.5), "1,234.50");
        assert_eq!(money(1_234_567.0), "1,234,567.00");
    }

    #[test]
    fn rows_align_labels_and_mark_unknown_pct() {
        let usage = json!({"five_hour": {"pct": null}, "seven_day": {"pct": 61.0}});
        assert_eq!(
            format_usage_lines(&usage, None, &[]),
            ["5h:    ?", "7d:  61%"]
        );
    }

    #[test]
    fn scoped_window_at_limit_is_flagged() {
        let usage = json!({"scoped": [{"name": "Fable", "pct": 100.0}]});
        assert_eq!(format_usage_lines(&usage, None, &[]), ["Fable: 100%  (!)"]);
    }
}
