# bfm — Brain.fm for Linux

An unofficial Brain.fm client for Linux, built as a **system service rather
than an app**: a D-Bus–activated daemon owns playback, and the CLI, both
desktop apps, your status bar and anything you write yourself are equal
clients of it.

```
bfm login --browser          # sign in, including Google/Apple/Facebook accounts
bfm play focus               # start a session
bfm-gtk                      # GNOME-native app   (or bfm-gtk --layer on Hyprland)
bfm-kde                      # Plasma-native app
playerctl -p bfm play-pause  # media keys, for free
```

> **Unofficial and unendorsed.** Brain.fm has not made, endorsed or
> supported this project. It talks to Brain.fm's private, undocumented web
> API, which can change without notice. You need your own paid Brain.fm
> account. Using a third-party client may conflict with Brain.fm's terms of
> service.

---

## Contents

- [Why a daemon](#why-a-daemon)
- [Install](#install)
- [Signing in](#signing-in)
- [Using it](#using-it)
- [Desktop integration](#desktop-integration)
- [Hooks](#hooks)
- [Writing your own client](#writing-your-own-client)
- [Building and testing](#building-and-testing)
- [How it is put together](#how-it-is-put-together)
- [Privacy and data](#privacy-and-data)
- [Known limitations](#known-limitations)
- [Credits](#credits)

---

## Why a daemon

Other unofficial clients exist and play audio perfectly well. What none of
them offer is a **public control surface**: the closest, `brainfm-tui`,
drives a headless TUI by forwarding keystrokes over a socket documented as
internal, which idles out after ten minutes. That is a remote control for a
text UI, not something other software can build on.

Here the daemon is the product:

- **`bfmd` is a real service.** D-Bus activated, no idle timeout, managed by
  systemd if you want it to be.
- **The control plane is public and versioned.** `dev.marcford.BrainFM1` is
  introspectable, additive-only within a major version, and tested like the
  API it is.
- **Nothing is privileged.** The CLI may not depend on the API or audio
  crates, and neither may the GUIs — enforced by tests. A plugin you write
  has exactly the same access the first-party apps do.

So this is a complete plugin, in any language:

```sh
busctl --user call dev.marcford.bfm /dev/marcford/bfm \
  dev.marcford.BrainFM1 StartSession ssasss focus "" 0 high 50m
```

---

## Install

### The short way

```sh
git clone https://github.com/MarcFord/brain.fm-linux-client
cd brain.fm-linux-client
./install.sh
```

One script for any distro. It works out what your system is, installs the
build dependencies with your own package manager, picks the front-end that
belongs on your desktop, builds, installs to `~/.local`, and wires the
result into the running session.

That last part matters more than it sounds: D-Bus reads service files when
the bus starts, so a freshly installed daemon is invisible until the bus
re-reads them. Without that step your first command fails with *"not
activatable"* on an installation that is otherwise perfect.

```sh
./install.sh --dry-run          # print the plan, change nothing
./install.sh --system           # /usr/local for all users (uses sudo)
./install.sh --gui both         # both front-ends regardless of desktop
./install.sh --gui none         # headless: daemon and CLI only
./install.sh --skip-deps        # don't touch the package manager
./install.sh --uninstall        # remove it again
```

Distro families it knows how to install dependencies for: Arch, Debian,
Fedora, openSUSE, Alpine and Void, including derivatives — it reads both `ID`
and `ID_LIKE` from `/etc/os-release`, because derivatives report their own
`ID` and matching on that alone fails on exactly the systems most likely to
run this. Anywhere else it tells you what to install and carries on with
`--skip-deps`. Package names are the least portable thing in the script, so a
failed dependency install is never fatal: the `pkg-config` check afterwards is
what decides, not the package manager's exit code.

Nothing here needs root unless you ask for `--system`. `bfmd` is a
session-bus daemon with a systemd *user* unit, so `~/.local` is where it
belongs.

### From packages

```sh
make package          # builds .deb and .rpm for bfm, bfm-gtk and bfm-kde
```

Three packages, so a server never pulls in a desktop stack and a GNOME user
never installs Qt:

| Package | Contents |
|---|---|
| `bfm` | daemon, CLI, D-Bus and systemd units, icons |
| `bfm-gtk` | GTK4/libadwaita app, depends on `bfm` |
| `bfm-kde` | Qt6/Kirigami app, depends on `bfm` |

An Arch `PKGBUILD` lives in `packaging/arch/`. See
[`docs/packaging.md`](docs/packaging.md) for the per-format reasoning,
including why AppImage is a poor fit for a daemon-backed app.

### From source

```sh
make            # builds the daemon, the CLI, and any GUI this machine can
sudo make install
```

`make build` detects what you have with `pkg-config` — not by distro package
name, since those differ everywhere and the libraries do not. A missing
toolkit skips a GUI; it never fails the build, so a headless server builds
the same tree as a Plasma desktop.

```sh
make detect     # show what was found and what will be built
make GUI=gtk    # or: kde, both, none
```

`make install` honours `PREFIX` and `DESTDIR`, so distro packaging can call
it directly. It installs a desktop entry only for a GUI that was actually
built — otherwise your application menu gains an entry that cannot launch.

Calling `make install` yourself leaves the session wiring to you, which
`./install.sh` would have done:

```sh
systemctl --user daemon-reload
busctl --user call org.freedesktop.DBus /org/freedesktop/DBus \
    org.freedesktop.DBus ReloadConfig       # or just log out and back in
```

Without the second command the service file you just installed is not yet
visible to the running bus, and the first `bfm` call fails with *"not
activatable"*.

### Dependencies

Always required: a C toolchain, `pkg-config`, ALSA headers, D-Bus.

| | Debian/Ubuntu | Fedora | Arch |
|---|---|---|---|
| Core | `libasound2-dev pkg-config` | `alsa-lib-devel pkgconf` | `alsa-lib pkgconf` |
| GTK app | `libgtk-4-dev libadwaita-1-dev` | `gtk4-devel libadwaita-devel` | `gtk4 libadwaita` |
| KDE app | `qt6-base-dev qt6-declarative-dev` + Kirigami | `qt6-qtdeclarative-devel kf6-kirigami-devel` | `qt6-declarative kirigami` |

The daemon is **D-Bus activated**: you never start it by hand. The first
call from any client launches it.

---

## Signing in

Brain.fm supports email, Google, Apple and Facebook. All of them work here,
but not all the same way — see [`docs/auth.md`](docs/auth.md) for why.

```sh
bfm login --email you@example.com   # password is prompted, never stored
bfm login --browser                 # Google / Apple / Facebook
bfm login --magic-link '<url>'      # if you were emailed one
bfm login --token-stdin < token     # scripts and headless hosts
```

**SSO accounts use `--browser`.** The provider login has to happen in a real
browser — we cannot drive Google's OAuth ourselves, and Google blocks
embedded webviews. `bfm` reads the resulting session straight out of your
browser profile, so there is no copying anything by hand.

Because token renewal is lazy and repeatable, that handoff is a **one-time
cost**, not a daily one: an expired token mints a fresh one on the next
call, indefinitely.

Only the session token is stored, in your keyring. The password is never
persisted under any method.

---

## Using it

```sh
bfm play focus --activity "Deep Work" --genre Electronic --effort high --timer 50m
bfm pause / resume / skip / stop
bfm timer "25m/5m x4"        # pomodoro; omit the argument to clear
bfm list states              # or: activities, genres
bfm status --json --watch    # one JSON line per change
bfm whoami / logout
```

Timers come in three shapes: **infinite** (the default), a **countdown**
(`50m`), or **intervals** (`25m/5m x4`). A finished timer ends the session.

---

## Desktop integration

**Two native apps, not one app with two themes.** Both render the same state
from the same shared core, so they cannot drift into describing it
differently, but each uses its own toolkit's real widgets.

- **`bfm-gtk`** — GTK4 + libadwaita. Native on GNOME, and the right choice
  on wlroots compositors.
- **`bfm-kde`** — Qt6 + Kirigami. Native on Plasma.

Both offer the same flow: pick a mental state, answer one question per
screen (activity → strength → genres → length), and play. The player shows
the track's own artwork, and favourite / don't-play-again / copy controls.

### Hyprland overlay

```sh
bfm-gtk --layer
```

Renders as a Wayland layer-shell overlay anchored top-right instead of a
window — bind it to a key and get a HUD with nothing to manage. Hyprland is
a compositor, not a toolkit, so this is a mode of the GTK app rather than a
third front-end.

Verify it with `make smoke-layer`.

### Status bar

`dist/waybar.jsonc` is a ready module. `bfm status --watch --json` emits
waybar's native shape, one line per change, with no polling.

`--json` never fails, because a status bar renders nothing at all if the
process exits: an absent daemon arrives in-band as `class: "unavailable"`,
so the bar can show the outage rather than claim `idle`. Plain `bfm status`
does the opposite and exits non-zero with something you can act on — it is
the command you run to find out whether this works, so it must not report a
dead daemon as a healthy idle one.

### Media keys

The daemon serves MPRIS, so `playerctl`, media keys, lock screens and your
bar's built-in MPRIS module all work with no configuration. Capabilities are
honest: `CanGoPrevious` and `CanSeek` are false, because Brain.fm sessions
only move forward and the decoder cannot seek.

---

## Hooks

Run a shell command when something happens. Event data arrives in the
environment, so quoting is never your problem.

```toml
# ~/.config/bfm/config.toml
[hooks]
session_start = "makoctl mode -a dnd"
session_end   = "makoctl mode -r dnd"
track_start   = "notify-send 'brain.fm' \"$BFM_TRACK\""
timer_phase   = "notify-send 'brain.fm' \"$BFM_TIMER_PHASE\""
```

| Event | Variables |
|---|---|
| `session_start` | `BFM_STATE`, `BFM_ACTIVITY` |
| `session_end` | `BFM_REASON` — `stopped`, `track-ended`, `timer-elapsed` |
| `track_start` | `BFM_TRACK`, `BFM_LENGTH` |
| `timer_phase` | `BFM_TIMER_PHASE`, `BFM_TIMER_REMAINING` |

Hooks are fire-and-forget: a slow one cannot stall playback, and two
triggered by the same event may finish in either order.

---

## Writing your own client

The D-Bus interface is the contract, and it is meant to be used.

```sh
busctl --user introspect dev.marcford.bfm /dev/marcford/bfm
```

**Stability promise.** The major version is in the interface name
(`BrainFM1`). Within it, changes are additive only — new members may appear,
existing signatures never change meaning. `InterfaceVersion` increments on
each addition, so feature-detect on that rather than probing for methods. It
has grown five times without breaking a call.

From Rust, depend on `bfm-proto` for a typed proxy:

```rust
let bfm = bfm_proto::BrainFM1Proxy::new(&zbus::Connection::session().await?).await?;
bfm.start_session("focus", "", &[], "high", "50m").await?;
```

Full method, property and signal listing in
[`docs/architecture.md`](docs/architecture.md).

---

## Building and testing

```sh
make check          # fmt, clippy -D warnings, and the full test suite
make smoke          # install + Hyprland overlay checks against the real system
```

The suite runs offline against recorded fixtures — **no test may touch the
live API**, and CI enforces it. The D-Bus surface is tested like the public
API it is, against a private bus via `dbus-run-session`.

Smoke checks exist because unit tests cannot see a compositor or a session
bus, and several bugs here hid in exactly that gap. They exit 77 to skip
where they cannot run, and each is known to fail when the thing it checks is
broken.

---

## How it is put together

```
crates/
  bfm-api      the private Brain.fm API: wire types, lazy token renewal
  bfm-proto    the public D-Bus contract; depend on this to build a client
  bfm-player   audio: progressive MP3, disk-backed buffering, gapless queue
  bfmd         the daemon: sessions, timers, hooks, MPRIS, BrainFM1
  bfm          CLI — a thin bfm-proto client
  bfm-ui-core  shared front-end core: state, wording, bus plumbing
  bfm-gtk      GTK4/libadwaita front-end
  bfm-kde      Qt6/Kirigami front-end
```

The rule holding it together: **no front-end may depend on `bfm-api` or
`bfm-player`.** The moment one can reach Brain.fm directly, the daemon stops
being the single source of truth and every third-party client becomes a
second-class citizen. There is a test for it.

---

## Privacy and data

- **Only the session token is stored**, in your keyring via Secret Service,
  falling back to a `0600` file where no keyring exists. Your password is
  never persisted.
- **Signed audio URLs are credentials** and are scrubbed from every log line
  and error message.
- **Cover art is cached** under `$XDG_CACHE_HOME/bfm/art`, the way any media
  player caches artwork.
- **Audio is never written to disk** beyond the playback buffer, and there is
  deliberately no download or offline feature.

---

## Known limitations

- The private API can break without warning. All knowledge of it is confined
  to `bfm-api`, so a break is a one-crate fix, and errors say "the API
  changed" rather than surfacing a decode failure.
- `bfm login --email` has only been exercised against its error paths; the
  maintainer's account is Google-linked.
- The Flatpak manifest is unproven — see [`docs/packaging.md`](docs/packaging.md).
- Snap and AppImage are not provided.

---

## Credits

The private API was mapped with reference to two MIT-licensed projects, then
verified against live traffic:

- [`dynamotn/brainfm-tui`](https://github.com/dynamotn/brainfm-tui)
- [`quintrino/brainfm-cli`](https://github.com/quintrino/brainfm-cli)

Both were useful starting points; where this client behaves differently, it
is because observed behaviour differed.

## License

MIT — see [LICENSE](LICENSE).
