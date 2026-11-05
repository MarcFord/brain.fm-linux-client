# Authentication

Brain.fm offers four sign-in providers (email, Google, Apple, Facebook) plus magic
links. `bfm` aims to support an account created by **any** of them. This document
explains how, and why the shape is what it is — the constraints here are external and
unpleasant, and the design only makes sense once they're stated.

## The constraint that drives everything

**We cannot perform Google/Apple/Facebook OAuth ourselves.**

- The web app authenticates against Brain.fm's own OAuth client
  (a **Web application** client). Web clients only accept redirect URIs their owner
  registered. We cannot add `http://127.0.0.1:…` to someone else's client, so the
  native-app pattern from RFC 8252 (loopback redirect + PKCE) is unavailable.
- Registering *our own* OAuth client doesn't help either: Brain.fm's API would receive a
  provider token issued to an audience it doesn't recognise.
- **Google actively blocks OAuth inside embedded webviews** (`disallowed_useragent`), so
  "just embed a WebKitGTK view" fails for the single most common provider.

So for SSO accounts there is no honest, self-contained CLI flow. The provider login must
happen in a **real browser**, and the resulting session has to be carried back.

## What makes that tolerable

Lazy renewal (see `docs/api.md`). A session token lasts 15 minutes, but presenting an
expired one returns a fresh token, repeatedly, without consuming the old one. A daemon
that keeps renewing therefore sustains a session **indefinitely** from a single
bootstrap.

That changes the UX calculus completely: a slightly clunky one-time handoff is
acceptable, because it is genuinely one-time — not a daily tax.

> Caveat: the renewal grace window is unmeasured. If a token can go stale beyond
> recovery after a long idle period, re-bootstrapping becomes occasional rather than
> never. `bfmd` surfaces that as `needs-login` rather than pretending.

## The methods

### 1. Email + password — fully self-service

```sh
bfm login --email you@example.com      # prompts; the password is never stored
```

`POST /v2/auth/email-login` with `{email, password}`. The password lives in a zeroizing
wrapper for the duration of the request and is never written anywhere. Only the
resulting session token reaches the keyring.

This is the only method that needs no browser. It is the default when the account has a
password.

### 2. Browser handoff — the universal fallback

```sh
bfm login --browser
```

Opens `https://my.brain.fm/` in the **system browser**, where the user signs in by
whatever means their account uses — Google, Apple, Facebook, email, all handled by
Brain.fm's real web app in a real browser, so no provider restrictions apply. `bfm` then
takes the resulting session token.

Transferring the token is the awkward step, and we should be honest that it is awkward.
Ranked by preference:

1. **Paste** (implemented first). `bfm` prints precisely where to find the token and
   reads it from stdin, never echoing it. Zero moving parts, works everywhere including
   over SSH.
2. **Loopback capture** (later, optional). `bfm` listens on `127.0.0.1` and provides a
   one-click bookmarklet that posts the token to it. Better UX, more moving parts, and
   browsers increasingly discourage console pasting.
3. **Driven browser** (later, optional, feature-gated). Drive a real Chromium via CDP
   and read the token once the user finishes logging in. Universal and smooth, but a
   heavy optional dependency — never a default.

### 3. Magic link — opportunistic

```sh
bfm login --magic-link 'https://my.brain.fm/…/<token>'
```

`GET /v2/auth/magic-link/{token}`. The web app can only **consume** these; nothing
requests one, so `bfm` cannot send you a link. Supported because it costs almost
nothing, and when a user happens to have a link it is the smoothest path of all.

### 4. Token on stdin — scripting and headless

```sh
bfm login --token-stdin < token.txt
```

For CI, headless boxes, and anyone automating setup. Identical to the paste path with
the prompting removed.

## Storage

- Only the **session token** is persisted, in the system keyring via Secret Service
  (`keyring` crate), falling back to a `0600` file in `0700` directories on hosts with
  no keyring daemon.
- The **password is never persisted** under any method — lazy renewal means we never
  need it again after the initial exchange.
- The daemon replaces the stored token every time a response carries a rotated one.

## What `bfmd` does with failure

A renewal that fails is the *only* signal that re-authentication is required. The daemon
does not guess, retry in a loop, or prompt: it emits
`AuthStateChanged(authenticated=false)` and `Error(code="needs-login")` on D-Bus, and
every client — CLI, status bar, Omarchy plugin — decides how to surface that.

Note that `POST /v2/auth/email-login` is **rate limited**: a handful of rapid attempts
returns `429` with an empty body. `bfm login` backs off rather than retrying.

## Status

| Method | API shape | Implemented |
|---|---|---|
| Email + password | Request confirmed; success body unverified | `bfm-api` |
| Browser handoff (paste) | No API call — reuses a web session token | planned, M1 |
| Magic link | `GET /auth/magic-link/{token}`, unverified | planned, M1 |
| Token on stdin | No API call | planned, M1 |
| Direct Google/Apple/Facebook exchange | Field names unverified; blocked by OAuth client ownership | **not planned** |
