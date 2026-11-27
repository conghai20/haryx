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

const userA, userB = "3f0c6a52-6a3e-4d2b-9a57-0b7d1d6f9e21", "0d5e8f8e-1111-4c2b-8a57-0b7d1d6f9e22"

func lastSeen(t *testing.T, pool *pgxpool.Pool, user string) time.Time {
	t.Helper()
	var at time.Time
	if err := pool.QueryRow(context.Background(), `select last_seen from users where user_id = $1`, user).Scan(&at); err != nil {
		t.Fatal(err)
	}
	return at
}

func setLastSeen(t *testing.T, pool *pgxpool.Pool, user, ago string) {
	t.Helper()
	if _, err := pool.Exec(context.Background(), `update users set last_seen = now() - $2::interval where user_id = $1`, user, ago); err != nil {
		t.Fatal(err)
	}
}

func TestTouchUser(t *testing.T) {
	pool := testPool(t)
	ctx := context.Background()
	if err := Migrate(ctx, pool); err != nil {
		t.Fatal(err)
	}
	store := NewStore(pool)

	if err := store.TouchUser(ctx, userA); err != nil {
		t.Fatal(err)
	}
	setLastSeen(t, pool, userA, "10 minutes")
	before := lastSeen(t, pool, userA)
	if err := store.TouchUser(ctx, userA); err != nil {
		t.Fatal(err)
	}
	if got := lastSeen(t, pool, userA); !got.Equal(before) {
		t.Errorf("touch within the hour wrote last_seen: %v → %v", before, got)
	}
	setLastSeen(t, pool, userA, "2 hours")
	stale := lastSeen(t, pool, userA)
	if err := store.TouchUser(ctx, userA); err != nil {
		t.Fatal(err)
	}
	if got := lastSeen(t, pool, userA); !got.After(stale.Add(time.Hour)) {
		t.Errorf("touch after two hours did not move last_seen: %v → %v", stale, got)
	}
	if err := store.TouchUser(ctx, "not-a-uuid"); err == nil {
		t.Error("touch accepted a user id that is not a uuid")
	}
}

func TestDeleteInactiveUsers(t *testing.T) {
	pool := testPool(t)
	ctx := context.Background()
	if err := Migrate(ctx, pool); err != nil {
		t.Fatal(err)
	}
	store := NewStore(pool)
	for _, u := range []string{userA, userB} {
		if err := store.TouchUser(ctx, u); err != nil {
			t.Fatal(err)
		}
	}
	setLastSeen(t, pool, userA, "366 days")
	setLastSeen(t, pool, userB, "364 days")

	n, err := store.DeleteInactiveUsers(ctx, 365*24*time.Hour)
	if err != nil || n != 1 {
		t.Fatalf("DeleteInactiveUsers = %d, %v; want 1", n, err)
	}
	var left []string
	rows, _ := pool.Query(ctx, `select user_id::text from users`)
	for rows.Next() {
		var u string
		_ = rows.Scan(&u)
		left = append(left, u)
	}
	if len(left) != 1 || left[0] != userB {
		t.Fatalf("users left %v, want only %s", left, userB)
	}
}

// Expiry erases a user only if every table that holds their data cascades from users. This
// holds the tables tasks 6–8 add to the rule.
func TestEveryUserTableCascadesFromUsers(t *testing.T) {
	pool := testPool(t)
	ctx := context.Background()
	if err := Migrate(ctx, pool); err != nil {
		t.Fatal(err)
	}

	// The check itself: it must flag a table without the cascade, and pass one with it.
	if _, err := pool.Exec(ctx, `
		create table probe_plain (user_id uuid);
		create table probe_restrict (user_id uuid references users);
		create table probe_cascade (user_id uuid references users on delete cascade)`); err != nil {
		t.Fatal(err)
	}
	got := tablesNotCascading(t, pool)
	if len(got) != 2 || got[0] != "probe_plain" || got[1] != "probe_restrict" {
		t.Fatalf("check flagged %v, want [probe_plain probe_restrict]", got)
	}
	if _, err := pool.Exec(ctx, `drop table probe_plain, probe_restrict, probe_cascade`); err != nil {
		t.Fatal(err)
	}

	if bad := tablesNotCascading(t, pool); len(bad) > 0 {
		t.Errorf("user_id without references users on delete cascade, so expiry would leave rows: %v", bad)
	}
}

func tablesNotCascading(t *testing.T, pool *pgxpool.Pool) []string {
	t.Helper()
	rows, err := pool.Query(context.Background(), `
		select c.table_name
		from information_schema.columns c
		where c.table_schema = 'public' and c.column_name = 'user_id' and c.table_name <> 'users'
		  and not exists(
		      select 1
		      from information_schema.key_column_usage k
		      join information_schema.referential_constraints r using (constraint_schema, constraint_name)
		      join information_schema.constraint_column_usage u
		        on u.constraint_schema = r.unique_constraint_schema and u.constraint_name = r.unique_constraint_name
		      where k.table_schema = c.table_schema and k.table_name = c.table_name
		        and k.column_name = 'user_id' and u.table_name = 'users' and r.delete_rule = 'CASCADE')
		order by 1`)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	var tables []string
	for rows.Next() {
		var table string
		if err := rows.Scan(&table); err != nil {
			t.Fatal(err)
		}
		tables = append(tables, table)
	}
	return tables
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
