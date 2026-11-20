//! Ported from `tests/test_usage_store.py`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::policy::failure_backoff_s;
use super::*;
use crate::oauth::{USAGE_SOURCE_HEADERS, iso_utc};
use crate::poll_policy::{
    self, POST_429_MAX_INTERVAL_S, POST_429_MIN_INTERVAL_S, PlanInput, RECENT_429_WINDOW_S,
};

#[derive(Clone)]
struct FakeClock(Arc<Mutex<f64>>);

impl FakeClock {
    fn new() -> Self {
        FakeClock(Arc::new(Mutex::new(1_000_000.0)))
    }
    fn now(&self) -> f64 {
        *self.0.lock().unwrap()
    }
    fn advance(&self, s: f64) {
        *self.0.lock().unwrap() += s;
    }
    fn set(&self, t: f64) {
        *self.0.lock().unwrap() = t;
    }
}

struct Fixture {
    _tmp: TempDir,
    clock: FakeClock,
    store: UsageStore,
}

fn fixture() -> Fixture {
    let tmp = TempDir::new().unwrap();
    let clock = FakeClock::new();
    let c = clock.clone();
    let store = UsageStore::with_clock(&tmp.path().join("cache"), Arc::new(move || c.now()));
    Fixture {
        _tmp: tmp,
        clock,
        store,
    }
}

fn ident() -> Identities {
    BTreeMap::from([
        ("1".into(), ("a@x.com".into(), String::new())),
        ("2".into(), ("b@x.com".into(), "org-2".into())),
    ])
}

fn one(num: &str, email: &str, org: &str) -> Identities {
    BTreeMap::from([(num.into(), (email.into(), org.into()))])
}

fn usage() -> Value {
    json!({"five_hour": {"pct": 25.0}, "seven_day": {"pct": 10.0}})
}

fn s(v: &str) -> Vec<String> {
    vec![v.to_owned()]
}

fn rec(num: &str, r: FetchRecord) -> BTreeMap<String, FetchRecord> {
    BTreeMap::from([(num.to_owned(), r)])
}

fn ok(u: Value) -> FetchRecord {
    FetchRecord::success(Some(u))
}

fn err(e: &str) -> FetchRecord {
    FetchRecord::failure(e, None)
}

fn err_after(e: &str, after: f64) -> FetchRecord {
    FetchRecord::failure(e, Some(after))
}

impl Fixture {
    fn record(&self, num: &str, r: FetchRecord) -> BTreeSet<String> {
        self.store
            .record(&rec(num, r), &ident(), None, None)
            .unwrap()
    }
    fn record_fenced(
        &self,
        num: &str,
        r: FetchRecord,
        claims: &BTreeMap<String, String>,
    ) -> BTreeSet<String> {
        self.store
            .record(&rec(num, r), &ident(), Some(claims), None)
            .unwrap()
    }
    fn entry(&self, num: &str) -> UsageEntry {
        self.store.entries(&ident(), &[]).remove(num).unwrap()
    }
    fn reserve(&self, num: &str, respect: bool) -> BTreeMap<String, String> {
        self.store
            .reserve(&s(num), &ident(), respect, false)
            .unwrap()
    }
    fn plan(&self, num: &str, plan: PollPlan) {
        self.store
            .set_poll_plan(&BTreeMap::from([(num.to_owned(), plan)]), &ident())
            .unwrap();
    }
    fn adopt(&self, num: &str, u: Value, age: f64, hold: Option<f64>) -> BTreeSet<String> {
        self.store
            .adopt(
                &BTreeMap::from([(num.to_owned(), (u, age))]),
                &ident(),
                hold,
            )
            .unwrap()
    }
    fn raw(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(&self.store.path).unwrap()).unwrap()
    }
    fn write_raw(&self, text: &str) {
        std::fs::create_dir_all(self.store.path.parent().unwrap()).unwrap();
        std::fs::write(&self.store.path, text).unwrap();
    }
}

fn decision_usage(entry: &UsageEntry) -> Option<Value> {
    match entry.decision_value() {
        Some(Decision::Usage(u)) => Some(u.clone()),
        _ => None,
    }
}

fn iso_at(t: f64) -> String {
    iso_utc(t).unwrap()
}

fn usage_resetting(now: f64, five: Option<f64>, seven: Option<f64>) -> Value {
    let mut five_w = json!({"pct": 25.0});
    if let Some(a) = five {
        five_w["resets_at"] = json!(iso_at(now + a));
    }
    let mut seven_w = json!({"pct": 10.0});
    if let Some(a) = seven {
        seven_w["resets_at"] = json!(iso_at(now + a));
    }
    json!({"five_hour": five_w, "seven_day": seven_w})
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

mod leases;
mod schedule;
mod trust;
