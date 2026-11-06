---
name: Bug report
about: Something does not work
labels: bug
---

**What happened, and what you expected instead**

**To reproduce**

**Versions**

```
bfm --version
bfmd --version
busctl --user get-property dev.marcford.bfm /dev/marcford/bfm \
  dev.marcford.BrainFM1 InterfaceVersion
```

Distro, desktop (GNOME / Plasma / Hyprland / other), Wayland or X11.

**Daemon log**

Run `BFM_LOG=debug bfmd` and paste the relevant part. Signed audio URLs are
scrubbed automatically, so the log is safe to share — but do check before
posting, and never paste a session token.

**If a GUI is involved**

Which one, and whether `bfm status` from the terminal shows the same
problem. That separates a front-end bug from a daemon one.
