package machines

import (
	"bytes"
	"context"
	"crypto"
	"crypto/rand"
	"crypto/rsa"
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

	"github.com/coreos/go-oidc/v3/oidc"
	jose "github.com/go-jose/go-jose/v4"
	"github.com/jackc/pgx/v5/pgconn"
	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/consent"
	"github.com/conghai20/haryx/server/api/internal/db"
)

const (
	userA, userB = "3f0c6a52-6a3e-4d2b-9a57-0b7d1d6f9e21", "0d5e8f8e-1111-4c2b-8a57-0b7d1d6f9e22"
	mac, pc      = "9b2e5c1a-4f0d-4a67-8d3e-1c7a5b0e2f91", "c41d7e08-2b9a-4e35-a6f1-70d8e3b94a52"
	sidMac       = "sid-mac"
	sidPC        = "sid-pc"
)

var (
	macA = auth.User{ID: userA, SessionID: sidMac, Client: "haryx-cli"}
	pcA  = auth.User{ID: userA, SessionID: sidPC, Client: "haryx-cli"}
	cliB = auth.User{ID: userB, SessionID: "sid-b", Client: "haryx-cli"}
	webA = auth.User{ID: userA, SessionID: "sid-web-a", Client: "haryx-platform"}
	webB = auth.User{ID: userB, SessionID: "sid-web-b", Client: "haryx-platform"}
)

type env struct {
	t    *testing.T
	pool *pgxpool.Pool
	h    http.Handler
}

// newEnv is a fresh schema with users A and B, and the machine and consent routes.
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
	for _, u := range []string{userA, userB} {
		if err := db.NewStore(pool).TouchUser(ctx, u); err != nil {
			t.Fatal(err)
		}
	}
	mux := http.NewServeMux()
	mux.Handle("/api/v1/consents/", consent.Routes(pool))
	mux.Handle("/api/v1/machines", Routes(pool))
	mux.Handle("/api/v1/machines/", Routes(pool))
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

func (e *env) grant(u auth.User) {
	e.t.Helper()
	e.expect(e.do(u, "PUT", "/api/v1/consents/machines", map[string]string{"version": "v1", "source": "cli"}), http.StatusOK, "")
}

func beat(id, name string) map[string]string {
	return map[string]string{"machineId": id, "name": name, "os": "macos", "version": "1.9.0"}
}

// machine decodes a 200 machine body, refusing fields the contract does not have.
func (e *env) machine(rec *httptest.ResponseRecorder) Machine {
	e.t.Helper()
	e.expect(rec, http.StatusOK, "")
	var m Machine
	strict(e.t, rec.Body.Bytes(), &m)
	return m
}

func (e *env) list(u auth.User) []Machine {
	e.t.Helper()
	rec := e.do(u, "GET", "/api/v1/machines", nil)
	e.expect(rec, http.StatusOK, "")
	var body Machines
	strict(e.t, rec.Body.Bytes(), &body)
	if body.Machines == nil {
		e.t.Fatalf("machines is null, want a list: %s", rec.Body)
	}
	return body.Machines
}

func strict(t *testing.T, raw []byte, v any) {
	t.Helper()
	dec := json.NewDecoder(bytes.NewReader(raw))
	dec.DisallowUnknownFields()
	if err := dec.Decode(v); err != nil {
		t.Fatalf("%v: %s", err, raw)
	}
}

func (e *env) count(sql string, args ...any) int {
	e.t.Helper()
	var n int
	if err := e.pool.QueryRow(context.Background(), sql, args...).Scan(&n); err != nil {
		e.t.Fatal(err)
	}
	return n
}

func (e *env) revoked(sid string) bool {
	e.t.Helper()
	got, err := db.NewStore(e.pool).IsRevoked(context.Background(), sid)
	if err != nil {
		e.t.Fatal(err)
	}
	return got
}

func TestContractRoundTrips(t *testing.T) {
	dir := os.Getenv("HARYX_CONTRACTS")
	if dir == "" {
		dir = "../../../../packages/contracts"
	}
	raw, err := os.ReadFile(filepath.Join(dir, "haryx", "machines.json"))
	if err != nil {
		t.Fatal(err)
	}
	var body Machines
	strict(t, raw, &body)
	again, _ := json.Marshal(body)
	var want, got any
	_ = json.Unmarshal(raw, &want)
	_ = json.Unmarshal(again, &got)
	if !reflect.DeepEqual(want, got) {
		t.Fatalf("re-encoded contract differs:\n%s\nwant\n%s", again, raw)
	}
}

