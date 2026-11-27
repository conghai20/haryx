package sync

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"

	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/db"
)

const userA, userB = "3f0c6a52-6a3e-4d2b-9a57-0b7d1d6f9e21", "0d5e8f8e-1111-4c2b-8a57-0b7d1d6f9e22"

// env is a fresh schema with users A and B, and the routes as the API mounts them.
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
	return &env{t: t, pool: pool, h: Routes(pool)}
}

// do sends a request as user ("" for none). body is a string, []byte, or anything to JSON-encode.
func (e *env) do(user, method, path string, body any, headers ...string) *httptest.ResponseRecorder {
	var r io.Reader
	switch b := body.(type) {
	case nil:
	case string:
		r = strings.NewReader(b)
	case []byte:
		r = bytes.NewReader(b)
	default:
		raw, err := json.Marshal(b)
		if err != nil {
			e.t.Fatal(err)
		}
		r = bytes.NewReader(raw)
	}
	req := httptest.NewRequest(method, path, r)
	for i := 0; i+1 < len(headers); i += 2 {
		req.Header.Set(headers[i], headers[i+1])
	}
	if user != "" {
		req = req.WithContext(auth.WithUser(req.Context(), auth.User{ID: user}))
	}
	rec := httptest.NewRecorder()
	e.h.ServeHTTP(rec, req)
	return rec
}

// expect checks the status and, for an error, the code.
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

func decode[T any](t *testing.T, rec *httptest.ResponseRecorder) T {
	t.Helper()
	var v T
	if err := json.Unmarshal(rec.Body.Bytes(), &v); err != nil {
		t.Fatalf("decode %s: %v", rec.Body, err)
	}
	return v
}

func (e *env) count(table, user string) int {
	e.t.Helper()
	var n int
	if err := e.pool.QueryRow(context.Background(), `select count(*) from `+table+` where user_id = $1`, user).Scan(&n); err != nil {
		e.t.Fatal(err)
	}
	return n
}

func TestNoUserAndUnknownPaths(t *testing.T) {
	e := newEnv(t)
	e.expect(e.do("", "GET", "/api/v1/sync/rows", nil), http.StatusUnauthorized, "unauthorized")
	e.expect(e.do(userA, "GET", "/api/v1/sync/elsewhere", nil), http.StatusNotFound, "not_found")
	e.expect(e.do(userA, "GET", "/api/v1/sync/rows/settings/", nil), http.StatusNotFound, "not_found")
}

// Every endpoint answers 500, not a partial result, when Postgres is gone.
func TestDatabaseGone(t *testing.T) {
	e := newEnv(t)
	e.pool.Close()
	for _, c := range []struct {
		method, path string
		body         any
		headers      []string
	}{
		{"GET", "/api/v1/sync/rows", nil, nil},
		{"POST", "/api/v1/sync/rows", map[string]string{"kind": "settings", "body": "eA=="}, nil},
		{"PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "eA=="}, []string{"If-Match", "1"}},
		{"DELETE", "/api/v1/sync/rows/settings/", nil, nil},
		{"DELETE", "/api/v1/sync/rows", nil, nil},
		{"GET", "/api/v1/profiles", nil, nil},
		{"PUT", "/api/v1/profiles/ab", "x", nil},
		{"GET", "/api/v1/profiles/ab", nil, nil},
		{"DELETE", "/api/v1/profiles/ab", nil, nil},
	} {
		rec := e.do(userA, c.method, c.path, c.body, c.headers...)
		if rec.Code != http.StatusInternalServerError {
			t.Errorf("%s %s: %d, want 500", c.method, c.path, rec.Code)
		}
	}
}
