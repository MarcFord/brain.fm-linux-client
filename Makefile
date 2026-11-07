# bfm — build and install
#
# The aim is that `make && make install` does the right thing on any Linux
# box: build the daemon and CLI always, and build whichever desktop
# front-ends this machine can actually compile. A missing toolkit is a
# reason to skip a GUI, never a reason to fail the build.

PREFIX      ?= /usr/local
DESTDIR     ?=
CARGO       ?= cargo
PROFILE     ?= release

BINDIR       = $(PREFIX)/bin
DATADIR      = $(PREFIX)/share
APPDIR       = $(DATADIR)/applications
DBUSDIR      = $(DATADIR)/dbus-1/services
DOCDIR       = $(DATADIR)/doc/bfm
ICONDIR      = $(DATADIR)/icons/hicolor/scalable/apps
SYMICONDIR   = $(DATADIR)/icons/hicolor/symbolic/apps
TARGETDIR    = target/$(PROFILE)

# Where a *user* unit may live depends on the kind of prefix, and getting it
# wrong fails silently: the unit installs, and systemd never looks there.
# Verified against `systemctl --user show -p UnitPath`:
#
#   /usr/lib/systemd/user         searched    <- convention for packages
#   /usr/local/lib/systemd/user   searched
#   ~/.local/lib/systemd/user     NOT searched
#   $XDG_DATA_HOME/systemd/user   searched    <- the per-user location
#
# So a home prefix has to use the data dir. System prefixes keep
# $PREFIX/lib/systemd/user, which is what distro packaging expects.
ifneq ($(filter $(HOME)/%,$(PREFIX)),)
SYSTEMDDIR   = $(DATADIR)/systemd/user
else
SYSTEMDDIR   = $(PREFIX)/lib/systemd/user
endif

ifeq ($(PROFILE),release)
CARGOFLAGS = --release
else
CARGOFLAGS =
endif

# ---------------------------------------------------------------- detection
#
# Checked with pkg-config rather than by looking for a distro package name,
# because the package names differ on every distro and the libraries do not.

HAVE_GTK := $(shell pkg-config --exists gtk4 libadwaita-1 2>/dev/null && echo yes)
HAVE_QT  := $(shell pkg-config --exists Qt6Core Qt6Quick 2>/dev/null && echo yes)
HAVE_KIRIGAMI := $(shell test -d "$$(pkg-config --variable=QT_INSTALL_QML Qt6Core 2>/dev/null)/org/kde/kirigami" \
                  -o -d /usr/lib/qt6/qml/org/kde/kirigami && echo yes)

# GUI = auto | gtk | kde | both | none
GUI ?= auto

ifeq ($(GUI),auto)
  GUI_TARGETS :=
  ifeq ($(HAVE_GTK),yes)
    GUI_TARGETS += bfm-gtk
  endif
  ifeq ($(HAVE_QT)$(HAVE_KIRIGAMI),yesyes)
    GUI_TARGETS += bfm-kde
  endif
else ifeq ($(GUI),both)
  GUI_TARGETS := bfm-gtk bfm-kde
else ifeq ($(GUI),none)
  GUI_TARGETS :=
else
  GUI_TARGETS := bfm-$(GUI)
endif

CORE_TARGETS := bfmd bfm
PKG_ARGS     := $(foreach t,$(CORE_TARGETS) $(GUI_TARGETS),-p $(t))

.PHONY: all build detect install uninstall check test smoke-layer smoke-install smoke package clean help

all: build

## detect: report what this machine can build
detect:
	@echo "Toolkits found on this system:"
	@echo "  GTK4 + libadwaita : $(if $(HAVE_GTK),yes,no)"
	@echo "  Qt6 Quick         : $(if $(HAVE_QT),yes,no)"
	@echo "  Kirigami          : $(if $(HAVE_KIRIGAMI),yes,no)"
	@echo
	@echo "GUI=$(GUI) -> building: $(if $(strip $(GUI_TARGETS)),$(GUI_TARGETS),none (headless: daemon + CLI only))"
	@echo "Always built       : $(CORE_TARGETS)"

