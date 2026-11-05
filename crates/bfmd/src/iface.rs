//! The `dev.marcford.BrainFM1` interface.
//!
//! This is the public API — the thing a status bar or a desktop plugin binds
//! to — so it speaks only about sessions, tracks and timers. No keystrokes,
//! no UI concepts, nothing that leaks how the daemon happens to be built.

use std::collections::HashMap;

use bfm_proto::{INTERFACE_VERSION, error_code};
use zbus::{Variant, interface, object_server::SignalEmitter};

use crate::timer::{Phase, Timer, TimerSpec};

/// Runtime state behind the interface.
pub struct BrainFm {
    pub playback: &'static str,
    pub mental_state: String,
    pub activity: String,
    pub volume: f64,
    pub authenticated: bool,
    pub timer: Option<Timer>,
}

impl Default for BrainFm {
    fn default() -> Self {
        Self {
            playback: "idle",
            mental_state: String::new(),
            activity: String::new(),
            volume: 1.0,
            authenticated: false,
            timer: None,
        }
    }
}

/// Until credentials are wired up (milestone 4), anything that would hit the
/// API says so plainly instead of failing obscurely.
fn needs_login() -> zbus::fdo::Error {
    zbus::fdo::Error::AccessDenied(format!(
        "{}: no stored session. Run `bfm login`.",
        error_code::NEEDS_LOGIN
    ))
}

#[interface(interface = "dev.marcford.BrainFM1")]
impl BrainFm {
    async fn start_session(
        &mut self,
        _state: &str,
        _activity: &str,
        _genres: Vec<String>,
        _effort: &str,
        _timer: &str,
    ) -> zbus::fdo::Result<String> {
        Err(needs_login())
    }

    async fn stop_session(&mut self) -> zbus::fdo::Result<()> {
        self.playback = "idle";
        self.timer = None;
        Ok(())
    }

    async fn pause(&mut self) -> zbus::fdo::Result<()> {
        if self.playback == "playing" {
            self.playback = "paused";
        }
        Ok(())
    }

    async fn resume(&mut self) -> zbus::fdo::Result<()> {
        if self.playback == "paused" {
            self.playback = "playing";
        }
        Ok(())
    }

    async fn skip_track(&mut self) -> zbus::fdo::Result<()> {
        Err(needs_login())
    }

    /// `"50m"`, `"25m/5m"`, `"25m/5m x4"`, or `""` to clear.
    ///
    /// Works without a session: a timer is useful on its own, and making it
    /// independent means a plugin can drive one without touching playback.
    async fn set_timer(&mut self, spec: &str) -> zbus::fdo::Result<()> {
        if spec.trim().is_empty() {
            self.timer = None;
            return Ok(());
        }
        let parsed =
            TimerSpec::parse(spec).map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
        self.timer = Some(Timer::start(parsed));
        Ok(())
    }

    async fn list_mental_states(&self) -> zbus::fdo::Result<Vec<String>> {
        if !self.authenticated {
            return Err(needs_login());
        }
        Ok(Vec::new())
    }

    async fn list_activities(&self, _state: &str) -> zbus::fdo::Result<Vec<(String, String)>> {
        Err(needs_login())
    }

    async fn list_genres(&self, _state: &str) -> zbus::fdo::Result<Vec<String>> {
        Err(needs_login())
    }

    /// One-shot snapshot, for scripts that would rather not track properties.
    async fn get_status(&self) -> zbus::fdo::Result<HashMap<String, zbus::OwnedVariant>> {
        let mut m: HashMap<String, zbus::OwnedVariant> = HashMap::new();
        let mut put = |k: &str, v: Variant<'_>| {
            if let Ok(owned) = zbus::OwnedVariant::try_from(v) {
                m.insert(k.to_owned(), owned);
            }
        };
        put("state", Variant::from(self.playback));
        put("mental_state", Variant::from(self.mental_state.clone()));
        put("activity", Variant::from(self.activity.clone()));
        put("authenticated", Variant::from(self.authenticated));
        put("volume", Variant::from(self.volume));
        put("timer_phase", Variant::from(self.timer_phase()));
        put("timer_remaining", Variant::from(self.timer_remaining()));
        Ok(m)
    }

    // ---- properties ----

    #[zbus(property)]
    async fn interface_version(&self) -> u32 {
        INTERFACE_VERSION
    }

    #[zbus(property)]
    async fn state(&self) -> String {
        self.playback.to_owned()
    }

    #[zbus(property)]
    async fn mental_state(&self) -> String {
        self.mental_state.clone()
    }

    #[zbus(property)]
    async fn activity(&self) -> String {
        self.activity.clone()
    }

    #[zbus(property)]
    async fn position(&self) -> u64 {
        0
    }

    #[zbus(property)]
    async fn authenticated(&self) -> bool {
        self.authenticated
    }

    #[zbus(property)]
    async fn volume(&self) -> f64 {
        self.volume
    }

    #[zbus(property)]
    async fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    #[zbus(property)]
    async fn timer(&self) -> HashMap<String, zbus::OwnedVariant> {
        let mut m: HashMap<String, zbus::OwnedVariant> = HashMap::new();
        let mut put = |k: &str, v: Variant<'_>| {
            if let Ok(owned) = zbus::OwnedVariant::try_from(v) {
                m.insert(k.to_owned(), owned);
            }
        };
        put("phase", Variant::from(self.timer_phase()));
        put("remaining", Variant::from(self.timer_remaining()));
        put(
            "cycle",
            Variant::from(self.timer.as_ref().map_or(0, |t| t.cycle())),
        );
        put(
            "cycles",
            Variant::from(self.timer.as_ref().map_or(0, |t| t.cycles())),
        );
        // Total lets a client render progress without re-deriving it.
        put(
            "total",
            Variant::from(self.timer.as_ref().map_or(0, |t| t.total().as_secs())),
        );
        m
    }

    // ---- signals ----

    #[zbus(signal)]
    pub async fn session_started(emitter: &SignalEmitter<'_>, session_id: &str)
    -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn session_ended(emitter: &SignalEmitter<'_>, reason: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn track_started(
        emitter: &SignalEmitter<'_>,
        name: &str,
        length: u64,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn timer_phase_changed(
        emitter: &SignalEmitter<'_>,
        phase: &str,
        remaining: u64,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn auth_state_changed(
        emitter: &SignalEmitter<'_>,
        authenticated: bool,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn error(emitter: &SignalEmitter<'_>, code: &str, message: &str) -> zbus::Result<()>;
}

impl BrainFm {
    pub fn timer_phase(&self) -> String {
        self.timer
            .as_ref()
            .map_or("none", |t| t.phase().as_str())
            .to_owned()
    }

    pub fn timer_remaining(&self) -> u64 {
        self.timer.as_ref().map_or(0, |t| t.remaining().as_secs())
    }

    /// Advance the timer. Returns the new phase when it changed, so the caller
    /// emits `TimerPhaseChanged` exactly on transitions and not every tick.
    pub fn tick_timer(&mut self, delta: std::time::Duration) -> Option<Phase> {
        self.timer.as_mut()?.advance(delta)
    }
}
