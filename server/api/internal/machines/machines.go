// Package machines serves the caller's machine registry and remote sign-out (plan task 8).
//
//	PUT    /api/v1/machines/self           haryx-cli heartbeat {"machineId", "name", "os", "version"}
//	GET    /api/v1/machines                the body of packages/contracts/haryx/machines.json
//	PATCH  /api/v1/machines/{id}           rename {"name"}
//	DELETE /api/v1/machines/{id}/session   haryx-platform sign-out, step 2 (A7)
//
// The heartbeat needs the machines consent; listing does not (after a withdrawal it is []). Every
// query is scoped to the caller, and a machine that is not theirs is 404.
package machines

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"regexp"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgconn"
	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/consent"
	"github.com/conghai20/haryx/server/api/internal/db"
	"github.com/conghai20/haryx/server/api/internal/web"
)

// revokeFor is how long a signed-out session's access tokens stay refused. It must be longer than
// the realm's access-token lifespan (5 minutes): Keycloak already refuses the session's refresh
// token (the web ended the session, A7), so after this every token it issued has expired.
const revokeFor = time.Hour

const (
	maxRequest = 4096
	maxName    = 64 // characters, as machines_name_check
)

var (
	// Lowercase and hyphenated, as the CLI writes it: the CLI finds its own row by comparing ids.
	machineID = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)
	version   = regexp.MustCompile(`^[0-9A-Za-z][0-9A-Za-z.+-]{0,31}$`) // as machines_version_check
	systems   = map[string]bool{"macos": true, "windows": true, "linux": true}
)

// Machine is one item of machines.json. Field order matches columns.
type Machine struct {
	MachineID     string    `json:"machineId"`
	Name          string    `json:"name"`
	OS            string    `json:"os"`
	Version       string    `json:"version"`
	LastSeenAt    time.Time `json:"lastSeenAt"`
	IsThisMachine bool      `json:"isThisMachine"`
	SignedIn      bool      `json:"signedIn"`
	// SessionID is the machine's Keycloak session: the web names it to the account API to end
	// that session (A7 step 1). It is the user's own, and not a credential.
	SessionID string `json:"sessionId"`
}

// Machines is the body of GET /api/v1/machines.
type Machines struct {
	Machines []Machine `json:"machines"`
}

// columns of a Machine from `machines as m`, where $2 is the caller's sid: the row whose session
// is the caller's is the machine asking.
const columns = `m.machine_id::text, m.name, m.os, m.version, m.last_seen_at, m.session_id = $2,
	not exists(select 1 from revoked_sessions r where r.session_id = m.session_id and r.until > now()),
	m.session_id`

type handler struct{ pool *pgxpool.Pool }

// Routes serves /api/v1/machines and /api/v1/machines/.
func Routes(pool *pgxpool.Pool) http.Handler {
	h := &handler{pool: pool}
	mux := http.NewServeMux()
	// The web is not a machine; and the CLI cannot end a Keycloak session itself (A7), so a
	// revoke from it would only pause that machine until the revocation expired.
	mux.Handle("PUT /api/v1/machines/self",
		only("haryx-cli", consent.Require(pool, consent.Machines, http.HandlerFunc(h.heartbeat))))
	mux.Handle("DELETE /api/v1/machines/{id}/session", only("haryx-platform", http.HandlerFunc(h.signOut)))
	mux.HandleFunc("GET /api/v1/machines", h.list)
	mux.HandleFunc("PATCH /api/v1/machines/{id}", h.rename)
	mux.HandleFunc("/", func(w http.ResponseWriter, _ *http.Request) {
		web.Error(w, http.StatusNotFound, "not_found")
	})
	return mux
}

// only lets through the callers whose token was issued to client, and answers 403
// forbidden_client to the others.
func only(client string, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		user, ok := auth.Caller(w, r)
		switch {
		case !ok:
		case user.Client != client:
			web.Error(w, http.StatusForbidden, "forbidden_client")
		default:
			next.ServeHTTP(w, r)
		}
	})
}

// Read is the user's machines, isThisMachine set for the one signed in with session sid.
func Read(ctx context.Context, q db.Querier, user, sid string) ([]Machine, error) {
	rows, _ := q.Query(ctx, `select `+columns+` from machines as m
		where m.user_id = $1 order by m.last_seen_at desc, m.machine_id`, user, sid)
	return collect(rows)
}

func (h *handler) list(w http.ResponseWriter, r *http.Request) {
	user, ok := auth.Caller(w, r)
	if !ok {
		return
	}
	list, err := Read(r.Context(), h.pool, user.ID, user.SessionID)
	if err != nil {
		web.InternalError(w, "list machines", err)
		return
	}
	web.JSON(w, http.StatusOK, Machines{Machines: list})
}

