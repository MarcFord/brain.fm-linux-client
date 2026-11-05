//! The state a front-end renders.
//!
//! Deliberately free of any toolkit type, so GTK and Qt render the same model
//! rather than each re-deriving it from raw D-Bus variants. Everything here is
//! plain data and pure functions, which is also what makes it testable without
//! a bus.

use std::collections::HashMap;

use zbus::OwnedVariant;

/// Mirrors `dev.marcford.BrainFM1`'s playback state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Playback {
    #[default]
    Idle,
    Buffering,
    Playing,
    Paused,
}

impl Playback {
    pub fn from_wire(s: &str) -> Self {
        match s {
            "playing" => Self::Playing,
            "paused" => Self::Paused,
            "buffering" => Self::Buffering,
            _ => Self::Idle,
        }
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Playing | Self::Buffering)
    }

    /// What a play/pause control should do next.
    pub fn toggles_to_play(self) -> bool {
        !matches!(self, Self::Playing | Self::Buffering)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub length: u64,
    pub neural_effect: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Timer {
    /// `none` | `work` | `break` | `done`
    pub phase: String,
    pub remaining: u64,
    pub cycle: u32,
    pub cycles: u32,
    pub total: u64,
}

impl Timer {
    pub fn is_running(&self) -> bool {
        !matches!(self.phase.as_str(), "" | "none")
    }

    /// Progress through the whole timer, `0.0..=1.0`.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        let done = self.total.saturating_sub(self.remaining) as f64;
        (done / self.total as f64).clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct State {
    pub connected: bool,
    pub authenticated: bool,
    pub playback: Playback,
    pub mental_state: String,
    pub activity: String,
    pub track: Option<Track>,
    pub position: u64,
    pub timer: Timer,
    pub volume: f64,
}

impl State {
    /// Progress through the current track, `0.0..=1.0`.
    pub fn track_fraction(&self) -> f64 {
        match &self.track {
            Some(t) if t.length > 0 => (self.position as f64 / t.length as f64).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }
}

/// What a front-end should show when nothing is playing.
pub const IDLE_LABEL: &str = "Nothing playing";

/// `h:mm:ss` past an hour, `m:ss` below it.
///
/// Brain.fm tracks run 30 minutes and sleep sessions run nine hours, so both
/// branches are reachable in normal use.
pub fn format_duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

// ---- variant extraction ----

pub fn var_str(m: &HashMap<String, OwnedVariant>, k: &str) -> Option<String> {
    m.get(k).and_then(|v| String::try_from(v.clone()).ok())
}

pub fn var_u64(m: &HashMap<String, OwnedVariant>, k: &str) -> Option<u64> {
    let v = m.get(k)?;
    // The daemon sends u64 for durations and u32 for counts; accept either so
    // a widened field upstream does not blank the UI.
    u64::try_from(v.clone())
        .ok()
        .or_else(|| u32::try_from(v.clone()).ok().map(u64::from))
}

pub fn var_u32(m: &HashMap<String, OwnedVariant>, k: &str) -> Option<u32> {
    let v = m.get(k)?;
    u32::try_from(v.clone()).ok().or_else(|| {
        u64::try_from(v.clone())
            .ok()
            .and_then(|n| u32::try_from(n).ok())
    })
}

pub fn var_f64(m: &HashMap<String, OwnedVariant>, k: &str) -> Option<f64> {
    m.get(k).and_then(|v| f64::try_from(v.clone()).ok())
}

pub fn track_from(m: &HashMap<String, OwnedVariant>) -> Option<Track> {
    let name = var_str(m, "name")?;
    Some(Track {
        id: var_str(m, "id").unwrap_or_default(),
        name,
        length: var_u64(m, "length").unwrap_or(0),
        neural_effect: var_f64(m, "neural_effect").unwrap_or(0.0),
    })
}

pub fn timer_from(m: &HashMap<String, OwnedVariant>) -> Timer {
    Timer {
        phase: var_str(m, "phase").unwrap_or_else(|| "none".into()),
        remaining: var_u64(m, "remaining").unwrap_or(0),
        cycle: var_u32(m, "cycle").unwrap_or(0),
        cycles: var_u32(m, "cycles").unwrap_or(0),
        total: var_u64(m, "total").unwrap_or(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::Variant;

    fn map(pairs: &[(&str, Variant<'static>)]) -> HashMap<String, OwnedVariant> {
        pairs
            .iter()
            .filter_map(|(k, v)| {
                OwnedVariant::try_from(v.clone())
                    .ok()
                    .map(|o| (k.to_string(), o))
            })
            .collect()
    }

    #[test]
    fn playback_maps_from_the_wire_strings() {
        assert_eq!(Playback::from_wire("playing"), Playback::Playing);
        assert_eq!(Playback::from_wire("buffering"), Playback::Buffering);
        assert_eq!(Playback::from_wire("paused"), Playback::Paused);
        assert_eq!(Playback::from_wire("idle"), Playback::Idle);
        // An unknown value must degrade to idle, not panic the UI.
        assert_eq!(Playback::from_wire("something-new"), Playback::Idle);
    }

    /// Buffering looks idle but is on its way to playing, so the button must
    /// offer pause rather than play.
    #[test]
    fn buffering_counts_as_active() {
        assert!(Playback::Buffering.is_active());
        assert!(!Playback::Buffering.toggles_to_play());
        assert!(Playback::Idle.toggles_to_play());
        assert!(Playback::Paused.toggles_to_play());
    }

    #[test]
    fn durations_grow_an_hour_field_when_needed() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(59), "0:59");
        assert_eq!(format_duration(1800), "30:00");
        // A sleep session.
        assert_eq!(format_duration(32400), "9:00:00");
        assert_eq!(format_duration(3661), "1:01:01");
    }

    #[test]
    fn track_parses_from_the_property_map() {
        let m = map(&[
            ("id", Variant::from("trk")),
            ("name", Variant::from("Topography")),
            ("length", Variant::from(1800u64)),
            ("neural_effect", Variant::from(0.06f64)),
        ]);
        let t = track_from(&m).unwrap();
        assert_eq!(t.name, "Topography");
        assert_eq!(t.length, 1800);
    }

    #[test]
    fn an_empty_track_map_is_no_track() {
        assert!(track_from(&HashMap::new()).is_none());
    }

    /// The daemon sends counts as u32 and durations as u64. Accept both
    /// everywhere so a widened field upstream does not blank the display.
    #[test]
    fn numeric_extraction_accepts_either_width() {
        let m = map(&[
            ("remaining", Variant::from(300u64)),
            ("cycle", Variant::from(2u32)),
        ]);
        assert_eq!(var_u64(&m, "remaining"), Some(300));
        assert_eq!(var_u32(&m, "remaining"), Some(300));
        assert_eq!(var_u64(&m, "cycle"), Some(2));
        assert_eq!(var_u32(&m, "cycle"), Some(2));
    }

    #[test]
    fn timer_defaults_to_not_running() {
        let t = timer_from(&HashMap::new());
        assert_eq!(t.phase, "none");
        assert!(!t.is_running());
        assert_eq!(t.fraction(), 0.0);
    }

    #[test]
    fn timer_fraction_reports_elapsed_not_remaining() {
        let t = Timer {
            phase: "work".into(),
            remaining: 300,
            total: 1200,
            ..Default::default()
        };
        assert!(t.is_running());
        assert!((t.fraction() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn a_zero_length_track_does_not_divide_by_zero() {
        let s = State {
            track: Some(Track {
                length: 0,
                ..Default::default()
            }),
            position: 10,
            ..Default::default()
        };
        assert_eq!(s.track_fraction(), 0.0);
    }

    #[test]
    fn track_fraction_is_clamped() {
        let s = State {
            track: Some(Track {
                length: 100,
                ..Default::default()
            }),
            position: 999,
            ..Default::default()
        };
        assert_eq!(s.track_fraction(), 1.0);
    }
}
