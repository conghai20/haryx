// Package sync serves the caller's end-to-end encrypted sync rows and profile files (plan task 6).
// It replaces Supabase's sync_rows table and profiles bucket. There is no row-level security to
// lean on: every query names the caller's user_id.
//
// JSON field names are snake_case, as PostgREST returned them. Times are RFC 3339. Errors are
// {"error": "<code>"} (internal/web).
package sync

import (
	"errors"
	"log/slog"
	"net/http"

	"github.com/jackc/pgx/v5/pgconn"
	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/web"
)

type handler struct{ pool *pgxpool.Pool }

// Routes serves /api/v1/sync/ and /api/v1/profiles. It is one handler so that the consent gate
// (task 7) can wrap all of it at once.
func Routes(pool *pgxpool.Pool) http.Handler {
	h := &handler{pool: pool}
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/v1/sync/rows", h.as(h.listRows))
	mux.HandleFunc("POST /api/v1/sync/rows", h.as(h.insertRow))
	mux.HandleFunc("DELETE /api/v1/sync/rows", h.as(h.deleteAllRows))
	// {key...}, not {key}: the empty key is a real one (`/api/v1/sync/rows/settings/`).
	mux.HandleFunc("PUT /api/v1/sync/rows/{kind}/{key...}", h.as(h.updateRow))
	mux.HandleFunc("DELETE /api/v1/sync/rows/{kind}/{key...}", h.as(h.deleteRow))
	mux.HandleFunc("GET /api/v1/profiles", h.as(h.listProfiles))
	mux.HandleFunc("PUT /api/v1/profiles/{name}", h.as(h.putProfile))
	mux.HandleFunc("GET /api/v1/profiles/{name}", h.as(h.getProfile))
	mux.HandleFunc("DELETE /api/v1/profiles/{name}", h.as(h.deleteProfile))
	mux.HandleFunc("/", func(w http.ResponseWriter, _ *http.Request) {
		web.Error(w, http.StatusNotFound, "not_found")
	})
	return mux
}

// as hands the handler the caller's user id. The auth middleware always sets one; a request
// without it is refused rather than run unscoped.
func (h *handler) as(fn func(http.ResponseWriter, *http.Request, string)) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		user, ok := auth.FromContext(r.Context())
		if !ok || user.ID == "" {
			web.Error(w, http.StatusUnauthorized, "unauthorized")
			return
		}
		fn(w, r, user.ID)
	}
}

func internalError(w http.ResponseWriter, what string, err error) {
	slog.Error(what, "err", err)
	web.Error(w, http.StatusInternalServerError, "internal")
}

// pgCode is the Postgres error's SQLSTATE and constraint, or empty strings.
func pgCode(err error) (code, constraint string) {
	var pgErr *pgconn.PgError
	if errors.As(err, &pgErr) {
		return pgErr.Code, pgErr.ConstraintName
	}
	return "", ""
}

func tooLarge(err error) bool {
	var maxErr *http.MaxBytesError
	return errors.As(err, &maxErr)
}
