package consent

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/jackc/pgx/v5/pgconn"
	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/db"
	syncapi "github.com/conghai20/haryx/server/api/internal/sync"
)

const userA, userB = "3f0c6a52-6a3e-4d2b-9a57-0b7d1d6f9e21", "0d5e8f8e-1111-4c2b-8a57-0b7d1d6f9e22"

var (
	cliA = auth.User{ID: userA, Client: "haryx-cli"}
	cliB = auth.User{ID: userB, Client: "haryx-cli"}
	webA = auth.User{ID: userA, Client: "haryx-platform"}
)

// env is a fresh schema with users A and B, and the consent routes plus the gated sync routes,
// as the API mounts them.
type env struct {
	t    *testing.T
	pool *pgxpool.Pool
	h    http.Handler
}

func newEnv(t *testing.T) *env {
	t.Helper()
	if os.Getenv("PGHOST") == "" {
		t.Skip("PGHOST not set: run server/api/test.sh")
	}
	ctx := context.Background()
	pool, err := db.Open(ctx)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(pool.Close)
	if _, err := pool.Exec(ctx, `drop schema public cascade; create schema public`); err != nil {
		t.Fatal(err)
	}
	if err := db.Migrate(ctx, pool); err != nil {
		t.Fatal(err)
	}
	store := db.NewStore(pool)
	for _, u := range []string{userA, userB} {
		if err := store.TouchUser(ctx, u); err != nil {
			t.Fatal(err)
		}
	}
	mux := http.NewServeMux()
	mux.Handle("/api/v1/consents", Routes(pool))
	mux.Handle("/api/v1/consents/", Routes(pool))
	data := Require(pool, Sync, syncapi.Routes(pool))
	mux.Handle("/api/v1/sync/", data)
	mux.Handle("/api/v1/profiles", data)
	mux.Handle("/api/v1/profiles/", data)
	return &env{t: t, pool: pool, h: mux}
}

