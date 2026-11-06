//! What a front-end should display.
//!
//! Shared by every front-end so GTK and KDE cannot drift into saying
//! different things about the same state. Toolkit-specific choices — icon
//! names, style classes — stay with their toolkit; the words do not.
//!
//! Pure functions, because widget trees need a display server to exercise
//! and anything decided inside one is effectively untestable.

use crate::{
    mode::SessionMode,
    state::{IDLE_LABEL, Playback, Timer, Track, format_duration},
};

/// Which page of the stack to show.
pub fn page(connected: bool, authenticated: bool, playback: Playback) -> &'static str {
    if !connected {
        "connecting"
    } else if !authenticated {
        "signin"
    } else if playback == Playback::Idle {
        "browse"
    } else {
        "player"
    }
}

pub fn track_name(track: Option<&Track>) -> String {
    track
        .map(|t| t.name.clone())
        .unwrap_or_else(|| IDLE_LABEL.to_owned())
}

/// The line under the title. Buffering takes priority: it explains the
/// silence, which is the question a user actually has at that moment.
pub fn track_detail(track: Option<&Track>, playback: Playback) -> String {
    if playback == Playback::Buffering {
        return "Buffering…".to_owned();
    }
    match track {
        Some(t) if t.neural_effect > 0.0 => {
            format!("Neural effect {:.0}%", t.neural_effect * 100.0)
        }
        _ => String::new(),
    }
}

pub fn position_label(position: u64, length: u64) -> String {
    format!(
        "{} / {}",
        format_duration(position),
        format_duration(length)
    )
}

/// What the timer row reads. With no timer running it describes the mode
/// that *will* apply, so the row is never blank.
pub fn timer_label(timer: &Timer, mode: SessionMode) -> String {
    if !timer.is_running() {
        return format!("{mode} · running until stopped");
    }
    let phase = title_case(&timer.phase);
    if timer.cycles > 1 {
        format!(
            "{phase} · {} left · {}/{}",
            format_duration(timer.remaining),
            timer.cycle,
            timer.cycles
        )
    } else {
        format!("{phase} · {} left", format_duration(timer.remaining))
    }
}

/// What the banner shows, and whether it is a problem.
///
/// Priority matters: a stale daemon explains away whole features, so it
/// outranks a transient failure, which in turn outranks a confirmation.
pub fn banner(
    stale: Option<String>,
    error: Option<&str>,
    notice: Option<&str>,
) -> Option<(bool, String)> {
    if let Some(msg) = stale {
        return Some((true, msg));
    }
    if let Some(e) = error {
        return Some((true, e.to_owned()));
    }
    notice.map(|n| (false, n.to_owned()))
}

pub fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn favorite_tooltip(is_favorite: bool) -> &'static str {
    if is_favorite {
        "Remove from favourites"
    } else {
        "Add to favourites"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(name: &str) -> Track {
        Track {
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn pages_follow_connection_then_auth_then_playback() {
        assert_eq!(page(false, false, Playback::Idle), "connecting");
        assert_eq!(page(false, true, Playback::Playing), "connecting");
        assert_eq!(page(true, false, Playback::Idle), "signin");
        assert_eq!(page(true, true, Playback::Idle), "browse");
        assert_eq!(page(true, true, Playback::Playing), "player");
        assert_eq!(page(true, true, Playback::Paused), "player");
    }

    /// Buffering must land on the player, not the browse page — otherwise
    /// starting a session visibly bounces back to the list.
    #[test]
    fn buffering_shows_the_player() {
        assert_eq!(page(true, true, Playback::Buffering), "player");
    }

    #[test]
    fn a_missing_track_still_has_a_title() {
        assert_eq!(track_name(None), IDLE_LABEL);
        assert_eq!(track_name(Some(&track("Topography"))), "Topography");
    }

    #[test]
    fn buffering_explains_itself_over_the_effect_level() {
        let t = Track {
            neural_effect: 0.42,
            ..track("x")
        };
        assert_eq!(track_detail(Some(&t), Playback::Buffering), "Buffering…");
        assert_eq!(
            track_detail(Some(&t), Playback::Playing),
            "Neural effect 42%"
        );
    }

    #[test]
    fn an_unknown_effect_level_shows_nothing_rather_than_zero_percent() {
        let t = track("x");
        assert_eq!(track_detail(Some(&t), Playback::Playing), "");
    }

    #[test]
    fn position_reads_as_elapsed_over_total() {
        assert_eq!(position_label(65, 1800), "1:05 / 30:00");
        assert_eq!(position_label(0, 0), "0:00 / 0:00");
    }

    #[test]
    fn an_idle_timer_describes_the_mode_rather_than_going_blank() {
        let label = timer_label(&Timer::default(), SessionMode::Infinite);
        assert!(label.contains("Infinite"), "{label}");
    }

    #[test]
    fn a_running_timer_shows_phase_and_remaining() {
        let t = Timer {
            phase: "work".into(),
            remaining: 1500,
            cycle: 1,
            cycles: 1,
            total: 1500,
        };
        assert_eq!(timer_label(&t, SessionMode::Infinite), "Work · 25:00 left");
    }

    #[test]
    fn multiple_cycles_show_progress_through_them() {
        let t = Timer {
            phase: "break".into(),
            remaining: 300,
            cycle: 2,
            cycles: 4,
            total: 7200,
        };
        assert_eq!(
            timer_label(&t, SessionMode::Infinite),
            "Break · 5:00 left · 2/4"
        );
    }

    #[test]
    fn a_stale_daemon_outranks_everything_else() {
        let b = banner(Some("old daemon".into()), Some("boom"), Some("copied"));
        assert_eq!(b, Some((true, "old daemon".to_owned())));
    }

    #[test]
    fn an_error_outranks_a_confirmation() {
        assert_eq!(
            banner(None, Some("boom"), Some("copied")),
            Some((true, "boom".into()))
        );
    }

    #[test]
    fn a_confirmation_is_not_shown_as_an_error() {
        assert_eq!(
            banner(None, None, Some("copied")),
            Some((false, "copied".into()))
        );
    }

    #[test]
    fn nothing_to_say_shows_no_banner() {
        assert_eq!(banner(None, None, None), None);
    }

    #[test]
    fn title_case_handles_empty_and_unicode() {
        assert_eq!(title_case(""), "");
        assert_eq!(title_case("focus"), "Focus");
        assert_eq!(title_case("é"), "É");
    }
}