func TestHeartbeatNeedsTheMachinesConsent(t *testing.T) {
	e := newEnv(t)
	e.expect(e.do(macA, "PUT", "/api/v1/machines/self", beat(mac, "Work MacBook")), http.StatusForbidden, "consent_required")
	if got := e.list(macA); len(got) != 0 {
		t.Fatalf("listed %v before any heartbeat", got)
	}
	e.grant(macA)

	before := time.Now().UTC().Truncate(time.Second)
	got := e.machine(e.do(macA, "PUT", "/api/v1/machines/self", beat(mac, "Work MacBook")))
	want := Machine{MachineID: mac, Name: "Work MacBook", OS: "macos", Version: "1.9.0",
		LastSeenAt: got.LastSeenAt, IsThisMachine: true, SignedIn: true, SessionID: "sid-mac"}
	if got != want || got.LastSeenAt.Before(before) || got.LastSeenAt.Location() != time.UTC {
		t.Fatalf("heartbeat: %+v", got)
	}
	// A re-login is a new sid: the row follows it. The name stays the registry's.
	relogin := macA
	relogin.SessionID = "sid-mac-2"
	got = e.machine(e.do(relogin, "PUT", "/api/v1/machines/self",
		map[string]string{"machineId": mac, "name": "Other", "os": "linux", "version": "2.0.0+b1"}))
	if got.Name != "Work MacBook" || got.OS != "linux" || got.Version != "2.0.0+b1" || !got.IsThisMachine {
		t.Fatalf("second heartbeat: %+v", got)
	}
	if n := e.count(`select count(*) from machines where session_id = 'sid-mac-2'`); n != 1 {
		t.Fatal("heartbeat did not record the new sid")
	}
	// isThisMachine is the caller's own session, so the old one no longer matches.
	if l := e.list(macA); len(l) != 1 || l[0].IsThisMachine {
		t.Fatalf("listed from the old sid: %+v", l)
	}

	// Withdrawal erases the caller's machines only, and closes the heartbeat.
	e.grant(cliB)
	e.machine(e.do(cliB, "PUT", "/api/v1/machines/self", beat(pc, "B's PC")))
	e.expect(e.do(macA, "DELETE", "/api/v1/consents/machines?source=cli", nil), http.StatusOK, "")
	if l := e.list(macA); len(l) != 0 {
		t.Fatalf("after withdrawal: %+v", l)
	}
	if l := e.list(cliB); len(l) != 1 {
		t.Fatalf("A's withdrawal touched B's machines: %+v", l)
	}
	e.expect(e.do(macA, "PUT", "/api/v1/machines/self", beat(mac, "Work MacBook")), http.StatusForbidden, "consent_required")
}

func TestClientRules(t *testing.T) {
	e := newEnv(t)
	e.grant(macA)
	e.expect(e.do(webA, "PUT", "/api/v1/machines/self", beat(mac, "Browser")), http.StatusForbidden, "forbidden_client")
	e.machine(e.do(macA, "PUT", "/api/v1/machines/self", beat(mac, "Work MacBook")))
	e.expect(e.do(macA, "DELETE", "/api/v1/machines/"+mac+"/session", nil), http.StatusForbidden, "forbidden_client")
	if e.revoked(sidMac) || e.count(`select count(*) from machines`) != 1 {
		t.Fatal("a haryx-cli sign-out did something")
	}
	// Both clients may list and rename.
	if l := e.list(webA); len(l) != 1 || l[0].IsThisMachine {
		t.Fatalf("web list: %+v", l)
	}
	if m := e.machine(e.do(webA, "PATCH", "/api/v1/machines/"+mac, map[string]string{"name": "Renamed"})); m.Name != "Renamed" {
		t.Fatalf("web rename: %+v", m)
	}
	if m := e.machine(e.do(macA, "PATCH", "/api/v1/machines/"+mac, map[string]string{"name": "Mine"})); m.Name != "Mine" || !m.IsThisMachine {
		t.Fatalf("cli rename: %+v", m)
	}
	// A heartbeat after a rename keeps the new name.
	if m := e.machine(e.do(macA, "PUT", "/api/v1/machines/self", beat(mac, "Work MacBook"))); m.Name != "Mine" {
		t.Fatalf("heartbeat undid the rename: %+v", m)
	}
}

