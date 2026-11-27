package sync

import (
	"encoding/json"
	"errors"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/jackc/pgx/v5"

	"github.com/conghai20/haryx/server/api/internal/web"
)

const (
	maxBody       = 262144  // characters of a row body, as the sync_rows_body_check
	maxRowRequest = 1 << 20 // bytes of a POST/PUT request: a full body, JSON-escaped, fits
)

var (
	kinds  = map[string]bool{"keyring": true, "settings": true, "app-prefs": true, "account": true, "credential": true, "profiles": true, "usage": true}
	rowKey = regexp.MustCompile(`^[0-9a-f]{0,64}$`)
)

// Row is a sync row as the client sees it. Field order matches rowColumns.
type Row struct {
	Kind      string    `json:"kind"`
	Key       string    `json:"key"`
	Body      string    `json:"body"`
	Version   int64     `json:"version"`
	UpdatedAt time.Time `json:"updatedAt"`
}

const rowColumns = `kind, key, body, version, updated_at`

// checkRow is the error code for a bad kind or key, or "".
func checkRow(kind, key string) string {
	if !kinds[kind] {
		return "invalid_kind"
	}
	if !rowKey.MatchString(key) {
		return "invalid_key"
	}
	return ""
}

// readBody decodes {"body": …} or {"kind", "key", "body"} into v and checks the body. It writes
// the error itself and reports whether to go on.
func readBody(w http.ResponseWriter, r *http.Request, v any, body func() string) bool {
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxRowRequest)).Decode(v); err != nil {
		if tooLarge(err) {
			web.Error(w, http.StatusRequestEntityTooLarge, "body_too_large")
		} else {
			web.Error(w, http.StatusBadRequest, "invalid_json")
		}
		return false
	}
	b := body()
	switch {
	case b == "" || strings.ContainsRune(b, 0): // Postgres text cannot hold NUL
		web.Error(w, http.StatusBadRequest, "invalid_body")
	case utf8.RuneCountInString(b) > maxBody:
		web.Error(w, http.StatusRequestEntityTooLarge, "body_too_large")
	default:
		return true
	}
	return false
}

func (h *handler) listRows(w http.ResponseWriter, r *http.Request, user string) {
	kind := r.URL.Query().Get("kind")
	if r.URL.Query().Has("kind") && !kinds[kind] {
		web.Error(w, http.StatusBadRequest, "invalid_kind")
		return
	}
	rows, _ := h.pool.Query(r.Context(), `select `+rowColumns+` from sync_rows
		where user_id = $1 and ($2 = '' or kind = $2) order by kind, key`, user, kind)
	list, err := pgx.CollectRows(rows, pgx.RowToStructByPos[Row])
	if err != nil {
		internalError(w, "list sync rows", err)
		return
	}
	web.JSON(w, http.StatusOK, list)
}

func (h *handler) insertRow(w http.ResponseWriter, r *http.Request, user string) {
	var in struct{ Kind, Key, Body string }
	if !readBody(w, r, &in, func() string { return in.Body }) {
		return
	}
	if code := checkRow(in.Kind, in.Key); code != "" {
		web.Error(w, http.StatusBadRequest, code)
		return
	}
	rows, _ := h.pool.Query(r.Context(), `insert into sync_rows (user_id, kind, key, body)
		values ($1, $2, $3, $4) returning `+rowColumns, user, in.Kind, in.Key, in.Body)
	row, err := pgx.CollectExactlyOneRow(rows, pgx.RowToStructByPos[Row])
	switch code, constraint := pgCode(err); {
	case err == nil:
		web.JSON(w, http.StatusCreated, row)
	case code == "23505":
		web.Error(w, http.StatusConflict, "exists")
	case constraint == "sync_rows_limit":
		web.Error(w, http.StatusUnprocessableEntity, "row_limit")
	default:
		internalError(w, "insert sync row", err)
	}
}

// updateRow replaces a row's body if it is still at the version in If-Match, and raises the
// version by one. A version that moved is 412, a row that is gone is 404.
func (h *handler) updateRow(w http.ResponseWriter, r *http.Request, user string) {
	kind, key := r.PathValue("kind"), r.PathValue("key")
	if code := checkRow(kind, key); code != "" {
		web.Error(w, http.StatusBadRequest, code)
		return
	}
	match := r.Header.Get("If-Match")
	if match == "" {
		web.Error(w, http.StatusPreconditionRequired, "if_match_required")
		return
	}
	version, err := strconv.ParseInt(strings.Trim(match, `"`), 10, 64)
	if err != nil || version < 1 {
		web.Error(w, http.StatusBadRequest, "invalid_if_match")
		return
	}
	var in struct{ Body string }
	if !readBody(w, r, &in, func() string { return in.Body }) {
		return
	}
	rows, _ := h.pool.Query(r.Context(), `update sync_rows
		set body = $5, version = version + 1, updated_at = now()
		where user_id = $1 and kind = $2 and key = $3 and version = $4
		returning `+rowColumns, user, kind, key, version, in.Body)
	row, err := pgx.CollectExactlyOneRow(rows, pgx.RowToStructByPos[Row])
	if err == nil {
		web.JSON(w, http.StatusOK, row)
		return
	}
	if !errors.Is(err, pgx.ErrNoRows) {
		internalError(w, "update sync row", err)
		return
	}
	var exists bool
	if err := h.pool.QueryRow(r.Context(), `select exists(select 1 from sync_rows
		where user_id = $1 and kind = $2 and key = $3)`, user, kind, key).Scan(&exists); err != nil {
		internalError(w, "update sync row", err)
		return
	}
	if exists {
		web.Error(w, http.StatusPreconditionFailed, "version_mismatch")
	} else {
		web.Error(w, http.StatusNotFound, "not_found")
	}
}

// deleteRow is idempotent: a row that is not there is already deleted.
func (h *handler) deleteRow(w http.ResponseWriter, r *http.Request, user string) {
	kind, key := r.PathValue("kind"), r.PathValue("key")
	if code := checkRow(kind, key); code != "" {
		web.Error(w, http.StatusBadRequest, code)
		return
	}
	h.exec(w, r, "delete sync row",
		`delete from sync_rows where user_id = $1 and kind = $2 and key = $3`, user, kind, key)
}

func (h *handler) deleteAllRows(w http.ResponseWriter, r *http.Request, user string) {
	h.exec(w, r, "delete sync rows", `delete from sync_rows where user_id = $1`, user)
}

// exec runs a statement and answers 204.
func (h *handler) exec(w http.ResponseWriter, r *http.Request, what, sql string, args ...any) {
	if _, err := h.pool.Exec(r.Context(), sql, args...); err != nil {
		internalError(w, what, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}
