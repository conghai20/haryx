// Package db is the API's Postgres: the pool, the embedded migrations, and revoked sessions.
package db

import (
	"context"
	"embed"
	"fmt"
	"io/fs"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgxpool"
)

//go:embed migrations/*.sql
var migrations embed.FS

// Any constant works; it only has to be the same for every instance of the API.
const migrateLock = 0x68617279 // "hary"

// Open connects with the libpq variables (PGHOST, PGUSER, PGPASSWORD…) that compose sets.
func Open(ctx context.Context) (*pgxpool.Pool, error) {
	pool, err := pgxpool.New(ctx, "")
	if err != nil {
		return nil, fmt.Errorf("postgres config: %w", err)
	}
	return pool, nil
}

// Migrate applies, in name order, every migrations/*.sql not yet recorded in schema_migrations.
// All of them run in one transaction under an advisory lock: two instances starting together
// apply each file once, and a failing file leaves the schema as it was.
func Migrate(ctx context.Context, pool *pgxpool.Pool) error {
	return pgx.BeginFunc(ctx, pool, func(tx pgx.Tx) error {
		if _, err := tx.Exec(ctx, `select pg_advisory_xact_lock($1)`, migrateLock); err != nil {
			return fmt.Errorf("migration lock: %w", err)
		}
		if _, err := tx.Exec(ctx, `create table if not exists schema_migrations (
			name text primary key, applied_at timestamptz not null default now())`); err != nil {
			return fmt.Errorf("schema_migrations: %w", err)
		}
		files, err := fs.ReadDir(migrations, "migrations") // sorted by name
		if err != nil {
			return err
		}
		for _, f := range files {
			if err := apply(ctx, tx, f.Name()); err != nil {
				return err
			}
		}
		return nil
	})
}

func apply(ctx context.Context, tx pgx.Tx, name string) error {
	var done bool
	if err := tx.QueryRow(ctx, `select exists(select 1 from schema_migrations where name = $1)`, name).Scan(&done); err != nil {
		return fmt.Errorf("migration %s: %w", name, err)
	}
	if done {
		return nil
	}
	sql, err := migrations.ReadFile("migrations/" + name)
	if err != nil {
		return err
	}
	if _, err := tx.Exec(ctx, string(sql)); err != nil {
		return fmt.Errorf("migration %s: %w", name, err)
	}
	if _, err := tx.Exec(ctx, `insert into schema_migrations (name) values ($1)`, name); err != nil {
		return fmt.Errorf("migration %s: %w", name, err)
	}
	return nil
}

// Store holds the queries that are not one feature's own.
type Store struct{ pool *pgxpool.Pool }

func NewStore(pool *pgxpool.Pool) *Store { return &Store{pool: pool} }

// IsRevoked reports whether a session was signed out from the web and its tokens may still be
// alive. It is auth.RevokedFunc.
func (s *Store) IsRevoked(ctx context.Context, sessionID string) (bool, error) {
	var revoked bool
	err := s.pool.QueryRow(ctx,
		`select exists(select 1 from revoked_sessions where session_id = $1 and until > now())`,
		sessionID).Scan(&revoked)
	return revoked, err
}

// TouchUser records a request from the user: it creates their row on the first one, and moves
// last_seen at most once an hour after that, so a busy user costs no write per request. Handlers
// run after it, so the row their foreign keys need always exists.
func (s *Store) TouchUser(ctx context.Context, userID string) error {
	_, err := s.pool.Exec(ctx, `insert into users (user_id) values ($1)
		on conflict (user_id) do update set last_seen = now()
		where users.last_seen < now() - interval '1 hour'`, userID)
	return err
}

// DeleteInactiveUsers deletes the users with no request for `retention`, and with them, through
// `on delete cascade`, everything HaryX holds about them.
func (s *Store) DeleteInactiveUsers(ctx context.Context, retention time.Duration) (int64, error) {
	tag, err := s.pool.Exec(ctx, `delete from users where last_seen < now() - $1::interval`, retention)
	return tag.RowsAffected(), err
}

// DeleteExpiredRevocations drops revocations past `until`: by then every token of that session
// has expired, and Keycloak refuses its refresh token.
func (s *Store) DeleteExpiredRevocations(ctx context.Context) (int64, error) {
	tag, err := s.pool.Exec(ctx, `delete from revoked_sessions where until <= now()`)
	return tag.RowsAffected(), err
}
