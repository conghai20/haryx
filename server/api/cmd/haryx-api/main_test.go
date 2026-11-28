package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os"
	"syscall"
	"testing"
	"time"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/db"
)

func TestRoutes(t *testing.T) {
	if os.Getenv("PGHOST") == "" {
		t.Skip("PGHOST not set: run server/api/test.sh")
	}
	ctx := context.Background()
	pool, err := db.Open(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer pool.Close()
	if err := db.Migrate(ctx, pool); err != nil {
		t.Fatal(err)
	}
	store := db.NewStore(pool)
	// Nothing listens on the issuer: no request below gets as far as a key lookup.
	h := routes(pool, store, auth.NewRemote("http://127.0.0.1:1/realms/wecoap", store.IsRevoked))

	get := func(path, authz string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodGet, path, nil)
		if authz != "" {
			req.Header.Set("Authorization", authz)
		}
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		return rec
	}

	if rec := get("/api/v1/health", ""); rec.Code != http.StatusOK || rec.Body.String() != "{\"status\":\"ok\"}\n" {
		t.Errorf("health: %d %s", rec.Code, rec.Body)
	}
	for _, path := range []string{"/api/v1/sync/rows", "/api/v1/machines", "/api/v1/me/export", "/api/v1/anything", "/api/v1/"} {
		if rec := get(path, ""); rec.Code != http.StatusUnauthorized {
			t.Errorf("%s without a token: %d, want 401", path, rec.Code)
		}
	}
	if rec := get("/api/v1/sync/rows", "Bearer x.y.z"); rec.Code != http.StatusUnauthorized {
		t.Errorf("bad token: %d, want 401", rec.Code)
	}
	if rec := get("/elsewhere", ""); rec.Code != http.StatusNotFound {
		t.Errorf("outside /api/v1/: %d, want 404", rec.Code)
	}

	// An authenticated request creates the caller's users row before its handler runs.
	const caller = "3f0c6a52-6a3e-4d2b-9a57-0b7d1d6f9e21"
	reached := false
	touched := touchUser(store, http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		var n int
		_ = pool.QueryRow(ctx, `select count(*) from users where user_id = $1`, caller).Scan(&n)
		reached = n == 1
	}))
	authed := func() *httptest.ResponseRecorder {
		req := httptest.NewRequest(http.MethodGet, "/api/v1/sync/rows", nil)
		req = req.WithContext(auth.WithUser(req.Context(), auth.User{ID: caller}))
		rec := httptest.NewRecorder()
		touched.ServeHTTP(rec, req)
		return rec
	}
	if rec := authed(); rec.Code != http.StatusOK || !reached {
		t.Errorf("touch: %d, handler saw the users row: %v", rec.Code, reached)
	}

	// A user idle past the retention goes, with a 364-day-idle one kept.
	const recent = "0d5e8f8e-1111-4c2b-8a57-0b7d1d6f9e22"
	if _, err := pool.Exec(ctx, `update users set last_seen = now() - interval '400 days';
		insert into users (user_id, last_seen) values ('`+recent+`', now() - interval '364 days')`); err != nil {
		t.Fatal(err)
	}
	sweepOnce(ctx, store)
	var users int
	_ = pool.QueryRow(ctx, `select count(*) from users where user_id = $1`, recent).Scan(&users)
	if users != 1 {
		t.Error("sweep deleted a user idle for 364 days")
	}
	if _, err := pool.Exec(ctx, `delete from users where user_id = $1`, recent); err != nil {
		t.Fatal(err)
	}
	// The loop itself returns once its context is done.
	stopped, cancel := context.WithCancel(ctx)
	cancel()
	sweep(stopped, store)
	_ = pool.QueryRow(ctx, `select count(*) from users`).Scan(&users)
	if users != 0 {
		t.Errorf("sweep left %d users idle for 400 days", users)
	}

	pool.Close()
	if rec := get("/api/v1/health", ""); rec.Code != http.StatusServiceUnavailable {
		t.Errorf("health with the database gone: %d, want 503", rec.Code)
	}
	reached = false
	if rec := authed(); rec.Code != http.StatusServiceUnavailable || reached {
		t.Errorf("touch with the database gone: %d, handler ran: %v", rec.Code, reached)
	}
}

// The whole binary: migrate, listen on :8080, answer health, and stop cleanly on SIGTERM.
func TestRunServesAndShutsDown(t *testing.T) {
	if os.Getenv("PGHOST") == "" {
		t.Skip("PGHOST not set: run server/api/test.sh")
	}
	t.Setenv("HARYX_SSO_ISSUER", "http://127.0.0.1:1/realms/wecoap")
	done := make(chan error, 1)
	go func() { done <- run() }()

	deadline := time.Now().Add(10 * time.Second)
	for {
		res, err := http.Get("http://127.0.0.1:8080/api/v1/health")
		if err == nil {
			res.Body.Close()
			if res.StatusCode != http.StatusOK {
				t.Fatalf("health: %d", res.StatusCode)
			}
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("never answered: %v", err)
		}
		time.Sleep(50 * time.Millisecond)
	}
	// The handler is installed before the listener, so the signal cannot kill the test binary.
	if err := syscall.Kill(os.Getpid(), syscall.SIGTERM); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("run: %v", err)
		}
	case <-time.After(15 * time.Second):
		t.Fatal("no shutdown after SIGTERM")
	}
}

func TestRunNeedsIssuer(t *testing.T) {
	t.Setenv("HARYX_SSO_ISSUER", "")
	if err := run(); err == nil {
		t.Fatal("run started without HARYX_SSO_ISSUER")
	}
}
