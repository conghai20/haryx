package auth

import (
	"context"
	"crypto/rand"
	"crypto/rsa"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	jose "github.com/go-jose/go-jose/v4"
)

const (
	testSub = "3f0c6a52-6a3e-4d2b-9a57-0b7d1d6f9e21"
	testSid = "8b1d2c3e-0000-4000-8000-000000000001"
)

// realm is a fake Keycloak: an RSA key published on a JWKS endpoint, and a token minter.
type realm struct {
	issuer string
	key    *rsa.PrivateKey
	kid    string
}

func newRealm(t *testing.T) *realm {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	r := &realm{key: key, kid: "realm-key"}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_ = json.NewEncoder(w).Encode(jose.JSONWebKeySet{Keys: []jose.JSONWebKey{
			{Key: &key.PublicKey, KeyID: r.kid, Algorithm: "RS256", Use: "sig"},
		}})
	}))
	t.Cleanup(srv.Close)
	r.issuer = srv.URL + "/realms/wecoap"
	return r
}

// verifier is the production one: keys fetched from the realm's certs endpoint.
func (r *realm) verifier(revoked RevokedFunc) *Verifier {
	return NewRemote(r.issuer, revoked)
}

// claims of a good haryx-cli access token, as Keycloak 26.8.2 issues it.
func (r *realm) claims() map[string]any {
	now := time.Now()
	return map[string]any{
		"iss": r.issuer, "sub": testSub, "aud": []string{Audience, "account"},
		"azp": "haryx-cli", "typ": "Bearer", "sid": testSid,
		"iat": now.Unix(), "exp": now.Add(5 * time.Minute).Unix(),
	}
}

func sign(t *testing.T, key *rsa.PrivateKey, kid string, claims map[string]any) string {
	t.Helper()
	opts := (&jose.SignerOptions{}).WithType("JWT").WithHeader("kid", kid)
	signer, err := jose.NewSigner(jose.SigningKey{Algorithm: jose.RS256, Key: key}, opts)
	if err != nil {
		t.Fatal(err)
	}
	payload, _ := json.Marshal(claims)
	jws, err := signer.Sign(payload)
	if err != nil {
		t.Fatal(err)
	}
	out, err := jws.CompactSerialize()
	if err != nil {
		t.Fatal(err)
	}
	return out
}

func unsigned(alg string, claims map[string]any) string {
	enc := base64.RawURLEncoding.EncodeToString
	header, _ := json.Marshal(map[string]string{"alg": alg, "typ": "JWT"})
	payload, _ := json.Marshal(claims)
	return enc(header) + "." + enc(payload) + "."
}

func notRevoked(context.Context, string) (bool, error) { return false, nil }

