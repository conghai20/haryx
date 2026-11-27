package db

import (
	"context"
	"os"
	"testing"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
)

// testPool connects to the Postgres named by the libpq variables. These tests need a real one:
// they run in `server/api/test.sh`, and are skipped when PGHOST is not set.
func testPool(t *testing.T) *pgxpool.Pool {
	t.Helper()
	if os.Getenv("PGHOST") == "" {
		t.Skip("PGHOST not set: run server/api/test.sh")
	}
	ctx := context.Background()
	pool, err := Open(ctx)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(pool.Close)
	if _, err := pool.Exec(ctx, `drop schema public cascade; create schema public`); err != nil {
		t.Fatal(err)
	}
	return pool
}

func TestMigrateIsIdempotentAndConcurrent(t *testing.T) {
	pool := testPool(t)
	ctx := context.Background()

	errs := make(chan error, 3)
	for range 3 {
		go func() { errs <- Migrate(ctx, pool) }()
	}
	for range 3 {
		if err := <-errs; err != nil {
			t.Fatal(err)
		}
	}
	if err := Migrate(ctx, pool); err != nil {
		t.Fatal(err)
	}
	var applied int
	if err := pool.QueryRow(ctx, `select count(*) from schema_migrations`).Scan(&applied); err != nil {
		t.Fatal(err)
	}
	files, _ := migrations.ReadDir("migrations")
	if applied != len(files) {
		t.Fatalf("schema_migrations has %d rows, want %d", applied, len(files))
	}
}

func TestRevokedSessions(t *testing.T) {
	pool := testPool(t)
	ctx := context.Background()
	if err := Migrate(ctx, pool); err != nil {
		t.Fatal(err)
	}
	store := NewStore(pool)
	if _, err := pool.Exec(ctx, `insert into revoked_sessions values
		('live', now() + interval '10 minutes'), ('stale', now() - interval '1 minute')`); err != nil {
		t.Fatal(err)
	}

	for sid, want := range map[string]bool{"live": true, "stale": false, "never": false} {
		got, err := store.IsRevoked(ctx, sid)
		if err != nil || got != want {
			t.Errorf("IsRevoked(%q) = %v, %v; want %v", sid, got, err, want)
		}
	}

	n, err := store.DeleteExpiredRevocations(ctx)
	if err != nil || n != 1 {
		t.Fatalf("DeleteExpiredRevocations = %d, %v; want 1", n, err)
	}
	if got, _ := store.IsRevoked(ctx, "live"); !got {
		t.Fatal("the sweep removed a live revocation")
	}
}

func TestIsRevokedFailsWhenDatabaseIsGone(t *testing.T) {
	pool := testPool(t)
	store := NewStore(pool)
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	pool.Close()
	if _, err := store.IsRevoked(ctx, "any"); err == nil {
		t.Fatal("IsRevoked returned no error on a closed pool; auth would fail open")
	}
}
