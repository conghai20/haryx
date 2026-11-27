//! Two machines over one in-memory sync server.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tempfile::TempDir;

use super::row::{self, Clock};
use super::{Connection, UsageShare};
use crate::poll_policy::HOURLY_REQUEST_CAP;
use crate::sync::crypto::SyncKey;
use crate::sync::remote::Memory;
use crate::usage_store::{FetchRecord, Identities, SharedState, UsageStore};

/// Near the in-memory server's clock, so the measured skew stays small.
const START: f64 = 1_800_000_000.0;

struct Machine {
    _tmp: TempDir,
    clock: Arc<Mutex<f64>>,
    store: UsageStore,
    share: UsageShare,
}

impl Machine {
    fn new(id: &str, server: &Arc<Memory>, secret: &str) -> Self {
        let tmp = TempDir::new().unwrap();
        let clock = Arc::new(Mutex::new(START));
        let read = Arc::clone(&clock);
        let store = UsageStore::with_clock(tmp.path(), Arc::new(move || *read.lock().unwrap()));
        let server = Arc::clone(server);
        let secret = secret.to_owned();
        let share = UsageShare::new(
            id,
            Box::new(move || {
                Ok(Connection {
                    remote: Box::new(Arc::clone(&server)),
                    key: SyncKey::from_secret(&secret)?,
                })
            }),
        );
        Machine {
            _tmp: tmp,
            clock,
            store,
            share,
        }
    }

    fn advance(&self, s: f64) {
        *self.clock.lock().unwrap() += s;
    }

    fn now(&self) -> f64 {
        *self.clock.lock().unwrap()
    }

    /// One collection pass: pull, reserve, claim, then record `outcome` and push for what it
    /// won. Returns the slots it fetched.
    fn collect(&self, outcome: &FetchRecord, active: &BTreeSet<String>) -> BTreeSet<String> {
        let ids = identities();
        let nums: Vec<String> = ids.keys().cloned().collect();
        let mut pass = self.share.pull(&self.store, &ids).expect("share available");
        let mut claims = self.store.reserve(&nums, &ids, true, true).unwrap();
        let lost = pass.claim(&self.store, &claims, &ids, active);
        self.store.release(&lost, &ids).unwrap();
        claims.retain(|num, _| !lost.contains_key(num));
        let outcomes: BTreeMap<String, FetchRecord> = claims
            .keys()
            .map(|num| (num.clone(), outcome.clone()))
            .collect();
        let plans = claims
            .keys()
            .map(|num| (num.clone(), (Some(self.now() + 300.0), Some(300.0))))
            .collect();
        let accepted = self
            .store
            .record(&outcomes, &ids, Some(&claims), Some(&plans))
            .unwrap();
        pass.push(&self.store, &accepted, &ids, active);
        accepted
    }

    fn entry(&self) -> crate::usage_store::UsageEntry {
        self.store.entries(&identities(), &[]).remove("1").unwrap()
    }
}

fn identities() -> Identities {
    BTreeMap::from([(
        "1".to_owned(),
        ("you@example.com".to_owned(), "org-1".to_owned()),
    )])
}

fn usage(pct: f64) -> Value {
    json!({"five_hour": {"utilization": pct, "resets_at": "2099-01-01T00:00:00Z"}})
}

fn pair() -> (Machine, Machine, Arc<Memory>) {
    let server = Arc::new(Memory::default());
    let secret = SyncKey::generate().to_secret();
    let a = Machine::new("aaaa", &server, &secret);
    let b = Machine::new("bbbb", &server, &secret);
    (a, b, server)
}

#[test]
fn a_reading_from_another_machine_is_adopted_instead_of_fetched() {
    let (a, b, _) = pair();
    let fetched = a.collect(&FetchRecord::success(Some(usage(42.0))), &BTreeSet::new());
    assert_eq!(fetched.len(), 1);

    b.advance(5.0);
    let fetched = b.collect(&FetchRecord::success(Some(usage(99.0))), &BTreeSet::new());
    assert!(
        fetched.is_empty(),
        "the fresh shared reading needs no fetch"
    );
    let entry = b.entry();
    assert!(entry.fetched_by_peer);
    assert_eq!(entry.last_good, Some(usage(42.0)));
    assert!(entry.next_poll_at.is_some(), "the shared plan came with it");
}

#[test]
fn the_second_machine_loses_the_lease_and_does_not_fetch() {
    let (a, b, _) = pair();
    let ids = identities();
    let nums: Vec<String> = ids.keys().cloned().collect();
    // Both read the (empty) server before either claims.
    let mut pass_a = a.share.pull(&a.store, &ids).unwrap();
    let mut pass_b = b.share.pull(&b.store, &ids).unwrap();
    let claims_a = a.store.reserve(&nums, &ids, true, true).unwrap();
    let claims_b = b.store.reserve(&nums, &ids, true, true).unwrap();
    assert_eq!(claims_a.len(), 1);
    assert_eq!(claims_b.len(), 1);

    let lost_a = pass_a.claim(&a.store, &claims_a, &ids, &BTreeSet::new());
    let lost_b = pass_b.claim(&b.store, &claims_b, &ids, &BTreeSet::new());
    assert!(lost_a.is_empty());
    assert_eq!(lost_b, claims_b);

    b.store.release(&lost_b, &ids).unwrap();
    let entry = b.entry();
    assert!(!entry.claimed(b.now()));
    assert_eq!(
        entry.requests_last_hour, 0,
        "a given-back lease is not a request"
    );
}

