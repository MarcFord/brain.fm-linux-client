#!/bin/sh
# Verify that `bfm-gtk --layer` really becomes a layer-shell overlay.
#
# This check exists because unit tests cannot see a compositor, and two
# separate faults slipped through for exactly that reason: GTK was eating
# the --layer flag before the binary ever saw it, and an earlier GUI was
# declared working on the strength of the process staying alive.
#
# The two assertions that matter, and why:
#   * a layer surface exists in the `bfm` namespace at the overlay level —
#     looking in `hyprctl clients` would find nothing even when it works,
#     because layer surfaces are not in the client list;
#   * no ordinary window appears — if layer-shell silently fell back, the
#     app would still "work" while being the wrong kind of surface.
#
# Exits 0 on success, 1 on failure, and 77 (skip) where it cannot run.

set -eu
BIN="${BFM_GTK_BIN:-./target/release/bfm-gtk}"

[ -x "$BIN" ] || { echo "skip: $BIN not built"; exit 77; }
command -v hyprctl >/dev/null 2>&1 || { echo "skip: no hyprctl; needs a wlroots compositor"; exit 77; }
[ -n "${WAYLAND_DISPLAY:-}" ] || { echo "skip: no Wayland session"; exit 77; }

"$BIN" --layer >/tmp/bfm-smoke-layer.log 2>&1 &
PID=$!
# shellcheck disable=SC2064
trap "kill $PID 2>/dev/null || true" EXIT
sleep 5

if ! kill -0 "$PID" 2>/dev/null; then
    echo "FAIL: exited immediately"
    sed 's/^/  /' /tmp/bfm-smoke-layer.log
    exit 1
fi

layers=$(hyprctl layers -j 2>/dev/null || echo '{}')
if ! printf '%s' "$layers" | grep -q '"bfm"'; then
    echo "FAIL: no layer surface in the 'bfm' namespace"
    sed 's/^/  /' /tmp/bfm-smoke-layer.log
    exit 1
fi

# Overlay is level 3. A surface on a lower level would sit under panels.
level=$(printf '%s' "$layers" | python3 -c '
import json,sys
d=json.load(sys.stdin)
for mon,v in d.items():
    for lvl,items in v.get("levels",{}).items():
        for it in items:
            if it.get("namespace")=="bfm":
                print(lvl); raise SystemExit
print("none")
')
[ "$level" = "3" ] || { echo "FAIL: layer is at level $level, expected 3 (overlay)"; exit 1; }

if hyprctl clients -j 2>/dev/null | grep -q 'dev.marcford.bfm.gtk'; then
    echo "FAIL: also opened an ordinary window; layer-shell did not take effect"
    exit 1
fi

echo "ok: overlay surface present in namespace 'bfm' at level $level, and no ordinary window"
