#!/usr/bin/env bash
# End to end (plan task 13): the real `haryx` binary as two machines of one WeCoAp ID, against a
# local copy of the production stack: Keycloak 26.8.2 configured by configure-local.mjs, the HaryX
# API built from server/api, and Postgres 18. Nothing here touches sso.wecoap.com.
#
#   cd cli && cargo build && cd .. && deploy/cli-e2e.sh
#
# Needs: Docker, node 22+, python3, curl, xxd (macOS: sessions go to a fake `security`).
#
# The three containers share Keycloak's network namespace, so the CLI on this machine and the API
# in its container both reach Keycloak as http://127.0.0.1:$KC_PORT, and tokens carry that issuer.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
BIN=${BIN:-$ROOT/cli/target/debug/haryx}
KC_PORT=${KC_PORT:-18084}
API_PORT=${API_PORT:-18181}
KC_IMAGE=quay.io/keycloak/keycloak:26.8.2
KC=haryx-e2e-kc PG=haryx-e2e-pg API=haryx-e2e-api
ISSUER=http://127.0.0.1:$KC_PORT/realms/wecoap
API_URL=http://127.0.0.1:$API_PORT/api/v1
PASS="correct horse battery staple"
T=$(mktemp -d)

cleanup() {
    docker rm -f "$API" "$PG" "$KC" >/dev/null 2>&1 || true
    rm -rf "$T"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok - $*"; }
sql() { docker exec -i "$PG" psql -U haryx -d haryx -tA -c "$1"; }
json() { python3 -I -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
wait_for() { for _ in $(seq 1 "$2"); do curl -sf -o /dev/null "$1" && return 0; sleep 2; done; fail "$1 never answered"; }

[ -x "$BIN" ] || fail "build the CLI first: cd cli && cargo build"
cleanup
mkdir -p "$T"

# --- the stack ------------------------------------------------------------------------------------
docker run -d --rm --name "$KC" -p "127.0.0.1:$KC_PORT:$KC_PORT" -p "127.0.0.1:$API_PORT:8080" \
    -e KC_HTTP_PORT="$KC_PORT" \
    -e KC_BOOTSTRAP_ADMIN_USERNAME=admin -e KC_BOOTSTRAP_ADMIN_PASSWORD=test-admin-password \
    "$KC_IMAGE" start-dev >/dev/null
docker run -d --rm --name "$PG" --network "container:$KC" \
    -e POSTGRES_DB=haryx -e POSTGRES_USER=haryx -e POSTGRES_PASSWORD=e2e postgres:18-alpine >/dev/null
docker build -q -t haryx-api:e2e "$ROOT/server/api" >/dev/null
wait_for "http://127.0.0.1:$KC_PORT/realms/master" 90

export KC_URL="http://127.0.0.1:$KC_PORT" KC_REALM=wecoap KC_ADMIN_USER=admin KC_ADMIN_PASSWORD=test-admin-password
export HARYX_PLATFORM_URL=http://127.0.0.1:5173
unset APPLE_SERVICES_ID APPLE_TEAM_ID APPLE_KEY_ID APPLE_P8_FILE
node "$ROOT/deploy/keycloak/test-haryx.mjs" setup >/dev/null
node "$ROOT/deploy/keycloak/configure-local.mjs" >/dev/null

for _ in $(seq 1 30); do docker exec "$PG" pg_isready -U haryx -d haryx >/dev/null 2>&1 && break; sleep 1; done
docker run -d --rm --name "$API" --network "container:$KC" \
    -e HARYX_SSO_ISSUER="$ISSUER" -e PGHOST=127.0.0.1 -e PGPORT=5432 -e PGDATABASE=haryx \
    -e PGUSER=haryx -e PGPASSWORD=e2e haryx-api:e2e >/dev/null
wait_for "$API_URL/health" 30
ok "local stack up: Keycloak $KC_PORT, API $API_PORT"

# --- two machines -------------------------------------------------------------------------------
# A fake `security` keeping Keychain items as files (macOS); other systems use the file vault.
fake_security() {
    cat > "$1/security" <<'EOF'
#!/bin/sh
dir="$(dirname "$0")/keychain"; mkdir -p "$dir"
if [ "$1" = "-i" ]; then set -- $(cat | tr -d '"'); fi
op="$1"; shift
while [ $# -gt 0 ]; do case "$1" in -a) a="$2"; shift 2;; -s) s="$2"; shift 2;; -X) x="$2"; shift 2;; *) shift;; esac; done
f="$dir/$s.$a"
case "$op" in
  add-generic-password) printf '%s' "$x" | xxd -r -p > "$f";;
  find-generic-password) [ -f "$f" ] || exit 44; cat "$f"; echo;;
  delete-generic-password) [ -f "$f" ] || exit 44; rm "$f";;
  *) exit 44;;