#[test]
fn a_live_lease_from_another_machine_holds_this_one() {
    let (a, b, _) = pair();
    let ids = identities();
    let nums: Vec<String> = ids.keys().cloned().collect();
    let mut pass_a = a.share.pull(&a.store, &ids).unwrap();
    let claims = a.store.reserve(&nums, &ids, true, true).unwrap();
    assert!(
        pass_a
            .claim(&a.store, &claims, &ids, &BTreeSet::new())
            .is_empty()
    );

    b.advance(1.0);
    b.share.pull(&b.store, &ids).unwrap();
    assert!(b.entry().held(b.now()));
    assert!(b.store.reserve(&nums, &ids, true, true).unwrap().is_empty());
}

#[test]
fn a_429_on_one_machine_parks_the_other() {
    let (a, b, _) = pair();
    a.collect(
        &FetchRecord::failure("http-429", Some(3600.0)),
        &BTreeSet::new(),
    );
    assert!(a.entry().in_backoff(a.now()));

    b.advance(10.0);
    let fetched = b.collect(&FetchRecord::success(Some(usage(1.0))), &BTreeSet::new());
    assert!(
        fetched.is_empty(),
        "no probe inside another machine's block"
    );
    let entry = b.entry();
    assert!(entry.in_backoff(b.now()));
    assert_eq!(entry.last_error.as_deref(), Some("http-429"));
    assert!(entry.last_429_at.is_some());
}

#[test]
fn the_hourly_cap_counts_every_machine() {
    let (_a, b, _) = pair();
    let ids = identities();
    let now = b.now();
    let peer = SharedState {
        requests: (0..HOURLY_REQUEST_CAP)
            .map(|i| (now - 60.0 * i as f64 - 1.0, "aaaa".to_owned()))
            .collect(),
        ..SharedState::default()
    };
    b.store
        .merge_peers(&BTreeMap::from([("1".to_owned(), peer)]), &ids, "bbbb")
        .unwrap();
    let entry = b.entry();
    assert_eq!(entry.requests_last_hour, HOURLY_REQUEST_CAP);
    assert_eq!(entry.peers_seen, 1);
    let nums: Vec<String> = ids.keys().cloned().collect();
    assert!(
        b.store
            .reserve(&nums, &ids, false, true)
            .unwrap()
            .is_empty()
    );

    // An hour later the oldest entries have aged out.
    b.advance(3600.0);
    assert_eq!(b.store.reserve(&nums, &ids, false, true).unwrap().len(), 1);
}

#[test]
fn an_account_active_elsewhere_reads_as_active() {
    let (a, b, _) = pair();
    let active = BTreeSet::from(["1".to_owned()]);
    a.collect(&FetchRecord::success(Some(usage(10.0))), &active);

    b.share.pull(&b.store, &identities()).unwrap();
    assert!(b.entry().active_elsewhere(b.now()));
    b.advance(crate::poll_policy::ACTIVE_MARK_TTL_S + 1.0);
    assert!(!b.entry().active_elsewhere(b.now()));
}

#[test]
fn own_rows_coming_back_change_nothing() {
    let (a, _, _) = pair();
    a.collect(&FetchRecord::success(Some(usage(10.0))), &BTreeSet::new());
    let before = a.entry();
    a.share.pull(&a.store, &identities()).unwrap();
    let after = a.entry();
    assert!(!after.fetched_by_peer);
    assert_eq!(after.fetched_at, before.fetched_at);
    assert_eq!(
        after.requests_last_hour, 1,
        "its own entry is not counted twice"
    );
    assert_eq!(after.peers_seen, 0);
}

#[test]
fn the_clock_offset_converts_both_ways() {
    let clock = Clock::new(1_000.5, Some(900));
    assert_eq!(clock.skew, 100.0);
    assert_eq!(clock.to_server(1_050.0), 950.0);
    assert_eq!(clock.to_local(950.0), 1_050.0);
    assert_eq!(Clock::new(1_000.0, None).skew, 0.0);

    let state = SharedState {
        fetched_at: Some(1_000.0),
        backoff_until: Some(2_000.0),
        requests: vec![(990.0, String::new())],
        active_on: BTreeMap::from([("cccc".to_owned(), 1_500.0)]),
        ..SharedState::default()
    };
    let identity = ("you@example.com".to_owned(), "org".to_owned());
    let values = row::write(&identity, &state, "aaaa", clock, 1_000.0);
    assert_eq!(values["fetchedAt"], json!(900.0));
    assert_eq!(values["fetchedBy"], json!("aaaa"));
    assert_eq!(values["requests"], json!([[890.0, "aaaa"]]));
    let back = row::read(&values, clock);
    assert_eq!(back.fetched_at, Some(1_000.0));
    assert_eq!(back.backoff_until, Some(2_000.0));
    assert_eq!(back.requests, vec![(990.0, "aaaa".to_owned())]);
    assert_eq!(row::identity(&values), Some(identity));
}

