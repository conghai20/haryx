#!/usr/bin/env bash
# The API's checks, in Docker (no Go on the host needed): gofmt, go vet, and go test -race with
# coverage against a throwaway Postgres 18.
#
#   server/api/test.sh            all packages
#   server/api/test.sh -run Auth  extra args go to `go test`
set -euo pipefail
cd "$(dirname "$0")"

GO_IMAGE=golang:1.27   # not -alpine: -race needs cgo
NET=haryx-api-test
PG=haryx-api-test-pg
cleanup() { docker rm -f "$PG" >/dev/null 2>&1 || true; docker network rm "$NET" >/dev/null 2>&1 || true; }
trap cleanup EXIT
cleanup

docker network create "$NET" >/dev/null
docker run -d --rm --name "$PG" --network "$NET" \
    -e POSTGRES_DB=haryx -e POSTGRES_USER=haryx -e POSTGRES_PASSWORD=test postgres:18-alpine >/dev/null
for _ in $(seq 1 30); do
    docker exec "$PG" pg_isready -U haryx -d haryx >/dev/null 2>&1 && break
    sleep 1
done

docker run --rm --network "$NET" -v "$PWD:/src" -w /src \
    -v haryx-go-mod:/go/pkg/mod -v haryx-go-build:/root/.cache/go-build \
    -v "$PWD/../../packages/contracts:/contracts:ro" -e HARYX_CONTRACTS=/contracts \
    -e PGHOST="$PG" -e PGDATABASE=haryx -e PGUSER=haryx -e PGPASSWORD=test \
    "$GO_IMAGE" sh -c '
        set -e
        unformatted=$(gofmt -l .)
        [ -z "$unformatted" ] || { echo "gofmt needed:"; echo "$unformatted"; exit 1; }
        go vet ./...
        go test -race -p 1 -count=1 -coverpkg=./... -coverprofile=/tmp/cover.out "$@" ./...
        go tool cover -func=/tmp/cover.out | grep -v "100.0%"
    ' sh "$@"
