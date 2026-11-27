#!/bin/sh
# Nightly pg_dump of the haryx database, kept 14 days. Runs in the backup container.
#
#   backup.sh          loop: one dump per day, after 03:00 (TZ), survives restarts
#   backup.sh once     one dump now, then exit (restore drill, first deploy)
#
# Connection comes from the PG* variables compose sets.
set -eu

KEEP_DAYS=14
HOUR=3
DIR=/backups

dump() {
    stamp=$(date +%Y%m%d-%H%M%S)
    tmp="$DIR/.haryx-$stamp.dump.partial"
    # -Fc: compressed, restorable table by table. A failed dump never replaces a good one.
    if ! pg_dump -Fc -f "$tmp"; then
        rm -f "$tmp"
        return 1
    fi
    mv "$tmp" "$DIR/haryx-$stamp.dump"
    find "$DIR" -name 'haryx-*.dump' -mtime +"$KEEP_DAYS" -delete
    echo "backup: wrote $DIR/haryx-$stamp.dump"
}

mkdir -p "$DIR"
if [ "${1:-}" = once ]; then
    dump
    exit 0
fi

while :; do
    today=$(date +%Y%m%d)
    if [ "$(date +%H)" -ge "$HOUR" ] && ! ls "$DIR"/haryx-"$today"-*.dump >/dev/null 2>&1; then
        dump || echo "backup: pg_dump failed, retrying next hour" >&2
    fi
    sleep 3600
done