func TestIsolation(t *testing.T) {
	e := newEnv(t)
	e.grant(macA)
	e.grant(cliB)
	e.machine(e.do(macA, "PUT", "/api/v1/machines/self", beat(mac, "A's Mac")))

	if l := e.list(cliB); len(l) != 0 {
		t.Fatalf("B listed A's machines: %+v", l)
	}
	if l := e.list(webB); len(l) != 0 {
		t.Fatalf("B's web listed A's machines: %+v", l)
	}
	e.expect(e.do(cliB, "PATCH", "/api/v1/machines/"+mac, map[string]string{"name": "Mine now"}), http.StatusNotFound, "not_found")
	e.expect(e.do(webB, "DELETE", "/api/v1/machines/"+mac+"/session", nil), http.StatusNotFound, "not_found")
	// B registering the same machine id makes B's own row; A's is untouched.
	e.machine(e.do(cliB, "PUT", "/api/v1/machines/self", beat(mac, "B's copy")))
	l := e.list(macA)
	if len(l) != 1 || l[0].Name != "A's Mac" || !l[0].IsThisMachine || e.revoked(sidMac) {
		t.Fatalf("A's machine after B's attempts: %+v, revoked %v", l, e.revoked(sidMac))
	}
	if n := e.count(`select count(*) from machines where user_id = $1 and session_id = $2`, userA, sidMac); n != 1 {
		t.Fatal("A's row changed")
	}
}

// Sign-out drops the machine and revokes exactly its sid: through the real token check, that
// machine is refused and A's other machine is not.
func TestSignOutRevokesOnlyThatMachine(t *testing.T) {
	e := newEnv(t)
	e.grant(macA)
	e.machine(e.do(macA, "PUT", "/api/v1/machines/self", beat(mac, "Work MacBook")))
	e.machine(e.do(pcA, "PUT", "/api/v1/machines/self", map[string]string{"machineId": pc, "name": "Home PC", "os": "windows", "version": "1.9.0"}))
	// An older revocation of the same sid is extended, not duplicated.
	if _, err := e.pool.Exec(context.Background(), `insert into revoked_sessions values ($1, now() - interval '1 minute')`, sidMac); err != nil {
		t.Fatal(err)
	}

	rec := e.do(webA, "DELETE", "/api/v1/machines/"+mac+"/session", nil)
	if rec.Code != http.StatusNoContent || rec.Body.Len() != 0 {
		t.Fatalf("sign-out: %d %s", rec.Code, rec.Body)
	}
	if !e.revoked(sidMac) || e.revoked(sidPC) {
		t.Fatalf("revoked: mac %v, pc %v; want only the mac", e.revoked(sidMac), e.revoked(sidPC))
	}
	if n := e.count(`select count(*) from revoked_sessions where until > now() + interval '55 minutes'`); n != 1 {
		t.Fatalf("%d revocations last the hour, want 1", n)
	}
	if l := e.list(pcA); len(l) != 1 || l[0].MachineID != pc || !l[0].SignedIn {
		t.Fatalf("after sign-out: %+v", l)
	}
	e.expect(e.do(webA, "DELETE", "/api/v1/machines/"+mac+"/session", nil), http.StatusNotFound, "not_found")

	// The real auth middleware, with the store's revoked check, in front of the routes.
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	const issuer = "https://sso.test/realms/wecoap"
	keys := &oidc.StaticKeySet{PublicKeys: []crypto.PublicKey{&key.PublicKey}}
	api := auth.New(issuer, keys, db.NewStore(e.pool).IsRevoked).Middleware(Routes(e.pool))
	for sid, want := range map[string]int{sidMac: http.StatusUnauthorized, sidPC: http.StatusOK} {
		req := httptest.NewRequest("GET", "/api/v1/machines", nil)
		req.Header.Set("Authorization", "Bearer "+token(t, key, issuer, sid))
		rec := httptest.NewRecorder()
		api.ServeHTTP(rec, req)
		if rec.Code != want {
			t.Errorf("token with sid %s: %d, want %d: %s", sid, rec.Code, want, rec.Body)
		}
	}
}

