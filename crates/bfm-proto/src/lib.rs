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

/// Typed client for `dev.marcford.BrainFM1`.
///
/// This is the supported way to drive `bfmd` from Rust. Shell clients can use
/// `busctl`/`gdbus` against the same interface; the D-Bus surface is the
/// contract, and this trait is a convenience over it.
///
/// ```no_run
/// # async fn demo() -> zbus::Result<()> {
/// let conn = zbus::Connection::session().await?;
/// let bfm = bfm_proto::BrainFM1Proxy::new(&conn).await?;
/// bfm.start_session("focus", "", &[], "high", "50m").await?;
/// # Ok(())
/// # }
/// ```
#[zbus::proxy(
    interface = "dev.marcford.BrainFM1",
    default_service = "dev.marcford.bfm",
    default_path = "/dev/marcford/bfm"
)]
pub trait BrainFM1 {
    /// Start playing. Empty `activity` means the state's default; empty
    /// `genres` means no filter. Returns the session id.
    fn start_session(
        &self,
        state: &str,
        activity: &str,
        genres: &[&str],
        effort: &str,
        timer: &str,
    ) -> zbus::Result<String>;

    fn stop_session(&self) -> zbus::Result<()>;
    fn pause(&self) -> zbus::Result<()>;
    fn resume(&self) -> zbus::Result<()>;
    fn skip_track(&self) -> zbus::Result<()>;

    /// Set or replace the timer: `"50m"`, `"25m/5m"`, `"25m/5m x4"`, or `""`
    /// to clear.
    fn set_timer(&self, spec: &str) -> zbus::Result<()>;

    fn list_mental_states(&self) -> zbus::Result<Vec<String>>;
    fn list_activities(&self, state: &str) -> zbus::Result<Vec<(String, String)>>;
    fn list_genres(&self, state: &str) -> zbus::Result<Vec<String>>;

    /// One-shot snapshot, for scripts that do not want to track properties.
    fn get_status(&self) -> zbus::Result<std::collections::HashMap<String, zbus::OwnedVariant>>;

    /// Increments on every additive change within `BrainFM1`. Feature-detect
    /// against this rather than sniffing for methods.
    #[zbus(property)]
    fn interface_version(&self) -> zbus::Result<u32>;

    /// `idle` | `buffering` | `playing` | `paused`
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn mental_state(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn activity(&self) -> zbus::Result<String>;

    /// Current track: `id`, `name`, `length`, `neural_effect`.
    #[zbus(property)]
    fn track(&self) -> zbus::Result<std::collections::HashMap<String, zbus::OwnedVariant>>;

    /// Seconds into the current track.
    #[zbus(property)]
    fn position(&self) -> zbus::Result<u64>;

    /// Timer state: `phase`, `remaining`, `cycle`, `cycles`.
    #[zbus(property)]
    fn timer(&self) -> zbus::Result<std::collections::HashMap<String, zbus::OwnedVariant>>;

    #[zbus(property)]
    fn authenticated(&self) -> zbus::Result<bool>;

    #[zbus(property)]
    fn volume(&self) -> zbus::Result<f64>;

    #[zbus(property)]
    fn set_volume(&self, volume: f64) -> zbus::Result<()>;

    #[zbus(signal)]
    fn session_started(&self, session_id: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn session_ended(&self, reason: &str) -> zbus::Result<()>;

    /// A new track began.
    ///
    /// Named `TrackStarted`, not `TrackChanged`: every property automatically
    /// gets a `<Name>Changed` notification, so a `TrackChanged` signal would
    /// collide with the `Track` property's own. Picking a distinct verb keeps
    /// both available to clients.
    #[zbus(signal)]
    fn track_started(&self, name: &str, length: u64) -> zbus::Result<()>;

    #[zbus(signal)]
    fn timer_phase_changed(&self, phase: &str, remaining: u64) -> zbus::Result<()>;

    #[zbus(signal)]
    fn auth_state_changed(&self, authenticated: bool) -> zbus::Result<()>;

    /// `code` is one of [`error_code`]; those strings are stable.
    #[zbus(signal)]
    fn error(&self, code: &str, message: &str) -> zbus::Result<()>;
}
