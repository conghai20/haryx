//! Engine state, persistence, quarantine, token freshening and the switch itself.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use log::{debug, warn};
use serde_json::{Map, Value, json};

use crate::error::Result;
use crate::lines::{BindingWindow, binding_window, line_room};
use crate::locking::{DEFAULT_LOCK_TIMEOUT, FileLock};
use crate::models::AccountPolicy;
use crate::oauth::{self, AccountIdentity};
use crate::settings::{AutoSwitchSettings, atomic_write_json, parse_model_names};
use crate::switcher::Switcher;
use crate::usage_store::Clock;

use super::events::{Event, EventBody, now_iso};
use super::{
    FRESHEN_BUFFER_MS, IDLE_HOLD_MAX_S, STATE_FILENAME, STATE_SCHEMA_VERSION, ThresholdMap,
    TickOutcome, account_ref, systemic_rank,
};

type JsonObject = Map<String, Value>;

/// The auto-switch engine (`AutoSwitchEngine`).
pub struct AutoSwitchEngine<'a> {
    pub(super) switcher: &'a Switcher,
    pub(super) settings: AutoSwitchSettings,
    pub(super) models: Vec<String>,
    on_event: Box<dyn FnMut(&Event) + 'a>,
    pub(super) dry_run: bool,
    state_path: PathBuf,
    pub(super) clock: Clock,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) unhealthy_ticks: i64,
    pub(super) health_account: Option<String>,
    pub(super) sleep_until_ts: Option<f64>,
    pub(super) blocked_wait_long: bool,
    pub(super) idle_hold_since: Option<f64>,
    pub(super) idle_hold_slow: bool,
    pub(super) model_check_done: bool,
}

impl<'a> AutoSwitchEngine<'a> {
    pub fn new(
        switcher: &'a Switcher,
        settings: AutoSwitchSettings,
        on_event: impl FnMut(&Event) + 'a,
        dry_run: bool,
    ) -> Self {
        let models = parse_model_names(settings.model.as_deref());
        let engine = AutoSwitchEngine {
            switcher,
            model_check_done: models.is_empty(),
            models,
            settings,
            on_event: Box::new(on_event),
            dry_run,
            state_path: switcher.backup_dir().join(STATE_FILENAME),
            clock: switcher.usage_store.clock(),
            stop: Arc::new(AtomicBool::new(false)),
            unhealthy_ticks: 0,
            health_account: None,
            sleep_until_ts: None,
            blocked_wait_long: false,
            idle_hold_since: None,
            idle_hold_slow: false,
        };
        let inputs = (
            engine.min_effective_threshold(None),
            engine.settings.weekly_threshold,
            engine.models.clone(),
        );
        switcher.set_poll_policy_inputs(inputs);
        engine
    }

    /// Replace the clock (tests).
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    pub fn settings(&self) -> &AutoSwitchSettings {
        &self.settings
    }

    /// A handle that stops [`Self::run_loop`] when set.
    pub fn stop_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    pub(super) fn now(&self) -> f64 {
        (self.clock)()
    }

    pub(super) fn emit(&mut self, event: Event) {
        (self.on_event)(&event);
    }

    pub(super) fn emit_body(&mut self, body: EventBody) {
        self.emit(Event::new(body));
    }

    // ------------------------------------------------------------------------------- state file

    fn state_lock(&self) -> Result<FileLock> {
        let dir = self
            .state_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_default();
        FileLock::lock(dir.join(".autoswitch_state.lock"), DEFAULT_LOCK_TIMEOUT)
    }

    pub(super) fn read_state(&self) -> JsonObject {
        std::fs::read_to_string(&self.state_path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|v| match v {
                Value::Object(map) => Some(map),
                _ => None,
            })
            .unwrap_or_default()
    }

    fn mutate_state(&self, mutator: impl FnOnce(&mut JsonObject)) -> Result<JsonObject> {
        let _lock = self.state_lock()?;
        let mut state = self.read_state();
        state.insert("schemaVersion".into(), json!(STATE_SCHEMA_VERSION));
        mutator(&mut state);
        atomic_write_json(&self.state_path, &Value::Object(state.clone()))?;
        Ok(state)
    }

    // ------------------------------------------------------------------------------- quarantine

