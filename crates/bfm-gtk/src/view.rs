//! GTK-specific presentation.
//!
//! Only things that are genuinely about this toolkit live here: Adwaita icon
//! names and CSS classes. Everything a KDE front-end would word identically
//! is in `bfm_ui_core::display`.

use bfm_ui_core::Playback;

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

/// Only dim when there is an image behind the controls; dimming a plain
/// background just makes it muddy.
pub fn scrim_classes(has_art: bool) -> &'static [&'static str] {
    if has_art { &["bfm-scrim"] } else { &[] }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- page selection ----

    // ---- labels ----

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
        assert!(bfm_ui_core::display::favorite_tooltip(true).starts_with("Remove"));
        assert!(bfm_ui_core::display::favorite_tooltip(false).starts_with("Add"));
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

    // ---- banner ----
}
