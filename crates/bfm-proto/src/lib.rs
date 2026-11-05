//! The public control-plane contract for `bfmd`.
//!
//! This crate *is* the stability promise. Everything a third party needs to
//! drive the daemon — the Omarchy plugin included — lives here, and nothing
//! else in the workspace is public API.
//!
//! # Stability
//!
//! * The interface name carries the major version ([`INTERFACE`]). A breaking
//!   change means `…BrainFM2`, served alongside `BrainFM1` for a deprecation
//!   window.
//! * Within a major version, changes are **additive only**. New methods,
//!   properties and signals may appear; existing signatures never change
//!   meaning.
//! * [`INTERFACE_VERSION`] increments on each additive change, so a client can
//!   feature-detect rather than version-sniff.

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

/// Well-known bus name the daemon owns.
pub const BUS_NAME: &str = "dev.marcford.bfm";
/// Object path the domain interface is served at.
pub const OBJECT_PATH: &str = "/dev/marcford/bfm";
/// The domain interface. Major version is part of the name.
pub const INTERFACE: &str = "dev.marcford.BrainFM1";
/// Bumped on every additive change within `BrainFM1`.
pub const INTERFACE_VERSION: u32 = 1;

/// MPRIS bus name, for `playerctl -p bfm` and media keys.
pub const MPRIS_BUS_NAME: &str = "org.mpris.MediaPlayer2.bfm";

/// Playback state. Stringly-typed on the wire so shell clients stay trivial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
#[serde(rename_all = "lowercase")]
pub enum PlaybackState {
    Idle,
    Buffering,
    Playing,
    Paused,
}

impl PlaybackState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Buffering => "buffering",
            Self::Playing => "playing",
            Self::Paused => "paused",
        }
    }
}

/// Requested neural effect level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
}

/// A timer phase, as reported by `TimerPhaseChanged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[zvariant(signature = "s")]
#[serde(rename_all = "lowercase")]
pub enum TimerPhase {
    Work,
    Break,
    Done,
}

/// Error codes carried by the `Error` signal. Stable strings — clients match
/// on these, so never repurpose one.
pub mod error_code {
    /// Credentials are needed; the daemon cannot renew unattended.
    pub const NEEDS_LOGIN: &str = "needs-login";
    /// The upstream API returned a shape we do not recognise.
    pub const SCHEMA_CHANGED: &str = "schema-changed";
    /// Network or upstream failure.
    pub const UPSTREAM: &str = "upstream";
    /// The audio device could not be opened.
    pub const AUDIO_DEVICE: &str = "audio-device";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These constants are the contract. Changing one silently breaks every
    /// installed plugin, so pin them in a test.
    #[test]
    fn contract_constants_are_pinned() {
        assert_eq!(BUS_NAME, "dev.marcford.bfm");
        assert_eq!(OBJECT_PATH, "/dev/marcford/bfm");
        assert_eq!(INTERFACE, "dev.marcford.BrainFM1");
        assert_eq!(MPRIS_BUS_NAME, "org.mpris.MediaPlayer2.bfm");
    }

    #[test]
    fn interface_name_matches_declared_major() {
        assert!(
            INTERFACE.ends_with("1"),
            "interface name must carry the major version"
        );
    }

    #[test]
    fn playback_states_are_lowercase_on_the_wire() {
        assert_eq!(PlaybackState::Buffering.as_str(), "buffering");
    }
}
