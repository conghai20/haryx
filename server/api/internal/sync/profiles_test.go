package sync

import (
	"bytes"
	"fmt"
	"net/http"
	"strings"
	gosync "sync"
	"testing"
)

func TestProfileLifecycle(t *testing.T) {
	e := newEnv(t)
	name := strings.Repeat("ab", 32)
	path := "/api/v1/profiles/" + name

	rec := e.do(userA, "PUT", path, []byte("cipher-1"))
	e.expect(rec, http.StatusOK, "")
	if got := decode[Profile](t, rec); got.Name != name || got.Bytes != 8 || got.UpdatedAt.IsZero() {
		t.Fatalf("put %+v", got)
	}
	rec = e.do(userA, "GET", path, nil)
	if rec.Code != http.StatusOK || rec.Body.String() != "cipher-1" || rec.Header().Get("Content-Type") != "application/octet-stream" {
		t.Fatalf("get: %d %q %q", rec.Code, rec.Body, rec.Header().Get("Content-Type"))
	}
	e.expect(e.do(userA, "PUT", path, []byte("cipher-two")), http.StatusOK, "")
	if rec := e.do(userA, "GET", path, nil); rec.Body.String() != "cipher-two" {
		t.Fatalf("replaced file reads %q", rec.Body)
	}
	e.expect(e.do(userA, "PUT", "/api/v1/profiles/cd", []byte{}), http.StatusOK, "")

	rec = e.do(userA, "GET", "/api/v1/profiles", nil)
	list := decode[[]Profile](t, rec)
	if len(list) != 2 || list[0].Name != name || list[0].Bytes != 10 || list[1].Name != "cd" || list[1].Bytes != 0 {
		t.Fatalf("list %+v", list)
	}

	e.expect(e.do(userA, "DELETE", path, nil), http.StatusNoContent, "")
	e.expect(e.do(userA, "DELETE", path, nil), http.StatusNoContent, "")
	e.expect(e.do(userA, "GET", path, nil), http.StatusNotFound, "not_found")

	for _, bad := range []string{"AB", "profile.json", strings.Repeat("a", 65)} {
		e.expect(e.do(userA, "PUT", "/api/v1/profiles/"+bad, "x"), http.StatusBadRequest, "invalid_name")
		e.expect(e.do(userA, "GET", "/api/v1/profiles/"+bad, nil), http.StatusBadRequest, "invalid_name")
		e.expect(e.do(userA, "DELETE", "/api/v1/profiles/"+bad, nil), http.StatusBadRequest, "invalid_name")
	}
	// Subfolders do not exist.
	e.expect(e.do(userA, "PUT", "/api/v1/profiles/ab/cd", "x"), http.StatusNotFound, "not_found")
}

func TestProfileSize(t *testing.T) {
	e := newEnv(t)
	full := bytes.Repeat([]byte{7}, 10<<20)
	rec := e.do(userA, "PUT", "/api/v1/profiles/aa", full)
	e.expect(rec, http.StatusOK, "")
	e.expect(e.do(userA, "PUT", "/api/v1/profiles/bb", append(full, 7)), http.StatusRequestEntityTooLarge, "file_too_large")
	if rec := e.do(userA, "GET", "/api/v1/profiles/aa", nil); rec.Body.Len() != 10<<20 {
		t.Fatalf("10 MB file reads back as %d bytes", rec.Body.Len())
	}
	if e.count("profile_files", userA) != 1 {
		t.Fatal("the oversized file was stored")
	}
}

func TestProfileIsolation(t *testing.T) {
	e := newEnv(t)
	e.expect(e.do(userA, "PUT", "/api/v1/profiles/ab", "mine"), http.StatusOK, "")

	if rec := e.do(userB, "GET", "/api/v1/profiles", nil); rec.Body.String() != "[]\n" {
		t.Fatalf("B lists %s", rec.Body)
	}
	e.expect(e.do(userB, "GET", "/api/v1/profiles/ab", nil), http.StatusNotFound, "not_found")
	e.expect(e.do(userB, "DELETE", "/api/v1/profiles/ab", nil), http.StatusNoContent, "")
	// B's file with the same name is B's own.
	e.expect(e.do(userB, "PUT", "/api/v1/profiles/ab", "theirs"), http.StatusOK, "")

	if rec := e.do(userA, "GET", "/api/v1/profiles/ab", nil); rec.Body.String() != "mine" {
		t.Fatalf("A's file reads %q after B's attempts", rec.Body)
	}
}

// 10 files per user, held under concurrent uploads; replacing one at the cap still works.
func TestProfileLimit(t *testing.T) {
	e := newEnv(t)
	const n = 15
	codes := make(chan int, n)
	var wg gosync.WaitGroup
	for i := range n {
		wg.Go(func() {
			codes <- e.do(userA, "PUT", fmt.Sprintf("/api/v1/profiles/%02x", i), "x").Code
		})
	}
	wg.Wait()
	close(codes)
	stored := 0
	for c := range codes {
		switch c {
		case http.StatusOK:
			stored++
		case http.StatusUnprocessableEntity:
		default:
			t.Fatalf("status %d", c)
		}
	}
	if stored != 10 || e.count("profile_files", userA) != 10 {
		t.Fatalf("%d stored, %d files; want 10", stored, e.count("profile_files", userA))
	}
	e.expect(e.do(userA, "PUT", "/api/v1/profiles/ffff", "x"), http.StatusUnprocessableEntity, "profile_limit")
	list := decode[[]Profile](t, e.do(userA, "GET", "/api/v1/profiles", nil))
	e.expect(e.do(userA, "PUT", "/api/v1/profiles/"+list[0].Name, "replaced"), http.StatusOK, "")
	e.expect(e.do(userB, "PUT", "/api/v1/profiles/ffff", "x"), http.StatusOK, "")
}
