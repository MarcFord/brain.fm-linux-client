//! What the widgets should show.
//!
//! Pure functions of state, deliberately separated from the widget tree.
//! GTK widgets need a display server to exercise, so anything decided inside
//! a `view!` block is effectively untestable; anything decided here is not.
//!
//! The split is the same one that paid off in `bfm-player`: keep the
//! decisions in plain functions and let the adapter be declarative wiring.
//! The difference is that this time the untested adapter is small enough
//! that very little judgement is left in it.

use bfm_ui_core::{IDLE_LABEL, Playback, SessionMode, Timer, Track, format_duration};

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

pub fn play_icon(playback: Playback) -> &'static str {
    if playback.toggles_to_play() {
        "media-playback-start-symbolic"
    } else {
        "media-playback-pause-symbolic"
    }
}

/// Adwaita ships no outline heart, so the unfavourited state uses the
/// emblem and relies on colour. Both names are verified present in the
/// theme; a missing one renders as a broken image.
pub fn favorite_icon(is_favorite: bool) -> &'static str {
    if is_favorite {
        "heart-filled-symbolic"
    } else {
        "emblem-favorite-symbolic"
    }
}

pub fn favorite_classes(is_favorite: bool) -> &'static [&'static str] {
    if is_favorite {
        &["flat", "circular", "bfm-faved"]
    } else {
        &["flat", "circular"]
    }
}

pub fn favorite_tooltip(is_favorite: bool) -> &'static str {
    if is_favorite {
        "Remove from favourites"
    } else {
        "Add to favourites"
    }
}

/// Only dim when there is an image behind the controls; dimming a plain
/// background just makes it muddy.
pub fn scrim_classes(has_art: bool) -> &'static [&'static str] {
    if has_art { &["bfm-scrim"] } else { &[] }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn track(name: &str) -> Track {
        Track {
            name: name.into(),
            ..Default::default()
        }
    }

    // ---- page selection ----

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

    // ---- labels ----

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

    // ---- controls ----

    #[test]
    fn the_transport_button_offers_the_opposite_of_what_is_happening() {
        assert_eq!(
            play_icon(Playback::Playing),
            "media-playback-pause-symbolic"
        );
        assert_eq!(play_icon(Playback::Paused), "media-playback-start-symbolic");
        assert_eq!(play_icon(Playback::Idle), "media-playback-start-symbolic");
        // Buffering is on its way to playing, so offer pause.
        assert_eq!(
            play_icon(Playback::Buffering),
            "media-playback-pause-symbolic"
        );
    }

    #[test]
    fn the_heart_changes_icon_class_and_tooltip_together() {
        assert_ne!(favorite_icon(true), favorite_icon(false));
        assert!(favorite_classes(true).contains(&"bfm-faved"));
        assert!(!favorite_classes(false).contains(&"bfm-faved"));
        assert!(favorite_tooltip(true).starts_with("Remove"));
        assert!(favorite_tooltip(false).starts_with("Add"));
    }

    /// Every icon name must exist in the theme. Adwaita has no outline
    /// heart and no thumbs-down, which is how this went wrong once already.
    #[test]
    fn icon_names_are_ones_the_theme_actually_ships() {
        for name in [
            favorite_icon(true),
            favorite_icon(false),
            play_icon(Playback::Playing),
            play_icon(Playback::Idle),
        ] {
            assert!(name.ends_with("-symbolic"), "{name} is not a symbolic icon");
            assert!(
                !name.contains("outline"),
                "no outline variants exist in Adwaita"
            );
            assert!(!name.contains("thumb"), "Adwaita ships no thumbs icons");
        }
    }

    #[test]
    fn the_scrim_appears_only_over_artwork() {
        assert!(scrim_classes(true).contains(&"bfm-scrim"));
        assert!(scrim_classes(false).is_empty());
    }

    // ---- timer ----

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

    // ---- banner ----

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
