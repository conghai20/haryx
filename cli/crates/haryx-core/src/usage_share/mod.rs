//! Usage share between a user's machines (`docs/usage-share-plan.md`).
//!
//! The usage endpoint's budget belongs to the Claude account, not to the machine. Every machine
//! of a HaryX user that collects usage for the same account therefore shares one encrypted sync
//! row per account (kind `usage`), read and written directly by the collector, outside the sync
//! pass:
//!
//! 1. [`UsageShare::pull`]: list the rows and merge them into the local store — a newer reading
//!    and its plan, a later backoff, another machine's live lease, the other machines' ledger
//!    entries and active marks. The local store then decides eligibility as usual.
//! 2. [`Pass::claim`]: compare-and-set this machine's lease (and its ledger entry) into each row
//!    it is about to fetch; a lost race gives the local lease back.
//! 3. [`Pass::push`]: write each recorded outcome back, so the other machines adopt it.
//!
//! Any failure (signed out, sync locked, offline, a server without the kind) leaves collection as
//! it was without the share; the collector then widens its own cadence by the other machines its
//! ledger saw ([`Pass`] absent or [`Pass::degraded`]).

mod row;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use log::debug;

pub use row::Clock;

use crate::account::{Backend, SessionStore};
use crate::poll_policy::ACTIVE_MARK_TTL_S;
use crate::sync::crypto::SyncKey;
use crate::sync::remote::{Api, Remote, Row};
use crate::sync::sources::label;
use crate::sync::{self, Document, KIND_USAGE, Opened, SyncError};
use crate::usage_store::{CLAIM_TTL_S, Identities, Identity, SharedState, UsageStore};

/// File under the store's cache directory naming this machine in shared rows.
const MACHINE_ID_FILE: &str = "machine-id";

/// A connection to the sync server, with this machine's sync key.
pub struct Connection {
    pub remote: Box<dyn Remote + Send + Sync>,
    pub key: SyncKey,
}

type Connect = Box<dyn Fn() -> Result<Connection, SyncError> + Send + Sync>;

/// This machine's handle on the shared usage rows.
pub struct UsageShare {
    machine: String,
    connect: Connect,
}

impl std::fmt::Debug for UsageShare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsageShare")
            .field("machine", &self.machine)
            .finish_non_exhaustive()
    }
}

/// Unix seconds now.
fn unix_now() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// This machine's id, made on first use: random, so it says nothing about the machine.
fn machine_id(store_root: &Path) -> String {
    let path: PathBuf = store_root.join("cache").join(MACHINE_ID_FILE);
    if let Ok(text) = std::fs::read_to_string(&path) {
        let id = text.trim();
        if !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return id.to_owned();
        }
    }
    let id = format!("{:016x}", rand::random::<u64>());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(err) = std::fs::write(&path, &id) {
        debug!("usage share: could not save the machine id: {err}");
    }
    id
}

/// The signed-in session's connection, renewing the access token when needed.
fn connect_store(store_root: &Path) -> Result<Connection, SyncError> {
    let sessions = SessionStore::for_store(store_root);
    let backend = Backend::from_env();
    let session = sessions
        .current(&backend, unix_now())?
        .ok_or(SyncError::NotSignedIn)?;
    let key = sync::unlocked_key(sessions.vault(), &session.user.id, backend.issuer())?
        .ok_or(SyncError::Locked)?;
    Ok(Connection {
        remote: Box::new(Api::new(&backend, &sessions, &session.access_token)),
        key,
    })
}

impl UsageShare {
    pub fn new(machine: impl Into<String>, connect: Connect) -> Self {
        UsageShare {
            machine: machine.into(),
            connect,
        }
    }

    /// The share of the haryx store at `store_root`, as the HaryX user signed in there.
    pub fn for_store(store_root: &Path) -> Self {
        let root = store_root.to_path_buf();
        UsageShare::new(
            machine_id(store_root),
            Box::new(move || connect_store(&root)),
        )
    }

    pub fn machine(&self) -> &str {
        &self.machine
    }

    /// Read the shared rows of `identities` and merge them into `store`. `None` when the share is
    /// unavailable this time (not signed in, locked, offline, refused); the reason is logged.
    pub fn pull(&self, store: &UsageStore, identities: &Identities) -> Option<Pass<'_>> {
        let conn = (self.connect)()
            .map_err(|err| debug!("usage share unavailable: {err}"))
            .ok()?;
        let listing = conn
            .remote
            .list_kind(KIND_USAGE)
            .map_err(|err| debug!("usage share: listing failed: {err}"))
            .ok()?;
        let clock = Clock::new(store.now(), listing.server_now);
        let keys: BTreeMap<String, String> = identities
            .iter()
            .map(|(num, identity)| (num.clone(), row_key(&conn.key, identity)))
            .collect();
        let mut slots = BTreeMap::new();
        for row in &listing.rows {
            let Some(num) = keys.iter().find(|(_, k)| **k == row.key).map(|(n, _)| n) else {
                continue;
            };
            slots.insert(num.clone(), Slot::of(&conn.key, row, clock));
        }
        let peers: BTreeMap<String, SharedState> = slots
            .iter()
            .filter_map(|(num, slot)| Some((num.clone(), slot.state.clone()?)))
            .collect();
        if let Err(err) = store.merge_peers(&peers, identities, &self.machine) {
            debug!("usage share: merging failed: {err}");
            return None;
        }
        Some(Pass {
            share: self,
            conn,
            clock,
            keys,
            slots,
            degraded: false,
        })
    }
}

