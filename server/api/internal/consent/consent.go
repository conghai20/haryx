// Package consent serves the caller's HaryX consents (plan task 7, Law 91) and gates endpoints on
// them.
//
//	GET    /api/v1/consents                    the body of packages/contracts/haryx/consents.json
//	PUT    /api/v1/consents/{purpose}          grant: {"version": "2026-11-01", "source": "macos"}
//	DELETE /api/v1/consents/{purpose}?source=  withdraw, and erase that purpose's data
//
// PUT and DELETE answer 200 with the same body as GET. Both are idempotent: granting the version
// already granted, or withdrawing what is not granted, records no event. Granting another version
// (a new policy) records a new grant.
//
// `source` is where the user answered: cli, macos or windows from a haryx-cli token. A
// haryx-platform token always records web, whatever it sends, and only it can record web.
package consent

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"regexp"
	"slices"
	"time"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/db"
	"github.com/conghai20/haryx/server/api/internal/web"
)

// The purposes, each consented to on its own.
const (
	Sync     = "sync"     // sync rows (usage included) and profile files
	Machines = "machines" // the machine registry
)

// purposes, in the order GET lists them.
var purposes = []string{Sync, Machines}

// erase deletes a purpose's data. It runs inside the withdrawal's transaction, so the data and
// the withdraw event go together or not at all.
var erase = map[string][]string{
	Sync: {
		`delete from sync_rows where user_id = $1`,
		`delete from profile_files where user_id = $1`,
	},
	Machines: {
		`delete from machines where user_id = $1`,
	},
}

var (
	// A policy version: a date such as 2026-11-01, or any short name.
	versionPattern = regexp.MustCompile(`^[0-9A-Za-z][0-9A-Za-z._-]{0,31}$`)
	cliSources     = map[string]bool{"cli": true, "macos": true, "windows": true}
)

const maxRequest = 4096

// Event is one grant or withdrawal, as consents.json has it.
type Event struct {
	Action  string    `json:"action"`
	Version string    `json:"version"`
	At      time.Time `json:"at"`
	Source  string    `json:"source"`
}

// Consent is one purpose. Version and At are the latest event's, null when never answered.
type Consent struct {
	Purpose string     `json:"purpose"`
	Granted bool       `json:"granted"`
	Version *string    `json:"version"`
	At      *time.Time `json:"at"`
	History []Event    `json:"history"`
}

// Consents is the body of GET /api/v1/consents.
type Consents struct {
	Consents []Consent `json:"consents"`
}

type handler struct{ pool *pgxpool.Pool }

// Routes serves /api/v1/consents and /api/v1/consents/{purpose}.
func Routes(pool *pgxpool.Pool) http.Handler {
	h := &handler{pool: pool}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/v1/consents", h.list)
	mux.HandleFunc("PUT /api/v1/consents/{purpose}", h.grant)
	mux.HandleFunc("DELETE /api/v1/consents/{purpose}", h.withdraw)
	mux.HandleFunc("/", func(w http.ResponseWriter, _ *http.Request) {
		web.Error(w, http.StatusNotFound, "not_found")
	})
	return mux
}

// Require lets a request through only while the caller has granted purpose, and answers
// 403 consent_required otherwise.
func Require(pool *pgxpool.Pool, purpose string, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		user, ok := auth.Caller(w, r)
		if !ok {
			return
		}
		var granted bool
		err := pool.QueryRow(r.Context(), `select exists(select 1 from consents
			where user_id = $1 and purpose = $2 and granted)`, user.ID, purpose).Scan(&granted)
		switch {
		case err != nil:
			web.InternalError(w, "consent check", err)
		case !granted:
			web.Error(w, http.StatusForbidden, "consent_required")
		default:
			next.ServeHTTP(w, r)
		}
	})
}

func (h *handler) list(w http.ResponseWriter, r *http.Request) {
	user, ok := auth.Caller(w, r)
	if !ok {
		return
	}
	body, err := Read(r.Context(), h.pool, user.ID)
	if err != nil {
		web.InternalError(w, "list consents", err)
		return
	}
	web.JSON(w, http.StatusOK, body)
}

func (h *handler) grant(w http.ResponseWriter, r *http.Request) {
	user, purpose, ok := target(w, r)
	if !ok {
		return
	}
	var in struct{ Version, Source string }
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxRequest)).Decode(&in); err != nil {
		web.Error(w, http.StatusBadRequest, "invalid_json")
		return
	}
	if !versionPattern.MatchString(in.Version) {
		web.Error(w, http.StatusBadRequest, "invalid_version")
		return
	}
	source, ok := sourceOf(user, in.Source)
	if !ok {
		web.Error(w, http.StatusBadRequest, "invalid_source")
		return
	}
	h.change(w, r, user.ID, func(ctx context.Context, tx pgx.Tx) error {
		granted, version, err := current(ctx, tx, user.ID, purpose)
		if err != nil || (granted && version == in.Version) {
			return err
		}
		return record(ctx, tx, user.ID, purpose, "grant", in.Version, source)
	})
}