// token is a haryx-cli access token for user A, as Keycloak issues it.
func token(t *testing.T, key *rsa.PrivateKey, issuer, sid string) string {
	t.Helper()
	signer, err := jose.NewSigner(jose.SigningKey{Algorithm: jose.RS256, Key: key}, (&jose.SignerOptions{}).WithType("JWT"))
	if err != nil {
		t.Fatal(err)
	}
	now := time.Now()
	payload, _ := json.Marshal(map[string]any{
		"iss": issuer, "sub": userA, "aud": []string{auth.Audience}, "azp": "haryx-cli", "typ": "Bearer",
		"sid": sid, "iat": now.Unix(), "exp": now.Add(5 * time.Minute).Unix(),
	})
	jws, err := signer.Sign(payload)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := jws.CompactSerialize()
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

// signedIn stays honest: a row whose sid is revoked says so.
func TestSignedInFollowsRevocation(t *testing.T) {
	e := newEnv(t)
	e.grant(macA)
	e.machine(e.do(macA, "PUT", "/api/v1/machines/self", beat(mac, "Work MacBook")))
	if _, err := e.pool.Exec(context.Background(), `insert into revoked_sessions values ($1, now() + interval '5 minutes')`, sidMac); err != nil {
		t.Fatal(err)
	}
	if l := e.list(webA); len(l) != 1 || l[0].SignedIn {
		t.Fatalf("revoked machine: %+v", l)
	}
}

// The database refuses a machine row without the consent, even past the gate.
func TestDatabaseRules(t *testing.T) {
	e := newEnv(t)
	_, err := e.pool.Exec(context.Background(), `insert into machines values ($1, $2, 'x', 'macos', '1', 's', now())`, userA, mac)
	var pgErr *pgconn.PgError
	if !errors.As(err, &pgErr) || pgErr.ConstraintName != "consent_required" {
		t.Fatalf("insert without consent: %v", err)
	}
	ungated := http.HandlerFunc((&handler{pool: e.pool}).heartbeat)
	e.expect(send(ungated, &macA, "PUT", "/api/v1/machines/self", beat(mac, "x")), http.StatusForbidden, "consent_required")
}

func TestBadRequests(t *testing.T) {
	e := newEnv(t)
	e.grant(macA)
	self := "/api/v1/machines/self"
	for _, c := range []struct {
		body any
		code string
	}{
		{"{", "invalid_json"},
		{strings.Repeat(" ", maxRequest+1), "invalid_json"},
		{beat("", "x"), "invalid_machine_id"},
		{beat(strings.ToUpper(mac), "x"), "invalid_machine_id"},
		{beat("not-a-uuid", "x"), "invalid_machine_id"},
		{beat(mac, ""), "invalid_name"},
		{beat(mac, "   "), "invalid_name"},
		{beat(mac, "a\tb"), "invalid_name"},
		{beat(mac, "a\x00b"), "invalid_name"},
		{beat(mac, strings.Repeat("é", maxName+1)), "invalid_name"},
		{map[string]string{"machineId": mac, "name": "x", "os": "ios", "version": "1"}, "invalid_os"},
		{map[string]string{"machineId": mac, "name": "x", "os": "macos", "version": ""}, "invalid_version"},
		{map[string]string{"machineId": mac, "name": "x", "os": "macos", "version": "1 2"}, "invalid_version"},
		{map[string]string{"machineId": mac, "name": "x", "os": "macos", "version": strings.Repeat("1", 33)}, "invalid_version"},
	} {
		e.expect(e.do(macA, "PUT", self, c.body), http.StatusBadRequest, c.code)
	}
	if m := e.machine(e.do(macA, "PUT", self, beat(mac, strings.Repeat("é", maxName)))); m.Name != strings.Repeat("é", maxName) {
		t.Fatalf("64-character name: %+v", m)
	}
	e.expect(e.do(macA, "PATCH", "/api/v1/machines/"+mac, map[string]string{"name": ""}), http.StatusBadRequest, "invalid_name")
	e.expect(e.do(macA, "PATCH", "/api/v1/machines/"+mac, "["), http.StatusBadRequest, "invalid_json")
	e.expect(e.do(macA, "PATCH", "/api/v1/machines/xyz", map[string]string{"name": "a"}), http.StatusBadRequest, "invalid_machine_id")
	e.expect(e.do(webA, "DELETE", "/api/v1/machines/xyz/session", nil), http.StatusBadRequest, "invalid_machine_id")
	e.expect(e.do(macA, "PATCH", "/api/v1/machines/"+pc, map[string]string{"name": "a"}), http.StatusNotFound, "not_found")
	e.expect(e.do(macA, "POST", "/api/v1/machines", nil), http.StatusNotFound, "not_found")
	for _, c := range []struct{ method, path string }{
		{"GET", "/api/v1/machines"}, {"PUT", self}, {"PATCH", "/api/v1/machines/" + mac}, {"DELETE", "/api/v1/machines/" + mac + "/session"},
	} {
		if rec := send(e.h, nil, c.method, c.path, nil); rec.Code != http.StatusUnauthorized {
			t.Errorf("%s %s without a user: %d, want 401", c.method, c.path, rec.Code)
		}
	}
}

func TestDatabaseGone(t *testing.T) {
	e := newEnv(t)
	e.grant(macA)
	e.pool.Close()
	h := &handler{pool: e.pool}
	e.expect(e.do(macA, "GET", "/api/v1/machines", nil), http.StatusInternalServerError, "internal")
	e.expect(send(http.HandlerFunc(h.heartbeat), &macA, "PUT", "/", beat(mac, "x")), http.StatusInternalServerError, "internal")
	e.expect(e.do(macA, "PATCH", "/api/v1/machines/"+mac, map[string]string{"name": "a"}), http.StatusInternalServerError, "internal")
	e.expect(e.do(webA, "DELETE", "/api/v1/machines/"+mac+"/session", nil), http.StatusInternalServerError, "internal")
}