    fn fingerprint(&self, number: &str, email: &str) -> Option<String> {
        let creds = self.switcher.read_account_credentials(number, email, false);
        if creds.is_empty() {
            None
        } else {
            oauth::credential_fingerprint(&creds)
        }
    }

    pub(super) fn quarantine(&mut self, number: &str, email: &str, reason: &str) -> Result<()> {
        let fingerprint = self.fingerprint(number, email);
        let entry = json!({
            "email": email,
            "reason": reason,
            "at": now_iso(),
            "refreshTokenFingerprint": fingerprint,
        });
        self.mutate_state(|state| {
            let q = state.entry("quarantine").or_insert_with(|| json!({}));
            if !q.is_object() {
                *q = json!({});
            }
            if let Some(q) = q.as_object_mut() {
                q.insert(number.to_owned(), entry);
            }
        })?;
        self.emit_body(EventBody::Quarantined {
            number: number.into(),
            email: email.into(),
            reason: reason.into(),
        });
        Ok(())
    }

    /// Drop quarantine entries whose slot was replaced or re-logged-in.
    pub(super) fn release_recovered_quarantines(
        &mut self,
        state: JsonObject,
    ) -> Result<JsonObject> {
        let Some(Value::Object(quarantine)) = state.get("quarantine") else {
            return Ok(state);
        };
        let mut to_release: Vec<(String, String, &str)> = Vec::new();
        for (number, entry) in quarantine {
            let stored_email = entry.get("email").and_then(Value::as_str).unwrap_or("");
            let email_now = self.switcher.account_email(number).unwrap_or_default();
            if email_now.is_empty() || email_now != stored_email {
                to_release.push((number.clone(), stored_email.to_owned(), "account-replaced"));
                continue;
            }
            let stored_fp = entry.get("refreshTokenFingerprint").and_then(Value::as_str);
            if self.fingerprint(number, &email_now).as_deref() != stored_fp {
                to_release.push((number.clone(), email_now, "credentials-replaced"));
            }
        }
        if to_release.is_empty() {
            return Ok(state);
        }
        let numbers: Vec<String> = to_release.iter().map(|(n, _, _)| n.clone()).collect();
        let state = self.mutate_state(|s| {
            if let Some(Value::Object(q)) = s.get_mut("quarantine") {
                for n in &numbers {
                    q.remove(n);
                }
            }
        })?;
        for (number, email, reason) in to_release {
            self.emit_body(EventBody::Unquarantined {
                number,
                email,
                reason: reason.into(),
            });
        }
        Ok(state)
    }

    // ---------------------------------------------------------------------------------- freshen

    /// Make sure a target's token survives the switch: `ok`, `skip-live-session`,
    /// `skip-live-login`, `transient`, `invalid_grant`, `identity-conflict` or a systemic status.
    pub(super) fn freshen_target(&self, number: &str, email: &str) -> String {
        if self.switcher.account_kind_for(number).ok() == Some("api_key") {
            return "ok".into();
        }
        if !self
            .switcher
            .live_session_pids_for(number, email)
            .is_empty()
        {
            return "skip-live-session".into();
        }
        let creds = self.switcher.read_account_credentials(number, email, false);
        if creds.is_empty() {
            return "transient".into();
        }
        let Some(data) = oauth::extract_oauth_data(&creds) else {
            return "invalid_grant".into();
        };
        let near_expiry = data
            .get("expiresAt")
            .and_then(Value::as_f64)
            .is_some_and(|expires| self.now() * 1000.0 + FRESHEN_BUFFER_MS >= expires);
        if !near_expiry {
            return "ok".into();
        }
        let outcome = self.switcher.consume_backup_grant(number, email, &creds);
        match outcome.error.as_deref() {
            None if outcome.credentials.is_some() => {
                if self.note_token_identity(number, outcome.token_account.as_ref()) {
                    "identity-conflict".into()
                } else {
                    "ok".into()
                }
            }
            Some("invalid_grant" | "no_refresh_token") => "invalid_grant".into(),
            Some("live-login") => "skip-live-login".into(),
            Some(status) if systemic_rank(status).is_some() => status.into(),
            _ => "transient".into(),
        }
    }

