// haryx-api: the HaryX API. It verifies WeCoAp SSO tokens and owns the HaryX Postgres.
//
// Configuration is the environment that deploy/compose.yaml sets: HARYX_SSO_ISSUER, and the
// libpq variables (PGHOST, PGPORT, PGDATABASE, PGUSER, PGPASSWORD). It listens on :8080.
package main

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"

	"github.com/conghai20/haryx/server/api/internal/auth"
	"github.com/conghai20/haryx/server/api/internal/consent"
	"github.com/conghai20/haryx/server/api/internal/db"
	"github.com/conghai20/haryx/server/api/internal/export"
	"github.com/conghai20/haryx/server/api/internal/machines"
	syncapi "github.com/conghai20/haryx/server/api/internal/sync"
	"github.com/conghai20/haryx/server/api/internal/web"
)

const (
	addr            = ":8080"
	shutdownTimeout = 10 * time.Second
	healthTimeout   = 2 * time.Second
	sweepEvery      = time.Hour

	// userRetention: a user with no request for this long is deleted with all their data (plan
	// A10). It is how a deleted WeCoAp ID's data is erased. The privacy policy states it.
	userRetention = 365 * 24 * time.Hour
)

func main() {
	slog.SetDefault(slog.New(slog.NewJSONHandler(os.Stdout, nil)))
	if err := run(); err != nil {
		slog.Error("haryx-api stopped", "err", err)
		os.Exit(1)
	}
}

func run() error {
	issuer := os.Getenv("HARYX_SSO_ISSUER")
	if issuer == "" {
		return errors.New("HARYX_SSO_ISSUER is not set")
	}
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	pool, err := db.Open(ctx)
	if err != nil {
		return err
	}
	defer pool.Close()
	if err := db.Migrate(ctx, pool); err != nil {
		return fmt.Errorf("migrate: %w", err)
	}
	store := db.NewStore(pool)
	go sweep(ctx, store)

	srv := &http.Server{
		Addr:              addr,
		Handler:           routes(pool, store, auth.NewRemote(issuer, store.IsRevoked)),
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       time.Minute, // profile uploads are up to 10 MB
		WriteTimeout:      time.Minute,
		IdleTimeout:       2 * time.Minute,
	}
	errs := make(chan error, 1)
	go func() { errs <- srv.ListenAndServe() }()
	slog.Info("haryx-api listening", "addr", addr, "issuer", issuer)

	select {
	case err := <-errs:
		return err
	case <-ctx.Done():
	}
	shutdown, cancel := context.WithTimeout(context.Background(), shutdownTimeout)
	defer cancel()
	return srv.Shutdown(shutdown)
}

// routes: /api/v1/health is open; everything else under /api/v1/ needs a valid token, and
// touches the caller's users row.
func routes(pool *pgxpool.Pool, store *db.Store, verifier *auth.Verifier) http.Handler {
	api := http.NewServeMux()
	api.HandleFunc("/", func(w http.ResponseWriter, _ *http.Request) {
		web.Error(w, http.StatusNotFound, "not_found")
	})
	consents := consent.Routes(pool)
	api.Handle("/api/v1/consents", consents)
	api.Handle("/api/v1/consents/", consents)
	// Sync rows and profiles answer 403 consent_required without the sync consent.
	data := consent.Require(pool, consent.Sync, syncapi.Routes(pool))
	api.Handle("/api/v1/sync/", data)
	api.Handle("/api/v1/profiles", data)
	api.Handle("/api/v1/profiles/", data)
	// The heartbeat answers 403 consent_required without the machines consent; the list does not.
	registry := machines.Routes(pool)
	api.Handle("/api/v1/machines", registry)
	api.Handle("/api/v1/machines/", registry)
	api.Handle("GET /api/v1/me/export", export.Handler(pool))

	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/v1/health", health(pool))
	mux.Handle("/api/v1/", verifier.Middleware(touchUser(store, api)))
	return mux
}

// touchUser keeps the caller's users row alive (plan A10). Without it a handler's insert would
// break the foreign key, so a failure stops the request.
func touchUser(store *db.Store, next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		user, _ := auth.FromContext(r.Context())
		if err := store.TouchUser(r.Context(), user.ID); err != nil {
			slog.Error("touch user", "err", err)
			web.Error(w, http.StatusServiceUnavailable, "unavailable")
			return
		}
		next.ServeHTTP(w, r)
	})
}

func health(pool *pgxpool.Pool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		ctx, cancel := context.WithTimeout(r.Context(), healthTimeout)
		defer cancel()
		if err := pool.Ping(ctx); err != nil {
			slog.Error("health: postgres", "err", err)
			web.Error(w, http.StatusServiceUnavailable, "database_unavailable")
			return
		}
		web.JSON(w, http.StatusOK, map[string]string{"status": "ok"})
	}
}

// sweep, at start and then hourly: inactive users (with their data) and expired revocations.
func sweep(ctx context.Context, store *db.Store) {
	ticker := time.NewTicker(sweepEvery)
	defer ticker.Stop()
	for {
		sweepOnce(ctx, store)
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
}

func sweepOnce(ctx context.Context, store *db.Store) {
	if n, err := store.DeleteInactiveUsers(ctx, userRetention); err != nil {
		slog.Error("sweep inactive users", "err", err)
	} else if n > 0 {
		slog.Info("deleted inactive users", "deleted", n)
	}
	if n, err := store.DeleteExpiredRevocations(ctx); err != nil {
		slog.Warn("sweep revoked sessions", "err", err)
	} else if n > 0 {
		slog.Info("swept revoked sessions", "deleted", n)
	}
}
