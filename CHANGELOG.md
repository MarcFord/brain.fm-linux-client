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

### Security

- **`bfm login --browser` could send another website's token to Brain.fm.**
  The profile read scanned the browser's `localStorage` files for anything
  JWT-shaped and recognised Brain.fm's session by its claims alone (`_id`,
  `iat`, `exp`, and no `iss` or `aud`). Nothing about that shape is unique
  to Brain.fm: a token from any other site the user is signed in to, with
  the same claims and a newer `iat`, would win the selection and be sent as
  a bearer token to `api.brain.fm` in the verification request — leaking
  that site's live credential to a third party, whether or not Brain.fm
  accepted it.

  The files are now read as the LevelDB records they are, and a token is
  taken only from a record whose key names an `https://…brain.fm` origin
  as first-party storage. Another site's token is never a candidate, no
  matter what its claims look like. The claim check stays, but only to
  skip the SSO provider's id token that Brain.fm's own web app stores next
  to the session.

  A consequence worth having: a session that Chrome has already compacted
  out of the write-ahead log is now found too. Those tables are Snappy
  compressed, which a byte scan could never see into, so the old read only
  worked for a recent login.

### Documentation

- How to gate on `bfm capabilities` safely: the probe is a subprocess, so its
  answer is asynchronous and anything event-driven fires first. Default to
  "the guarantee is absent" and let the probe turn a fallback off, never the
  reverse. Reported from the Omarchy plugin, where both MPRIS edges arrive
  before the probe answers — the safe direction, but a client gated the other
  way round would trust a guarantee it had not yet received, in a window that
  is usually empty at startup and so survives testing.

## [0.1.2] — 2026-11-07

Serves `BrainFM1` interface version **6**, unchanged from 0.1.1.

### Added

- **`bfm capabilities`**, so a client can gate on evidence rather than on a
  version string. Prints the named behaviours this binary guarantees, as JSON
  or as greppable `key value` lines.

  It exists because not every guarantee is visible on the bus. `--watch`
  following the daemon in and out of existence, added in 0.1.1, is *behaviour*
  rather than a D-Bus member: no interface version moves for it, and nothing
  on the bus reveals it. A client could only trust a README line, and being
  wrong meant a status bar showing `playing` for a daemon that was gone — a
  bar that looks like a working bar, which is the least diagnosable failure
  this project has. Raised by the Omarchy plugin, which was carrying a
  workaround precisely because the fix was undetectable.

  Deliberate properties:

  - It needs **no session bus and no daemon**, and starts neither. Asking a
    program what it can do must not require running a service.
  - It advertises **CLI behaviour only**. Daemon features stay gated on
    `InterfaceVersion`, because a probe that claimed them would lie the
    moment a new CLI met an old daemon — the exact case it exists to resolve.
  - `interfaceVersion` in the output is what the CLI was *built against*, not
    what any daemon serves.
  - Feature names are **permanent**; a test locks the released set so a
    rename cannot pass silently.

## [0.1.1] — 2026-11-07

Serves `BrainFM1` interface version **6**.

### Added

- **Timer hooks you can script against.** `timer_set` and `timer_cleared` are
  new, and every timer event — including the existing `timer_phase` — now
  carries the shape of the timer: `BFM_TIMER_SPEC`, `BFM_TIMER_MODE`,
  `BFM_TIMER_CYCLE`, `BFM_TIMER_CYCLES`, `BFM_TIMER_UNBOUNDED` and
  `BFM_TIMER_TOTAL`.

  Two gaps closed. `timer_phase` fires only on *transitions*, and the first
  work period begins without one, so there was nothing to hook when a pomodoro
  started — `timer_set` covers that, including when the timer came from
  `bfm play --timer`. And the hook environment knew less than the `Timer` D-Bus
  property did: a script could see `break` but not that it was break 3 of 4.
- **Endless intervals.** `"25m/5m x*"` (or `xinf`) cycles work and break until
  stopped, matching what the web player's intervals mode actually does. There
  was previously no spec for it at all: a missing count means exactly one
  cycle, and `x0` was rejected, so a client could only approximate continuous
  cycling with an absurd count like `x99`. A missing count still means one
  cycle — changing that would silently alter what existing clients get from a
  spec they already send.