fn row_key(key: &SyncKey, identity: &Identity) -> String {
    key.row_key(&label("usage", &identity.0, &identity.1))
}

/// What the server holds for one account.
#[derive(Debug, Clone)]
struct Slot {
    version: i64,
    /// `None`: the row did not open with this key (replaced on the next write).
    state: Option<SharedState>,
    /// Written by a newer HaryX in a shape this one does not read: never overwritten.
    unreadable: bool,
}

impl Slot {
    fn of(key: &SyncKey, row: &Row, clock: Clock) -> Self {
        match sync::open(key, row) {
            Opened::Doc(doc) => Slot {
                version: row.version,
                state: Some(row::read(&doc.values, clock)),
                unreadable: false,
            },
            Opened::Foreign => Slot {
                version: row.version,
                state: None,
                unreadable: false,
            },
            Opened::Unreadable => Slot {
                version: row.version,
                state: None,
                unreadable: true,
            },
        }
    }
}

/// How one write went.
enum Wrote {
    Done,
    /// Another machine wrote first.
    Lost,
    /// The server could not be reached or refused: stop coordinating this pass.
    Failed,
}

/// One collection pass's view of the shared rows.
pub struct Pass<'a> {
    share: &'a UsageShare,
    conn: Connection,
    clock: Clock,
    keys: BTreeMap<String, String>,
    slots: BTreeMap<String, Slot>,
    degraded: bool,
}

