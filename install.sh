#!/bin/sh
# install.sh — build and install bfm for whatever machine this is.
#
# `make install` already does the right thing once the build dependencies are
# present and you know which front-ends to build. This script is the layer
# above that: work out what this system is, get the dependencies in place,
# decide which GUI actually belongs here, then hand over to make.
#
# Design rules, learned the hard way:
#   * Distro detection reads ID *and* ID_LIKE. Arch derivatives report their
#     own ID (Omarchy is ID=omarchy, ID_LIKE=arch), so matching ID alone
#     fails on exactly the systems most likely to run this.
#   * Package names are the least portable thing here, so a failed dependency
#     install is reported and survivable, never fatal and never silent. The
#     check that matters is pkg-config afterwards, not the package manager's
#     exit code.
#   * A per-user install is the default. bfmd is a session-bus daemon with a
#     systemd *user* unit; it belongs in $HOME, and nothing here needs root.
#   * The post-install wiring is part of the job. Without a D-Bus ReloadConfig
#     the freshly installed service file is invisible to the running session
#     and the first command fails with "not activatable".

set -eu

SELF_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)

# ----------------------------------------------------------------- options
MODE=user
PREFIX=
GUI=
SKIP_DEPS=0
ASSUME_YES=0
DRY_RUN=0
DO_UNINSTALL=0
PRINT_DEPS=0

usage() {
    cat <<EOF
Build and install bfm, an unofficial Brain.fm client for Linux.

Usage: ./install.sh [options]

  --system          install to /usr/local for all users (needs sudo)
  --prefix DIR      install somewhere specific
                    (default: ~/.local, or /usr/local with --system)
  --gui WHICH       auto | gtk | kde | both | none
                    (default: chosen from this desktop session)
  --skip-deps       do not touch the package manager
  --yes, -y         do not ask before installing packages
  --dry-run         print what would happen, change nothing
  --uninstall       remove a previous install from the same prefix
  --print-deps      print the packages this distro needs, then exit
                    (one group per line: core/gtk/kde/rust)
  --help, -h        this text

Examples:
  ./install.sh                     # per-user install, GUI matched to desktop
  ./install.sh --system --gui both # system-wide, both front-ends
  ./install.sh --gui none          # headless: daemon and CLI only
EOF
}

while [ $# -gt 0 ]; do
    case $1 in
        --system)    MODE=system ;;
        --prefix)    [ $# -ge 2 ] || { echo "--prefix needs a directory" >&2; exit 2; }
                     PREFIX=$2; shift ;;
        --prefix=*)  PREFIX=${1#--prefix=} ;;
        --gui)       [ $# -ge 2 ] || { echo "--gui needs a value" >&2; exit 2; }
                     GUI=$2; shift ;;
        --gui=*)     GUI=${1#--gui=} ;;
        --skip-deps) SKIP_DEPS=1 ;;
        -y|--yes)    ASSUME_YES=1 ;;
        --dry-run)   DRY_RUN=1 ;;
        --uninstall) DO_UNINSTALL=1 ;;
        --print-deps) PRINT_DEPS=1 ;;
        -h|--help)   usage; exit 0 ;;
        *)           echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

case ${GUI:-auto} in
    auto|gtk|kde|both|none) : ;;
    *) echo "--gui must be auto, gtk, kde, both or none (got '$GUI')" >&2; exit 2 ;;
esac

if [ -z "$PREFIX" ]; then
    if [ "$MODE" = system ]; then PREFIX=/usr/local; else PREFIX=$HOME/.local; fi
fi