func send(h http.Handler, user *auth.User, method, path string, body any) *httptest.ResponseRecorder {
	var r io.Reader
	switch b := body.(type) {
	case nil:
	case string:
		r = strings.NewReader(b)
	default:
		raw, _ := json.Marshal(b)
		r = bytes.NewReader(raw)
	}
	req := httptest.NewRequest(method, path, r)
	if user != nil {
		req = req.WithContext(auth.WithUser(req.Context(), *user))
	}
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func (e *env) do(user auth.User, method, path string, body any) *httptest.ResponseRecorder {
	return send(e.h, &user, method, path, body)
}

func (e *env) expect(rec *httptest.ResponseRecorder, status int, code string) {
	e.t.Helper()
	if rec.Code != status {
		e.t.Fatalf("status %d, want %d: %s", rec.Code, status, rec.Body)
	}
	if code == "" {
		return
	}
	var got struct{ Error string }
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil || got.Error != code {
		e.t.Fatalf("error %q, want %q: %s", got.Error, code, rec.Body)
	}
}

// consents decodes a 200 consents body, refusing fields the contract does not have.
func (e *env) consents(rec *httptest.ResponseRecorder) map[string]Consent {
	e.t.Helper()
	e.expect(rec, http.StatusOK, "")
	dec := json.NewDecoder(rec.Body)
	dec.DisallowUnknownFields()
	var body Consents
	if err := dec.Decode(&body); err != nil {
		e.t.Fatal(err)
	}
	if len(body.Consents) != 2 || body.Consents[0].Purpose != Sync || body.Consents[1].Purpose != Machines {
		e.t.Fatalf("purposes %+v, want sync then machines", body.Consents)
	}
	return map[string]Consent{Sync: body.Consents[0], Machines: body.Consents[1]}
}

func grant(version, source string) map[string]string {
	return map[string]string{"version": version, "source": source}
}

func (e *env) count(sql string, args ...any) int {
	e.t.Helper()
	var n int
	if err := e.pool.QueryRow(context.Background(), sql, args...).Scan(&n); err != nil {
		e.t.Fatal(err)
	}
	return n
}

func (e *env) exec(sql string, args ...any) error {
	_, err := e.pool.Exec(context.Background(), sql, args...)
	return err
}

func TestContractRoundTrips(t *testing.T) {
	dir := os.Getenv("HARYX_CONTRACTS")
	if dir == "" {
		dir = "../../../../packages/contracts"
	}
	raw, err := os.ReadFile(filepath.Join(dir, "haryx", "consents.json"))
	if err != nil {
		t.Fatal(err)
	}
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	var body Consents
	if err := dec.Decode(&body); err != nil {
		t.Fatal(err)
	}
	again, _ := json.Marshal(body)
	var want, got any
	_ = json.Unmarshal(raw, &want)
	_ = json.Unmarshal(again, &got)
	if !reflect.DeepEqual(want, got) {
		t.Fatalf("re-encoded contract differs:\n%s\nwant\n%s", again, raw)
	}
}

func TestNeverAnswered(t *testing.T) {
	e := newEnv(t)
	rec := e.do(cliA, "GET", "/api/v1/consents", nil)
	want := `{"consents":[{"purpose":"sync","granted":false,"version":null,"at":null,"history":[]},` +
		`{"purpose":"machines","granted":false,"version":null,"at":null,"history":[]}]}` + "\n"
	if rec.Code != http.StatusOK || rec.Body.String() != want {
		t.Fatalf("%d %s", rec.Code, rec.Body)
	}
}

func TestGrantOpensTheGateAndWithdrawClosesIt(t *testing.T) {
	e := newEnv(t)
	e.expect(e.do(cliA, "GET", "/api/v1/sync/rows", nil), http.StatusForbidden, "consent_required")
	e.expect(e.do(cliA, "GET", "/api/v1/profiles", nil), http.StatusForbidden, "consent_required")
	e.expect(e.do(cliA, "PUT", "/api/v1/profiles/ab", "x"), http.StatusForbidden, "consent_required")

	before := time.Now().UTC().Truncate(time.Second)
	got := e.consents(e.do(cliA, "PUT", "/api/v1/consents/sync", grant("2026-11-01", "macos")))[Sync]
	if !got.Granted || *got.Version != "2026-11-01" || got.At.Before(before) || len(got.History) != 1 ||
		got.History[0] != (Event{Action: "grant", Version: "2026-11-01", At: *got.At, Source: "macos"}) {
		t.Fatalf("after grant: %+v", got)
	}
	e.expect(e.do(cliA, "GET", "/api/v1/sync/rows", nil), http.StatusOK, "")
	e.expect(e.do(cliA, "PUT", "/api/v1/profiles/ab", "x"), http.StatusOK, "")
	// One user's consent is not another's.
	e.expect(e.do(cliB, "GET", "/api/v1/sync/rows", nil), http.StatusForbidden, "consent_required")
	// Machines is its own consent.
	if m := e.consents(e.do(cliA, "GET", "/api/v1/consents", nil))[Machines]; m.Granted {
		t.Fatal("granting sync granted machines")
	}

	got = e.consents(e.do(cliA, "DELETE", "/api/v1/consents/sync?source=cli", nil))[Sync]
	if got.Granted || *got.Version != "2026-11-01" || len(got.History) != 2 ||
		got.History[1].Action != "withdraw" || got.History[1].Source != "cli" {
		t.Fatalf("after withdraw: %+v", got)
	}
	e.expect(e.do(cliA, "GET", "/api/v1/sync/rows", nil), http.StatusForbidden, "consent_required")
}

func TestGrantIsIdempotentPerVersion(t *testing.T) {
	e := newEnv(t)
	for range 2 {
		e.consents(e.do(cliA, "PUT", "/api/v1/consents/machines", grant("2026-11-01", "cli")))
	}
	if n := e.count(`select count(*) from consent_events`); n != 1 {
		t.Fatalf("granting the same version twice recorded %d events, want 1", n)
	}
	got := e.consents(e.do(cliA, "PUT", "/api/v1/consents/machines", grant("2027-01-01", "windows")))[Machines]
	if !got.Granted || *got.Version != "2027-01-01" || len(got.History) != 2 || got.History[1].Version != "2027-01-01" {
		t.Fatalf("re-consent to a new version: %+v", got)
	}
	got = e.consents(e.do(cliA, "DELETE", "/api/v1/consents/machines?source=windows", nil))[Machines]
	if got.Granted || *got.Version != "2027-01-01" || got.History[2].Version != "2027-01-01" {
		t.Fatalf("withdrawal records the version it withdraws: %+v", got)
	}
	// Withdrawing what is not granted, or never was, records nothing.
	e.consents(e.do(cliA, "DELETE", "/api/v1/consents/machines?source=cli", nil))
	e.consents(e.do(cliA, "DELETE", "/api/v1/consents/sync?source=cli", nil))
	if n := e.count(`select count(*) from consent_events`); n != 3 {
		t.Fatalf("%d events, want 3", n)
	}
}

func TestWithdrawingSyncErasesTheCallersData(t *testing.T) {
	e := newEnv(t)
	for _, u := range []auth.User{cliA, cliB} {
		e.consents(e.do(u, "PUT", "/api/v1/consents/sync", grant("2026-11-01", "cli")))
		for i, kind := range []string{"keyring", "settings", "usage", "usage", "profiles"} {
			if err := e.exec(`insert into sync_rows (user_id, kind, key, body) values ($1, $2, $3, 'eA==')`,
				u.ID, kind, strings.Repeat("a", i)); err != nil {
				t.Fatal(err)
			}
		}
		e.expect(e.do(u, "PUT", "/api/v1/profiles/ab", "x"), http.StatusOK, "")
	}

	e.consents(e.do(cliA, "DELETE", "/api/v1/consents/sync?source=macos", nil))
	for _, table := range []string{"sync_rows", "profile_files"} {
		if n := e.count(`select count(*) from `+table+` where user_id = $1`, userA); n != 0 {
			t.Errorf("%s: %d left for the withdrawing user", table, n)
		}
	}
	if n := e.count(`select count(*) from sync_rows where user_id = $1`, userB); n != 5 {
		t.Errorf("the other user has %d rows, want 5", n)
	}
	if n := e.count(`select count(*) from profile_files where user_id = $1`, userB); n != 1 {
		t.Errorf("the other user has %d profiles, want 1", n)
	}
	withdrawals := `select count(*) from consent_events where user_id = $1 and action = 'withdraw'`
	if n := e.count(withdrawals, userA); n != 1 {
		t.Errorf("%d withdraw events, want 1", n)
	}
	e.consents(e.do(cliA, "DELETE", "/api/v1/consents/sync?source=macos", nil))
	if n := e.count(withdrawals, userA); n != 1 {
		t.Errorf("withdrawing twice: %d withdraw events, want 1", n)
	}
}

// The erasure and the withdraw event commit together: if the event cannot be written, the data
// is still there and the consent still granted.
func TestWithdrawIsOneTransaction(t *testing.T) {
	e := newEnv(t)
	e.consents(e.do(cliA, "PUT", "/api/v1/consents/sync", grant("2026-11-01", "cli")))
	e.expect(e.do(cliA, "PUT", "/api/v1/profiles/ab", "x"), http.StatusOK, "")
	if err := e.exec(`insert into sync_rows (user_id, kind, body) values ($1, 'settings', 'eA==')`, userA); err != nil {
		t.Fatal(err)
	}
	if err := e.exec(`create function fail() returns trigger language plpgsql as $$
		begin raise exception 'injected'; end $$;
		create trigger fail before insert on consent_events for each row execute function fail()`); err != nil {
		t.Fatal(err)
	}

	e.expect(e.do(cliA, "DELETE", "/api/v1/consents/sync?source=cli", nil), http.StatusInternalServerError, "internal")
	if n := e.count(`select count(*) from sync_rows`) + e.count(`select count(*) from profile_files`); n != 2 {
		t.Errorf("a failed withdrawal deleted data: %d of 2 left", n)
	}
	if n := e.count(`select count(*) from consents where granted`); n != 1 {
		t.Error("a failed withdrawal changed the consent")
	}
}

func TestSourceFollowsTheClient(t *testing.T) {
	e := newEnv(t)
	for _, c := range []struct {
		user   auth.User
		method string
		path   string
		body   any
		source string // recorded, or "" for 400 invalid_source
	}{
		{webA, "PUT", "/api/v1/consents/sync", grant("v1", "cli"), "web"},
		{webA, "DELETE", "/api/v1/consents/sync?source=macos", nil, "web"},
		{webA, "PUT", "/api/v1/consents/sync", map[string]string{"version": "v1"}, "web"},
		{webA, "DELETE", "/api/v1/consents/sync", nil, "web"},
		{cliA, "PUT", "/api/v1/consents/sync", grant("v1", "web"), ""},
		{cliA, "PUT", "/api/v1/consents/sync", grant("v1", "sso"), ""},
		{cliA, "PUT", "/api/v1/consents/sync", grant("v1", ""), ""},
		{cliA, "PUT", "/api/v1/consents/sync", grant("v1", "windows"), "windows"},
		{cliA, "DELETE", "/api/v1/consents/sync?source=web", nil, ""},
		{cliA, "DELETE", "/api/v1/consents/sync", nil, ""},
		{cliA, "DELETE", "/api/v1/consents/sync?source=macos", nil, "macos"},
	} {
		rec := e.do(c.user, c.method, c.path, c.body)
		if c.source == "" {
			e.expect(rec, http.StatusBadRequest, "invalid_source")
			continue
		}
		h := e.consents(rec)[Sync].History
		if last := h[len(h)-1]; last.Source != c.source {
			t.Errorf("%s %s %v as %s: source %q, want %q", c.method, c.path, c.body, c.user.Client, last.Source, c.source)
		}
	}
}

func TestBadRequests(t *testing.T) {
	e := newEnv(t)
	for _, v := range []string{"", " ", "-1", "a/b", "2026 11", strings.Repeat("a", 33), "ü"} {
		e.expect(e.do(cliA, "PUT", "/api/v1/consents/sync", grant(v, "cli")), http.StatusBadRequest, "invalid_version")
	}
	e.consents(e.do(cliA, "PUT", "/api/v1/consents/sync", grant(strings.Repeat("a", 32), "cli")))
	e.expect(e.do(cliA, "PUT", "/api/v1/consents/sync", "{"), http.StatusBadRequest, "invalid_json")
	e.expect(e.do(cliA, "PUT", "/api/v1/consents/sync", strings.Repeat(" ", maxRequest+1)), http.StatusBadRequest, "invalid_json")
	e.expect(e.do(cliA, "PUT", "/api/v1/consents/usage", grant("v1", "cli")), http.StatusBadRequest, "invalid_purpose")
	e.expect(e.do(cliA, "DELETE", "/api/v1/consents/usage?source=cli", nil), http.StatusBadRequest, "invalid_purpose")
	e.expect(e.do(cliA, "GET", "/api/v1/consents/sync", nil), http.StatusNotFound, "not_found")
	for _, c := range []struct{ method, path string }{
		{"GET", "/api/v1/consents"}, {"PUT", "/api/v1/consents/sync"}, {"DELETE", "/api/v1/consents/sync"}, {"GET", "/api/v1/sync/rows"},
	} {
		if rec := send(e.h, nil, c.method, c.path, nil); rec.Code != http.StatusUnauthorized {
			t.Errorf("%s %s without a user: %d, want 401", c.method, c.path, rec.Code)
		}
	}
}

// The database holds the rules too: no purpose data without its consent, even past the gate (a
// withdrawal racing an upload), and events are never rewritten.
func TestDatabaseRules(t *testing.T) {
	e := newEnv(t)
	err := e.exec(`insert into sync_rows (user_id, kind, body) values ($1, 'settings', 'eA==')`, userA)
	var pgErr *pgconn.PgError
	if !errors.As(err, &pgErr) || pgErr.ConstraintName != "consent_required" {
		t.Fatalf("insert without consent: %v", err)
	}
	ungated := syncapi.Routes(e.pool)
	e.expect(send(ungated, &cliA, "POST", "/api/v1/sync/rows", map[string]string{"kind": "settings", "body": "eA=="}),
		http.StatusForbidden, "consent_required")
	e.expect(send(ungated, &cliA, "PUT", "/api/v1/profiles/ab", "x"), http.StatusForbidden, "consent_required")

	e.consents(e.do(cliA, "PUT", "/api/v1/consents/sync", grant("v1", "cli")))
	if err := e.exec(`update consent_events set source = 'web'`); err == nil {
		t.Error("an event was rewritten")
	}
}

// User expiry (A10) takes the consents and their events with the user.
func TestExpiryDeletesConsents(t *testing.T) {
	e := newEnv(t)
	for _, u := range []auth.User{cliA, cliB} {
		e.consents(e.do(u, "PUT", "/api/v1/consents/sync", grant("v1", "cli")))
	}
	if err := e.exec(`update users set last_seen = now() - interval '400 days' where user_id = $1`, userA); err != nil {
		t.Fatal(err)
	}
	if _, err := db.NewStore(e.pool).DeleteInactiveUsers(context.Background(), 365*24*time.Hour); err != nil {
		t.Fatal(err)
	}
	for _, table := range []string{"consents", "consent_events"} {
		if n := e.count(`select count(*) from `+table+` where user_id = $1`, userA); n != 0 {
			t.Errorf("%s: %d left for the expired user", table, n)
		}
		if n := e.count(`select count(*) from `+table+` where user_id = $1`, userB); n != 1 {
			t.Errorf("%s: %d for the active user, want 1", table, n)
		}
	}
}

func TestDatabaseGone(t *testing.T) {
	e := newEnv(t)
	e.pool.Close()
	for _, c := range []struct {
		method, path string
		body         any
	}{
		{"GET", "/api/v1/consents", nil},
		{"PUT", "/api/v1/consents/sync", grant("v1", "cli")},
		{"DELETE", "/api/v1/consents/sync?source=cli", nil},
		{"GET", "/api/v1/sync/rows", nil},
	} {
		e.expect(e.do(cliA, c.method, c.path, c.body), http.StatusInternalServerError, "internal")
	}
}