    /// Whether the token's identity contradicts the slot (backfilling a missing uuid).
    fn note_token_identity(&self, number: &str, token_account: Option<&AccountIdentity>) -> bool {
        let Some(ta) = token_account else {
            return false;
        };
        let ta_uuid = ta.uuid.trim();
        if ta_uuid.is_empty() {
            return false;
        }
        let Ok(slot) = self.switcher.account_identity(number) else {
            return false;
        };
        if let Some(ta_org) = ta.organization_uuid.as_deref().filter(|o| !o.is_empty())
            && !slot.organization_uuid.is_empty()
            && ta_org != slot.organization_uuid
        {
            return true;
        }
        if slot.uuid.is_empty() {
            if let Err(e) = self
                .switcher
                .backfill_account_uuid(number, ta_uuid, None, None)
            {
                debug!("uuid backfill failed for account {number}: {e}");
            }
            return false;
        }
        slot.uuid != ta_uuid
    }

    // ------------------------------------------------------------------------------------- tick

    /// One evaluation; errors become an `error` event and [`TickOutcome::Error`].
    pub fn tick(&mut self) -> TickOutcome {
        match self.tick_inner() {
            Ok(outcome) => {
                let current = self.health_account.clone();
                if let Err(e) = self.persist_health_counters(current.as_deref()) {
                    warn!("could not persist auto-switch health counters: {e}");
                }
                outcome
            }
            Err(e) => {
                self.emit(Event::error(e.to_string(), true));
                TickOutcome::Error
            }
        }
    }

    pub(super) fn policies(&self) -> Result<HashMap<String, AccountPolicy>> {
        Ok(self.switcher.account_policies()?.into_iter().collect())
    }

    pub(super) fn resolve_thresholds(
        &self,
        policies: &HashMap<String, AccountPolicy>,
    ) -> ThresholdMap {
        ThresholdMap::new(
            policies
                .iter()
                .filter_map(|(num, p)| p.threshold.map(|t| (num.clone(), t)))
                .collect(),
            self.settings.threshold,
        )
    }

    pub(super) fn min_effective_threshold(&self, thresholds: Option<&ThresholdMap>) -> f64 {
        let owned;
        let thresholds = match thresholds {
            Some(t) => t,
            None => {
                let policies = self.policies().unwrap_or_default();
                owned = self.resolve_thresholds(&policies);
                &owned
            }
        };
        self.switcher
            .switchable_account_numbers()
            .unwrap_or_default()
            .iter()
            .map(|n| thresholds.get(n))
            .reduce(f64::min)
            .unwrap_or(self.settings.threshold)
    }

    pub(super) fn refresh_poll_policy_inputs(&self, thresholds: &ThresholdMap) {
        self.switcher.set_poll_policy_inputs((
            self.min_effective_threshold(Some(thresholds)),
            self.settings.weekly_threshold,
            self.models.clone(),
        ));
    }

    /// Change the global threshold live (menu bar / settings reload).
    pub fn apply_threshold(&mut self, threshold: f64) {
        self.settings.threshold = threshold;
        self.switcher.set_poll_policy_inputs((
            self.min_effective_threshold(None),
            self.settings.weekly_threshold,
            self.models.clone(),
        ));
    }

    pub(super) fn load_health_counters(&mut self, state: &JsonObject, current: &str) {
        let at = state.get("healthTickAt").and_then(number);
        let fresh = state.get("healthAccount").and_then(Value::as_str) == Some(current)
            && at.is_some_and(|at| {
                let age = self.now() - at;
                (0.0..=2.0 * IDLE_HOLD_MAX_S).contains(&age)
            });
        self.unhealthy_ticks = state
            .get("unhealthyTicks")
            .and_then(Value::as_i64)
            .filter(|t| fresh && *t > 0)
            .unwrap_or(0);
        self.idle_hold_since = state
            .get("idleHoldSince")
            .and_then(number)
            .filter(|_| fresh);
    }

    fn persist_health_counters(&self, current: Option<&str>) -> Result<()> {
        if self.dry_run {
            return Ok(());
        }
        let (ticks, since) = (self.unhealthy_ticks, self.idle_hold_since);
        let unhealthy = ticks != 0 || since.is_some();
        if !unhealthy && !self.read_state().contains_key("healthAccount") {
            return Ok(());
        }
        let now = self.now();
        self.mutate_state(|state| match current {
            Some(current) if unhealthy => {
                state.insert("unhealthyTicks".into(), json!(ticks));
                state.insert("idleHoldSince".into(), json!(since));
                state.insert("healthAccount".into(), json!(current));
                state.insert("healthTickAt".into(), json!(now));
            }
            _ => {
                for key in [
                    "unhealthyTicks",
                    "idleHoldSince",
                    "healthAccount",
                    "healthTickAt",
                ] {
                    state.remove(key);
                }
            }
        })?;
        Ok(())
    }