#[test]
fn a_reading_a_second_ahead_is_taken_as_now() {
    let (_a, b, _) = pair();
    let ids = identities();
    let ahead = SharedState {
        last_good: Some(usage(5.0)),
        fetched_at: Some(b.now() + 0.8),
        fetched_by: Some("aaaa".to_owned()),
        ..SharedState::default()
    };
    b.store
        .merge_peers(
            &BTreeMap::from([("1".to_owned(), ahead.clone())]),
            &ids,
            "bbbb",
        )
        .unwrap();
    assert_eq!(b.entry().fetched_at, Some(b.now()));

    let far = SharedState {
        last_good: Some(usage(6.0)),
        fetched_at: Some(b.now() + 3600.0),
        ..ahead
    };
    b.advance(1.0);
    b.store
        .merge_peers(&BTreeMap::from([("1".to_owned(), far)]), &ids, "bbbb")
        .unwrap();
    assert_eq!(
        b.entry().last_good,
        Some(usage(5.0)),
        "a far-future reading is not believed"
    );
}

#[test]
fn an_unreachable_server_is_no_share() {
    let tmp = TempDir::new().unwrap();
    let store = UsageStore::new(tmp.path());
    let share = UsageShare::new(
        "aaaa",
        Box::new(|| Err(crate::sync::SyncError::NotSignedIn)),
    );
    assert!(share.pull(&store, &identities()).is_none());
}

/// Two machines against a real HaryX API (the local compose stack): the `usage` kind, the
/// compare-and-set lease and the server clock. Run with
/// `HARYX_E2E_API=<…/api/v1> HARYX_E2E_TOKEN=<user access token> cargo test -- --ignored`.
#[test]
#[ignore = "needs a running HaryX API"]
fn two_machines_against_the_api() {
    use crate::account::{Backend, SessionStore};
    use crate::sync::remote::Api;
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} is not set"));
    let (api, token) = (env("HARYX_E2E_API"), env("HARYX_E2E_TOKEN"));
    let secret = SyncKey::generate().to_secret();
    let machine = |id: &str| {
        let tmp = TempDir::new().unwrap();
        let store = UsageStore::new(tmp.path());
        let (api, token, secret) = (api.clone(), token.clone(), secret.clone());
        let share = UsageShare::new(
            id,
            Box::new(move || {
                // No stored session: a 401 is "not signed in", never a renewal.
                let backend = Backend::new("http://127.0.0.1:9/realms/none", &api);
                let sessions = SessionStore::file(std::env::temp_dir().join("haryx-e2e-none"));
                Ok(Connection {
                    remote: Box::new(Api::new(&backend, &sessions, &token)),
                    key: SyncKey::from_secret(&secret)?,
                })
            }),
        );
        (tmp, store, share)
    };
    let ids = identities();
    let nums: Vec<String> = ids.keys().cloned().collect();
    let (_ta, store_a, share_a) = machine("aaaa");
    let (_tb, store_b, share_b) = machine("bbbb");

    // Both read before either claims; only the first lease wins on the server.
    let mut pass_a = share_a.pull(&store_a, &ids).expect("server reachable");
    let mut pass_b = share_b.pull(&store_b, &ids).expect("server reachable");
    assert!(
        pass_a.clock.skew.abs() < 120.0,
        "server clock read from Date"
    );
    let claims_a = store_a.reserve(&nums, &ids, true, true).unwrap();
    let claims_b = store_b.reserve(&nums, &ids, true, true).unwrap();
    assert!(
        pass_a
            .claim(&store_a, &claims_a, &ids, &BTreeSet::new())
            .is_empty()
    );
    assert!(!pass_a.degraded(), "the server accepts the usage kind");
    assert_eq!(
        pass_b.claim(&store_b, &claims_b, &ids, &BTreeSet::new()),
        claims_b
    );
    store_b.release(&claims_b, &ids).unwrap();

    // A records and shares; B adopts the reading instead of fetching.
    let outcomes = BTreeMap::from([("1".to_owned(), FetchRecord::success(Some(usage(33.0))))]);
    let plans = BTreeMap::from([("1".to_owned(), (Some(store_a.now() + 300.0), Some(300.0)))]);
    let accepted = store_a
        .record(&outcomes, &ids, Some(&claims_a), Some(&plans))
        .unwrap();
    pass_a.push(&store_a, &accepted, &ids, &BTreeSet::new());
    assert!(!pass_a.degraded());

    share_b.pull(&store_b, &ids).unwrap();
    let entry = store_b.entries(&ids, &[]).remove("1").unwrap();
    assert!(entry.fetched_by_peer);
    assert_eq!(entry.last_good, Some(usage(33.0)));
    assert!(store_b.reserve(&nums, &ids, true, true).unwrap().is_empty());
}
