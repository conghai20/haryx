#!/bin/sh
# End to end: the real `haryx` binary as two machines of one HaryX account, against the local
# Supabase stack (real PostgREST, row-level security, Storage and the delete-account function).
#
#   cd server && npx supabase start && sh supabase/tests/cli-e2e.sh
#
# Needs: a debug build (cli/target/debug/haryx), curl, python3, xxd. Signs users up with email and
# password, which the local stack confirms automatically; the hosted project has email sign-up off.
set -eu

API=${API:-http://127.0.0.1:54321}
KEY=${KEY:-sb_publishable_ACJWlzQHlZjBrEguHvfOxg_3BJgxAaH}
DB=${DB:-postgresql://postgres:postgres@127.0.0.1:54322/postgres}
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
BIN=$ROOT/cli/target/debug/haryx
PASS="correct horse battery staple"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
EMAIL="e2e-$(date +%s)@example.test"

fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok - $*"; }
sql() { docker exec -i supabase_db_haryx psql -U postgres -tA -c "$1"; }

# A fake `security` keeping Keychain items as files, so nothing touches this Mac's Keychain.
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

# Sign in with the password and store the session the way `haryx login` would.
machine() {
  dir=$T/$1; mkdir -p "$dir/.claude" "$dir/store"
  fake_security "$dir"
  curl -sf -X POST "$API/auth/v1/token?grant_type=password" -H "apikey: $KEY" \
    -H 'content-type: application/json' -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" |
    python3 -I -c '
import json, sys
s = json.load(sys.stdin)
print(json.dumps({"access_token": s["access_token"], "refresh_token": s["refresh_token"],
  "expires_at": s["expires_at"], "project": sys.argv[1],
  "user": {"id": s["user"]["id"], "email": s["user"]["email"], "provider": "email"}}))' "$API" \
    > "$dir/session.json"
  if [ "$(uname)" = Darwin ]; then
    mkdir -p "$dir/keychain" && cp "$dir/session.json" "$dir/keychain/haryx-account.session"
  else
    echo "this script stores sessions the macOS way; run it on macOS" >&2; exit 2
  fi
}

hx() {
  m=$1; shift
  env HOME="$T/$m" HARYX_HOME="$T/$m/store" HARYX_SECURITY_BIN="$T/$m/security" \
    HARYX_SUPABASE_URL="$API" HARYX_SUPABASE_KEY="$KEY" CLAUDE_CONFIG_DIR= NO_COLOR=1 \
    "$BIN" "$@"
}

json() { python3 -I -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

[ -x "$BIN" ] || fail "build the CLI first: cd cli && cargo build"
curl -sf -X POST "$API/auth/v1/signup" -H "apikey: $KEY" -H 'content-type: application/json' \
  -d "{\"email\":\"$EMAIL\",\"password\":\"$PASS\"}" > /dev/null || fail "sign-up"
machine A
machine B

[ "$(hx A whoami --json | json 'd["signedIn"]')" = True ] || fail "whoami"
ok "both machines are signed in"

[ "$(hx A sync has-passphrase --json | json 'd["hasPassphrase"]')" = False ] || fail "fresh account"
[ "$(printf '%s\n' "$PASS" | hx A sync unlock --passphrase-stdin --json | json 'd["created"]')" = True ] ||
  fail "A sets the passphrase"
[ "$(printf 'wrong passphrase!\n' | hx B sync unlock --passphrase-stdin --json | json 'd["code"]')" = wrong_passphrase ] ||
  fail "B refuses a wrong passphrase"
[ "$(printf '%s\n' "$PASS" | hx B sync unlock --passphrase-stdin --json | json 'd["created"]')" = False ] ||
  fail "B unlocks"
ok "the first machine sets the passphrase; the second must know it"

hx A config set autoswitch.threshold 77 > /dev/null
printf '{"theme":"dark","mascotStyles":{"x@example.com":{"species":"otter"}}}' | hx A sync prefs set > /dev/null
hx A add-token sk-ant-api03-e2e-test-key --email ci@example.test > /dev/null
[ "$(hx A sync --json | json 'd["ok"]')" = True ] || fail "A syncs"
out=$(hx B sync --json)
echo "$out" | json '",".join(sorted(d["changed"]))' | grep -q "app-prefs,credential,settings" ||
  fail "B received settings, prefs and the token account: $out"
[ "$(hx B config get autoswitch.threshold)" = 77 ] || [ "$(hx B config get autoswitch.threshold)" = 77.0 ] ||
  fail "setting on B: $(hx B config get autoswitch.threshold)"
[ "$(hx B sync prefs get | json 'd["theme"]')" = dark ] || fail "prefs on B"
hx B list --json | grep -q ci@example.test || fail "token account on B"
ok "settings, app preferences and an API-key account travel A -> B"

body=$(sql "select string_agg(body || key, '') from public.sync_rows")
for leak in ci@example.test sk-ant-api03 autoswitch dark otter; do
  echo "$body" | grep -q "$leak" && fail "the server can read '$leak'"
done
ok "the server holds ciphertext and opaque keys only"

echo '{"permissions":{"allow":["Bash(ls)"]}}' > "$T/A/.claude/settings.json"
hx A ai profile save work > /dev/null
hx A sync profile push work > /dev/null
[ "$(hx B sync profile list --json | json 'd["profiles"][0]["name"]')" = work ] || fail "profile list"
hx B sync profile pull work > /dev/null
hx B ai profile list | grep -q work || fail "profile imported on B"
[ "$(sql "select count(*) from storage.objects where bucket_id = 'profiles' and name not like '%work%'")" = 1 ] ||
  fail "one encrypted object with an opaque name"
ok "a setup profile travels A -> B through Storage, encrypted"

hx A config set autoswitch.threshold 66 > /dev/null
hx A sync --json > /dev/null
hx B sync --json > /dev/null
[ "$(hx B config get autoswitch.threshold)" = 66 ] || [ "$(hx B config get autoswitch.threshold)" = 66.0 ] ||
  fail "an update travels"
ok "a later change travels too"

[ "$(hx A account delete --yes --json | json 'd["ok"]')" = True ] || fail "delete account"
[ "$(sql "select count(*) from auth.users where email = '$EMAIL'")" = 0 ] || fail "user gone"
[ "$(sql "select count(*) from public.sync_rows")" = 0 ] || fail "rows gone"
[ "$(sql "select count(*) from storage.objects where bucket_id = 'profiles'")" = 0 ] || fail "files gone"
[ "$(hx A whoami --json | json 'd["signedIn"]')" = False ] || fail "A signed out"
code=$(hx B sync --json | json 'd["code"]' || true)
case "$code" in signed_out|refused|key_changed|server) ;; *) fail "B after deletion: $code";; esac
ok "account delete removes the user, rows and files; the other machine stops ($code)"

echo "PASS"
