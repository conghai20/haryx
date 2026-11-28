// Package export serves GET /api/v1/me/export (plan task 8, Law 91 access): everything HaryX holds
// about the caller, as one JSON file. Any HaryX client may ask.
package export

import (
	"context"
	"fmt"
	"net/http"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/consent"
	"github.com/conghai20/haryx/server/api/internal/machines"
	syncapi "github.com/conghai20/haryx/server/api/internal/sync"
	"github.com/conghai20/haryx/server/api/internal/web"
)

// Notes say what the file holds and what it does not.
var Notes = []string{
	"syncRows bodies and profile files are end-to-end encrypted: only your sync passphrase opens them.",
	"profiles lists your profile files without their contents; download each with GET /api/v1/profiles/{name}.",
	"Your WeCoAp ID profile (name, email, sign-in methods) is held by WeCoAp SSO, not HaryX: see https://sso.wecoap.com/realms/wecoap/account/.",
}

// User is HaryX's own record of the caller.
type User struct {
	ID        string    `json:"id"`
	FirstSeen time.Time `json:"firstSeen"`
	LastSeen  time.Time `json:"lastSeen"`
}

// Export is the downloaded file. Consents and machines are the contracts' items, syncRows and
// profiles what the sync API returns.
type Export struct {
	ExportedAt time.Time          `json:"exportedAt"`
	User       User               `json:"user"`
	Consents   []consent.Consent  `json:"consents"`
	Machines   []machines.Machine `json:"machines"`
	SyncRows   []syncapi.Row      `json:"syncRows"`
	Profiles   []syncapi.Profile  `json:"profiles"`
	Notes      []string           `json:"notes"`
}

// Handler serves GET /api/v1/me/export.
func Handler(pool *pgxpool.Pool) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		user, ok := auth.Caller(w, r)
		if !ok {
			return
		}
		body, err := read(r.Context(), pool, user)
		if err != nil {
			web.InternalError(w, "export", err)
			return
		}
		name := fmt.Sprintf("haryx-export-%s.json", body.ExportedAt.Format(time.DateOnly))
		w.Header().Set("Content-Disposition", `attachment; filename="`+name+`"`)
		web.JSON(w, http.StatusOK, body)
	})
}

// read takes every table in one repeatable-read snapshot, so the parts agree with each other.
func read(ctx context.Context, pool *pgxpool.Pool, user auth.User) (Export, error) {
	var out Export
	opts := pgx.TxOptions{IsoLevel: pgx.RepeatableRead, AccessMode: pgx.ReadOnly}
	err := pgx.BeginTxFunc(ctx, pool, opts, func(tx pgx.Tx) error {
		var err error
		if err = tx.QueryRow(ctx, `select now(), user_id::text, first_seen, last_seen from users
			where user_id = $1`, user.ID).Scan(&out.ExportedAt, &out.User.ID, &out.User.FirstSeen, &out.User.LastSeen); err != nil {
			return fmt.Errorf("user: %w", err)
		}
		consents, err := consent.Read(ctx, tx, user.ID)
		if err != nil {
			return fmt.Errorf("consents: %w", err)
		}
		out.Consents = consents.Consents
		if out.Machines, err = machines.Read(ctx, tx, user.ID, user.SessionID); err != nil {
			return fmt.Errorf("machines: %w", err)
		}
		if out.SyncRows, err = syncapi.Rows(ctx, tx, user.ID, ""); err != nil {
			return fmt.Errorf("sync rows: %w", err)
		}
		if out.Profiles, err = syncapi.Profiles(ctx, tx, user.ID); err != nil {
			return fmt.Errorf("profiles: %w", err)
		}
		return nil
	})
	out.ExportedAt = out.ExportedAt.UTC()
	out.User.FirstSeen, out.User.LastSeen = out.User.FirstSeen.UTC(), out.User.LastSeen.UTC()
	out.Notes = Notes
	return out, err
}
