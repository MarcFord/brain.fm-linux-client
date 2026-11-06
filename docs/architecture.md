# Architecture

## Thesis

Prior art already plays Brain.fm audio in a terminal. `dynamotn/brainfm-tui` has MPRIS,
a background daemon, and `playerctl` support today. So "add MPRIS and a daemon" is not
a differentiator — it's table stakes we inherit.

The gap is this: **brainfm-tui's daemon has no public API.** Its socket protocol is
`Hello`/`Key`/`Reload`/`Stop` — it forwards *keystrokes* to a headless TUI, is
explicitly documented as internal, and idles out after 10 minutes. That is a remote
control for a text UI, not a service other software can build on.

So the differentiator is a change of kind, not a longer feature list:

> **The daemon is the product. Its control plane is a stable, versioned, documented
> public API. The CLI, the status bar module, and the Omarchy plugin are all just
> clients of it — none of them privileged.**

Everything below follows from that one commitment. If a design choice makes the daemon
easier to embed, script, or extend, it wins.

## Control plane: D-Bus

One public API, on the session bus. Not a bespoke Unix socket.

Why D-Bus is correct here and a custom socket is not:

- **It is already running.** Every Linux desktop session has a bus. No socket path
  negotiation, no lock files, no idle-timeout races.
- **Activation is free.** D-Bus service activation starts `bfmd` on first call. That
  replaces hand-rolled spawn-or-attach logic entirely.
- **Introspectable.** `busctl introspect` documents the API at runtime. A plugin author
  discovers the surface without reading our source.
- **Language-agnostic and shell-friendly.** Omarchy plugins are shell-heavy; `busctl
  call` and `gdbus` are one-liners. A bespoke JSON socket means every consumer writes a
  client.
- **Signals are push.** Status bars subscribe instead of polling.

### Two interfaces

**1. `org.mpris.MediaPlayer2.bfm`** — standard media control. Gets us media keys,
`playerctl`, waybar's built-in MPRIS module, and lock-screen widgets for free. MPRIS is
deliberately generic, so it carries play/pause/next/metadata and nothing domain-specific.

**2. `dev.marcford.BrainFM1`** — the domain API, for everything MPRIS cannot express:

```
Methods
  StartSession(state: s, activity: s, genres: as, effort: s, duration: s) -> session_id: s
  StopSession()
  SkipTrack()
  SetTimer(spec: s)                  # "50m", "25m/5m", "until 17:00"
  ListMentalStates() -> aa{sv}
  ListActivities(state: s) -> aa{sv}
  ListGenres(state: s) -> aa{sv}
  Favorite(track_id: s, on: b)
  Dislike(track_id: s)
  GetStatus() -> a{sv}               # one-shot snapshot for scripts

Properties (all read-only except Volume)
  InterfaceVersion: u                # stability contract, see below
  State: s                           # idle | buffering | playing | paused
  MentalState: s
  Activity: s
  Track: a{sv}                       # id, name, length, neural effect level
  Position: t
  Timer: a{sv}                        # phase, remaining, cycle index
  Authenticated: b
  Volume: d

Signals
  SessionStarted(session_id: s)
  SessionEnded(reason: s)
  TrackStarted(name: s, length: t)
  TimerPhaseChanged(phase: s, remaining: t)
  AuthStateChanged(authenticated: b)
  Error(code: s, message: s)
```

Note what is *absent*: no keystrokes, no UI concepts, no view state. The API speaks only
about sessions, tracks, and timers. That is what makes it embeddable.

### Stability contract

Because an Omarchy plugin will be built against this, the contract has to hold:

- The interface name carries the major version (`…BrainFM1`). A breaking change means
  `BrainFM2`, served *alongside* `BrainFM1` for a deprecation window.
- Within a major: **additive only.** New methods, properties, and signals may appear;
  existing signatures never change meaning.
- `InterfaceVersion` increments on each additive change, so a client can feature-detect
  rather than version-sniff.
- The D-Bus surface is covered by integration tests run against a private bus. It is
  tested like an API, because it is one.

## Workspace layout

A cargo workspace, not one binary — because the point is that pieces are reusable.

```
crates/
  bfm-api      brain.fm HTTP client. Typed DTOs, implicit token rotation, JWT uid
               decode. Knows nothing about audio, D-Bus, or config.
               Publishable: other people's clients can use it.
  bfm-proto    D-Bus interface definitions and shared types. The stability contract,
               as code. Publishable: native Rust plugins depend on this and nothing else.
  bfm-player   Audio engine. `trait Player` + rodio implementation.
  bfmd         The daemon. Session orchestration, timer state machine, hook runner,
               MPRIS server, BrainFM1 server, config, secrets.
  bfm          CLI. A thin bfm-proto client. No API or audio code whatsoever.
  bfm-ui-core  Shared front-end core: state model, duration formatting, bus
               plumbing, browser-profile session scan. Toolkit-agnostic, so
               GTK and Qt render one model rather than each deriving its own.
  bfm-gtk      GTK4 + libadwaita front-end (GNOME, Hyprland), with a Wayland
               layer-shell overlay mode.
  bfm-tui      Optional ratatui frontend. Also just a client. (Later.)
```

