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
trap 'rm -rf "$STAGE"' EXIT

make install PREFIX="$STAGE" GUI=none >/dev/null

for f in bin/bfmd bin/bfm \
         share/dbus-1/services/dev.marcford.bfm.service \
         lib/systemd/user/bfmd.service \
         share/icons/hicolor/scalable/apps/dev.marcford.bfm.svg; do
    [ -e "$STAGE/$f" ] || { echo "FAIL: missing $f"; exit 1; }
done

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
RESULT="$STAGE/result.txt"
XDG_DATA_DIRS="$STAGE/share" BFM_TOKEN_STORE=memory \
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
