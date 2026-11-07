#!/bin/sh
# Verify `make install` produces a working installation.
#
# The assertion that matters is D-Bus activation: the daemon must start on
# demand from its installed service file, with nothing launched by hand.
# Checking only that files landed in the right places would pass even if
# activation were broken, and activation is how every front-end starts its
# backend.
#
# Two things this had to get right, both of which produced false failures
# first time:
#   * PREFIX must be a real, self-consistent location. A DESTDIR-staged
#     tree records Exec=/usr/bin/bfmd while the binary lives elsewhere, so
#     it can never activate.
#   * XDG_DATA_DIRS must be set before dbus-run-session, or the bus starts
#     without it and never sees the service file.
#
# Exits 0 on success, 1 on failure, 77 (skip) where it cannot run.

set -eu
command -v dbus-run-session >/dev/null 2>&1 || { echo "skip: no dbus-run-session"; exit 77; }

STAGE=$(mktemp -d)
HOMESTAGE=""

# The activated daemon outlives the private bus that started it, so each run
# used to leave a bfmd behind pointing at a staging dir that no longer exists.
# Reap by matching the executable path against our own staging dirs; never by
# process name alone, which would kill the user's real daemon. The empty-var
# guard matters: an unset stage would make the pattern `/*` and match
# everything.
cleanup() {
    for pid in $(pgrep -x bfmd 2>/dev/null); do
        exe=$(readlink -f "/proc/$pid/exe" 2>/dev/null) || continue
        [ -n "$STAGE" ] && case "$exe" in "$STAGE"/*) kill "$pid" 2>/dev/null; continue ;; esac
        [ -n "$HOMESTAGE" ] && case "$exe" in "$HOMESTAGE"/*) kill "$pid" 2>/dev/null ;; esac
    done
    rm -rf "$STAGE" ${HOMESTAGE:+"$HOMESTAGE"}
}
trap cleanup EXIT

make install PREFIX="$STAGE" GUI=none >/dev/null

for f in bin/bfmd bin/bfm \
         share/dbus-1/services/dev.marcford.bfm.service \
         lib/systemd/user/bfmd.service \
         share/icons/hicolor/scalable/apps/dev.marcford.bfm.svg; do
    [ -e "$STAGE/$f" ] || { echo "FAIL: missing $f"; exit 1; }
done

# A per-user prefix must put the unit somewhere systemd actually searches.
# $PREFIX/lib/systemd/user is correct for /usr and /usr/local but is NOT on
# the user unit path under $HOME, and installing there fails silently: the
# file appears, `systemctl --user` never sees it. This staged under /tmp
# before, so it never caught that.
# ~/.cache is not guaranteed to exist -- a container running as root often
# has no XDG dirs at all, and mktemp would fail under `set -e` before any
# assertion ran.
mkdir -p "$HOME/.cache"
HOMESTAGE=$(mktemp -d "$HOME/.cache/bfm-smoke.XXXXXX")
make install PREFIX="$HOMESTAGE" GUI=none >/dev/null
if [ ! -e "$HOMESTAGE/share/systemd/user/bfmd.service" ]; then
    echo "FAIL: home-prefix install put the unit outside the user unit path"
    find "$HOMESTAGE" -name 'bfmd.service' | sed 's/^/  found at: /'
    exit 1
fi

# A hard-coded prefix here is the bug the templates exist to prevent.
if grep -q '@BINDIR@' "$STAGE/share/dbus-1/services/dev.marcford.bfm.service"; then
    echo "FAIL: service file still contains an unsubstituted @BINDIR@"
    exit 1
fi

# GUI=none must not leave a menu entry that cannot launch.
[ -d "$STAGE/share/applications" ] && { echo "FAIL: headless install left a desktop entry"; exit 1; }

# Capture through a file, not command substitution. The activated daemon
# inherits the pipe and never exits by design, so `out=$(...)` waits for a
# writer that will never close and hangs forever.
# XDG_DATA_HOME must be redirected too, not just XDG_DATA_DIRS. D-Bus searches
# the home data dir independently, so once bfm is actually installed to
# ~/.local the private bus sees that service file and activates the *installed*
# daemon instead of the staged one -- the check then passes no matter how
# broken the staged tree is.
RESULT="$STAGE/result.txt"
XDG_DATA_HOME="$STAGE/share" XDG_DATA_DIRS="$STAGE/share" BFM_TOKEN_STORE=memory \
  dbus-run-session -- sh -c "busctl --user get-property \
    dev.marcford.bfm /dev/marcford/bfm dev.marcford.BrainFM1 \
    InterfaceVersion >\"$RESULT\" 2>&1" >/dev/null 2>&1 || true
out=$(cat "$RESULT" 2>/dev/null || echo "no output")

case "$out" in
    *"u "*) echo "ok: installed tree activates on demand (InterfaceVersion ${out##* })" ;;
    *)      echo "FAIL: D-Bus activation did not start the daemon"
            echo "  $out"
            exit 1 ;;
esac
