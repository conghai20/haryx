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
	"github.com/conghai20/haryx/server/api/internal/db"
	"github.com/conghai20/haryx/server/api/internal/web"
)

const (
	addr             = ":8080"
	shutdownTimeout  = 10 * time.Second
	healthTimeout    = 2 * time.Second
	revocationSweeps = time.Hour
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
	go sweepRevocations(ctx, store)

	srv := &http.Server{
		Addr:              addr,
		Handler:           routes(pool, auth.NewRemote(issuer, store.IsRevoked)),
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

// routes: /api/v1/health is open; everything else under /api/v1/ needs a valid token. Tasks 6–8
// register their endpoints on `api`.
func routes(pool *pgxpool.Pool, verifier *auth.Verifier) http.Handler {
	api := http.NewServeMux()
	api.HandleFunc("/", func(w http.ResponseWriter, _ *http.Request) {
		web.Error(w, http.StatusNotFound, "not_found")
	})

	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/v1/health", health(pool))
	mux.Handle("/api/v1/", verifier.Middleware(api))
	return mux
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

func sweepRevocations(ctx context.Context, store *db.Store) {
	ticker := time.NewTicker(revocationSweeps)
	defer ticker.Stop()
	for {
		if n, err := store.DeleteExpiredRevocations(ctx); err != nil {
			slog.Warn("sweep revoked sessions", "err", err)
		} else if n > 0 {
			slog.Info("swept revoked sessions", "deleted", n)
		}
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
}