say()  { printf '%s\n' "$*"; }
step() { printf '\n==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

run() {
    if [ "$DRY_RUN" = 1 ]; then printf '   would run: %s\n' "$*"; return 0; fi
    "$@"
}

# Root is needed only for a system prefix, and only for the install step.
SUDO=
need_root_for_install() {
    case $PREFIX in
        "$HOME"/*|"$HOME") return 1 ;;
        *) [ "$(id -u)" != 0 ] ;;
    esac
}
if need_root_for_install; then
    have sudo || die "installing to $PREFIX needs root, but sudo is not available.
Re-run as root, or choose a per-user prefix: ./install.sh --prefix \"\$HOME/.local\""
    SUDO=sudo
fi

ask() {
    # $1 = question. Yes unless the user says otherwise; non-interactive
    # shells must not block, so no tty means no.
    [ "$ASSUME_YES" = 1 ] && return 0
    [ -t 0 ] || { warn "not interactive, assuming no: $1"; return 1; }
    printf '%s [Y/n] ' "$1"
    read -r reply || return 1
    case $reply in n|N|no|NO|No) return 1 ;; *) return 0 ;; esac
}

# ------------------------------------------------------------ distro family
#
# ID first, then every word of ID_LIKE, so derivatives resolve to the family
# whose package manager they actually use.
FAMILY=unknown
PRETTY=$(. /etc/os-release 2>/dev/null && printf '%s' "${PRETTY_NAME:-${NAME:-unknown}}")

classify() {
    case $1 in
        arch|archarm|manjaro|endeavouros|garuda|omarchy|artix|cachyos) echo arch ;;
        debian|ubuntu|linuxmint|pop|elementary|zorin|raspbian|devuan)  echo debian ;;
        fedora|rhel|centos|rocky|almalinux|ol|nobara)                 echo fedora ;;
        opensuse*|suse|sles)                                          echo suse ;;
        alpine)                                                       echo alpine ;;
        void)                                                         echo void ;;
        gentoo)                                                       echo gentoo ;;
        *)                                                            echo "" ;;
    esac
}

if [ -r /etc/os-release ]; then
    _id=$(. /etc/os-release && printf '%s' "${ID:-}")
    _like=$(. /etc/os-release && printf '%s' "${ID_LIKE:-}")
    FAMILY=$(classify "$_id")
    if [ -z "$FAMILY" ]; then
        for w in $_like; do
            FAMILY=$(classify "$w")
            [ -n "$FAMILY" ] && break
        done
    fi
    [ -z "$FAMILY" ] && FAMILY=unknown
fi

# Fall back to whichever package manager is actually present. A system can
# have an unrecognised ID and still be obviously Debian underneath.
if [ "$FAMILY" = unknown ]; then
    if   have pacman;  then FAMILY=arch
    elif have apt-get; then FAMILY=debian
    elif have dnf || have yum; then FAMILY=fedora
    elif have zypper; then FAMILY=suse
    elif have apk;    then FAMILY=alpine
    elif have xbps-install; then FAMILY=void
    fi
fi

# ------------------------------------------------------- package name tables
#
# Three groups: core (always), gtk, kde. Names drift between releases; when
# one is wrong the pkg-config check below is what catches it.
pkgs_for() {
    group=$1
    case "$FAMILY:$group" in
        arch:core)   echo "base-devel pkgconf dbus alsa-lib" ;;
        arch:gtk)    echo "gtk4 libadwaita gtk4-layer-shell" ;;
        arch:kde)    echo "qt6-base qt6-declarative kirigami" ;;
        arch:rust)   echo "rust" ;;

        debian:core) echo "build-essential pkg-config libdbus-1-dev libasound2-dev" ;;
        debian:gtk)  echo "libgtk-4-dev libadwaita-1-dev libgtk4-layer-shell-dev" ;;
        debian:kde)  echo "qt6-base-dev qt6-declarative-dev libkf6kirigami-dev" ;;
        debian:rust) echo "rustc cargo" ;;

        fedora:core) echo "gcc gcc-c++ pkgconf-pkg-config dbus-devel alsa-lib-devel" ;;
        fedora:gtk)  echo "gtk4-devel libadwaita-devel gtk4-layer-shell-devel" ;;
        fedora:kde)  echo "qt6-qtbase-devel qt6-qtdeclarative-devel kf6-kirigami-devel" ;;
        fedora:rust) echo "rust cargo" ;;

        suse:core)   echo "gcc gcc-c++ pkg-config dbus-1-devel alsa-devel" ;;
        suse:gtk)    echo "gtk4-devel libadwaita-devel gtk4-layer-shell-devel" ;;
        suse:kde)    echo "qt6-base-devel qt6-declarative-devel kf6-kirigami-devel" ;;
        suse:rust)   echo "rust cargo" ;;

        alpine:core) echo "build-base pkgconf dbus-dev alsa-lib-dev" ;;
        alpine:gtk)  echo "gtk4.0-dev libadwaita-dev gtk4-layer-shell-dev" ;;
        alpine:kde)  echo "qt6-qtbase-dev qt6-qtdeclarative-dev" ;;
        alpine:rust) echo "rust cargo" ;;

        void:core)   echo "base-devel pkg-config dbus-devel alsa-lib-devel" ;;
        void:gtk)    echo "gtk4-devel libadwaita-devel gtk4-layer-shell-devel" ;;
        void:kde)    echo "qt6-base-devel qt6-declarative-devel kirigami2-devel" ;;
        void:rust)   echo "rust cargo" ;;

        *) echo "" ;;
    esac
}

install_pkgs() {
    [ $# -gt 0 ] || return 0
    case $FAMILY in
        arch)   run $SUDO pacman -S --needed --noconfirm "$@" ;;
        debian) run $SUDO apt-get update -qq && run $SUDO apt-get install -y "$@" ;;
        fedora) if have dnf; then run $SUDO dnf install -y "$@"; else run $SUDO yum install -y "$@"; fi ;;
        suse)   run $SUDO zypper --non-interactive install "$@" ;;
        alpine) run $SUDO apk add "$@" ;;
        void)   run $SUDO xbps-install -Sy "$@" ;;
        *)      return 1 ;;
    esac
}

# Dependency installation needs root even for a per-user install.
PKG_SUDO=
if [ "$(id -u)" != 0 ] && [ "$SKIP_DEPS" = 0 ]; then
    have sudo && PKG_SUDO=sudo
fi

# -------------------------------------------------------- toolkit detection
# Presence is not enough. bfm-gtk opts into the gtk4 `v4_22` and libadwaita
# `v1_9` feature sets, because it uses Picture::set_content_fit and
# ToolbarView, so those are hard build-time minimums. Ubuntu 24.04 ships GTK
# 4.14 and Debian 13 ships 4.18: pkg-config --exists succeeds on both and the
# build then dies ten minutes in with a gdk4-sys error naming a version
# nobody can install. Check the version here instead.
GTK_MIN=4.22
ADW_MIN=1.9
has_gtk() {
    pkg-config --exists gtk4 libadwaita-1 2>/dev/null || return 1
    pkg-config --atleast-version="$GTK_MIN" gtk4 2>/dev/null || return 1
    pkg-config --atleast-version="$ADW_MIN" libadwaita-1 2>/dev/null
}

# Distinguish "no GTK" from "GTK too old", so the message can say which.
gtk_too_old() {
    pkg-config --exists gtk4 2>/dev/null || return 1
    ! pkg-config --atleast-version="$GTK_MIN" gtk4 2>/dev/null
}
has_qt()  { pkg-config --exists Qt6Core Qt6Quick 2>/dev/null; }
has_kirigami() {
    qmldir=$(pkg-config --variable=QT_INSTALL_QML Qt6Core 2>/dev/null || true)
    [ -n "$qmldir" ] && [ -d "$qmldir/org/kde/kirigami" ] && return 0
    [ -d /usr/lib/qt6/qml/org/kde/kirigami ] && return 0
    [ -d /usr/lib64/qt6/qml/org/kde/kirigami ]
}

# Which front-end belongs on this machine, before considering what is
# installable. A graphical session is required for either.
desktop_choice() {
    if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ] && [ -z "${XDG_CURRENT_DESKTOP:-}" ]; then
        echo none; return
    fi
    # Case-insensitively, without bashisms.
    d=$(printf '%s' "${XDG_CURRENT_DESKTOP:-}" | tr '[:upper:]' '[:lower:]')
    case $d in
        *kde*|*plasma*) echo kde ;;
        *)              echo gtk ;;
    esac
}

# --------------------------------------------------------------- print-deps
#
# Exists so CI can verify these package names against real distro images
# without keeping its own copy of the tables, which would drift.
if [ "$PRINT_DEPS" = 1 ]; then
    say "family: $FAMILY"
    say "core: $(pkgs_for core)"
    say "gtk: $(pkgs_for gtk)"
    say "kde: $(pkgs_for kde)"
    say "rust: $(pkgs_for rust)"
    exit 0
fi

# ----------------------------------------------------------------- uninstall
if [ "$DO_UNINSTALL" = 1 ]; then
    step "Removing bfm from $PREFIX"
    run $SUDO make -C "$SELF_DIR" uninstall PREFIX="$PREFIX"
    # `A && B || C` is not if-then-else: C runs when B fails too. These are
    # all best-effort refreshes, so each one is an explicit if instead.
    if [ -z "$SUDO" ] && have systemctl; then
        run systemctl --user daemon-reload || true
    fi
    if have update-desktop-database; then
        run update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
    fi
    if have gtk-update-icon-cache; then
        run gtk-update-icon-cache -q -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null || true
    fi
    say ""
    say "Removed. A session token may remain in your keyring; clear it with:"
    say "  secret-tool clear service bfm   # or your keyring's own UI"
    exit 0
fi

# -------------------------------------------------------------------- report
step "Looking at this system"
say "  distro      : $PRETTY  (family: $FAMILY)"
say "  desktop     : ${XDG_CURRENT_DESKTOP:-none}  (session: ${XDG_SESSION_TYPE:-unknown})"
say "  prefix      : $PREFIX"
say "  mode        : $MODE"

if [ "$FAMILY" = unknown ] && [ "$SKIP_DEPS" = 0 ]; then
    warn "unrecognised distribution, so dependencies cannot be installed automatically."
    warn "Install them yourself and re-run with --skip-deps."
    SKIP_DEPS=1
fi
if [ "$FAMILY" = gentoo ] && [ "$SKIP_DEPS" = 0 ]; then
    warn "Gentoo: emerge is not driven automatically. Install gtk4/libadwaita and/or"
    warn "qt6 + kirigami yourself, then re-run with --skip-deps."
    SKIP_DEPS=1
fi

# ------------------------------------------------------------ decide the GUI
WANT=${GUI:-auto}
AUTO_CHOSE=0
if [ "$WANT" = auto ]; then
    AUTO_CHOSE=1
    WANT=$(desktop_choice)
    if [ "$WANT" = none ]; then
        say "  front-end   : undecided (no graphical session visible from here)"
    else
        say "  front-end   : $WANT (matched to this desktop)"
    fi
else
    say "  front-end   : $WANT (requested)"
fi

# ------------------------------------------------------------- dependencies
if [ "$SKIP_DEPS" = 0 ]; then
    need=$(pkgs_for core)
    case $WANT in
        gtk)  need="$need $(pkgs_for gtk)" ;;
        kde)  need="$need $(pkgs_for kde)" ;;
        both) need="$need $(pkgs_for gtk) $(pkgs_for kde)" ;;
    esac
    have cargo || need="$need $(pkgs_for rust)"

    step "Installing build dependencies"
    say "  $need"
    if ask "Install these with the system package manager?"; then
        SUDO_SAVE=$SUDO; SUDO=$PKG_SUDO
        # A wrong package name must not abort the run: the pkg-config check
        # below is the real gate, and a partial install is often enough.
        # shellcheck disable=SC2086  # deliberate split: $need is a list
        if ! install_pkgs $need; then
            warn "the package manager reported a failure."
            warn "Continuing; the checks below will say whether it actually matters."
        fi
        SUDO=$SUDO_SAVE
    else
        say "  skipped."
    fi
else
    step "Skipping dependency installation"
fi

# -------------------------------------------------------------------- rust
step "Checking the Rust toolchain"
MIN_RUST=1.98
if ! have cargo; then
    die "cargo not found. Install Rust $MIN_RUST or newer:
  https://rustup.rs    (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh)
then re-run this script."
fi
RUSTV=$(cargo --version 2>/dev/null | awk '{print $2}')
# sort -V puts the smaller first; if the minimum is not first, cargo is older.
if [ "$(printf '%s\n%s\n' "$MIN_RUST" "$RUSTV" | sort -V | head -1)" != "$MIN_RUST" ]; then
    die "cargo $RUSTV is too old; this workspace needs $MIN_RUST or newer (edition 2024).
Update with 'rustup update', or install a newer toolchain."
fi
say "  cargo $RUSTV at $(command -v cargo)"

# ALSA is not optional: rodio -> cpal -> alsa-sys links against it, and
# without the headers the build dies well into a long compile with an
# alsa-sys error that says nothing about what to install.
step "Checking audio headers"
if pkg-config --exists alsa 2>/dev/null; then
    say "  ALSA development headers: yes"
else
    die "ALSA development headers are missing, and the audio backend cannot
build without them. Install them and re-run:
  Arch      pacman -S alsa-lib
  Debian    apt-get install libasound2-dev
  Fedora    dnf install alsa-lib-devel
  openSUSE  zypper install alsa-devel
  Alpine    apk add alsa-lib-dev
  Void      xbps-install alsa-lib-devel"
fi

# -------------------------------------------- confirm the GUI is buildable
step "Checking toolkits"
gtk_ok=no; kde_ok=no
has_gtk && gtk_ok=yes
{ has_qt && has_kirigami; } && kde_ok=yes
if [ "$gtk_ok" = yes ]; then
    say "  GTK4 + libadwaita : yes ($(pkg-config --modversion gtk4), libadwaita $(pkg-config --modversion libadwaita-1))"
elif gtk_too_old; then
    say "  GTK4 + libadwaita : too old (gtk4 $(pkg-config --modversion gtk4); needs >= $GTK_MIN)"
    warn "The GTK app needs GTK >= $GTK_MIN and libadwaita >= $ADW_MIN. Your distro"
    warn "ships an older GTK, so it cannot be built here. The daemon and CLI are"
    warn "unaffected, and the Qt front-end may still work. Debian, Ubuntu and other"
    warn "fixed-release distros will catch up; rolling ones (Arch, Tumbleweed) and"
    warn "recent Fedora already have it."
else
    say "  GTK4 + libadwaita : no"
fi
say "  Qt6 + Kirigami    : $kde_ok"

BUILD_GUI=$WANT
case $WANT in
    gtk)  [ "$gtk_ok" = yes ] || {
              if [ "$kde_ok" = yes ]; then
                  warn "GTK4 is unavailable; falling back to the KDE front-end."
                  BUILD_GUI=kde
              else
                  warn "no GUI toolkit is available; installing the daemon and CLI only."
                  BUILD_GUI=none
              fi
          } ;;
    kde)  [ "$kde_ok" = yes ] || {
              if [ "$gtk_ok" = yes ]; then
                  warn "Qt6/Kirigami is unavailable; falling back to the GTK front-end."
                  BUILD_GUI=gtk
              else
                  warn "no GUI toolkit is available; installing the daemon and CLI only."
                  BUILD_GUI=none
              fi
          } ;;
    both) if [ "$gtk_ok$kde_ok" != yesyes ]; then
              if   [ "$gtk_ok" = yes ]; then warn "Qt6/Kirigami missing; building GTK only."; BUILD_GUI=gtk
              elif [ "$kde_ok" = yes ]; then warn "GTK4 missing; building KDE only.";        BUILD_GUI=kde
              else warn "no GUI toolkit is available; daemon and CLI only.";                 BUILD_GUI=none
              fi
          fi ;;
esac
# No session was visible, so the GUI was left undecided above. Installed
# toolkits are the better evidence here: XDG_CURRENT_DESKTOP is unset over SSH
# and on a plain TTY, which says nothing about whether this is a desktop.
if [ "$BUILD_GUI" = none ] && [ "$AUTO_CHOSE" = 1 ]; then
    if   [ "$gtk_ok" = yes ]; then
        BUILD_GUI=gtk
        say "  no session visible, but GTK4 is installed -> building the GTK front-end"
    elif [ "$kde_ok" = yes ]; then
        BUILD_GUI=kde
        say "  no session visible, but Qt6/Kirigami is installed -> building the KDE front-end"
    else
        say "  no session and no toolkits -> daemon and CLI only (override with --gui)"
    fi
fi

say "  building    : $BUILD_GUI"

# ------------------------------------------------------------ build, install
step "Building (this takes a few minutes the first time)"
run make -C "$SELF_DIR" build GUI="$BUILD_GUI"

step "Installing to $PREFIX"
run $SUDO make -C "$SELF_DIR" install PREFIX="$PREFIX" GUI="$BUILD_GUI"

# -------------------------------------------------------------- wire it up
#
# Without this the install is present but inert in the *current* session.
step "Wiring it into the session"

# Each of these reports only what it actually did: a dry run that prints
# "reloaded" when it reloaded nothing is worse than printing nothing.
if have systemctl && [ -z "$SUDO" ]; then
    if run systemctl --user daemon-reload; then
        [ "$DRY_RUN" = 1 ] || say "  systemd user units reloaded"
    else
        warn "systemctl --user daemon-reload failed"
    fi
fi

# The one that bites: D-Bus reads service files when the bus starts, so a
# newly installed service is invisible until the bus re-reads them. Without
# this the first command fails with "not activatable" on an install that is
# in fact perfect.
if have busctl; then
    # The redirect has to sit inside the branch, not around `run`, or a dry
    # run swallows its own "would run" line and then claims success.
    if [ "$DRY_RUN" = 1 ]; then
        run busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
            org.freedesktop.DBus ReloadConfig
    elif busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
            org.freedesktop.DBus ReloadConfig >/dev/null 2>&1; then
        say "  session D-Bus re-read its service files"
    else
        warn "could not ask D-Bus to reload; log out and back in if activation fails"
    fi
fi

if have update-desktop-database; then
    if run update-desktop-database "$PREFIX/share/applications" 2>/dev/null; then
        [ "$DRY_RUN" = 1 ] || say "  desktop database updated"
    fi
fi
if have gtk-update-icon-cache; then
    if run gtk-update-icon-cache -q -f -t "$PREFIX/share/icons/hicolor" 2>/dev/null; then
        [ "$DRY_RUN" = 1 ] || say "  icon cache updated"
    fi
fi

# ------------------------------------------------------------------- verify
step "Verifying"

if [ "$DRY_RUN" = 1 ]; then
    say "  (dry run: nothing was installed, so nothing to verify)"
    exit 0
fi

case :$PATH: in
    *:"$PREFIX/bin":*) : ;;
    *) warn "$PREFIX/bin is not on your PATH. Add this to your shell profile:"
       warn "  export PATH=\"$PREFIX/bin:\$PATH\"" ;;
esac

# D-Bus only finds service files under $XDG_DATA_HOME or $XDG_DATA_DIRS. An
# unusual prefix installs correctly and is still never activatable, and the
# activation check below would then be answered by some *other* installed
# daemon and report a success that means nothing. So establish this first.
SVCDIR=$PREFIX/share/dbus-1/services
ON_BUS_PATH=no
[ "$PREFIX/share" = "${XDG_DATA_HOME:-$HOME/.local/share}" ] && ON_BUS_PATH=yes
IFS=:
for d in ${XDG_DATA_DIRS:-/usr/local/share:/usr/share}; do
    [ "$PREFIX/share" = "$d" ] && ON_BUS_PATH=yes
done
unset IFS
if [ "$ON_BUS_PATH" = no ]; then
    warn "$SVCDIR is not on the D-Bus search path, so the daemon cannot be"
    warn "activated on demand from this prefix. Either install to a standard"
    warn "location (~/.local or --system), or add it for your session:"
    warn "  export XDG_DATA_DIRS=\"$PREFIX/share:\$XDG_DATA_DIRS\""
fi

# Activation is the assertion worth making: it is how every front-end starts
# its backend, and checking that files landed would pass even with it broken.
if [ "$ON_BUS_PATH" = yes ] && have busctl; then
    ver=$(busctl --user get-property dev.marcford.bfm /dev/marcford/bfm \
             dev.marcford.BrainFM1 InterfaceVersion 2>&1 || true)
    case $ver in
        *"u "*) say "  daemon activates on demand (InterfaceVersion ${ver##* })" ;;
        *)      warn "the daemon did not activate: $ver"
                warn "Try: $PREFIX/bin/bfmd --version   (and journalctl --user -u bfmd -n 20)" ;;
    esac
fi

say ""
say "Installed to $PREFIX."
say ""
say "Next:"
say "  bfm login --browser     sign in (Google/Apple/Facebook/email, in your browser)"
say "  bfm play focus          start a session"
say "  bfm status              what is playing"
case $BUILD_GUI in
    none) ;;
    *)    say ""
          # The same prefix test as above: a desktop entry outside the XDG
          # data path is installed but will not appear in any menu.
          if [ "$ON_BUS_PATH" = yes ]; then
              say "The desktop app is in your application menu as \"Brain.fm\"."
          else
              say "The desktop app was installed, but a menu entry under this prefix"
              say "will not be picked up until $PREFIX/share is on XDG_DATA_DIRS."
              say "Run it directly from $PREFIX/bin (bfm-gtk and/or bfm-kde)."
          fi ;;
esac
say ""
say "Uninstall with: ./install.sh --uninstall --prefix \"$PREFIX\""
