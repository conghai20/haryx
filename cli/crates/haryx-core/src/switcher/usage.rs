//! Usage collection: upstream `_build_accounts_info`, `_collect_usage_entries`, `_static_usage_sentinel`,
//! `_fetch_account_usage`, `_read_only_fetch`, `_run_usage_fetches`, `_slot_token_dead`,
//! `_entry_token_dead`, `_plans_after_fetch`, `_replan_new_active`, `_poll_policy_inputs`,
//! `_usage_settings`, `_probe_options`, the subscription-plan pass and `accounts_snapshot`.
//!
//! Every surface gets usage from [`Switcher::collect_usage_entries`]. A caller only nominates accounts;
//! whether a request is sent is decided by `UsageStore::reserve` under the store lock. The fetch runs
//! with no store lock held, and a failure only updates backoff fields, so the last good reading keeps
//! being served. Collection never fails outward: a failure becomes a state.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use log::{debug, warn};
use serde_json::{Map, Value, json};

use super::{LineageKey, Switcher};
use crate::credentials::{ActiveCredentials, looks_like_api_key};
use crate::json_output::{
    USAGE_API_KEY, USAGE_KEYCHAIN_UNAVAILABLE, USAGE_NO_CREDENTIALS, USAGE_RELOGIN_REQUIRED,
    USAGE_TOKEN_EXPIRED,
};
use crate::models::{AccountSnapshot, AccountsSnapshot};
use crate::oauth::{self, UsageFetch};
use crate::poll_policy::{self, PlanInput};
use crate::session;
use crate::settings::{self, UsageSettings};
use crate::subscription::{self, PlanStore};
use crate::usage_store::{FetchRecord, Identities, PollPlan, UsageEntry, with_sentinel};

/// Stagger between request starts so N accounts never hit the endpoint in the same instant.
const FETCH_STAGGER: Duration = Duration::from_millis(250);

/// One managed account as a pass sees it (upstream's 7-tuple from `_build_accounts_info`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountInfo {
    pub number: i64,
    pub email: String,
    pub org_name: String,
    pub org_uuid: String,
    pub is_active: bool,
    /// The live credential for the active slot, the backup otherwise ("" when none).
    pub credentials: String,
    pub alias: String,
}

impl AccountInfo {
    pub fn num(&self) -> String {
        self.number.to_string()
    }
}

/// Poll-planning inputs: threshold, weekly threshold, model names.
pub type PollInputs = (f64, Option<f64>, Vec<String>);

/// Mutable per-process state of the collector (upstream's instance caches).
#[derive(Default)]
pub struct CollectState {
    /// This pass's active-credential read verdict.
    pub(crate) active_verdict: Option<ActiveCredentials>,
    /// Cached ownership verdicts per credential lineage.
    pub(crate) probe_verdicts: HashMap<LineageKey, bool>,
    /// Provenance warnings already logged, so each is said once.
    pub(crate) provenance_warned: HashSet<(String, String, String)>,
    pub(crate) poll_inputs_override: Option<PollInputs>,
    poll_inputs_cache: Option<(Option<std::time::SystemTime>, PollInputs)>,
    usage_settings_cache: Option<(Option<std::time::SystemTime>, UsageSettings)>,
}

fn str_field<'a>(account: &'a Value, key: &str) -> &'a str {
    account.get(key).and_then(Value::as_str).unwrap_or_default()
}

