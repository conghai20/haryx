#!/usr/bin/env bash
# Local realm for tests: configure-local.mjs against a throwaway Keycloak 26.8.2 (the live version), run
# twice to prove it is idempotent, then real sign-ins against the result.
#
#   deploy/keycloak/test-local.sh
#   APPLE_JAR=path/to/apple-identity-provider-1.17.0.jar deploy/keycloak/test-local.sh
#
# With APPLE_JAR, the Apple provider is configured too, with a throwaway key.
set -euo pipefail
cd "$(dirname "$0")"

IMAGE=quay.io/keycloak/keycloak:26.8.2
NAME=haryx-kc-test
PORT=${PORT:-18083}
WORK=$(mktemp -d)
trap 'docker stop "$NAME" >/dev/null 2>&1 || true' EXIT

mounts=()
if [ -n "${APPLE_JAR:-}" ]; then
    mounts=(-v "$(cd "$(dirname "$APPLE_JAR")" && pwd)/$(basename "$APPLE_JAR"):/opt/keycloak/providers/apple.jar:ro")
    openssl ecparam -name prime256v1 -genkey -noout 2>/dev/null | openssl pkcs8 -topk8 -nocrypt -out "$WORK/AuthKey_TEST.p8"
    export APPLE_SERVICES_ID=com.wecoap.sso.test APPLE_TEAM_ID=TEAM123456 APPLE_KEY_ID=KEY1234567 APPLE_P8_FILE="$WORK/AuthKey_TEST.p8"
else
    unset APPLE_SERVICES_ID APPLE_TEAM_ID APPLE_KEY_ID APPLE_P8_FILE
fi

# A previous run's --rm container can still be shutting down.
docker rm -f haryx-kc-test >/dev/null 2>&1 || true
docker run -d --rm --name "$NAME" -p "127.0.0.1:$PORT:8080" ${mounts[@]+"${mounts[@]}"} \
    -e KC_BOOTSTRAP_ADMIN_USERNAME=admin -e KC_BOOTSTRAP_ADMIN_PASSWORD=test-admin-password \
    "$IMAGE" start-dev >/dev/null
for _ in $(seq 1 90); do
    curl -sf -o /dev/null "http://127.0.0.1:$PORT/realms/master" && break
    sleep 2
done

export KC_URL="http://127.0.0.1:$PORT" KC_REALM=wecoap KC_ADMIN_USER=admin KC_ADMIN_PASSWORD=test-admin-password
export HARYX_PLATFORM_URL=http://127.0.0.1:5173

node test-haryx.mjs setup
node configure-local.mjs
node test-haryx.mjs snapshot "$WORK/first.json"
node test-haryx.mjs tamper
node configure-local.mjs
node test-haryx.mjs snapshot "$WORK/second.json"
if diff -u "$WORK/first.json" "$WORK/second.json"; then
    echo "PASS a second run undoes hand edits and changes nothing else"
else
    echo "FAIL a second run changed the realm"
    exit 1
fi
node -e 'const s = require(process.argv[1]); console.log(JSON.stringify({ realm: s.realm, clients: s.adminClients, webAccountScope: s["haryx-platform"].accountScope, cliAccountScope: s["haryx-cli"].accountScope, cliOptional: s["haryx-cli"].optionalScopes, webOptional: s["haryx-platform"].optionalScopes, providers: s.providers.map(p => p.alias + (p.trustEmail ? " TRUSTS EMAIL" : "")) }))' "$WORK/second.json"
node test-haryx.mjs check