The hard rule that keeps this honest: **`bfm` must not depend on `bfm-api` or
`bfm-player` — and neither may any front-end.** The moment a first-party client can talk
to Brain.fm directly, the daemon stops being
the product and the plugin surface rots.

## Internals of `bfmd`

- **tokio** multi-threaded runtime; **zbus** for both D-Bus interfaces.
- **The audio thread's runtime must be multi-threaded.** `StreamDownload`
  establishes the stream and then keeps downloading on a background task. On a
  current-thread runtime that task only runs while something is blocked on it, so the
  download stops the instant `new_http` returns and the decoder waits forever. The
  symptom is silence with no error and a session stuck in `buffering`.
- **Audio on its own thread.** rodio owns the output device; the daemon talks to it over
  typed command/event channels. Confirmed against live traffic: the audio is progressive
  MP3 from `audio2.brain.fm`, not HLS, so `stream-download` → `rodio::Decoder` is
  sufficient and we need no `mpv` subprocess and no GStreamer. See `docs/api.md`.
- **Bounded buffering, not in-memory.** Tracks are **30 minutes** long (`1800s`
  observed), so a whole track is ~30–60 MB. Use temp-file or bounded storage rather than
  `stream-download`'s `MemoryStorageProvider`, which `brainfm-tui` uses on the assumption
  of short tracks.
- **Queue, not playlist.** With 30-minute tracks, prefetch-before-end is a minor
  optimisation rather than the core of the design. Fetch the next track when the current
  one is a minute or two from ending.
- **Lazy token renewal.** Never refresh proactively. Every API response is checked for an
  envelope `token` field; when present it replaces the stored one. A renewal failure is
  the only "re-login required" signal, and it surfaces on D-Bus as `AuthStateChanged`.
- **Generation counters** on session requests and track loads, so a superseded response
  can't resurrect stale playback. (Learned from brainfm-tui; it's a real race.)
- **Timer state machine** is first-class and lives in the daemon, not the UI — that's
  what makes `SetTimer("25m/5m")` and `TimerPhaseChanged` available to every client.
- **No idle timeout.** It's a system service managed by systemd, not a spawned helper.

## Secrets

- Refresh/session token in the **keyring** via Secret Service, with a `0600`-in-`0700`
  file fallback for headless boxes.
- Password lives only in a zeroizing wrapper, excluded from `Debug`, never persisted,
  never crosses the bus.
- A **`tracing` layer that scrubs** bearer tokens and signed `tokenedUrl` query strings
  from every log line and error message. Signed URLs leaking into a bug report is the
  realistic failure mode.

## Plugin model

Three tiers, deliberately ordered cheapest-first:

**Tier 1 — D-Bus (zero coupling).** Any language, or shell:
```sh
busctl --user call dev.marcford.bfm /dev/marcford/bfm \
  dev.marcford.BrainFM1 StartSession 'sssass' focus deep-work 0 high 50m
```
This is what the Omarchy plugin targets. It needs nothing from us but a stable name.

**Tier 2 — hooks (no code).** The daemon runs user commands on lifecycle events, event
data passed in the environment. Declarative, in `config.toml`:
```toml
[hooks]
session_start = "makoctl mode -a dnd"
session_end   = "makoctl mode -r dnd"
timer_phase   = "notify-send 'bfm' \"$BFM_TIMER_PHASE\""
```
This covers most "integrate with my desktop" wants without anyone writing a client.

**Tier 3 — Rust crates.** `bfm-proto` for native clients, `bfm-api` for people who want
the API without our daemon.

## Desktop integration

- **systemd user unit**, D-Bus activated (`Type=dbus`, `BusName=dev.marcford.bfm`), so
  it starts on demand and is managed like any other service.