    // ---------------------------------------------------------------------------- window math

    pub(super) fn binding(&self, value: Option<&Value>, line: f64) -> Option<BindingWindow> {
        binding_window(
            &oauth::relevant_windows(value, &self.models),
            line,
            self.settings.weekly_threshold,
            Some(self.settings.threshold),
        )
    }

    pub(super) fn room(&self, value: Option<&Value>, line: f64) -> Option<f64> {
        line_room(
            &oauth::relevant_windows(value, &self.models),
            line,
            self.settings.weekly_threshold,
        )
    }

    pub(super) fn in_cooldown(&self, state: &JsonObject) -> bool {
        state
            .get("lastSwitchAt")
            .and_then(number)
            .is_some_and(|last| self.now() - last < self.settings.cooldown_seconds)
    }

    // ---------------------------------------------------------------------------------- perform

    pub(super) fn perform(
        &mut self,
        number: &str,
        email: &str,
        trigger: &str,
        left: (Option<f64>, f64),
    ) -> Result<TickOutcome> {
        if self.dry_run {
            let current = self.switcher.current_account_number()?;
            let from = match current {
                Some(c) => Some(account_ref(&c, &self.switcher.account_email(&c)?)),
                None => None,
            };
            self.emit_body(EventBody::Switch {
                trigger: trigger.into(),
                from,
                to: Some(account_ref(number, email)),
                warnings: Vec::new(),
                dry_run: true,
            });
            return Ok(TickOutcome::Switched);
        }
        let result = {
            let _lock = self.state_lock()?;
            let mut state = self.read_state();
            if matches!(trigger, "proactive" | "consume-first" | "priority")
                && self.in_cooldown(&state)
            {
                drop(_lock);
                self.emit(Event::no_switch("cooldown", ""));
                return Ok(TickOutcome::NoAction);
            }
            let result = self
                .switcher
                .switch_to(number, true, false)?
                .unwrap_or(Value::Null);
            if result.get("switched").and_then(Value::as_bool) != Some(true) {
                drop(_lock);
                let detail = result.get("reason").and_then(Value::as_str).unwrap_or("");
                self.emit(Event::no_switch("already-active", detail));
                return Ok(TickOutcome::NoAction);
            }
            let (left_headroom, recovery) = left;
            state.insert("schemaVersion".into(), json!(STATE_SCHEMA_VERSION));
            state.insert("lastSwitchAt".into(), json!(self.now()));
            state.insert("lastSwitchTo".into(), json!(number));
            let from_num = result.get("from").and_then(|f| f.get("number")).cloned();
            state.insert("lastSwitchFrom".into(), from_num.unwrap_or(Value::Null));
            state.insert("leftHeadroom".into(), json!(left_headroom));
            let recovery = recovery.is_finite().then_some(recovery);
            state.insert("leftRecoveryAt".into(), json!(recovery));
            state.insert("leftTrigger".into(), json!(trigger));
            atomic_write_json(&self.state_path, &Value::Object(state))?;
            result
        };
        let warnings = result
            .get("warnings")
            .and_then(Value::as_array)
            .map(|w| {
                w.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        self.emit_body(EventBody::Switch {
            trigger: trigger.into(),
            from: result.get("from").cloned().filter(|v| !v.is_null()),
            to: result.get("to").cloned().filter(|v| !v.is_null()),
            warnings,
            dry_run: false,
        });
        Ok(TickOutcome::Switched)
    }

    /// Accounts quarantined in `state`.
    pub(super) fn quarantined(state: &JsonObject) -> HashSet<String> {
        match state.get("quarantine") {
            Some(Value::Object(q)) => q.keys().cloned().collect(),
            _ => HashSet::new(),
        }
    }
}

/// A JSON number that is not a bool.
pub(super) fn number(value: &Value) -> Option<f64> {
    value.as_f64()
}