impl Switcher {
    fn with_state<T>(&self, f: impl FnOnce(&mut CollectState) -> T) -> T {
        let mut state = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut state)
    }

    pub(crate) fn record_active_verdict(&self, verdict: Option<ActiveCredentials>) {
        self.with_state(|s| s.active_verdict = verdict);
    }

    /// This pass's active-read verdict; a clean one if it never read.
    pub(crate) fn active_verdict(&self) -> ActiveCredentials {
        self.with_state(|s| s.active_verdict.clone())
            .unwrap_or(ActiveCredentials {
                value: Some(String::new()),
                keychain_unavailable: false,
                degraded: false,
            })
    }

    pub(crate) fn probe_verdict(&self, key: &LineageKey) -> Option<bool> {
        self.with_state(|s| s.probe_verdicts.get(key).copied())
    }

    pub(crate) fn set_probe_verdict(&self, key: LineageKey, verdict: bool) {
        self.with_state(|s| s.probe_verdicts.insert(key, verdict));
    }

    /// Whether this is the first time `(num, email, kind)` is warned about (and mark it warned).
    pub(crate) fn warn_once(&self, num: &str, email: &str, kind: &str) -> bool {
        self.with_state(|s| {
            s.provenance_warned
                .insert((num.to_owned(), email.to_owned(), kind.to_owned()))
        })
    }

    pub(crate) fn clear_warned(&self, num: &str, email: &str, kind: &str) {
        self.with_state(|s| {
            s.provenance_warned
                .remove(&(num.to_owned(), email.to_owned(), kind.to_owned()))
        });
    }

    /// The lineage key of `creds` for a slot, or `None` when the slot's identity cannot be read.
    pub(crate) fn lineage_of(&self, num: &str, email: &str, creds: &str) -> Option<LineageKey> {
        let fp = oauth::credential_fingerprint(creds).unwrap_or_default();
        self.lineage_key(num, email, &fp).ok()
    }

    /// Per-account rows for a pass: the active slot reads Claude Code's live store, every other slot its
    /// backup. Records the active-read verdict for the fetches that follow.
    pub fn build_accounts_info(&self) -> Vec<AccountInfo> {
        let data = self
            .get_sequence_data_migrated()
            .ok()
            .flatten()
            .unwrap_or_default();
        let active_num = self
            .get_current_account()
            .and_then(|(email, org)| Self::find_account_slot(&data, &email, &org));
        self.record_active_verdict(None);
        let empty = Map::new();
        let accounts = data
            .get("accounts")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        let sequence = data
            .get("sequence")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        sequence
            .iter()
            .filter_map(Value::as_i64)
            .map(|number| {
                let num = number.to_string();
                let account = accounts.get(&num).cloned().unwrap_or(Value::Null);
                let email = account
                    .get("email")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                let is_active = active_num.as_deref() == Some(num.as_str());
                let credentials = if is_active {
                    let active = self.read_active_credentials();
                    let value = active.value.clone().unwrap_or_default();
                    self.record_active_verdict(Some(active));
                    value
                } else {
                    self.read_account_credentials(&num, &email, true)
                };
                AccountInfo {
                    number,
                    org_name: str_field(&account, "organizationName").to_owned(),
                    org_uuid: str_field(&account, "organizationUuid").to_owned(),
                    alias: str_field(&account, "alias").to_owned(),
                    email,
                    is_active,
                    credentials,
                }
            })
            .collect()
    }

    fn identities(infos: &[AccountInfo]) -> Identities {
        infos
            .iter()
            .map(|i| (i.num(), (i.email.clone(), i.org_uuid.clone())))
            .collect()
    }

    /// Settings-file mtime, the cache key of both settings caches.
    fn settings_mtime(&self) -> Option<std::time::SystemTime> {
        std::fs::metadata(settings::settings_path(&self.backup_dir))
            .and_then(|m| m.modified())
            .ok()
    }

    /// Poll-planning inputs: the hosting engine's pinned values when set, else the settings file
    /// (reloaded only when it changes).
    pub fn poll_policy_inputs(&self) -> PollInputs {
        if let Some(pinned) = self.with_state(|s| s.poll_inputs_override.clone()) {
            return pinned;
        }
        let mtime = self.settings_mtime();
        if let Some(hit) = self.with_state(|s| {
            s.poll_inputs_cache
                .as_ref()
                .filter(|(m, _)| *m == mtime)
                .map(|(_, v)| v.clone())
        }) {
            return hit;
        }
        let loaded = settings::load_settings(&self.backup_dir);
        let inputs = (
            loaded.threshold,
            loaded.weekly_threshold,
            settings::parse_model_names(loaded.model.as_deref()),
        );
        self.with_state(|s| s.poll_inputs_cache = Some((mtime, inputs.clone())));
        inputs
    }

    /// Pin the poll-planning inputs to an engine's effective (CLI-merged) settings.
    pub fn set_poll_policy_inputs(&self, inputs: PollInputs) {
        self.with_state(|s| s.poll_inputs_override = Some(inputs));
    }

    pub fn clear_poll_policy_inputs(&self) {
        self.with_state(|s| s.poll_inputs_override = None);
    }

    /// The share with this user's other machines, when the setting allows it. Never built under
    /// `cfg(test)` (it would reach the real Keychain session): tests inject one.
    pub(crate) fn usage_share(&self) -> Option<&crate::usage_share::UsageShare> {
        if !self.usage_settings().share_between_machines {
            return None;
        }
        self.usage_share
            .get_or_init(|| {
                if cfg!(test) {
                    None
                } else {
                    Some(crate::usage_share::UsageShare::for_store(&self.backup_dir))
                }
            })
            .as_ref()
    }

    fn usage_settings(&self) -> UsageSettings {
        let mtime = self.settings_mtime();
        if let Some(hit) = self.with_state(|s| {
            s.usage_settings_cache
                .as_ref()
                .filter(|(m, _)| *m == mtime)
                .map(|(_, v)| *v)
        }) {
            return hit;
        }
        let loaded = settings::load_usage_settings(&self.backup_dir);
        self.with_state(|s| s.usage_settings_cache = Some((mtime, loaded)));
        loaded
    }

    /// A fetch request carrying the probe options (`usage.headerProbe`, `usage.fableProbe`).
    pub(crate) fn usage_request<'a>(
        &self,
        num: &'a str,
        email: &'a str,
        creds: &'a str,
        is_active: bool,
        last_usage: Option<&'a Value>,
    ) -> UsageFetch<'a> {
        let usage = self.usage_settings();
        let mut req = UsageFetch::new(num, email, creds, is_active);
        req.header_probe = usage.header_probe;
        req.scoped_probe = usage.fable_probe;
        req.last_usage = last_usage;
        req
    }

    pub(crate) fn fetch_record(outcome: oauth::UsageOutcome) -> FetchRecord {
        FetchRecord {
            usage: outcome.usage.map(Value::Object),
            error: outcome.error,
            retry_after_s: outcome.retry_after_s,
            struck_fp: outcome.struck_fp,
            ..Default::default()
        }
    }

    /// A sentinel derivable without any network call (re-derived every pass, never persisted).
    fn static_usage_sentinel(&self, info: &AccountInfo) -> Option<&'static str> {
        let creds = info.credentials.as_str();
        if looks_like_api_key(Some(creds)) {
            return Some(USAGE_API_KEY);
        }
        if !creds.is_empty() && oauth::extract_access_token(creds).is_some() {
            return None;
        }
        if info.is_active {
            let verdict = self.active_verdict();
            if verdict.keychain_unavailable || verdict.value.is_none() {
                return Some(USAGE_KEYCHAIN_UNAVAILABLE);
            }
        } else if self
            .read_account_credentials_ex(&info.num(), &info.email, true)
            .1
        {
            return Some(USAGE_KEYCHAIN_UNAVAILABLE);
        }
        Some(USAGE_NO_CREDENTIALS)
    }

    /// One network fetch for one account. Never fails outward.
    fn fetch_account_usage(
        &self,
        info: &AccountInfo,
        rejected_fp: Option<&str>,
        last_usage: Option<&Value>,
    ) -> FetchRecord {
        let num = info.num();
        let email = info.email.as_str();
        if info.is_active {
            return self.fetch_active_usage(
                &num,
                email,
                &info.credentials,
                &info.org_uuid,
                last_usage,
            );
        }
        let mut creds = info.credentials.clone();
        let mut has_live_session = !self.live_session_pids(&num, email).is_empty();
        let session_dir = self.session_dir(&num, email);
        let mut session_creds =
            session::read_session_credentials(&self.profile_keychain(), &session_dir)
                .filter(|c| !c.is_empty());
        if session_creds.is_some()
            && session::session_identity_drifted(&session_dir, email, &info.org_uuid)
        {
            debug!(
                "Session profile for account {num} is logged in as a different account; fetching usage \
                 from the backup credential"
            );
            session_creds = None;
            has_live_session = false;
        }
        if let Some(profile) = session_creds.clone().filter(|_| !has_live_session) {
            let adopted = match self.adopt_session_credential(&num, email, &info.org_uuid) {
                Ok(adopted) => adopted,
                Err(crate::Error::Lock(_)) => None,
                Err(err) => {
                    warn!(
                        "Could not adopt account {num}'s session profile credential into its backup: {err}"
                    );
                    None
                }
            };
            if adopted == Some(true) {
                creds = profile;
            }
            if adopted.is_some() {
                session_creds = None;
            }
        }
        if let Some(profile) = session_creds
            && let Some(o) = oauth::extract_oauth_data(&profile)
            && o.get("accessToken")
                .and_then(Value::as_str)
                .is_some_and(|t| !t.is_empty())
        {
            if oauth::is_oauth_token_expired(o.get("expiresAt")) {
                return FetchRecord::sentinel(USAGE_TOKEN_EXPIRED);
            }
            return self.read_only_fetch(&num, email, &profile, rejected_fp, last_usage);
        }
        if has_live_session || self.is_default_login_from_another_profile(email, &info.org_uuid) {
            let expires =
                oauth::extract_oauth_data(&creds).and_then(|o| o.get("expiresAt").cloned());
            if oauth::is_oauth_token_expired(expires.as_ref()) {
                return FetchRecord::sentinel(USAGE_TOKEN_EXPIRED);
            }
            return self.read_only_fetch(&num, email, &creds, rejected_fp, last_usage);
        }
        let refresh = |n: &str, e: &str, snap: &str| self.consume_backup_grant(n, e, snap);
        let mut req = self.usage_request(&num, email, &creds, false, last_usage);
        req.refresh_via = Some(&refresh);
        Self::fetch_record(self.api.try_fetch_usage_for_account(&req))
    }

    /// A fetch with a live session's credential, which haryx must never refresh: a 401 means the live
    /// claude rotated past our copy, so the slot reads as expired and the refused token is stamped.
    fn read_only_fetch(
        &self,
        num: &str,
        email: &str,
        creds: &str,
        rejected_fp: Option<&str>,
        last_usage: Option<&Value>,
    ) -> FetchRecord {
        let stamp = oauth::access_token_fingerprint(creds);
        if stamp.is_some() && stamp.as_deref() == rejected_fp {
            return FetchRecord::sentinel(USAGE_TOKEN_EXPIRED);
        }
        let req = self.usage_request(num, email, creds, true, last_usage);
        let outcome = self.api.try_fetch_usage_for_account(&req);
        if outcome.error.as_deref() == Some("http-401") {
            return FetchRecord::sentinel(USAGE_TOKEN_EXPIRED).with_rejected_fp(stamp);
        }
        FetchRecord {
            struck_fp: None,
            ..Self::fetch_record(outcome)
        }
    }

    /// Fetch the given accounts in parallel, staggering request starts.
    fn run_usage_fetches(
        &self,
        infos: &[&AccountInfo],
        entries: &BTreeMap<String, UsageEntry>,
    ) -> BTreeMap<String, FetchRecord> {
        std::thread::scope(|scope| {
            let handles: Vec<_> = infos
                .iter()
                .enumerate()
                .map(|(idx, info)| {
                    scope.spawn(move || {
                        if idx > 0 {
                            std::thread::sleep(FETCH_STAGGER * idx as u32);
                        }
                        let num = info.num();
                        let entry = entries.get(&num);
                        let record = self.fetch_account_usage(
                            info,
                            entry.and_then(|e| e.rejected_fingerprint.as_deref()),
                            entry.and_then(|e| e.last_good.as_ref()),
                        );
                        (num, record)
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        })
    }

    /// Store-backed usage collection: one entry per account. `fetch = None` (on-demand callers) makes
    /// every account a candidate but respects stored poll plans; the auto engine passes an explicit set.
    pub fn collect_usage_entries(
        &self,
        infos: &[AccountInfo],
        fetch: Option<&HashSet<String>>,
        scheduled: bool,
    ) -> BTreeMap<String, UsageEntry> {
        let store = &self.usage_store;
        let identities = Self::identities(infos);
        let by_num: BTreeMap<String, &AccountInfo> = infos.iter().map(|i| (i.num(), i)).collect();
        let (_, _, models) = self.poll_policy_inputs();
        let mut sentinels: BTreeMap<String, String> = by_num
            .iter()
            .filter_map(|(num, info)| {
                self.static_usage_sentinel(info)
                    .map(|s| (num.clone(), s.to_owned()))
            })
            .collect();

        let mut entries = store.entries(&identities, &models);
        for (num, info) in &by_num {
            if sentinels.contains_key(num) {
                continue;
            }
            let entry = entries.get(num).cloned().unwrap_or_default();
            if self.entry_token_dead(&entry, num, &info.email, &info.credentials) {
                sentinels.insert(num.clone(), USAGE_RELOGIN_REQUIRED.to_owned());
            } else if entry.auth_dead_strikes > 0 && entry.token_dead() {
                let one: Identities = [(num.clone(), identities[num].clone())].into();
                if let Err(err) = store.clear_dead_token(std::slice::from_ref(num), &one) {
                    debug!("clear_dead_token failed for {num}: {err}");
                }
                entries = store.entries(&identities, &models);
            }
        }
        let requested: Vec<String> = by_num
            .keys()
            .filter(|n| !sentinels.contains_key(*n) && fetch.is_none_or(|f| f.contains(*n)))
            .cloned()
            .collect();
        // On-demand callers respect stored plans; the auto engine passes its own set.
        let respect_plans = fetch.is_none();
        let repair_overslept = fetch.is_none() || scheduled;
        let active: BTreeSet<String> = by_num
            .iter()
            .filter(|(_, info)| info.is_active)
            .map(|(num, _)| num.clone())
            .collect();
        // Ask the other machines only when something here would be fetched.
        let mut pass = self.usage_share().and_then(|share| {
            if !store.any_eligible(&requested, &identities, respect_plans, repair_overslept) {
                return None;
            }
            let pass = share.pull(store, &identities);
            entries = store.entries(&identities, &models);
            pass
        });
        let mut claims = store
            .reserve(&requested, &identities, respect_plans, repair_overslept)
            .unwrap_or_else(|err| {
                debug!("usage reserve failed: {err}");
                BTreeMap::new()
            });
        if let Some(pass) = pass.as_mut() {
            let lost = pass.claim(store, &claims, &identities, &active);
            if !lost.is_empty() {
                if let Err(err) = store.release(&lost, &identities) {
                    debug!("usage release failed: {err}");
                }
                claims.retain(|num, _| !lost.contains_key(num));
                entries = store.entries(&identities, &models);
            }
        }
        let coordinated = pass.as_ref().is_some_and(|p| !p.degraded());
        let now = store.now();
        for (num, info) in &by_num {
            if sentinels.contains_key(num) || !info.is_active || claims.contains_key(num) {
                continue;
            }
            if entries.get(num).is_some_and(|e| e.held(now)) {
                continue;
            }
            let expired = oauth::extract_oauth_data(&info.credentials)
                .is_some_and(|o| oauth::is_oauth_token_expired(o.get("expiresAt")));
            if expired {
                sentinels.insert(num.clone(), USAGE_TOKEN_EXPIRED.to_owned());
            }
        }

        let mut shared = BTreeSet::new();
        if !claims.is_empty() {
            let pre = entries.clone();
            let targets: Vec<&AccountInfo> = claims
                .keys()
                .filter_map(|n| by_num.get(n).copied())
                .collect();
            let records = self.run_usage_fetches(&targets, &pre);
            let plans = self.plans_after_fetch(&records, &pre, &by_num, coordinated);
            let accepted = store
                .record(&records, &identities, Some(&claims), Some(&plans))
                .unwrap_or_else(|err| {
                    debug!("usage record failed: {err}");
                    Default::default()
                });
            shared.clone_from(&accepted);
            for num in &accepted {
                if let Some(sentinel) = records.get(num).and_then(|r| r.sentinel.clone()) {
                    sentinels.insert(num.clone(), sentinel);
                }
            }
            entries = store.entries(&identities, &models);
            for num in &accepted {
                let info = by_num[num];
                let entry = entries.get(num).cloned().unwrap_or_default();
                if self.entry_token_dead(&entry, num, &info.email, &info.credentials) {
                    sentinels.insert(num.clone(), USAGE_RELOGIN_REQUIRED.to_owned());
                }
            }
        }
        if let Some(pass) = pass.as_mut() {
            pass.push(store, &shared, &identities, &active);
            entries = store.entries(&identities, &models);
        }
        by_num
            .keys()
            .map(|num| {
                let entry = entries.remove(num).unwrap_or_default();
                (
                    num.clone(),
                    with_sentinel(entry, sentinels.get(num).map(String::as_str)),
                )
            })
            .collect()
    }

    /// Whether a slot is quarantined as refresh-token-dead right now (what `import`'s auto-heal asks).
    pub fn slot_token_dead(&self, num: &str, email: &str) -> bool {
        let org = self
            .get_sequence_data()
            .ok()
            .flatten()
            .and_then(|d| {
                d.get("accounts")?
                    .get(num)?
                    .get("organizationUuid")?
                    .as_str()
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let ident: Identities = [(num.to_owned(), (email.to_owned(), org))].into();
        let Some(entry) = self.usage_store.entries(&ident, &[]).remove(num) else {
            return false;
        };
        let is_active = self.current_account_number().ok().flatten().as_deref() == Some(num);
        let (backup, unreadable) = self.read_account_credentials_ex(num, email, false);
        if unreadable {
            return false;
        }
        let stored = if is_active {
            match self.store.read_active_credentials().value {
                Some(value) => value,
                None => return false,
            }
        } else {
            backup
        };
        self.entry_token_dead(&entry, num, email, &stored)
    }

    /// Dead when the strike binds to the stored credential, or to the backup when the stored one moved.
    fn entry_token_dead(&self, entry: &UsageEntry, num: &str, email: &str, stored: &str) -> bool {
        if !entry.token_dead() {
            return false;
        }
        let threshold = crate::usage_store::AUTH_DEAD_STRIKES;
        if entry.token_dead_with(threshold, oauth::credential_fingerprint(stored).as_deref()) {
            return true;
        }
        let (backup, unreadable) = self.read_account_credentials_ex(num, email, false);
        if unreadable {
            return true;
        }
        !backup.is_empty()
            && entry.token_dead_with(threshold, oauth::credential_fingerprint(&backup).as_deref())
    }

    fn plans_after_fetch(
        &self,
        records: &BTreeMap<String, FetchRecord>,
        pre: &BTreeMap<String, UsageEntry>,
        by_num: &BTreeMap<String, &AccountInfo>,
        coordinated: bool,
    ) -> BTreeMap<String, PollPlan> {
        let now = self.usage_store.now();
        let (threshold, weekly_threshold, models) = self.poll_policy_inputs();
        records
            .iter()
            .filter(|(_, rec)| rec.sentinel.is_none() && rec.error.is_none())
            .map(|(num, rec)| {
                let before = pre.get(num);
                let input = PlanInput {
                    prev_interval_s: before.and_then(|b| b.poll_interval_s),
                    prev_usage: before.and_then(|b| b.last_good.as_ref()),
                    new_usage: rec.usage.as_ref(),
                    // Active on any machine sharing usage counts: the schedule is shared.
                    is_active: by_num.get(num).is_some_and(|i| i.is_active)
                        || before.is_some_and(|b| b.active_elsewhere(now)),
                    threshold,
                    weekly_threshold,
                    models: &models,
                    recent_429: before.is_some_and(|b| b.recent_429(now)),
                    now,
                };
                let (next, interval) = poll_policy::plan_after_fetch_random(&input);
                // Not coordinated with the machines this account's ledger saw: poll that much
                // less often, so together they stay inside the one budget.
                let peers = if coordinated {
                    0
                } else {
                    before.map_or(0, |b| b.peers_seen)
                };
                let (next, interval) = poll_policy::spread_for_peers(next, interval, now, peers);
                (num.clone(), (Some(next), Some(interval)))
            })
            .collect()
    }

    /// After a switch, pull the new active's next poll forward to the minimum interval. Never fatal.
    pub fn replan_new_active(&self, number: &str, email: &str, org_uuid: &str) {
        let identities: Identities =
            [(number.to_owned(), (email.to_owned(), org_uuid.to_owned()))].into();
        let now = self.usage_store.now();
        let Some(entry) = self.usage_store.entries(&identities, &[]).remove(number) else {
            return;
        };
        let Some(fetched_at) = entry.fetched_at else {
            return;
        };
        let next_poll = now.max(fetched_at + poll_policy::MIN_INTERVAL_S);
        if entry.next_poll_at.is_some_and(|n| n <= next_poll) {
            return;
        }
        let plan: BTreeMap<String, PollPlan> = [(
            number.to_owned(),
            (Some(next_poll), Some(poll_policy::MIN_INTERVAL_S)),
        )]
        .into();
        if let Err(err) = self.usage_store.set_poll_plan(&plan, &identities) {
            warn!("Post-switch poll re-plan failed (switch itself succeeded): {err}");
        }
    }

    /// Usage entries for every account (on-demand collection).
    pub fn usage_entries_by_account(
        &self,
        fetch: Option<&HashSet<String>>,
        scheduled: bool,
    ) -> BTreeMap<String, UsageEntry> {
        let infos = self.build_accounts_info();
        self.collect_usage_entries(&infos, fetch, scheduled)
    }

    /// Each account's last fetch time (store read only).
    pub fn usage_fetch_stamps(&self) -> BTreeMap<String, Option<f64>> {
        let data = self.get_sequence_data().ok().flatten().unwrap_or_default();
        let identities: Identities = data
            .get("accounts")
            .and_then(Value::as_object)
            .map(|accounts| {
                accounts
                    .iter()
                    .map(|(num, info)| {
                        (
                            num.clone(),
                            (
                                str_field(info, "email").to_owned(),
                                str_field(info, "organizationUuid").to_owned(),
                            ),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.usage_store
            .entries(&identities, &[])
            .into_iter()
            .map(|(num, entry)| (num, entry.fetched_at))
            .collect()
    }

    pub(crate) fn subscription_store(&self) -> PlanStore {
        PlanStore::with_clock(
            &self
                .backup_dir
                .join("cache")
                .join(subscription::PLAN_FILENAME),
            self.usage_store.clock(),
        )
    }

    /// Per-slot plan records as stored (no credential or network access).
    pub fn stored_subscription_plans(
        &self,
        infos: &[AccountInfo],
    ) -> BTreeMap<String, Map<String, Value>> {
        let records = self.subscription_store().load();
        infos
            .iter()
            .filter_map(|info| {
                let record = records.get(&subscription::identity_key(
                    &info.email,
                    Some(&info.org_uuid),
                ))?;
                subscription::valid_record(record)
                    .then(|| record.as_object().cloned())
                    .flatten()
                    .map(|r| (info.num(), r))
            })
            .collect()
    }

    /// An empty `fetch` set means no network eligibility, for plans too.
    pub(crate) fn plans_for_pass(
        &self,
        infos: &[AccountInfo],
        fetch: Option<&HashSet<String>>,
    ) -> BTreeMap<String, Map<String, Value>> {
        if fetch.is_some_and(HashSet::is_empty) {
            return self.stored_subscription_plans(infos);
        }
        self.collect_subscription_plans(infos)
    }

    /// Refresh due plans for surfaces whose snapshots are store-only.
    pub fn refresh_subscription_plans(&self) {
        let infos = self.build_accounts_info();
        self.collect_subscription_plans(&infos);
    }

    /// Per-slot subscription plan, refreshed on its own six-hour clock. Only an already-valid access
    /// token is used; network happens with no lock held.
    fn collect_subscription_plans(
        &self,
        infos: &[AccountInfo],
    ) -> BTreeMap<String, Map<String, Value>> {
        let store = self.subscription_store();
        let records = store.load();
        let data = self.get_sequence_data().ok().flatten().unwrap_or_default();
        let mut updates = Map::new();
        for info in infos {
            let key = subscription::identity_key(&info.email, Some(&info.org_uuid));
            if !subscription::is_due(records.get(&key), store.now()) {
                continue;
            }
            let mut token = subscription::usable_access_token(Some(&info.credentials));
            if token.is_none() && !info.is_active {
                let backup = self.read_account_credentials(&info.num(), &info.email, true);
                token = subscription::usable_access_token(Some(&backup));
            }
            let Some(token) = token else {
                continue;
            };
            let started_at = store.now();
            let fetched = subscription::fetch_plan(&token, subscription::FETCH_TIMEOUT);
            let slot_uuid = data
                .get("accounts")
                .and_then(|a| a.get(info.num()))
                .map(|a| str_field(a, "uuid").trim().to_owned())
                .unwrap_or_default();
            let record = match &fetched.plan {
                Some(plan) if subscription::matches_slot(&fetched, &slot_uuid, &info.org_uuid) => {
                    json!({"plan": plan, "fetchedAt": started_at})
                }
                _ => json!({
                    "retryAt": subscription::next_retry_at(started_at, fetched.retry_after_s),
                    "attemptedAt": started_at,
                }),
            };
            updates.insert(key, record);
        }
        if !updates.is_empty()
            && let Err(err) = store.save(&updates)
        {
            debug!("Could not save subscription plans: {err}");
        }
        self.stored_subscription_plans(infos)
    }

    /// One coherent view for dashboards: metadata, active slot, usage and plans from a single pass.
    pub fn accounts_snapshot(&self, fetch: Option<&HashSet<String>>) -> AccountsSnapshot {
        let infos = self.build_accounts_info();
        let mut entries = self.collect_usage_entries(&infos, fetch, false);
        let mut plans = self.plans_for_pass(&infos, fetch);
        let seq = self.get_sequence_data().ok().flatten().unwrap_or_default();
        let accounts: Vec<AccountSnapshot> = infos
            .iter()
            .map(|info| {
                let n = info.num();
                AccountSnapshot {
                    kind: self.account_kind(Some(&n)).unwrap_or("oauth").to_owned(),
                    switchable: self.account_is_switchable(&n).unwrap_or(false),
                    usage: entries.remove(&n).unwrap_or_default(),
                    disabled: Self::disabled_from_data(&seq, &n),
                    login_expires_at: oauth::login_expires_at_epoch(&info.credentials),
                    policy: Self::policy_from_data(&seq, &n),
                    subscription_plan: plans.remove(&n),
                    number: n,
                    email: info.email.clone(),
                    org_name: info.org_name.clone(),
                    org_uuid: info.org_uuid.clone(),
                    is_active: info.is_active,
                    alias: info.alias.clone(),
                }
            })
            .collect();
        AccountsSnapshot {
            active_number: infos.iter().find(|i| i.is_active).map(AccountInfo::num),
            accounts,
            taken_at: self.usage_store.now(),
        }
    }
}