impl Pass<'_> {
    /// Whether a write failed this pass, so the machines are not coordinated right now.
    pub fn degraded(&self) -> bool {
        self.degraded
    }

    fn me(&self) -> &str {
        &self.share.machine
    }

    /// Compare-and-set `state` as the row of `num`.
    fn write(&mut self, num: &str, identity: &Identity, state: &SharedState, now: f64) -> Wrote {
        let Some(row_key) = self.keys.get(num).cloned() else {
            return Wrote::Failed;
        };
        let current = self.slots.get(num).map(|s| s.version);
        let doc = Document {
            kind: KIND_USAGE.to_owned(),
            key: row_key.clone(),
            version: current.map_or(1, |v| v + 1),
            values: row::write(identity, state, self.me(), self.clock, now),
            changed_at: self.clock.to_server(now) as i64,
        };
        let sealed = serde_json::to_vec(&doc)
            .map_err(|e| SyncError::Local(e.to_string()))
            .and_then(|plain| self.conn.key.seal(&plain));
        let body = match sealed {
            Ok(body) => body,
            Err(err) => {
                debug!("usage share: sealing failed: {err}");
                return Wrote::Failed;
            }
        };
        let result = match current {
            Some(version) => self
                .conn
                .remote
                .update(KIND_USAGE, &row_key, &body, version),
            None => self.conn.remote.insert(KIND_USAGE, &row_key, &body),
        };
        match result {
            Ok(Some(row)) => {
                self.slots.insert(
                    num.to_owned(),
                    Slot {
                        version: row.version,
                        state: Some(state.clone()),
                        unreadable: false,
                    },
                );
                Wrote::Done
            }
            Ok(None) => Wrote::Lost,
            Err(err) => {
                debug!("usage share: writing failed: {err}");
                self.degraded = true;
                Wrote::Failed
            }
        }
    }

    /// The active marks to write: the other machines' live ones, and this machine's when the
    /// account is active here.
    fn marks(&self, num: &str, is_active: bool, now: f64) -> BTreeMap<String, f64> {
        let mut marks: BTreeMap<String, f64> = self
            .slots
            .get(num)
            .and_then(|s| s.state.as_ref())
            .map(|s| s.active_on.clone())
            .unwrap_or_default();
        marks.retain(|by, until| by != self.me() && *until > now);
        if is_active {
            marks.insert(self.me().to_owned(), now + ACTIVE_MARK_TTL_S);
        }
        marks
    }

    /// Take the shared lease of every account this machine reserved. Returns the reservations
    /// lost to another machine (the caller releases them and does not fetch). A failed write keeps
    /// the remaining reservations: without the server, collection goes on as before the share.
    pub fn claim(
        &mut self,
        store: &UsageStore,
        claims: &BTreeMap<String, String>,
        identities: &Identities,
        active: &BTreeSet<String>,
    ) -> BTreeMap<String, String> {
        let now = store.now();
        let local = store.shared_states(identities);
        let mut lost = BTreeMap::new();
        for (num, claim_id) in claims {
            if self.degraded {
                break;
            }
            let Some(identity) = identities.get(num) else {
                continue;
            };
            let slot = self.slots.get(num);
            if slot.is_some_and(|s| s.unreadable) {
                continue;
            }
            let remote = slot.and_then(|s| s.state.clone());
            let taken = remote.as_ref().is_some_and(|r| {
                r.claim_until.is_some_and(|until| until > now)
                    && r.claim_by.as_deref() != Some(self.me())
            });
            if taken {
                lost.insert(num.clone(), claim_id.clone());
                continue;
            }
            let mut state = remote
                .or_else(|| local.get(num).cloned())
                .unwrap_or_default();
            state.claim_until = Some(now + CLAIM_TTL_S);
            state.claim_by = Some(self.me().to_owned());
            state.requests.retain(|(_, by)| by != self.me());
            state.requests.extend(
                local
                    .get(num)
                    .map(|l| l.requests.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(_, by)| by == crate::usage_store::LOCAL_MACHINE),
            );
            state.active_on = self.marks(num, active.contains(num), now);
            if let Wrote::Lost = self.write(num, identity, &state, now) {
                lost.insert(num.clone(), claim_id.clone());
            }
        }
        lost
    }

    /// Share the outcomes this machine recorded for `nums`, clearing its leases, and renew its
    /// active mark on the other active accounts. A row another machine changed since this pass
    /// read it is read again, merged, and written once more.
    pub fn push(
        &mut self,
        store: &UsageStore,
        nums: &BTreeSet<String>,
        identities: &Identities,
        active: &BTreeSet<String>,
    ) {
        let now = store.now();
        let mut local = store.shared_states(identities);
        for num in nums {
            if self.degraded {
                return;
            }
            let Some(identity) = identities.get(num) else {
                continue;
            };
            if self.slots.get(num).is_some_and(|s| s.unreadable) {
                continue;
            }
            for attempt in 0..2 {
                let Some(mut state) = local.get(num).cloned() else {
                    break;
                };
                state.claim_until = None;
                state.claim_by = None;
                state.active_on = self.marks(num, active.contains(num), now);
                match self.write(num, identity, &state, now) {
                    Wrote::Done | Wrote::Failed => break,
                    Wrote::Lost if attempt == 0 => {
                        if !self.reread(store, num, identities) {
                            break;
                        }
                        local = store.shared_states(identities);
                    }
                    Wrote::Lost => debug!("usage share: row {num} kept changing; not shared"),
                }
            }
        }
        self.mark_active(store, nums, identities, active, now);
    }

    /// Renew this machine's active mark where it is about to lapse (accounts not just pushed).
    fn mark_active(
        &mut self,
        store: &UsageStore,
        done: &BTreeSet<String>,
        identities: &Identities,
        active: &BTreeSet<String>,
        now: f64,
    ) {
        for num in active.difference(done) {
            if self.degraded {
                return;
            }
            let Some(identity) = identities.get(num) else {
                continue;
            };
            let Some(slot) = self.slots.get(num).cloned() else {
                continue;
            };
            let Some(mut state) = slot.state.filter(|_| !slot.unreadable) else {
                continue;
            };
            let fresh = state
                .active_on
                .get(self.me())
                .is_some_and(|until| *until > now + ACTIVE_MARK_TTL_S / 2.0);
            if fresh {
                continue;
            }
            state.active_on = self.marks(num, true, now);
            let _ = self.write(num, identity, &state, now);
        }
        let _ = store;
    }

    /// Read one row again after a lost write and merge it; false when that failed.
    fn reread(&mut self, store: &UsageStore, num: &str, identities: &Identities) -> bool {
        let Some(row_key) = self.keys.get(num).cloned() else {
            return false;
        };
        let listing = match self.conn.remote.list_kind(KIND_USAGE) {
            Ok(listing) => listing,
            Err(err) => {
                debug!("usage share: re-reading failed: {err}");
                self.degraded = true;
                return false;
            }
        };
        let Some(row) = listing.rows.iter().find(|r| r.key == row_key) else {
            self.slots.remove(num);
            return true;
        };
        let slot = Slot::of(&self.conn.key, row, self.clock);
        if let Some(state) = &slot.state {
            let one = BTreeMap::from([(num.to_owned(), state.clone())]);
            if let Err(err) = store.merge_peers(&one, identities, self.me()) {
                debug!("usage share: merging failed: {err}");
                return false;
            }
        }
        self.slots.insert(num.to_owned(), slot);
        true
    }
}
