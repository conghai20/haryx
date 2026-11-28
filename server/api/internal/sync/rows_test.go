package sync

import (
	"context"
	"fmt"
	"net/http"
	"strings"
	gosync "sync"
	"testing"
	"time"
)

func row(kind, key, body string) map[string]string {
	return map[string]string{"kind": kind, "key": key, "body": body}
}

func TestRowLifecycle(t *testing.T) {
	e := newEnv(t)

	before := time.Now().Add(-time.Minute)
	rec := e.do(userA, "POST", "/api/v1/sync/rows", row("settings", "", "YWdlLWE="))
	e.expect(rec, http.StatusCreated, "")
	created := decode[Row](t, rec)
	if created.Kind != "settings" || created.Key != "" || created.Body != "YWdlLWE=" || created.Version != 1 || created.UpdatedAt.Before(before) {
		t.Fatalf("created %+v", created)
	}
	if !strings.Contains(rec.Body.String(), `"updatedAt":"`) {
		t.Fatalf("field names: %s", rec.Body)
	}
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("settings", "", "b3RoZXI=")), http.StatusConflict, "exists")
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("usage", "ab12", "eA==")), http.StatusCreated, "")

	// The empty key is addressed with a trailing slash.
	rec = e.do(userA, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "Yg=="}, "If-Match", "1")
	e.expect(rec, http.StatusOK, "")
	if got := decode[Row](t, rec); got.Version != 2 || got.Body != "Yg==" {
		t.Fatalf("updated %+v", got)
	}
	// A quoted ETag-style version is read the same.
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "Yw=="}, "If-Match", `"2"`), http.StatusOK, "")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "eg=="}, "If-Match", "2"), http.StatusPreconditionFailed, "version_mismatch")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "eg=="}, "If-Match", "9"), http.StatusPreconditionFailed, "version_mismatch")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "eg=="}), http.StatusPreconditionRequired, "if_match_required")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "eg=="}, "If-Match", "*"), http.StatusBadRequest, "invalid_if_match")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "eg=="}, "If-Match", "0"), http.StatusBadRequest, "invalid_if_match")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/account/ff", map[string]string{"body": "eg=="}, "If-Match", "1"), http.StatusNotFound, "not_found")

	rec = e.do(userA, "GET", "/api/v1/sync/rows", nil)
	e.expect(rec, http.StatusOK, "")
	list := decode[[]Row](t, rec)
	if len(list) != 2 || list[0].Kind != "settings" || list[0].Version != 3 || list[0].Body != "Yw==" || list[1].Kind != "usage" {
		t.Fatalf("list %+v", list)
	}
	rec = e.do(userA, "GET", "/api/v1/sync/rows?kind=usage", nil)
	if got := decode[[]Row](t, rec); len(got) != 1 || got[0].Key != "ab12" {
		t.Fatalf("list kind %+v", got)
	}
	rec = e.do(userA, "GET", "/api/v1/sync/rows?kind=keyring", nil)
	if rec.Body.String() != "[]\n" {
		t.Fatalf("empty list is %q, want []", rec.Body)
	}

	e.expect(e.do(userA, "DELETE", "/api/v1/sync/rows/usage/ab12", nil), http.StatusNoContent, "")
	e.expect(e.do(userA, "DELETE", "/api/v1/sync/rows/usage/ab12", nil), http.StatusNoContent, "")
	if n := e.count("sync_rows", userA); n != 1 {
		t.Fatalf("%d rows after deleting one of two", n)
	}
	e.expect(e.do(userA, "DELETE", "/api/v1/sync/rows", nil), http.StatusNoContent, "")
	if n := e.count("sync_rows", userA); n != 0 {
		t.Fatalf("%d rows after delete all", n)
	}
}

func TestRowValidation(t *testing.T) {
	e := newEnv(t)
	full := strings.Repeat("A", 262144)
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("account", strings.Repeat("f", 64), full)), http.StatusCreated, "")

	for _, c := range []struct {
		name   string
		body   any
		status int
		code   string
	}{
		{"unknown kind", row("secrets", "", "eA=="), 400, "invalid_kind"},
		{"no kind", map[string]string{"body": "eA=="}, 400, "invalid_kind"},
		{"readable key", row("account", "You@Example.com", "eA=="), 400, "invalid_key"},
		{"uppercase key", row("account", "AB", "eA=="), 400, "invalid_key"},
		{"long key", row("account", strings.Repeat("a", 65), "eA=="), 400, "invalid_key"},
		{"empty body", row("settings", "", ""), 400, "invalid_body"},
		{"NUL body", row("settings", "", "a\x00b"), 400, "invalid_body"},
		{"body over 256 KB", row("settings", "", full+"A"), 413, "body_too_large"},
		{"request over 1 MB", row("settings", "", strings.Repeat("A", 1<<20)), 413, "body_too_large"},
		{"not JSON", "kind=settings", 400, "invalid_json"},
	} {
		t.Run(c.name, func(t *testing.T) {
			e.t = t
			e.expect(e.do(userA, "POST", "/api/v1/sync/rows", c.body), c.status, c.code)
		})
	}
	e.t = t

	e.expect(e.do(userA, "GET", "/api/v1/sync/rows?kind=secrets", nil), 400, "invalid_kind")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/secrets/", map[string]string{"body": "eA=="}, "If-Match", "1"), 400, "invalid_kind")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/account/a/b", map[string]string{"body": "eA=="}, "If-Match", "1"), 400, "invalid_key")
	e.expect(e.do(userA, "PUT", "/api/v1/sync/rows/account/"+strings.Repeat("f", 64), map[string]string{"body": ""}, "If-Match", "1"), 400, "invalid_body")
	e.expect(e.do(userA, "DELETE", "/api/v1/sync/rows/secrets/", nil), 400, "invalid_kind")
	if n := e.count("sync_rows", userA); n != 1 {
		t.Fatalf("%d rows; only the full-size one should be stored", n)
	}
}