func TestMiddleware(t *testing.T) {
	r := newRealm(t)
	otherKey, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	with := func(change func(c map[string]any)) string {
		c := r.claims()
		change(c)
		return sign(t, r.key, r.kid, c)
	}
	good := sign(t, r.key, r.kid, r.claims())

	tests := []struct {
		name    string
		header  string
		revoked RevokedFunc
		want    int
	}{
		{"good haryx-cli token", "Bearer " + good, nil, http.StatusOK},
		{"good haryx-platform token", "Bearer " + with(func(c map[string]any) { c["azp"] = "haryx-platform" }), nil, http.StatusOK},
		{"aud as a single string", "Bearer " + with(func(c map[string]any) { c["aud"] = Audience }), nil, http.StatusOK},
		{"scheme is case-insensitive", "bearer " + good, nil, http.StatusOK},
		{"no Authorization header", "", nil, http.StatusUnauthorized},
		{"not a Bearer scheme", "Basic " + good, nil, http.StatusUnauthorized},
		{"empty token", "Bearer ", nil, http.StatusUnauthorized},
		{"garbage", "Bearer not.a.jwt", nil, http.StatusUnauthorized},
		{"wrong iss", "Bearer " + with(func(c map[string]any) { c["iss"] = "https://sso.example.com/realms/wecoap" }), nil, http.StatusUnauthorized},
		{"another realm on the same host", "Bearer " + with(func(c map[string]any) { c["iss"] = strings.Replace(r.issuer, "wecoap", "master", 1) }), nil, http.StatusUnauthorized},
		{"wrong aud (AR Onion's API)", "Bearer " + with(func(c map[string]any) { c["aud"] = []string{"aronion-api", "account"} }), nil, http.StatusUnauthorized},
		{"no aud", "Bearer " + with(func(c map[string]any) { delete(c, "aud") }), nil, http.StatusUnauthorized},
		{"wrong azp (another product's client)", "Bearer " + with(func(c map[string]any) { c["azp"] = "aronion-web" }), nil, http.StatusUnauthorized},
		{"no azp", "Bearer " + with(func(c map[string]any) { delete(c, "azp") }), nil, http.StatusUnauthorized},
		{"ID token, not an access token", "Bearer " + with(func(c map[string]any) { c["typ"] = "ID" }), nil, http.StatusUnauthorized},
		{"expired", "Bearer " + with(func(c map[string]any) { c["exp"] = time.Now().Add(-time.Minute).Unix() }), nil, http.StatusUnauthorized},
		{"sub not a uuid", "Bearer " + with(func(c map[string]any) { c["sub"] = "service-account-x" }), nil, http.StatusUnauthorized},
		{"no sid", "Bearer " + with(func(c map[string]any) { delete(c, "sid") }), nil, http.StatusUnauthorized},
		{"revoked sid", "Bearer " + good, func(_ context.Context, sid string) (bool, error) { return sid == testSid, nil }, http.StatusUnauthorized},
		{"revoked check fails: closed", "Bearer " + good, func(context.Context, string) (bool, error) { return false, errors.New("db down") }, http.StatusServiceUnavailable},
		{"alg=none", "Bearer " + unsigned("none", r.claims()), nil, http.StatusUnauthorized},
		{"HS256 with the public key as secret", "Bearer " + hs256WithPublicKey(t, r), nil, http.StatusUnauthorized},
		{"unknown kid, foreign key", "Bearer " + sign(t, otherKey, "attacker-key", r.claims()), nil, http.StatusUnauthorized},
		{"known kid, foreign key", "Bearer " + sign(t, otherKey, r.kid, r.claims()), nil, http.StatusUnauthorized},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			revoked := tt.revoked
			if revoked == nil {
				revoked = notRevoked
			}
			var got User
			h := r.verifier(revoked).Middleware(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
				got, _ = FromContext(req.Context())
			}))
			req := httptest.NewRequest(http.MethodGet, "/api/v1/sync/rows", nil)
			if tt.header != "" {
				req.Header.Set("Authorization", tt.header)
			}
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, req)

			if rec.Code != tt.want {
				t.Fatalf("status %d, want %d (body %s)", rec.Code, tt.want, rec.Body)
			}
			if tt.want == http.StatusOK {
				if got.ID != testSub || got.SessionID != testSid || !Clients[got.Client] {
					t.Fatalf("caller %+v", got)
				}
				return
			}
			if got != (User{}) {
				t.Fatalf("handler ran for a refused request: %+v", got)
			}
			if tt.want == http.StatusUnauthorized && rec.Header().Get("WWW-Authenticate") == "" {
				t.Fatal("401 without WWW-Authenticate")
			}
		})
	}
}

// The classic RS256→HS256 confusion: sign with HMAC, using the public key as the secret.
func hs256WithPublicKey(t *testing.T, r *realm) string {
	t.Helper()
	pub, err := (&jose.JSONWebKey{Key: &r.key.PublicKey}).MarshalJSON()
	if err != nil {
		t.Fatal(err)
	}
	signer, err := jose.NewSigner(jose.SigningKey{Algorithm: jose.HS256, Key: pub},
		(&jose.SignerOptions{}).WithType("JWT").WithHeader("kid", r.kid))
	if err != nil {
		t.Fatal(err)
	}
	payload, _ := json.Marshal(r.claims())
	jws, err := signer.Sign(payload)
	if err != nil {
		t.Fatal(err)
	}
	out, _ := jws.CompactSerialize()
	return out
}
