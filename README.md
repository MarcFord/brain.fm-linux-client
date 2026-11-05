# bfm — Brain.fm for Linux

An unofficial Brain.fm client for Linux, built as a **system service rather than
an app**. The daemon owns playback and exposes a stable, versioned D-Bus API;
the CLI, your status bar, and third-party plugins are all equal clients of it.

> **Unofficial and unendorsed.** Brain.fm has not made, endorsed, or supported
> this project. It talks to Brain.fm's private, undocumented web API, which can
> change without notice. You need your own paid Brain.fm account. Using a
> third-party client may conflict with Brain.fm's terms of service.

## Status

Usable end to end, if rough. `bfm login` works for every sign-in provider,
`bfm play focus` starts a session, and `bfm status --watch --json` feeds a
status bar. The control plane is live. `bfmd` serves `dev.marcford.BrainFM1`
on the session bus today — introspect it with
`busctl --user introspect dev.marcford.bfm /dev/marcford/bfm` — and session
timers work end to end over it. Playback is not wired up yet: anything needing
the API answers with the stable `needs-login` code until credentials land.

Milestone 0 (mapping the private API) is complete — see
[`docs/api.md`](docs/api.md) and [`docs/auth.md`](docs/auth.md). Everything else
is in progress; `bfmd` and `bfm` are stubs today.

**Sign-in**: accounts created with email, Google, Apple or Facebook are all
supported. Only email/password can be driven entirely from the terminal; the
SSO providers sign in through your real browser once, and `bfm` adopts the
session from there. Since token renewal is lazy and non-consuming, that handoff
is a one-time cost rather than a daily one. See [`docs/auth.md`](docs/auth.md)
for why it has to work that way.

## Why another one

Two unofficial Rust clients already exist and both play audio well. Neither
exposes a public control surface: the closest, `dynamotn/brainfm-tui`, drives a
headless TUI by **forwarding keystrokes** over an internal socket that is
documented as private and idles out after ten minutes.

That is a remote control for a text UI, not something other software can build
on. `bfm` inverts it:

- **The daemon is the product.** `bfmd` is a D-Bus-activated systemd user
  service with no idle timeout.
- **The control plane is public and versioned.** `dev.marcford.BrainFM1` is
  introspectable, additive-only within a major version, and tested like the API
  it is.
- **Everything else is a client.** The CLI must not depend on the API or audio
  crates — enforced by a test.

So a plugin is a one-liner in any language:

```sh
busctl --user call dev.marcford.bfm /dev/marcford/bfm \
  dev.marcford.BrainFM1 StartSession 'sssass' focus deep-work 0 high 50m
```

## Layout

| Crate | Purpose |
|---|---|
| `bfm-api` | The private Brain.fm API. Typed wire models, lazy token rotation |
| `bfm-proto` | The public D-Bus contract. Depend on this to build a client |
| `bfm-player` | Audio engine — progressive MP3, bounded buffering |
| `bfmd` | The daemon: sessions, timers, hooks, MPRIS, `BrainFM1` |
| `bfm` | Thin CLI client |

## Desktop integration

- **MPRIS** — media keys, `playerctl -p bfm`, lock-screen widgets
- **systemd** — `dist/bfmd.service`, D-Bus activated, hardened
- **waybar** — `dist/waybar.jsonc`; `bfm status --watch --json` streams state
- **Hooks** — run a command on session start/end, track start, and timer
  phase changes; event data arrives in the environment. See
  [`dist/config.example.toml`](dist/config.example.toml)

```toml
# ~/.config/bfm/config.toml
[hooks]
session_start = "makoctl mode -a dnd"
session_end   = "makoctl mode -r dnd"
timer_phase   = "notify-send bfm \"$BFM_TIMER_PHASE\""
```

## Building

```sh
cargo build --workspace
cargo test --workspace
```

Needs ALSA headers (`libasound2-dev` / `alsa-lib`) and a session D-Bus.

## Privacy

Your password is never stored — only the session token, in the system keyring
via Secret Service. Signed audio URLs are bearer credentials and are scrubbed
from logs and error messages. Nothing is cached to disk beyond the playback
buffer, and there is deliberately **no download or offline feature**.

## Credits

The shape of the private API was mapped with reference to two MIT-licensed
projects, then verified against live traffic. Thanks to:

- [`dynamotn/brainfm-tui`](https://github.com/dynamotn/brainfm-tui)
- [`quintrino/brainfm-cli`](https://github.com/quintrino/brainfm-cli)

Where this project's [`docs/api.md`](docs/api.md) disagrees with them, it is
because the live capture showed something different.

## License

MIT — see [LICENSE](LICENSE).
