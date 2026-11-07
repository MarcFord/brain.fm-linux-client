# Changelog

All notable changes to this project are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Two things are versioned independently and both matter:

- **The release version**, below. Ordinary semver.
- **`BrainFM1`, the D-Bus interface version**, which only ever goes up and
  only ever gains members. Third-party clients and status bars depend on it,
  so a breaking change there means a new interface served alongside the old
  one, never an edit to this one. Each entry notes the interface version when
  it moved.

`[Unreleased]` collects work that has landed on `main` but has not been
tagged. A release cannot be cut without a section for its version: the
release workflow reads this file for its notes and refuses a tag that has no
entry here.

## [Unreleased]

### Fixed

- The Flatpak build never ran. It referenced
  `bilelmoussaoui/flatpak-github-actions:kde-6.8`, a Docker Hub tag that does
  not exist — that repository stops at `kde-6.7` — so the job failed on the
  image pull and the manifest itself was never exercised at all. It now uses
  `ghcr.io/flathub-infra/flatpak-github-actions`, where the project actually
  publishes.
- Flatpak builds ran with no network while `cargo` needed to fetch crates,
  which could not have worked. The manifest now asks for network during the
  build and says plainly why that makes it unsuitable for Flathub as it
  stands.
- The Flatpak targeted the KDE 6.8 runtime while the Qt front-end was written
  against 6.11, so the QML could have been asked to run on an older Kirigami
  than it expects. Now tracks 6.11.

### Added

- AppStream metadata for the Flatpak
  (`packaging/flatpak/dev.marcford.bfm.metainfo.xml`), so the app presents
  itself properly to software centres. Validated by `appstreamcli` in CI,
  which also checks that it carries a `<release>` for the tag being built.
- The Flatpak bundle is now also built on pushes to the protected branches,
  not only on tags, so the manifest can be iterated on without cutting a
  release. It is informational in both places and gates nothing.
- The full suite runs on pull requests into `main` and `testing`, which is
  what lets it be a *required* check there. Both branches are now protected
  and require all seventeen of its jobs.

## [0.1.0] — 2026-11-07

First release. Serves `BrainFM1` interface version 5.

### The daemon and the control plane

- `bfmd`, a D-Bus service on the session bus, activated on demand: no client
  ever starts it by hand, and the first call launches it.
- `BrainFM1` at `dev.marcford.bfm` / `/dev/marcford/bfm` — a public,
  versioned, additive-only contract. It is the product; the CLI and both
  desktop apps are ordinary clients of it with no privileges a third-party
  plugin lacks, and a test enforces that no front-end can reach the API or
  the audio engine directly.
- `bfm-proto` ships the contract and a ready `BrainFM1Proxy`, so writing
  another client needs no code from this repository beyond that crate.
- MPRIS 2 as `org.mpris.MediaPlayer2.bfm`, so `playerctl`, media keys, lock
  screens and any bar's MPRIS module work with no configuration.
  Capabilities are honest: `CanGoPrevious` and `CanSeek` are false, because
  Brain.fm sessions only move forward and the decoder cannot seek.
- Hooks on state changes, for scripting around sessions.
- A systemd user unit, installed where systemd actually searches for one.

### Playback

- Pure-Rust audio: `stream-download` into `rodio`, no `mpv` subprocess and no
  GStreamer, because Brain.fm serves progressive MP3 rather than HLS.
- Disk-backed buffering. Tracks run around 30 minutes, so an in-memory
  buffer would cost 30–60 MB per track.
- Gapless queueing from the session's track list, with prefetch.
- Session shapes: **infinite** (the default), a **countdown** (`50m`), or
  **intervals** (`25m/5m x4`). A finished timer ends the session.
- Mental state, activity, genre and neural-effect selection.
- Favourite, dislike and share.

### Sign-in

- Email and password, fully self-service, prompted and never stored.
- Browser handoff for Google, Apple and Facebook accounts: the session is
  read straight out of a local Chromium-family profile where possible, and
  pasted without echo where it is not. Those providers cannot be done any
  other way — the web app authenticates against Brain.fm's own OAuth client,
  and Google blocks OAuth inside embedded webviews.
- Magic links, and a token on stdin for headless and scripted setup.
- Only the session token is persisted, in the system keyring via Secret
  Service, falling back to a `0600` file in `0700` directories where no
  keyring daemon exists. The password is never persisted under any method.
- Sessions renew lazily and indefinitely from one bootstrap, so the one-time
  handoff really is one-time. A failed renewal is the only signal that
  re-authentication is needed, and it surfaces as `needs-login` rather than
  being guessed at.

### Desktop apps

- **GTK4 + libadwaita** front-end for GNOME, Hyprland and anything else,
  including a Wayland layer-shell overlay mode (`--layer`) for Hyprland.
- **Qt6 + Kirigami** front-end for KDE Plasma, so the app looks native there
  rather than like a GTK app in a Qt session.
- Both are real native applications over the same D-Bus contract, not a web
  view in a window, and both share one presentation core so they cannot drift
  apart in behaviour.
- A wizard-style flow for session setup, rather than dialogs that need
  scrolling and have no clear commit point.
- Cover art from Brain.fm, cached locally.
- An original app icon. Brain.fm's own mark is deliberately not used.

### Installing

- `./install.sh` works out the distribution, installs build dependencies with
  the system package manager, picks the front-end that suits the desktop,
  builds, installs to `~/.local`, and wires the result into the running
  session. Arch, Debian, Fedora, openSUSE, Alpine and Void families,
  derivatives included.
- `make build` detects toolkits with `pkg-config` rather than by distro
  package name; `make install` honours `PREFIX` and `DESTDIR` so distro
  packaging can call it.
- `.deb` and `.rpm` for the daemon and CLI, a portable headless tarball, and
  `SHA256SUMS`, built from one tag.
- A waybar module and an example configuration.

### Known limitations

- **The GTK app needs GTK ≥ 4.22 and libadwaita ≥ 1.9.** It uses
  `Picture::set_content_fit` and `ToolbarView`, so this is a build-time
  minimum rather than a preference. Debian 13 and Ubuntu 24.04 ship GTK 4.18
  and 4.14, so it cannot be built there yet. `install.sh` says so and falls
  back instead of failing partway through a build. The daemon, CLI, MPRIS and
  status bar have no such requirement.
- **No prebuilt desktop packages.** A front-end built against a current GTK
  would carry a glibc too new for the distributions a `.deb` or `.rpm`
  targets, so the apps are build-from-source for now.
- **`bfm login --email` has only been exercised against its error paths.**
  The maintainer's account is Google-linked, so the success path is covered by
  recorded fixtures rather than by a live sign-in.
- **The Flatpak manifest has never been built end to end**, and cannot fail a
  release until it does.
- **The renewal grace window is unmeasured.** If a token can go stale beyond
  recovery after a long idle period, re-bootstrapping becomes occasional
  rather than never.
- Brain.fm's API is private and undocumented. It can change without notice;
  when it does, the fix belongs in `bfm-api` and nowhere else.

[Unreleased]: https://github.com/MarcFord/brain.fm-linux-client/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/MarcFord/brain.fm-linux-client/releases/tag/v0.1.0