esac
EOF
    chmod +x "$1/security"
}

hx() {
    m=$1; shift
    env HOME="$T/$m" HARYX_HOME="$T/$m/store" HARYX_SECURITY_BIN="$T/$m/security" \
        HARYX_SSO_ISSUER="$ISSUER" HARYX_API_URL="$API_URL" CLAUDE_CONFIG_DIR= NO_COLOR=1 \
        "$BIN" "$@"
}

# `haryx login --no-browser`, approved the way a person would on another device.
login() {
    m=$1
    mkdir -p "$T/$m/.claude" "$T/$m/store"
    fake_security "$T/$m"
    hx "$m" login --no-browser --json > "$T/$m.login" 2>&1 &
    pid=$!
    url=
    for _ in $(seq 1 30); do
        url=$(grep -m1 '"event":"browser"' "$T/$m.login" 2>/dev/null | json 'd["url"]' 2>/dev/null || true)
        [ -n "$url" ] && break
        sleep 0.5
    done
    [ -n "$url" ] || fail "$m: no sign-in link: $(cat "$T/$m.login")"
    node "$ROOT/deploy/keycloak/test-haryx.mjs" approve "$url"
    wait "$pid" || fail "$m: login: $(cat "$T/$m.login")"
    grep -q '"event":"signed_in"' "$T/$m.login" || fail "$m: not signed in: $(cat "$T/$m.login")"
}

login A
login B
[ "$(hx A whoami --json | json 'd["signedIn"]')" = True ] || fail "whoami A"
[ "$(hx B whoami --json | json 'd["user"]["provider"]')" = email ] || fail "whoami B provider"
ok "two machines sign in with the device grant"

# --- consent gates sync ---------------------------------------------------------------------------
out=$(hx A sync has-passphrase --json || true)
[ "$(echo "$out" | json 'd.get("code")')" = consent_required ] || fail "sync before consent: $out"
[ "$(hx A account consent grant sync --json | json '[c["granted"] for c in d["consents"] if c["purpose"]=="sync"][0]')" = True ] || fail "grant sync"
[ "$(hx B account consents --json | json '[c["granted"] for c in d["consents"] if c["purpose"]=="sync"][0]')" = True ] ||
    fail "B sees the consent"
[ "$(sql "select count(*) from consent_events where purpose = 'sync' and action = 'grant'")" = 1 ] ||
    fail "one grant event"
ok "sync waits for the Sync consent; one grant covers both machines"

# --- passphrase, settings, preferences, an API-key account --------------------------------------
[ "$(hx A sync has-passphrase --json | json 'd["hasPassphrase"]')" = False ] || fail "fresh account"
[ "$(printf '%s\n' "$PASS" | hx A sync unlock --passphrase-stdin --json | json 'd["created"]')" = True ] ||
    fail "A sets the passphrase"
[ "$(printf 'wrong passphrase!\n' | hx B sync unlock --passphrase-stdin --json | json 'd["code"]')" = wrong_passphrase ] ||
    fail "B refuses a wrong passphrase"
[ "$(printf '%s\n' "$PASS" | hx B sync unlock --passphrase-stdin --json | json 'd["created"]')" = False ] ||
    fail "B unlocks"
ok "the first machine sets the passphrase; the second must know it"

hx A config set autoswitch.threshold 77 >/dev/null
printf '{"theme":"dark","mascotStyles":{"x@example.com":{"species":"otter"}}}' | hx A sync prefs set >/dev/null
hx A add-token sk-ant-api03-e2e-test-key --email ci@example.test >/dev/null
[ "$(hx A sync --json | json 'd["ok"]')" = True ] || fail "A syncs"
out=$(hx B sync --json)
echo "$out" | json '",".join(sorted(d["changed"]))' | grep -q "app-prefs,credential,settings" ||
    fail "B received settings, prefs and the token account: $out"
case "$(hx B config get autoswitch.threshold)" in 77|77.0) ;; *) fail "setting on B";; esac
[ "$(hx B sync prefs get | json 'd["theme"]')" = dark ] || fail "prefs on B"
hx B list --json | grep -q ci@example.test || fail "token account on B"
ok "settings, app preferences and an API-key account travel A -> B"