// User B never reads, changes or deletes A's rows, through any endpoint.
func TestRowIsolation(t *testing.T) {
	e := newEnv(t)
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("settings", "", "YQ==")), http.StatusCreated, "")
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("account", "ab", "YQ==")), http.StatusCreated, "")

	if rec := e.do(userB, "GET", "/api/v1/sync/rows", nil); rec.Body.String() != "[]\n" {
		t.Fatalf("B lists %s", rec.Body)
	}
	if rec := e.do(userB, "GET", "/api/v1/sync/rows?kind=settings", nil); rec.Body.String() != "[]\n" {
		t.Fatalf("B lists kind %s", rec.Body)
	}
	e.expect(e.do(userB, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "aGFjaw=="}, "If-Match", "1"), http.StatusNotFound, "not_found")
	e.expect(e.do(userB, "DELETE", "/api/v1/sync/rows/settings/", nil), http.StatusNoContent, "")
	e.expect(e.do(userB, "DELETE", "/api/v1/sync/rows", nil), http.StatusNoContent, "")
	// B's own row with the same kind and key is B's, and A's stays as it was.
	e.expect(e.do(userB, "POST", "/api/v1/sync/rows", row("settings", "", "Qg==")), http.StatusCreated, "")
	e.expect(e.do(userB, "PUT", "/api/v1/sync/rows/settings/", map[string]string{"body": "QjI="}, "If-Match", "1"), http.StatusOK, "")

	rec := e.do(userA, "GET", "/api/v1/sync/rows", nil)
	list := decode[[]Row](t, rec)
	if len(list) != 2 || list[1].Kind != "settings" || list[1].Body != "YQ==" || list[1].Version != 1 {
		t.Fatalf("A's rows after B's attempts: %+v", list)
	}
}

// Many machines updating from the same version: exactly one wins, the rest get 412.
func TestConcurrentUpdates(t *testing.T) {
	e := newEnv(t)
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("usage", "aa", "MA==")), http.StatusCreated, "")

	const n = 10
	codes := make(chan int, n)
	var wg gosync.WaitGroup
	for i := range n {
		wg.Go(func() {
			codes <- e.do(userA, "PUT", "/api/v1/sync/rows/usage/aa", map[string]string{"body": fmt.Sprintf("%d", i)}, "If-Match", "1").Code
		})
	}
	wg.Wait()
	close(codes)
	won := 0
	for c := range codes {
		switch c {
		case http.StatusOK:
			won++
		case http.StatusPreconditionFailed:
		default:
			t.Fatalf("status %d", c)
		}
	}
	var version int64
	_ = e.pool.QueryRow(context.Background(), `select version from sync_rows where user_id = $1`, userA).Scan(&version)
	if won != 1 || version != 2 {
		t.Fatalf("%d updates won, version %d; want 1 and 2", won, version)
	}
}

// 500 rows per user, held under concurrent inserts; a duplicate at the cap is still 409.
func TestRowLimit(t *testing.T) {
	e := newEnv(t)
	if _, err := e.pool.Exec(context.Background(), `insert into sync_rows (user_id, kind, key, body)
		select $1, 'account', lpad(to_hex(n), 64, '0'), 'eA==' from generate_series(1, 499) as n`, userA); err != nil {
		t.Fatal(err)
	}
	const n = 5
	codes := make(chan int, n)
	var wg gosync.WaitGroup
	for i := range n {
		wg.Go(func() {
			codes <- e.do(userA, "POST", "/api/v1/sync/rows", row("usage", fmt.Sprintf("%02x", i), "eA==")).Code
		})
	}
	wg.Wait()
	close(codes)
	created := 0
	for c := range codes {
		switch c {
		case http.StatusCreated:
			created++
		case http.StatusUnprocessableEntity:
		default:
			t.Fatalf("status %d", c)
		}
	}
	if created != 1 || e.count("sync_rows", userA) != 500 {
		t.Fatalf("%d created, %d rows; want 1 and 500", created, e.count("sync_rows", userA))
	}
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("settings", "", "eA==")), http.StatusUnprocessableEntity, "row_limit")
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("account", strings.Repeat("0", 63)+"1", "eA==")), http.StatusConflict, "exists")
	// The cap is per user.
	e.expect(e.do(userB, "POST", "/api/v1/sync/rows", row("settings", "", "eA==")), http.StatusCreated, "")
}

// Expiring a user (plan A10) takes their rows and files with them.
func TestUserDeletionCascades(t *testing.T) {
	e := newEnv(t)
	e.expect(e.do(userA, "POST", "/api/v1/sync/rows", row("settings", "", "eA==")), http.StatusCreated, "")
	e.expect(e.do(userA, "PUT", "/api/v1/profiles/ab", "x"), http.StatusOK, "")
	if _, err := e.pool.Exec(context.Background(), `delete from users where user_id = $1`, userA); err != nil {
		t.Fatal(err)
	}
	if e.count("sync_rows", userA) != 0 || e.count("profile_files", userA) != 0 {
		t.Fatal("deleting the user left data behind")
	}
}
