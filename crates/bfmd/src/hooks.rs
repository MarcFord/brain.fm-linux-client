//! Lifecycle hooks: run a command when something happens.
//!
//! This is the "no code required" extension tier from `docs/architecture.md`.
//! Most "make it talk to my desktop" wants — toggle do-not-disturb, dim the
//! lights, post a notification, pause Slack — are one shell line, and making
//! people write a D-Bus client for that would be absurd.
//!
//! Hooks are fire-and-forget by design. A hook that hangs must never stall
//! playback, so nothing is awaited and nothing is reaped for its exit status.
//!
//! The consequence, which is worth knowing before you rely on it: **hooks are
//! not ordered relative to each other.** When a timer elapses, `timer_phase`
//! and `session_end` are spawned back to back and may finish in either order.
//! If a hook needs ordering, do the sequencing inside that one script.

use std::{collections::HashMap, path::PathBuf, process::Stdio};

use serde::Deserialize;

/// `[hooks]` in `$XDG_CONFIG_HOME/bfm/config.toml`.
#[derive(Debug, Default, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Hooks {
    /// A session began.
    pub session_start: Option<String>,
    /// A session ended. `$BFM_REASON` says why.
    pub session_end: Option<String>,
    /// A new track started. `$BFM_TRACK` is its name.
    pub track_start: Option<String>,
    /// A timer moved between work and break. `$BFM_TIMER_PHASE` is the new one.
    pub timer_phase: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    hooks: Hooks,
}

/// Which hook to run, and what to tell it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    SessionStart { state: String, activity: String },
    SessionEnd { reason: String },
    TrackStart { track: String, length: u64 },
    TimerPhase { phase: String, remaining: u64 },
}

impl Event {
    fn name(&self) -> &'static str {
        match self {
            Self::SessionStart { .. } => "session_start",
            Self::SessionEnd { .. } => "session_end",
            Self::TrackStart { .. } => "track_start",
            Self::TimerPhase { .. } => "timer_phase",
        }
    }

    /// Event data, passed as environment rather than as argv so a hook never
    /// has to care about quoting.
    pub fn env(&self) -> HashMap<&'static str, String> {
        let mut e = HashMap::new();
        e.insert("BFM_EVENT", self.name().to_owned());
        match self {
            Self::SessionStart { state, activity } => {
                e.insert("BFM_STATE", state.clone());
                e.insert("BFM_ACTIVITY", activity.clone());
            }
            Self::SessionEnd { reason } => {
                e.insert("BFM_REASON", reason.clone());
            }
            Self::TrackStart { track, length } => {
                e.insert("BFM_TRACK", track.clone());
                e.insert("BFM_LENGTH", length.to_string());
            }
            Self::TimerPhase { phase, remaining } => {
                e.insert("BFM_TIMER_PHASE", phase.clone());
                e.insert("BFM_TIMER_REMAINING", remaining.to_string());
            }
        }
        e
    }
}

