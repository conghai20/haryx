package sync

import (
	"errors"
	"io"
	"net/http"
	"regexp"
	"time"

	"github.com/jackc/pgx/v5"

	"github.com/conghai20/haryx/server/api/internal/web"
)

// maxProfileBytes is the largest profile file, as the profile_files_size_check and the CLI's
// MAX_BYTES. The 10-per-user cap is the profile_files_limit trigger.
const maxProfileBytes = 10 << 20

var profileName = regexp.MustCompile(`^[0-9a-f]{1,64}$`)

// Profile describes a stored profile file. Field order matches the queries below.
type Profile struct {
	Name      string    `json:"name"`
	Bytes     int64     `json:"bytes"`
	UpdatedAt time.Time `json:"updated_at"`
}

// profileFile is the {name} path value, or "" after answering 400.
func profileFile(w http.ResponseWriter, r *http.Request) string {
	name := r.PathValue("name")
	if !profileName.MatchString(name) {
		web.Error(w, http.StatusBadRequest, "invalid_name")
		return ""
	}
	return name
}

func (h *handler) listProfiles(w http.ResponseWriter, r *http.Request, user string) {
	rows, _ := h.pool.Query(r.Context(), `select name_hmac, octet_length(body), updated_at
		from profile_files where user_id = $1 order by name_hmac`, user)
	list, err := pgx.CollectRows(rows, pgx.RowToStructByPos[Profile])
	if err != nil {
		internalError(w, "list profiles", err)
		return
	}
	web.JSON(w, http.StatusOK, list)
}

// putProfile stores the raw request body under the name, replacing a file of the same name.
func (h *handler) putProfile(w http.ResponseWriter, r *http.Request, user string) {
	name := profileFile(w, r)
	if name == "" {
		return
	}
	body, err := io.ReadAll(http.MaxBytesReader(w, r.Body, maxProfileBytes))
	if err != nil {
		if tooLarge(err) {
			web.Error(w, http.StatusRequestEntityTooLarge, "file_too_large")
		} else {
			web.Error(w, http.StatusBadRequest, "invalid_body")
		}
		return
	}
	rows, _ := h.pool.Query(r.Context(), `insert into profile_files (user_id, name_hmac, body)
		values ($1, $2, $3)
		on conflict (user_id, name_hmac) do update set body = excluded.body, updated_at = now()
		returning name_hmac, octet_length(body), updated_at`, user, name, body)
	profile, err := pgx.CollectExactlyOneRow(rows, pgx.RowToStructByPos[Profile])
	switch _, constraint := pgCode(err); {
	case err == nil:
		web.JSON(w, http.StatusOK, profile)
	case constraint == "profile_files_limit":
		web.Error(w, http.StatusUnprocessableEntity, "profile_limit")
	default:
		internalError(w, "put profile", err)
	}
}

func (h *handler) getProfile(w http.ResponseWriter, r *http.Request, user string) {
	name := profileFile(w, r)
	if name == "" {
		return
	}
	var body []byte
	err := h.pool.QueryRow(r.Context(), `select body from profile_files
		where user_id = $1 and name_hmac = $2`, user, name).Scan(&body)
	switch {
	case errors.Is(err, pgx.ErrNoRows):
		web.Error(w, http.StatusNotFound, "not_found")
	case err != nil:
		internalError(w, "get profile", err)
	default:
		w.Header().Set("Content-Type", "application/octet-stream")
		_, _ = w.Write(body)
	}
}

// deleteProfile is idempotent, like deleteRow.
func (h *handler) deleteProfile(w http.ResponseWriter, r *http.Request, user string) {
	name := profileFile(w, r)
	if name == "" {
		return
	}
	h.exec(w, r, "delete profile",
		`delete from profile_files where user_id = $1 and name_hmac = $2`, user, name)
}
