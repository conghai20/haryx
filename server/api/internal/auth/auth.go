// Package auth checks WeCoAp SSO (Keycloak) access tokens for the HaryX API.
//
// A request passes only with a Bearer access token that the realm signed (RS256, a key in its
// JWKS), that is unexpired, issued by the configured issuer, meant for this API (aud ∋ haryx-api),
// requested by a HaryX client (azp), and whose session has not been signed out from the web.
package auth

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"net/http"
	"regexp"
	"strings"
	"time"

	"github.com/coreos/go-oidc/v3/oidc"

	"github.com/conghai20/haryx/server/api/internal/web"
)

// Audience is what the realm's audience mapper puts in `aud` for both HaryX clients.
const Audience = "haryx-api"

// Clients are the realm clients whose tokens the API accepts (`azp`).
var Clients = map[string]bool{"haryx-cli": true, "haryx-platform": true}

// Keycloak subjects are UUIDs, and every table keys on user_id uuid.
var uuidPattern = regexp.MustCompile(`^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$`)

// ErrUnauthorized is any reason a token is refused. The detail is logged, never sent.
var ErrUnauthorized = errors.New("unauthorized")

// User is the caller, taken from a verified token.
type User struct {
	ID        string // `sub`
	SessionID string // `sid`: the Keycloak session, one per signed-in machine or browser
	Client    string // `azp`
}

// RevokedFunc reports whether a session was signed out from the web (revoked_sessions).
type RevokedFunc func(ctx context.Context, sessionID string) (bool, error)

// Verifier checks tokens. It is safe for concurrent use.
type Verifier struct {
	verifier *oidc.IDTokenVerifier
	revoked  RevokedFunc
}

// New checks tokens against keys. Production passes NewRemote's key set.
func New(issuer string, keys oidc.KeySet, revoked RevokedFunc) *Verifier {
	// go-oidc's verifier fits access tokens too: it checks the signature, iss, exp and that aud
	// contains ClientID. Only RS256, the realm's algorithm, is accepted, so alg=none and HS256
	// signed with the public key are refused.
	cfg := &oidc.Config{ClientID: Audience, SupportedSigningAlgs: []string{oidc.RS256}}
	return &Verifier{verifier: oidc.NewVerifier(issuer, keys, cfg), revoked: revoked}
}

// NewRemote reads the realm's keys from Keycloak's JWKS endpoint, again when a token names an
// unknown kid (a key rotation). No discovery request is made at start, so the API starts even
// while Keycloak is down.
func NewRemote(issuer string, revoked RevokedFunc) *Verifier {
	ctx := oidc.ClientContext(context.Background(), &http.Client{Timeout: 10 * time.Second})
	keys := oidc.NewRemoteKeySet(ctx, strings.TrimSuffix(issuer, "/")+"/protocol/openid-connect/certs")
	return New(issuer, keys, revoked)
}

type claims struct {
	Typ string `json:"typ"`
	Azp string `json:"azp"`
	Sid string `json:"sid"`
}

// Verify returns the caller, ErrUnauthorized (wrapped with the reason), or a revoked-check error.
func (v *Verifier) Verify(ctx context.Context, raw string) (User, error) {
	token, err := v.verifier.Verify(ctx, raw)
	if err != nil {
		return User{}, fmt.Errorf("%w: %v", ErrUnauthorized, err)
	}
	var c claims
	if err := token.Claims(&c); err != nil {
		return User{}, fmt.Errorf("%w: claims: %v", ErrUnauthorized, err)
	}
	switch {
	case c.Typ != "Bearer":
		return User{}, fmt.Errorf("%w: typ %q is not an access token", ErrUnauthorized, c.Typ)
	case !Clients[c.Azp]:
		return User{}, fmt.Errorf("%w: azp %q", ErrUnauthorized, c.Azp)
	case !uuidPattern.MatchString(token.Subject):
		return User{}, fmt.Errorf("%w: sub %q", ErrUnauthorized, token.Subject)
	case c.Sid == "":
		// Without a session id the token could never be revoked from the web.
		return User{}, fmt.Errorf("%w: no sid", ErrUnauthorized)
	}
	revoked, err := v.revoked(ctx, c.Sid)
	if err != nil {
		return User{}, fmt.Errorf("revoked-session check: %w", err)
	}
	if revoked {
		return User{}, fmt.Errorf("%w: session %s signed out", ErrUnauthorized, c.Sid)
	}
	return User{ID: token.Subject, SessionID: c.Sid, Client: c.Azp}, nil
}

type contextKey struct{}

// Middleware lets a request through only with a valid token, and puts the caller in its context.
func (v *Verifier) Middleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		raw, ok := bearer(r)
		if !ok {
			unauthorized(w)
			return
		}
		user, err := v.Verify(r.Context(), raw)
		if errors.Is(err, ErrUnauthorized) {
			slog.Info("token refused", "path", r.URL.Path, "reason", err)
			unauthorized(w)
			return
		}
		if err != nil {
			// Fail closed: a session that might be revoked is not let through.
			slog.Error("auth", "path", r.URL.Path, "err", err)
			web.Error(w, http.StatusServiceUnavailable, "unavailable")
			return
		}
		next.ServeHTTP(w, r.WithContext(WithUser(r.Context(), user)))
	})
}

// WithUser puts a caller in ctx, as Middleware does. Handler tests use it to skip tokens.
func WithUser(ctx context.Context, user User) context.Context {
	return context.WithValue(ctx, contextKey{}, user)
}

// FromContext returns the caller that Middleware put in the request's context.
func FromContext(ctx context.Context) (User, bool) {
	user, ok := ctx.Value(contextKey{}).(User)
	return user, ok
}

func bearer(r *http.Request) (string, bool) {
	scheme, token, ok := strings.Cut(r.Header.Get("Authorization"), " ")
	if !ok || !strings.EqualFold(scheme, "Bearer") || token == "" {
		return "", false
	}
	return token, true
}

func unauthorized(w http.ResponseWriter) {
	w.Header().Set("WWW-Authenticate", `Bearer realm="haryx"`)
	web.Error(w, http.StatusUnauthorized, "unauthorized")
}