// heartbeat registers the calling machine or refreshes it: its os, version, the sid of the token
// it signed in with (a new sign-in is a new sid), and last seen. The name is only set on the
// first heartbeat, so that a rename from the web is not undone by the next one.
func (h *handler) heartbeat(w http.ResponseWriter, r *http.Request) {
	user, _ := auth.FromContext(r.Context()) // only() checked it
	var in struct{ MachineID, Name, OS, Version string }
	if !decode(w, r, &in) {
		return
	}
	code := ""
	switch {
	case !machineID.MatchString(in.MachineID):
		code = "invalid_machine_id"
	case !validName(in.Name):
		code = "invalid_name"
	case !systems[in.OS]:
		code = "invalid_os"
	case !version.MatchString(in.Version):
		code = "invalid_version"
	}
	if code != "" {
		web.Error(w, http.StatusBadRequest, code)
		return
	}
	rows, _ := h.pool.Query(r.Context(), `insert into machines as m
			(user_id, session_id, machine_id, name, os, version, last_seen_at)
		values ($1, $2, $3, $4, $5, $6, now())
		on conflict (user_id, machine_id) do update set os = excluded.os, version = excluded.version,
			session_id = excluded.session_id, last_seen_at = excluded.last_seen_at
		returning `+columns, user.ID, user.SessionID, in.MachineID, in.Name, in.OS, in.Version)
	machine, err := one(rows)
	var pgErr *pgconn.PgError
	switch {
	case err == nil:
		web.JSON(w, http.StatusOK, machine)
	case errors.As(err, &pgErr) && pgErr.ConstraintName == "consent_required": // withdrawn in flight
		web.Error(w, http.StatusForbidden, "consent_required")
	default:
		web.InternalError(w, "machine heartbeat", err)
	}
}

func (h *handler) rename(w http.ResponseWriter, r *http.Request) {
	user, ok := auth.Caller(w, r)
	if !ok {
		return
	}
	id, ok := pathID(w, r)
	if !ok {
		return
	}
	var in struct{ Name string }
	if !decode(w, r, &in) {
		return
	}
	if !validName(in.Name) {
		web.Error(w, http.StatusBadRequest, "invalid_name")
		return
	}
	rows, _ := h.pool.Query(r.Context(), `update machines as m set name = $4
		where m.user_id = $1 and m.machine_id = $3 returning `+columns, user.ID, user.SessionID, id, in.Name)
	machine, err := one(rows)
	switch {
	case err == nil:
		web.JSON(w, http.StatusOK, machine)
	case errors.Is(err, pgx.ErrNoRows):
		web.Error(w, http.StatusNotFound, "not_found")
	default:
		web.InternalError(w, "rename machine", err)
	}
}

// signOut is step 2 of a web sign-out (A7): the web has ended the machine's Keycloak session with
// the user's own token, and this refuses that session's access tokens still alive. The sid comes
// from the caller's own machine row, never from the request. Dropping the row and revoking its
// sid are one statement, so one transaction.
func (h *handler) signOut(w http.ResponseWriter, r *http.Request) {
	user, _ := auth.FromContext(r.Context()) // only() checked it
	id, ok := pathID(w, r)
	if !ok {
		return
	}
	tag, err := h.pool.Exec(r.Context(), `with gone as (
			delete from machines where user_id = $1 and machine_id = $2 returning session_id)
		insert into revoked_sessions (session_id, until) select session_id, now() + $3::interval from gone
		on conflict (session_id) do update set until = greatest(revoked_sessions.until, excluded.until)`,
		user.ID, id, revokeFor)
	switch {
	case err != nil:
		web.InternalError(w, "sign out machine", err)
	case tag.RowsAffected() == 0:
		web.Error(w, http.StatusNotFound, "not_found")
	default:
		w.WriteHeader(http.StatusNoContent)
	}
}

func collect(rows pgx.Rows) ([]Machine, error) {
	list, err := pgx.CollectRows(rows, pgx.RowToStructByPos[Machine])
	for i := range list {
		list[i].LastSeenAt = list[i].LastSeenAt.UTC().Truncate(time.Second)
	}
	return list, err
}

// one is the single machine a statement returned, or pgx.ErrNoRows.
func one(rows pgx.Rows) (Machine, error) {
	list, err := collect(rows)
	if err == nil && len(list) == 0 {
		err = pgx.ErrNoRows
	}
	if err != nil {
		return Machine{}, err
	}
	return list[0], nil
}

// pathID is the {id} path value, or false after answering 400.
func pathID(w http.ResponseWriter, r *http.Request) (string, bool) {
	id := r.PathValue("id")
	if !machineID.MatchString(id) {
		web.Error(w, http.StatusBadRequest, "invalid_machine_id")
		return "", false
	}
	return id, true
}

func decode(w http.ResponseWriter, r *http.Request, v any) bool {
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxRequest)).Decode(v); err != nil {
		web.Error(w, http.StatusBadRequest, "invalid_json")
		return false
	}
	return true
}

// validName: 1 to 64 characters, not all blank, no control characters.
func validName(name string) bool {
	if strings.TrimSpace(name) == "" || utf8.RuneCountInString(name) > maxName {
		return false
	}
	return !strings.ContainsFunc(name, unicode.IsControl)
}
