package export

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/consent"
	"github.com/conghai20/haryx/server/api/internal/db"
	"github.com/conghai20/haryx/server/api/internal/machines"
	syncapi "github.com/conghai20/haryx/server/api/internal/sync"
)

const (
	userA, userB = "3f0c6a52-6a3e-4d2b-9a57-0b7d1d6f9e21", "0d5e8f8e-1111-4c2b-8a57-0b7d1d6f9e22"
	machineA     = "9b2e5c1a-4f0d-4a67-8d3e-1c7a5b0e2f91"
	machineB     = "c41d7e08-2b9a-4e35-a6f1-70d8e3b94a52"
)

var (
	cliA = auth.User{ID: userA, SessionID: "sid-a", Client: "haryx-cli"}
	webA = auth.User{ID: userA, SessionID: "sid-web-a", Client: "haryx-platform"}
	cliB = auth.User{ID: userB, SessionID: "sid-b", Client: "haryx-cli"}
)

// newEnv is a fresh schema with users A and B, and the routes the export reads from.
func newEnv(t *testing.T) (*pgxpool.Pool, http.Handler) {
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
	mux.Handle("/api/v1/machines/", machines.Routes(pool))
	data := consent.Require(pool, consent.Sync, syncapi.Routes(pool))
	mux.Handle("/api/v1/sync/", data)
	mux.Handle("/api/v1/profiles/", data)
	mux.Handle("GET /api/v1/me/export", Handler(pool))
	return pool, mux
}

func do(t *testing.T, h http.Handler, user auth.User, method, path string, body any) *httptest.ResponseRecorder {
	t.Helper()
	var raw []byte
	switch b := body.(type) {
	case nil:
	case string:
		raw = []byte(b)
	default:
		raw, _ = json.Marshal(b)
	}
	req := httptest.NewRequest(method, path, bytes.NewReader(raw))
	req = req.WithContext(auth.WithUser(req.Context(), user))
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code >= 300 {
		t.Fatalf("%s %s: %d %s", method, path, rec.Code, rec.Body)
	}
	return rec
}

// fill gives the user data in every table: both consents, a sync row, a profile and a machine.
func fill(t *testing.T, h http.Handler, u auth.User, machine, secret string) {
	t.Helper()
	for _, p := range []string{"sync", "machines"} {
		do(t, h, u, "PUT", "/api/v1/consents/"+p, map[string]string{"version": "v1", "source": "cli"})
	}
	do(t, h, u, "POST", "/api/v1/sync/rows", map[string]string{"kind": "settings", "body": secret})
	do(t, h, u, "PUT", "/api/v1/profiles/ab", secret)
	do(t, h, u, "PUT", "/api/v1/machines/self",
		map[string]string{"machineId": machine, "name": secret, "os": "linux", "version": "1.0.0"})
}

func export(t *testing.T, h http.Handler, u auth.User) (Export, string) {
	t.Helper()
	rec := do(t, h, u, "GET", "/api/v1/me/export", nil)
	dec := json.NewDecoder(bytes.NewReader(rec.Body.Bytes()))
	dec.DisallowUnknownFields()
	var body Export
	if err := dec.Decode(&body); err != nil {
		t.Fatal(err)
	}
	want := `attachment; filename="haryx-export-` + time.Now().UTC().Format(time.DateOnly) + `.json"`
	if got := rec.Header().Get("Content-Disposition"); got != want {
		t.Errorf("Content-Disposition %q, want %q", got, want)
	}
	return body, rec.Body.String()
}

