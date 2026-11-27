#!/usr/bin/env bash
# Makes deploy/.env from .env.example, or completes an existing one. Rerunning never changes a
# value already set: it only fills generated values that are missing and names hand-filled ones
# that are still empty.
set -euo pipefail
cd "$(dirname "$0")"

ENV_FILE=.env
GENERATED=(HARYX_DB_PASSWORD HARYX_API_ADMIN_SECRET)
HAND=(APPLE_SERVICES_ID APPLE_TEAM_ID APPLE_KEY_ID APPLE_P8_FILE SMTP_USER SMTP_PASSWORD)

[ -f "$ENV_FILE" ] || { cp .env.example "$ENV_FILE"; chmod 600 "$ENV_FILE"; }

value_of() { sed -n "s/^$1=\([^ #]*\).*/\1/p" "$ENV_FILE" | head -1; }

set_value() { # name value — replace the line, or append it if the file predates the variable
    local tmp
    tmp=$(mktemp)
    if grep -q "^$1=" "$ENV_FILE"; then
        awk -v k="$1" -v v="$2" 'index($0, k "=") == 1 { print k "=" v; next } { print }' "$ENV_FILE" >"$tmp"
    else
        cat "$ENV_FILE" >"$tmp"
        printf '%s=%s\n' "$1" "$2" >>"$tmp"
    fi
    cat "$tmp" >"$ENV_FILE"
    rm -f "$tmp"
}

# Hex: safe in a URL, a shell, YAML and a Keycloak client secret without quoting.
for name in "${GENERATED[@]}"; do
    if [ -z "$(value_of "$name")" ]; then
        set_value "$name" "$(openssl rand -hex 32)"
        echo "generated $name"
    fi
done

missing=()
for name in "${HAND[@]}"; do
    [ -n "$(value_of "$name")" ] || missing+=("$name")
done
if [ ${#missing[@]} -gt 0 ]; then
    echo "still to fill in $PWD/$ENV_FILE (configure-haryx.mjs skips Apple or SMTP without them):"
    printf '  %s\n' "${missing[@]}"
fi