## build: build the daemon, the CLI, and any GUI this machine supports
build: detect
	$(CARGO) build $(CARGOFLAGS) $(PKG_ARGS)

## check: formatting, lints and the full test suite
check: check-scripts
	$(CARGO) fmt --all -- --check
	RUSTFLAGS="-D warnings" $(CARGO) clippy --workspace --all-targets
	@command -v dbus-run-session >/dev/null \
	  && dbus-run-session -- $(CARGO) test --workspace \
	  || { echo "note: dbus-run-session absent; D-Bus contract tests will skip"; \
	       $(CARGO) test --workspace; }

test: check

## check-scripts: shellcheck + parse the shell scripts, and test the CI asserts
#
# install.sh is the entry point for most users, and the CI assertion helpers
# guard the packaging, so both are checked here and not only in CI.
check-scripts:
	@for f in install.sh dist/*.sh; do sh -n "$$f" || exit 1; done
	@echo "ok: all shell scripts parse"
	@# An `||` fallback here would swallow real shellcheck findings and print
	@# "absent" instead, so presence is tested separately from the result.
	@if command -v shellcheck >/dev/null 2>&1; then \
	   shellcheck -s sh install.sh dist/smoke-install.sh dist/smoke-layer.sh \
	     dist/ci-assert-test.sh \
	   && shellcheck dist/ci-assert.sh \
	   && echo "ok: shellcheck clean"; \
	 else \
	   echo "note: shellcheck absent; skipped"; \
	 fi
	@./dist/ci-assert-test.sh >/dev/null && echo "ok: CI assertion helpers fail when they should"

## install: daemon, CLI, service files, and any built GUI
install: build
	install -Dm755 $(TARGETDIR)/bfmd $(DESTDIR)$(BINDIR)/bfmd
	install -Dm755 $(TARGETDIR)/bfm  $(DESTDIR)$(BINDIR)/bfm
	@# Service files carry an absolute path, so the prefix is substituted
	@# rather than baked in: a hard-coded /usr/bin is wrong everywhere else.
	install -d $(DESTDIR)$(SYSTEMDDIR) $(DESTDIR)$(DBUSDIR)
	sed 's|@BINDIR@|$(BINDIR)|g' dist/bfmd.service.in \
	  > $(DESTDIR)$(SYSTEMDDIR)/bfmd.service
	sed 's|@BINDIR@|$(BINDIR)|g' dist/dev.marcford.bfm.service.in \
	  > $(DESTDIR)$(DBUSDIR)/dev.marcford.bfm.service
	@# The icon lives with the daemon package rather than either GUI, so
	@# both desktop entries can reference it and neither duplicates it.
	install -Dm644 dist/icons/dev.marcford.bfm.svg \
	  $(DESTDIR)$(ICONDIR)/dev.marcford.bfm.svg
	install -Dm644 dist/icons/dev.marcford.bfm-symbolic.svg \
	  $(DESTDIR)$(SYMICONDIR)/dev.marcford.bfm-symbolic.svg
	install -Dm644 dist/config.example.toml $(DESTDIR)$(DOCDIR)/config.example.toml
	install -Dm644 dist/waybar.jsonc        $(DESTDIR)$(DOCDIR)/waybar.jsonc
	@# A desktop entry only for a GUI that was actually built, or the
	@# application menu gains an entry that cannot start.
	@for gui in $(GUI_TARGETS); do \
	  if [ -x "$(TARGETDIR)/$$gui" ]; then \
	    install -Dm755 "$(TARGETDIR)/$$gui" "$(DESTDIR)$(BINDIR)/$$gui"; \
	    install -Dm644 "dist/dev.marcford.bfm.$${gui#bfm-}.desktop" \
	      "$(DESTDIR)$(APPDIR)/dev.marcford.bfm.$${gui#bfm-}.desktop"; \
	    echo "installed $$gui and its desktop entry"; \
	  fi; \
	done
	@echo
	@echo "Installed to $(DESTDIR)$(PREFIX)."
	@echo "The daemon is D-Bus activated: no need to start it by hand."
	@if [ -z "$(DESTDIR)" ]; then \
	  echo "Run 'systemctl --user daemon-reload' to pick up the unit."; \
	fi

## uninstall: remove everything install put down
uninstall:
	rm -f $(DESTDIR)$(BINDIR)/bfmd $(DESTDIR)$(BINDIR)/bfm \
	      $(DESTDIR)$(BINDIR)/bfm-gtk $(DESTDIR)$(BINDIR)/bfm-kde
	rm -f $(DESTDIR)$(SYSTEMDDIR)/bfmd.service
	rm -f $(DESTDIR)$(DBUSDIR)/dev.marcford.bfm.service
	rm -f $(DESTDIR)$(APPDIR)/dev.marcford.bfm.gtk.desktop
	rm -f $(DESTDIR)$(APPDIR)/dev.marcford.bfm.kde.desktop
	rm -f $(DESTDIR)$(ICONDIR)/dev.marcford.bfm.svg
	rm -f $(DESTDIR)$(SYMICONDIR)/dev.marcford.bfm-symbolic.svg
	rm -rf $(DESTDIR)$(DOCDIR)

## smoke-layer: verify --layer really becomes a Hyprland overlay (skips elsewhere)
smoke-layer:
	@BFM_GTK_BIN=$(TARGETDIR)/bfm-gtk ./dist/smoke-layer.sh; \
	  rc=$$?; [ $$rc -eq 77 ] && echo "(skipped)" && exit 0; exit $$rc

## smoke-install: prove `make install` yields a tree that D-Bus can activate
smoke-install:
	@./dist/smoke-install.sh; rc=$$?; \
	  [ $$rc -eq 77 ] && echo "(skipped)" && exit 0; exit $$rc

## smoke: every smoke check
smoke: smoke-install smoke-layer

## package: build .deb and .rpm packages (needs nfpm)
PKGVERSION ?= $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
PKGARCH    ?= amd64
STAGEDIR   ?= stage
PKGOUT     ?= dist-pkg

package:
	@# Debug binaries are ~10x larger; packaging them once produced a 72MB
	@# package for a daemon and a CLI. Refuse rather than rely on memory.
	@test "$(PROFILE)" = "release" || { \
	  echo "error: packaging requires PROFILE=release (got '$(PROFILE)')"; exit 1; }
	@command -v nfpm >/dev/null || { \
	  echo "error: nfpm not found. go install github.com/goreleaser/nfpm/v2/cmd/nfpm@latest"; exit 1; }
	$(MAKE) build
	rm -rf $(STAGEDIR) $(PKGOUT)
	$(MAKE) install PREFIX=/usr DESTDIR=$(PWD)/$(STAGEDIR)
	mkdir -p $(PKGOUT)
	@for cfg in packaging/nfpm/*.yaml; do \
	  name=$$(basename $$cfg .yaml); \
	  if [ "$$name" = "bfm" ] || [ -x "$(STAGEDIR)/usr/bin/$$name" ]; then \
	    for fmt in deb rpm; do \
	      ARCH=$(PKGARCH) VERSION=$(PKGVERSION) nfpm package -f $$cfg -p $$fmt -t $(PKGOUT)/; \
	    done; \
	  else \
	    echo "skipping $$name: not built on this machine"; \
	  fi; \
	done
	@echo
	@ls -1 $(PKGOUT)

clean:
	$(CARGO) clean
	rm -rf $(STAGEDIR) $(PKGOUT)

## help: list targets
help:
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/^## /  /'
