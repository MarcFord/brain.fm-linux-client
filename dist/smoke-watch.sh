#!/bin/sh
# Verify `bfm status --watch --json` survives the daemon coming and going.
#
# Property-change signals alone cannot tell a watcher that the daemon is not
# there, because a daemon that is not running emits nothing. That left two
# failures that looked like silence rather than errors, both reported from a
# real bar widget built on this stream:
#
#   1. Started with no daemon, the watcher printed one `unavailable` and then
#      stayed silent forever, even once a daemon appeared. It never exits, so
#      nothing retried it.
#   2. If the daemon died mid-stream the watcher printed nothing at all, so a
#      status bar went on showing `playing` for a daemon that was gone.
#
# Both are observable only by moving the daemon underneath a live watcher,
# which no unit test can do. Hence a smoke check.
#
# The bus must not be able to see an installed service file, or the watcher's
# first read activates a daemon and neither transition happens. The XDG
# variables therefore have to be set before dbus-run-session starts, not
# inside it -- getting that wrong makes this check quietly test nothing.
#
# Exits 0 on success, 1 on failure, 77 (skip) where it cannot run.

set -u

BIN=${BFM_BIN_DIR:-target/release}

if [ "${1:-}" != "--inner" ]; then
    command -v dbus-run-session >/dev/null 2>&1 || {
        echo "skip: no dbus-run-session"; exit 77; }
    # Not `A && B || C`: C runs when B fails too, which is the trap that had
    # already disabled several checks in this repo.
    if [ ! -x "$BIN/bfm" ] || [ ! -x "$BIN/bfmd" ]; then
        echo "skip: $BIN/bfm or $BIN/bfmd not built"
        exit 77
    fi
    exec env XDG_DATA_HOME=/nonexistent XDG_DATA_DIRS=/nonexistent \
        BFM_TOKEN_STORE=memory \
        dbus-run-session -- "$0" --inner
fi

BFM="$BIN/bfm"
BFMD="$BIN/bfmd"
OUT=$(mktemp -d)

# Only ever reap what this script started, by pid, never by name.
started=""
cleanup() {
    for p in $started; do kill -9 "$p" 2>/dev/null; done
    rm -rf "$OUT"
}
trap cleanup EXIT

fails=0
note() { printf '%s\n' "$*"; }
bad()  { printf 'FAIL: %s\n' "$*" >&2; fails=$((fails + 1)); }

# ------------------------------------------------- a watcher must recover
: > "$OUT/g1"
"$BFM" status --watch --json > "$OUT/g1" 2>/dev/null &
w1=$!; started="$started $w1"
sleep 2

grep -q '"class":"unavailable"' "$OUT/g1" \
    || bad "with no daemon, the watcher did not report unavailable"

"$BFMD" >/dev/null 2>&1 &
d1=$!; started="$started $d1"
sleep 4

if grep -qE '"class":"(idle|buffering|playing|paused)"' "$OUT/g1"; then
    note "ok: a watcher that started with no daemon recovered when one appeared"
else
    bad "the watcher never reported a state after the daemon appeared"
    sed 's/^/  got: /' "$OUT/g1" >&2
fi

kill "$w1" 2>/dev/null
kill "$d1" 2>/dev/null
sleep 1

# --------------------------------------- a dying daemon must be reported
"$BFMD" >/dev/null 2>&1 &
d2=$!; started="$started $d2"
sleep 3

: > "$OUT/g2"
"$BFM" status --watch --json > "$OUT/g2" 2>/dev/null &
w2=$!; started="$started $w2"
sleep 3

grep -qE '"class":"(idle|buffering|playing|paused)"' "$OUT/g2" \
    || bad "the watcher reported no state while the daemon was running"

# Braced with wait so the shell does not print its own "Killed" job notice.
{ kill -9 "$d2" 2>/dev/null; wait "$d2"; } 2>/dev/null
sleep 4

if [ "$(tail -1 "$OUT/g2" | grep -c '"class":"unavailable"')" = 1 ]; then
    note "ok: a watcher reported the daemon dying mid-stream"
else
    bad "after the daemon was killed, the watcher did not report unavailable"
    sed 's/^/  got: /' "$OUT/g2" >&2
fi

# A watcher that exits is also wrong: a bar module would simply vanish.
if kill -0 "$w2" 2>/dev/null; then
    note "ok: the watcher stayed alive through the outage"
else
    bad "the watcher exited when the daemon died; a bar module would disappear"
fi

[ "$fails" -eq 0 ] || exit 1
echo "ok: the watch stream tracks the daemon appearing and disappearing"