- **waybar**: a `custom` module running `bfm status --watch --json`, which streams a
  JSON line per state change (`text`/`alt`/`tooltip`/`class`, waybar's native shape).
  The built-in MPRIS module also works via tier 1; `--watch` exists because it can show
  timer phase and neural effect level, which MPRIS metadata cannot carry.
- **playerctl / media keys**: free, via MPRIS.
- Shell completions (`clap_complete`) and a man page (`clap_mangen`) generated in CI.

## Testing

The API is undocumented and can break any Tuesday, so tests must not touch the network:

- **`wiremock` + recorded fixtures** for every `bfm-api` endpoint. Fixtures are the
  executable version of `docs/api.md`.
- **`insta`** snapshots for CLI output and any TUI rendering.
- **D-Bus integration tests** under `dbus-run-session` against a private bus, asserting
  the published interface matches `bfm-proto`. This is the one test suite that must
  never be allowed to rot.
- A separate, manual, never-in-CI smoke target that hits the real API, for confirming
  `docs/api.md` is still accurate.

## Milestones

| # | Deliverable | Exit criteria |
|---|---|---|
| 0 | ~~Live API verification~~ **done 2026-11-05** | Envelope, auth, renewal, catalog and track payload captured; `fixtures/catalog.json` recorded |
| 1 | `bfm-api` | Email login, lazy token rotation, keyring storage, session create, catalog. Fully wiremock-tested |
| 2 | ~~`bfm-player`~~ **written, unrun** | Progressive stream plays, gapless queue, volume, fade |
| 3 | ~~`bfmd` + `bfm-proto`~~ **done** | `BrainFM1` served and introspectable, timers end to end, D-Bus activation, auth and playback wired |
| 4 | ~~`bfm` CLI~~ **done** | Thin client: `login` (4 methods), `play`, `stop`, `timer`, `list`, `status --json --watch` |
| 5 | ~~Desktop citizen~~ **mostly done** | MPRIS served, systemd unit, waybar module, D-Bus activation. Completions and man page outstanding |
| 6 | ~~Extensibility~~ **done** | Timer state machine, hooks, published proxy. **Omarchy plugin unblocked** |
| 7 | ~~Native GUI~~ **GTK done; KDE broken** | `bfm-gtk` verified end to end, buttons included. `bfm-kde` compiles, runs and creates **no window** — see below |
| 8 | `bfm-tui` | Optional frontend, as a client |

Milestone 6 is the real finish line — it's the one the next project depends on.

## Risks

- **No official API.** All Brain.fm knowledge is confined to `bfm-api` so breakage is a
  one-crate fix. Errors must say "the API changed" rather than surfacing a serde error.
- **Terms of service.** A third-party client for an account you pay for is a gray area
  and likely violates Brain.fm's ToS. Mitigations that cost nothing: prominent
  unofficial/unendorsed disclaimer, **no download or offline-cache feature**, no content
  retention beyond the in-memory playback buffer, no credential workarounds.
- **Naming.** Avoid their trademark in the binary and crate names; `bfm` throughout.
- **Attribution.** API knowledge came from two MIT-licensed projects. Credit both in the
  README and `docs/api.md`; write our own implementation rather than vendoring theirs.


## Known broken: the KDE front-end does not display

`bfm-kde` builds, starts, connects to Wayland and runs its event loop, but
creates no window. **It has never displayed anything.**

### The standout clue

`engine.load()` on a **deliberately nonexistent URL** also produces no
warning. A working `QQmlApplicationEngine` always complains about a missing
component. Zero diagnostics for a guaranteed-bad load says the problem is
upstream of our QML, and it is what blocks every other line of enquiry.

### Ruled out, each by test rather than by reasoning

| Suspect | How it was eliminated |
|---|---|
| Our QML | `qml6` renders the exact same `Probe.qml` and shows a window |
| Kirigami, Qt, Wayland | Same — all work outside our binary |
| The qrc path | Read the UTF-16 resource names out of the compiled binary: it is `qrc:/qt/qml/dev/marcford/bfm/qml/Main.qml`, which is what the code uses |
| `visible: false` | QQC2's `ApplicationWindow` does default to invisible and Kirigami inherits it — a real bug, now fixed, but not the cause |
| Release-mode log suppression | The debug build behaves identically |
| Platform plugin | `wayland` and `xcb` forced individually; both plugins present; no window either way |
| A null app or engine | Both non-null; `application_name()` returns `"bfm-kde"`, so the singleton is live |
| qrc vs filesystem | A `file://` URL to a real, known-good file behaves the same |
| Registration order | Calling `init_qml_module!` after `QGuiApplication` changes nothing |
| Our own `Main.qml` complexity | A five-line Kirigami window from the same module also shows nothing |

### Where to pick it up

1. **Install a Qt message handler** (`qInstallMessageHandler`) through a
   small C++ shim. Until Qt can report, everything else is guesswork — and
   the silence on a bogus URL suggests its output never reaches stderr in
   this binary.
2. Connect `objectCreationFailed` / `objectCreated`. cxx-qt generates
   connect helpers for `#[qsignal]`s, but `on_object_created` did not
   resolve on `Pin<&mut QQmlApplicationEngine>` — worth finding the correct
   form, since it answers directly whether the component instantiates.
3. Build the upstream `qml_minimal` example against this exact Qt and
   cxx-qt. If it also shows nothing, the fault is environmental rather than
   ours, which would redirect the whole search.

### A verification lesson worth keeping

This was originally reported as working. The test ran it offscreen and
concluded success because the process survived eight seconds —
`QGuiApplication::exec` runs forever whether or not a window exists. For a
GUI, the only smoke test that means anything is *did a window appear*.