# --- usage (task 12) ------------------------------------------------------------------------------
usage_row() {
    dir="$T/$1/store/usage/days"; mkdir -p "$dir"
    hour=$(( $(date +%s) / 3600 * 3600 ))
    day=$(python3 -I -c "import datetime,sys; print(datetime.datetime.fromtimestamp(int(sys.argv[1]), datetime.timezone.utc).strftime('%Y-%m-%d'))" "$hour")
    printf '{"hour":%s,"account":1,"project":"/Users/me/secret-project","session":"sess-e2e","model":"claude-opus-4-5","replies":3,"input":10,"output":%s,"cacheRead":0,"cacheWrite5m":0,"cacheWrite1h":0}\n' \
        "$hour" "$2" >> "$dir/$day.jsonl"
}
usage_row A 500
usage_row B 70
hx A sync --json >/dev/null
hx B sync --json >/dev/null
[ "$(sql "select count(*) from sync_rows where kind = 'usage'")" = 2 ] || fail "one usage row per machine"
ok "each machine writes its own monthly usage row"

body=$(sql "select string_agg(body || key, '') from sync_rows")
for leak in ci@example.test sk-ant-api03 autoswitch dark otter secret-project sess-e2e claude-opus; do
    echo "$body" | grep -q "$leak" && fail "the server can read '$leak'"
done
ok "the server holds ciphertext and opaque keys only"

# --- profiles ---------------------------------------------------------------------------------------
echo '{"permissions":{"allow":["Bash(ls)"]}}' > "$T/A/.claude/settings.json"
hx A ai profile save work >/dev/null
hx A sync profile push work >/dev/null
[ "$(hx B sync profile list --json | json 'd["profiles"][0]["name"]')" = work ] || fail "profile list"
hx B sync profile pull work >/dev/null
hx B ai profile list | grep -q work || fail "profile imported on B"
[ "$(sql "select count(*) from profile_files")" = 1 ] || fail "one encrypted profile file"
ok "a setup profile travels A -> B, encrypted"

# --- machines --------------------------------------------------------------------------------------
[ "$(hx A account consent grant machines --json | json '[c["granted"] for c in d["consents"] if c["purpose"]=="machines"][0]')" = True ] || fail "grant machines"
hx A sync --json >/dev/null
hx B sync --json >/dev/null
machines=$(hx A account machines --json)
[ "$(echo "$machines" | json 'len(d["machines"])')" = 2 ] || fail "two machines: $machines"
[ "$(echo "$machines" | json 'sum(m["isThisMachine"] for m in d["machines"])')" = 1 ] || fail "this machine: $machines"
id=$(echo "$machines" | json '[m["machineId"] for m in d["machines"] if m["isThisMachine"]][0]')
hx A account machine rename "$id" "E2E laptop" --json >/dev/null
hx A sync --json >/dev/null
[ "$(hx B account machines --json | json '[m["name"] for m in d["machines"] if m["machineId"]=="'"$id"'"][0]')" = "E2E laptop" ] ||
    fail "a rename survives the next heartbeat"
ok "the machine registry lists both machines and keeps a rename"

# --- export ----------------------------------------------------------------------------------------
hx A account export "$T/export.json" >/dev/null
python3 -I -c '
import json, sys
d = json.load(open(sys.argv[1]))
text = json.dumps(d)
assert "E2E laptop" in text, "machines in the export"
assert "grant" in text, "consent history in the export"
' "$T/export.json" || fail "export"
ok "export holds the machines and the consent history"

# --- withdrawal erases ----------------------------------------------------------------------------
[ "$(hx B account consent withdraw sync --yes --json | json '[c["granted"] for c in d["consents"] if c["purpose"]=="sync"][0]')" = False ] || fail "withdraw sync"
[ "$(sql "select count(*) from sync_rows")" = 0 ] || fail "rows gone"
[ "$(sql "select count(*) from profile_files")" = 0 ] || fail "profiles gone"
[ "$(sql "select count(*) from consent_events where purpose = 'sync' and action = 'withdraw'")" = 1 ] ||
    fail "the withdrawal is recorded"
code=$(hx A sync --json | json 'd.get("code")' || true)
[ "$code" = consent_required ] || fail "A after the withdrawal: $code"
ok "withdrawing Sync erases rows and profiles; the other machine stops ($code)"

# --- sign out --------------------------------------------------------------------------------------
hx A logout --json >/dev/null
[ "$(hx A whoami --json | json 'd["signedIn"]')" = False ] || fail "A signed out"
[ "$(hx B whoami --json | json 'd["signedIn"]')" = True ] || fail "B stays signed in"
ok "signing out one machine leaves the other"

echo "PASS"