impl Hooks {
    /// `$XDG_CONFIG_HOME/bfm/config.toml`, else `~/.config/bfm/config.toml`.
    pub fn default_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("bfm").join("config.toml")
    }

    /// Load from disk. A missing file means no hooks; a malformed one is
    /// reported and then ignored, because a typo in config must not stop the
    /// daemon from playing music.
    pub fn load() -> Self {
        let path = Self::default_path();
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "could not read config");
                return Self::default();
            }
        };
        Self::parse(&raw).unwrap_or_else(|e| {
            tracing::error!(path = %path.display(), error = %e, "ignoring malformed config");
            Self::default()
        })
    }

    pub fn parse(raw: &str) -> Result<Self, toml::de::Error> {
        Ok(toml::from_str::<Config>(raw)?.hooks)
    }

    fn command_for(&self, event: &Event) -> Option<&str> {
        let cmd = match event {
            Event::SessionStart { .. } => &self.session_start,
            Event::SessionEnd { .. } => &self.session_end,
            Event::TrackStart { .. } => &self.track_start,
            Event::TimerPhase { .. } => &self.timer_phase,
        };
        cmd.as_deref().map(str::trim).filter(|c| !c.is_empty())
    }

    pub fn any_configured(&self) -> bool {
        [
            &self.session_start,
            &self.session_end,
            &self.track_start,
            &self.timer_phase,
        ]
        .into_iter()
        .any(|c| c.as_deref().is_some_and(|s| !s.trim().is_empty()))
    }

    /// Run the hook for `event`, if one is configured.
    ///
    /// Returns immediately. The child is detached with its output discarded:
    /// we are not a process supervisor, and a chatty hook should not fill the
    /// daemon's log.
    pub fn fire(&self, event: &Event) {
        let Some(cmd) = self.command_for(event) else {
            return;
        };
        tracing::debug!(event = event.name(), "firing hook");

        let mut child = std::process::Command::new("sh");
        child
            .arg("-c")
            .arg(cmd)
            .envs(event.env())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        match child.spawn() {
            Ok(_) => {}
            Err(e) => tracing::warn!(event = event.name(), error = %e, "hook failed to start"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_config_has_no_hooks() {
        let h = Hooks::parse("").unwrap();
        assert!(!h.any_configured());
    }

    #[test]
    fn parses_the_documented_shape() {
        let h = Hooks::parse(
            r#"
            [hooks]
            session_start = "makoctl mode -a dnd"
            session_end   = "makoctl mode -r dnd"
            timer_phase   = "notify-send bfm \"$BFM_TIMER_PHASE\""
            "#,
        )
        .unwrap();
        assert_eq!(h.session_start.as_deref(), Some("makoctl mode -a dnd"));
        assert_eq!(h.session_end.as_deref(), Some("makoctl mode -r dnd"));
        assert!(h.track_start.is_none());
        assert!(h.any_configured());
    }

    /// A typo should be loud, not silently ignored as "no hook".
    #[test]
    fn an_unknown_key_is_an_error() {
        let err = Hooks::parse("[hooks]\nsesion_start = \"oops\"\n");
        assert!(err.is_err(), "a misspelled key must not pass silently");
    }

    #[test]
    fn whitespace_only_commands_do_not_count() {
        let h = Hooks::parse("[hooks]\nsession_start = \"   \"\n").unwrap();
        assert!(!h.any_configured());
        assert!(
            h.command_for(&Event::SessionStart {
                state: "focus".into(),
                activity: "a".into()
            })
            .is_none()
        );
    }

    #[test]
    fn env_carries_the_event_name_and_its_data() {
        let e = Event::TimerPhase {
            phase: "break".into(),
            remaining: 300,
        };
        let env = e.env();
        assert_eq!(env["BFM_EVENT"], "timer_phase");
        assert_eq!(env["BFM_TIMER_PHASE"], "break");
        assert_eq!(env["BFM_TIMER_REMAINING"], "300");
    }

    #[test]
    fn each_event_exposes_its_own_variables() {
        let s = Event::SessionStart {
            state: "focus".into(),
            activity: "deep".into(),
        }
        .env();
        assert_eq!(s["BFM_STATE"], "focus");
        assert_eq!(s["BFM_ACTIVITY"], "deep");

        let t = Event::TrackStart {
            track: "Alcatraz".into(),
            length: 1800,
        }
        .env();
        assert_eq!(t["BFM_TRACK"], "Alcatraz");
        assert_eq!(t["BFM_LENGTH"], "1800");

        let e = Event::SessionEnd {
            reason: "timer-elapsed".into(),
        }
        .env();
        assert_eq!(e["BFM_REASON"], "timer-elapsed");
    }

    /// The real contract: a hook actually runs and sees its environment.
    #[test]
    fn a_configured_hook_runs_with_the_event_in_its_environment() {
        let out = std::env::temp_dir().join(format!("bfm-hook-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&out);

        let h = Hooks {
            track_start: Some(format!("printf '%s' \"$BFM_TRACK\" > {}", out.display())),
            ..Default::default()
        };
        h.fire(&Event::TrackStart {
            track: "Alcatraz".into(),
            length: 1800,
        });

        // Fire-and-forget, so poll briefly rather than assuming it has run.
        let mut seen = String::new();
        for _ in 0..50 {
            if let Ok(s) = std::fs::read_to_string(&out)
                && !s.is_empty()
            {
                seen = s;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&out);
        assert_eq!(seen, "Alcatraz");
    }

    #[test]
    fn firing_an_unconfigured_event_does_nothing() {
        Hooks::default().fire(&Event::SessionEnd {
            reason: "stopped".into(),
        });
    }
}