- **The `Timer` property now reports `mode`, `spec` and `unbounded`**
  (interface version **6**). A countdown and a single-cycle interval were
  indistinguishable — both `cycles=1, phase="work"` — so a client could not
  restore what the user had chosen and had to keep its own shadow copy, which
  went wrong after a restart. `spec` returns the string as set. Reported by a
  bar widget that hit exactly that.
- Timer specs now accept units and the cycle marker in any case, so `7H` and
  `25m/5m X4` work alongside `7h` and `x4`. `7h` is the natural way to write
  sleep's seven-hour suggestion, and the previous rejection of `7H` was
  gratuitous.

### Fixed

- **`StartSession` did not record the timer spec**, so `Timer.spec` came back
  empty for a session started with `--timer` while being correct for one set
  afterwards with `SetTimer`. That is backwards: starting a session with a
  timer is the common path, so the field existed and was right in the case a
  client least needed it. `mode`, `cycles`, `unbounded` and `total` were
  correct on both paths, which is what made it easy to miss.
- **`SetTimer` did not emit a change for the `Timer` property**, so a client
  with a caching proxy — zbus's default — set a timer and read back the value
  from before its own call. Found by a contract test written for the additions
  above; it failed on its first run for this reason rather than the one it was
  written for.
- `bfm timer ""` printed `timer set: ` while actually clearing the timer. An
  empty or blank argument now reports `timer cleared`, like passing none.
- `bfm status --watch` did not notice the daemon appearing or disappearing, so
  a status bar built on it was silently wrong in both directions: a watcher
  started with no daemon printed one `unavailable` and then stayed silent
  forever even once a daemon came up, and a watcher whose daemon died
  mid-stream printed nothing at all, leaving a bar showing `playing`
  indefinitely for a daemon that was gone. It now follows
  `NameOwnerChanged`, emitting `unavailable` when the name is lost and a fresh
  state line when it returns. Reported from a real bar widget built on this
  stream.
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
- The Flatpak build failed at AppStream compose with `icon-file-read-error`:
  the KDE SDK cannot read the SVG icon that `make install` ships. Raster icons
  pre-rendered from the same SVG are now installed in the Flatpak build only,
  so distro package contents are unchanged. Adding them alongside the SVG was
  not enough — compose *prefers* a scalable icon and fails on it — so the
  Flatpak tree drops the SVG as well.

### Added

- `bfm list session-lengths [--state <s>]`, printing `seconds<TAB>label`.
  `ListSessionLengths` has been on the interface since version 2, but the CLI
  never exposed it, so every CLI-driven client had to hard-code durations —
  precisely what that method exists to prevent. The real lists diverge sharply
  by state: focus suggests 60/90/120 minutes, relax 15/45/60, and sleep
  7/8/9 *hours*, so one hardcoded list is wrong for most states. Reported by a
  bar widget that had to invent its own.
- AppStream metadata for the Flatpak
  (`packaging/flatpak/dev.marcford.bfm.metainfo.xml`), so the app presents
  itself properly to software centres. Validated by `appstreamcli` in CI,
  which also checks that it carries a `<release>` for the tag being built.
- The Flatpak bundle is now also built on pushes to the protected branches,
  not only on tags, so the manifest can be iterated on without cutting a
  release. It is informational in both places and gates nothing.

  With the above fixed it now builds end to end for the first time:
  `appstreamcli compose` succeeds, the icon, D-Bus service file and metainfo
  are exported, and `flatpak build-bundle` produces a bundle. It stays
  non-blocking for two honest reasons: nothing yet checks that the bundle
  installs and launches, which is a stronger claim than that it compiles; and
  it pulls runtimes from Flathub, so an outage there is not a reason to block
  a release.
- The full suite runs on pull requests into `main` and `testing`, which is
  what lets it be a *required* check there. Both branches are now protected
  and require all seventeen of its jobs.
- `make smoke-watch`, which moves a real daemon underneath a live watcher in
  both directions. The watch bugs above were invisible to unit tests because
  they are failures of silence, and only observable by starting and killing a
  daemon under a running watcher.

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

[Unreleased]: https://github.com/MarcFord/brain.fm-linux-client/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/MarcFord/brain.fm-linux-client/releases/tag/v0.1.2
[0.1.1]: https://github.com/MarcFord/brain.fm-linux-client/releases/tag/v0.1.1
[0.1.0]: https://github.com/MarcFord/brain.fm-linux-client/releases/tag/v0.1.0