func (h *handler) withdraw(w http.ResponseWriter, r *http.Request) {
	user, purpose, ok := target(w, r)
	if !ok {
		return
	}
	source, ok := sourceOf(user, r.URL.Query().Get("source"))
	if !ok {
		web.Error(w, http.StatusBadRequest, "invalid_source")
		return
	}
	h.change(w, r, user.ID, func(ctx context.Context, tx pgx.Tx) error {
		granted, version, err := current(ctx, tx, user.ID, purpose)
		if err != nil {
			return err
		}
		// Erased even when not granted: a withdrawn purpose holds no data, whatever came before.
		for _, sql := range erase[purpose] {
			if _, err := tx.Exec(ctx, sql, user.ID); err != nil {
				return err
			}
		}
		if !granted {
			return nil
		}
		return record(ctx, tx, user.ID, purpose, "withdraw", version, source)
	})
}

// change runs fn in a transaction that holds the caller's users row, so one user's consent
// changes, and the require_consent trigger's inserts, run one at a time. It answers with the
// consents as the transaction left them.
func (h *handler) change(w http.ResponseWriter, r *http.Request, user string, fn func(context.Context, pgx.Tx) error) {
	ctx := r.Context()
	var body Consents
	err := pgx.BeginFunc(ctx, h.pool, func(tx pgx.Tx) error {
		if _, err := tx.Exec(ctx, `select 1 from users where user_id = $1 for no key update`, user); err != nil {
			return err
		}
		if err := fn(ctx, tx); err != nil {
			return err
		}
		var err error
		body, err = Read(ctx, tx, user)
		return err
	})
	if err != nil {
		web.InternalError(w, "change consent", err)
		return
	}
	web.JSON(w, http.StatusOK, body)
}

// current is the purpose's state: not granted and no version when never answered.
func current(ctx context.Context, tx pgx.Tx, user, purpose string) (granted bool, version string, err error) {
	err = tx.QueryRow(ctx, `select granted, version from consents where user_id = $1 and purpose = $2`,
		user, purpose).Scan(&granted, &version)
	if errors.Is(err, pgx.ErrNoRows) {
		return false, "", nil
	}
	return granted, version, err
}

// record appends the event and moves the current state to it, at the same instant.
func record(ctx context.Context, tx pgx.Tx, user, purpose, action, version, source string) error {
	_, err := tx.Exec(ctx, `with event as (
			insert into consent_events (user_id, purpose, action, version, source)
			values ($1, $2, $3, $4, $5) returning at)
		insert into consents (user_id, purpose, granted, version, at)
		select $1, $2, $3 = 'grant', $4, at from event
		on conflict (user_id, purpose) do update
			set granted = excluded.granted, version = excluded.version, at = excluded.at`,
		user, purpose, action, version, source)
	return err
}

// Read builds the GET body from the events: the state of a purpose is its latest event's.
func Read(ctx context.Context, q db.Querier, user string) (Consents, error) {
	type row struct {
		Purpose, Action, Version string
		At                       time.Time
		Source                   string
	}
	rows, _ := q.Query(ctx, `select purpose, action, version, at, source from consent_events
		where user_id = $1 order by id`, user)
	events, err := pgx.CollectRows(rows, pgx.RowToStructByPos[row])
	if err != nil {
		return Consents{}, err
	}
	list := make([]Consent, len(purposes))
	for i, p := range purposes {
		list[i] = Consent{Purpose: p, History: []Event{}}
	}
	for _, e := range events {
		c := &list[slices.Index(purposes, e.Purpose)]
		at := e.At.UTC().Truncate(time.Second)
		c.History = append(c.History, Event{Action: e.Action, Version: e.Version, At: at, Source: e.Source})
		c.Granted, c.Version, c.At = e.Action == "grant", &e.Version, &at
	}
	return Consents{Consents: list}, nil
}

// target is the caller and a known {purpose}, or false with the error written.
func target(w http.ResponseWriter, r *http.Request) (auth.User, string, bool) {
	user, ok := auth.Caller(w, r)
	if !ok {
		return user, "", false
	}
	purpose := r.PathValue("purpose")
	if !slices.Contains(purposes, purpose) {
		web.Error(w, http.StatusBadRequest, "invalid_purpose")
		return user, "", false
	}
	return user, purpose, true
}

// sourceOf is the source to record for what the client claimed, or false if it may not claim it.
func sourceOf(user auth.User, claimed string) (string, bool) {
	if user.Client == "haryx-platform" {
		return "web", true
	}
	return claimed, cliSources[claimed]
}
