//! How long a session runs.
//!
//! The daemon takes a single spec string, which is right for a wire format
//! but wrong for a UI: a user picks a *mode* and then its parameters. This
//! is the translation, kept here rather than in a front-end so GTK and Qt
//! cannot disagree about what "25/5 four times" means.

use std::fmt;

/// The three shapes a session can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionMode {
    /// Plays until stopped. A session arrives with ~40 tracks, so this is
    /// genuinely open-ended rather than a long fixed run.
    #[default]
    Infinite,
    /// A single countdown, then the session ends.
    Timer { minutes: u32 },
    /// Alternating work and break periods.
    Intervals { work: u32, rest: u32, cycles: u32 },
}

impl SessionMode {
    /// Render to the daemon's timer spec. Infinite is the empty string,
    /// which is how `SetTimer` clears a timer.
    pub fn to_spec(self) -> String {
        match self {
            Self::Infinite => String::new(),
            Self::Timer { minutes } => format!("{minutes}m"),
            Self::Intervals { work, rest, cycles } if cycles > 1 => {
                format!("{work}m/{rest}m x{cycles}")
            }
            Self::Intervals { work, rest, .. } => format!("{work}m/{rest}m"),
        }
    }

    /// Recover a mode from a spec, so the UI can reflect a timer set by the
    /// CLI or a plugin rather than overwriting it.
    ///
    /// Only the minute-granularity forms this type can represent round-trip;
    /// anything else (`90s`, `1h30m`) is reported as `None` so the caller can
    /// show it as-is instead of silently rounding someone's timer.
    pub fn from_spec(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Some(Self::Infinite);
        }

        let (body, cycles) = match spec.split_once('x') {
            Some((b, c)) => (b.trim(), c.trim().parse().ok()?),
            None => (spec, 1u32),
        };
        if cycles == 0 {
            return None;
        }

        let minutes = |s: &str| -> Option<u32> { s.trim().strip_suffix('m')?.parse().ok() };

        match body.split_once('/') {
            Some((w, r)) => Some(Self::Intervals {
                work: minutes(w)?,
                rest: minutes(r)?,
                cycles,
            }),
            None if cycles == 1 => Some(Self::Timer {
                minutes: minutes(body)?,
            }),
            None => None,
        }
    }

    /// Total length, for showing the user what they just chose.
    pub fn total_minutes(self) -> Option<u32> {
        match self {
            Self::Infinite => None,
            Self::Timer { minutes } => Some(minutes),
            Self::Intervals { work, rest, cycles } => {
                Some(work * cycles + rest * cycles.saturating_sub(1))
            }
        }
    }
}

impl fmt::Display for SessionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Infinite => f.write_str("Infinite"),
            Self::Timer { minutes } => write!(f, "{minutes} minutes"),
            Self::Intervals { work, rest, cycles } if cycles > 1 => {
                write!(f, "{work}/{rest} × {cycles}")
            }
            Self::Intervals { work, rest, .. } => write!(f, "{work}/{rest}"),
        }
    }
}

/// A duration the service itself suggests for a mental state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLength {
    pub seconds: u32,
    pub label: String,
}

impl SessionLength {
    pub fn minutes(&self) -> u32 {
        self.seconds / 60
    }
}

/// Fallback durations, used before the catalog has loaded or if the daemon
/// is too old to offer them.
///
/// Deliberately state-specific: a sensible sleep session is nine hours and a
/// sensible focus session is ninety minutes, so one list for everything would
/// be useless for both.
pub fn fallback_lengths(state: &str) -> Vec<u32> {
    match state {
        "sleep" => vec![420, 480, 540],
        "relax" | "meditate" => vec![15, 30, 45, 60],
        _ => vec![30, 60, 90, 120],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infinite_is_the_empty_spec() {
        assert_eq!(SessionMode::Infinite.to_spec(), "");
        assert_eq!(SessionMode::from_spec(""), Some(SessionMode::Infinite));
        assert_eq!(SessionMode::from_spec("   "), Some(SessionMode::Infinite));
    }

    #[test]
    fn a_plain_timer_round_trips() {
        let m = SessionMode::Timer { minutes: 50 };
        assert_eq!(m.to_spec(), "50m");
        assert_eq!(SessionMode::from_spec("50m"), Some(m));
    }

    #[test]
    fn intervals_round_trip_with_and_without_cycles() {
        let four = SessionMode::Intervals {
            work: 25,
            rest: 5,
            cycles: 4,
        };
        assert_eq!(four.to_spec(), "25m/5m x4");
        assert_eq!(SessionMode::from_spec("25m/5m x4"), Some(four));

        let once = SessionMode::Intervals {
            work: 25,
            rest: 5,
            cycles: 1,
        };
        assert_eq!(once.to_spec(), "25m/5m", "a single cycle needs no suffix");
        assert_eq!(SessionMode::from_spec("25m/5m"), Some(once));
    }

    /// The daemon accepts specs this type cannot express. Reporting None lets
    /// the UI show them verbatim rather than rounding someone's timer.
    #[test]
    fn specs_outside_this_model_are_not_silently_rounded() {
        for s in ["90s", "1h30m", "25m/5m x0", "nonsense", "25m/"] {
            assert_eq!(
                SessionMode::from_spec(s),
                None,
                "{s:?} should not round-trip"
            );
        }
    }

    #[test]
    fn totals_account_for_the_gaps_between_cycles() {
        assert_eq!(SessionMode::Infinite.total_minutes(), None);
        assert_eq!(SessionMode::Timer { minutes: 50 }.total_minutes(), Some(50));
        // Four work periods, three breaks between them.
        assert_eq!(
            SessionMode::Intervals {
                work: 25,
                rest: 5,
                cycles: 4
            }
            .total_minutes(),
            Some(115)
        );
    }

    /// Whatever the UI renders must be something the daemon's own parser
    /// accepts; these are the exact forms documented for `SetTimer`.
    #[test]
    fn generated_specs_match_the_documented_grammar() {
        let specs: Vec<String> = [
            SessionMode::Infinite,
            SessionMode::Timer { minutes: 90 },
            SessionMode::Intervals {
                work: 50,
                rest: 10,
                cycles: 2,
            },
        ]
        .into_iter()
        .map(SessionMode::to_spec)
        .collect();
        assert_eq!(specs, vec!["", "90m", "50m/10m x2"]);
    }

    #[test]
    fn fallback_lengths_differ_by_state() {
        assert_eq!(fallback_lengths("sleep").first(), Some(&420));
        assert_eq!(fallback_lengths("focus").first(), Some(&30));
        assert_ne!(fallback_lengths("sleep"), fallback_lengths("focus"));
    }

    #[test]
    fn display_is_human_readable() {
        assert_eq!(SessionMode::Infinite.to_string(), "Infinite");
        assert_eq!(SessionMode::Timer { minutes: 90 }.to_string(), "90 minutes");
        assert_eq!(
            SessionMode::Intervals {
                work: 25,
                rest: 5,
                cycles: 4
            }
            .to_string(),
            "25/5 × 4"
        );
    }
}