func TestExportHoldsEverythingOfTheCallerAndNothingElse(t *testing.T) {
	_, h := newEnv(t)
	fill(t, h, cliA, machineA, "secretOfA")
	fill(t, h, cliB, machineB, "secretOfB")
	do(t, h, cliA, "DELETE", "/api/v1/consents/sync?source=cli", nil) // history with a withdrawal
	do(t, h, cliA, "PUT", "/api/v1/consents/sync", map[string]string{"version": "v2", "source": "cli"})
	do(t, h, cliA, "POST", "/api/v1/sync/rows", map[string]string{"kind": "usage", "key": "aa", "body": "secretOfA"})
	do(t, h, cliA, "PUT", "/api/v1/profiles/cd", "secretOfA!")

	for _, u := range []auth.User{cliA, webA} {
		got, raw := export(t, h, u)
		if strings.Contains(raw, userB) || strings.Contains(raw, "secretOfB") || strings.Contains(raw, machineB) {
			t.Fatalf("A's export holds B's data: %s", raw)
		}
		if got.User.ID != userA || got.User.FirstSeen.IsZero() || got.User.LastSeen.Before(got.User.FirstSeen) ||
			got.ExportedAt.Before(got.User.LastSeen) {
			t.Errorf("user %+v at %v", got.User, got.ExportedAt)
		}
		if len(got.Consents) != 2 || !got.Consents[0].Granted || len(got.Consents[0].History) != 3 ||
			len(got.Consents[1].History) != 1 {
			t.Errorf("consents %+v", got.Consents)
		}
		if len(got.Machines) != 1 || got.Machines[0].MachineID != machineA || got.Machines[0].Name != "secretOfA" ||
			got.Machines[0].IsThisMachine != (u.Client == "haryx-cli") {
			t.Errorf("machines %+v", got.Machines)
		}
		if len(got.SyncRows) != 1 || got.SyncRows[0].Kind != "usage" || got.SyncRows[0].Body != "secretOfA" {
			// the settings row went with the sync withdrawal
			t.Errorf("sync rows %+v", got.SyncRows)
		}
		if len(got.Profiles) != 1 || got.Profiles[0] != (syncapi.Profile{Name: "cd", Bytes: 10, UpdatedAt: got.Profiles[0].UpdatedAt}) {
			t.Errorf("profiles %+v", got.Profiles)
		}
		if strings.Contains(raw, "secretOfA!") {
			t.Error("the export carries a profile file's contents")
		}
		if len(got.Notes) != len(Notes) || !strings.Contains(raw, "sso.wecoap.com/realms/wecoap/account") {
			t.Errorf("notes %v", got.Notes)
		}
	}

	got, raw := export(t, h, cliB)
	if strings.Contains(raw, userA) || strings.Contains(raw, "secretOfA") || strings.Contains(raw, machineA) ||
		len(got.SyncRows) != 1 || len(got.Machines) != 1 || len(got.Profiles) != 1 {
		t.Fatalf("B's export: %s", raw)
	}
}

func TestExportOfAnEmptyUserHasEmptyLists(t *testing.T) {
	_, h := newEnv(t)
	_, raw := export(t, h, cliA)
	for _, field := range []string{`"machines":[]`, `"syncRows":[]`, `"profiles":[]`, `"history":[]`} {
		if !strings.Contains(raw, field) {
			t.Errorf("no %s in %s", field, raw)
		}
	}
}

func TestExportFailures(t *testing.T) {
	pool, h := newEnv(t)
	for _, c := range []struct {
		user   *auth.User
		status int
	}{
		{nil, http.StatusUnauthorized},
		{&auth.User{ID: "5a1b2c3d-0000-4000-8000-000000000000", SessionID: "s", Client: "haryx-cli"}, http.StatusInternalServerError}, // no users row
	} {
		req := httptest.NewRequest("GET", "/api/v1/me/export", nil)
		if c.user != nil {
			req = req.WithContext(auth.WithUser(req.Context(), *c.user))
		}
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != c.status || rec.Header().Get("Content-Disposition") != "" {
			t.Errorf("%v: %d, want %d", c.user, rec.Code, c.status)
		}
	}
	pool.Close()
	req := httptest.NewRequest("GET", "/api/v1/me/export", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req.WithContext(auth.WithUser(req.Context(), cliA)))
	if rec.Code != http.StatusInternalServerError {
		t.Errorf("database gone: %d, want 500", rec.Code)
	}
}
