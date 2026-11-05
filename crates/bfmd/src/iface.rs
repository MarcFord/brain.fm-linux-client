//! The `dev.marcford.BrainFM1` interface.
//!
//! This is the public API — the thing a status bar or a desktop plugin binds
//! to — so it speaks only about sessions, tracks and timers. No keystrokes,
//! no UI concepts, nothing that leaks how the daemon happens to be built.

use std::{collections::HashMap, sync::Mutex, time::Duration};

use bfm_api::{ApiError, AuthMethod, EffectLevel, Password, SessionRequest, SessionToken};
use bfm_player::{Command, Event, NullPlayer, Player};
use bfm_proto::{INTERFACE_VERSION, auth_method, error_code};
use zbus::{Variant, interface, object_server::SignalEmitter};

use crate::{
    hooks::{self, Hooks},
    store::{MemoryStore, TokenStore},
    timer::{Phase, Timer, TimerSpec},
};

/// What the client is told about the current track.
#[derive(Debug, Default, Clone)]
pub struct TrackInfo {
    pub id: String,
    pub name: String,
    pub length: u64,
    pub neural_effect: f64,
}

/// Runtime state behind the interface.
pub struct BrainFm {
    pub playback: &'static str,
    pub mental_state: String,
    pub activity: String,
    pub volume: f64,
    pub timer: Option<Timer>,
    pub track: Option<TrackInfo>,
    /// Seconds into the current track, fed by the audio engine.
    pub position: u64,

    api: bfm_api::Client,
    store: Box<dyn TokenStore>,
    hooks: Hooks,
    /// `Mutex` because the audio handle is `Send` but not `Sync`, and zbus
    /// requires the served type to be both.
    player: Mutex<Box<dyn Player>>,
}

impl BrainFm {
    pub fn new(store: Box<dyn TokenStore>, player: Box<dyn Player>) -> Self {
        let mut api = bfm_api::Client::new().expect("build http client");

        // Adopt a stored session at startup. A token past its expiry is still
        // fine: the first call renews it lazily.
        match store.load().and_then(|raw| SessionToken::parse(raw).ok()) {
            Some(t) => {
                tracing::info!(store = store.describe(), "restored a stored session");
                api.set_token(t);
            }
            None => tracing::info!(store = store.describe(), "no stored session"),
        }

        Self {
            playback: "idle",
            mental_state: String::new(),
            activity: String::new(),
            volume: 1.0,
            timer: None,
            track: None,
            position: 0,
            hooks: Hooks::load(),
            api,
            store,
            player: Mutex::new(player),
        }
    }
}

impl Default for BrainFm {
    /// Test-shaped: nothing persisted, no audio device opened.
    fn default() -> Self {
        Self::new(
            Box::new(MemoryStore::default()),
            Box::new(NullPlayer::default()),
        )
    }
}

fn needs_login() -> zbus::fdo::Error {
    zbus::fdo::Error::AccessDenied(format!(
        "{}: no stored session. Run `bfm login`.",
        error_code::NEEDS_LOGIN
    ))
}

/// Map an API failure onto a D-Bus error, preserving the stable code so
/// clients can branch on it without parsing prose.
fn api_error(e: ApiError) -> zbus::fdo::Error {
    if e.needs_login() {
        return zbus::fdo::Error::AccessDenied(format!("{}: {e}", error_code::NEEDS_LOGIN));
    }
    match e {
        ApiError::SchemaChanged { .. } => {
            zbus::fdo::Error::Failed(format!("{}: {e}", error_code::SCHEMA_CHANGED))
        }
        other => zbus::fdo::Error::Failed(format!("{}: {other}", error_code::UPSTREAM)),
    }
}

fn effect_from(s: &str) -> Result<EffectLevel, zbus::fdo::Error> {
    match s.trim().to_ascii_lowercase().as_str() {
        "low" => Ok(EffectLevel::Low),
        "" | "medium" => Ok(EffectLevel::Medium),
        "high" => Ok(EffectLevel::High),
        other => Err(zbus::fdo::Error::InvalidArgs(format!(
            "effort must be low, medium or high (got {other:?})"
        ))),
    }
}

#[interface(interface = "dev.marcford.BrainFM1")]
impl BrainFm {
    // ---- auth ----

    async fn login(
        &mut self,
        method: &str,
        identifier: &str,
        secret: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let auth = match method {
            auth_method::EMAIL => AuthMethod::Email {
                email: identifier.to_owned(),
                password: Password::new(secret),
            },
            auth_method::MAGIC_LINK => AuthMethod::magic_link(secret)
                .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?,
            auth_method::TOKEN => AuthMethod::ExistingToken {
                raw: secret.to_owned(),
            },
            other => {
                return Err(zbus::fdo::Error::InvalidArgs(format!(
                    "unknown auth method {other:?}"
                )));
            }
        };

        let token = self.api.login(auth).await.map_err(api_error)?;
        // Persist before adopting: a token we cannot store is a token the
        // user silently loses on restart.
        self.store
            .save(token.as_str())
            .map_err(|e| zbus::fdo::Error::Failed(format!("could not store the session: {e}")))?;
        self.api.set_token(token);

        let _ = self.authenticated_changed(&emitter).await;
        let _ = Self::auth_state_changed(&emitter, true).await;
        Ok(())
    }

    async fn logout(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.api.forget_token();
        self.store
            .clear()
            .map_err(|e| zbus::fdo::Error::Failed(format!("could not clear the session: {e}")))?;
        self.playback = "idle";
        self.track = None;

        let _ = self.authenticated_changed(&emitter).await;
        let _ = self.state_changed(&emitter).await;
        let _ = self.track_changed(&emitter).await;
        let _ = Self::auth_state_changed(&emitter, false).await;
        Ok(())
    }

    // ---- playback ----

    async fn start_session(
        &mut self,
        state: &str,
        activity: &str,
        genres: Vec<String>,
        effort: &str,
        timer: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<String> {
        self.require_auth()?;
        let effect = effect_from(effort)?;

        if !timer.trim().is_empty() {
            let spec = TimerSpec::parse(timer)
                .map_err(|e| zbus::fdo::Error::InvalidArgs(e.to_string()))?;
            self.timer = Some(Timer::start(spec));
        }

        let activity_id = self.resolve_activity(state, activity).await?;
        let mut req = SessionRequest::new(activity_id.clone(), effect);
        if !genres.is_empty() {
            req = req.with_genres(genres);
        }

        let (servings, rot) = self.api.start_session(&req).await.map_err(api_error)?;
        self.persist_rotation(&rot);

        let serving = servings.into_iter().next().ok_or_else(|| {
            zbus::fdo::Error::Failed(format!(
                "{}: the API returned a session with no tracks",
                error_code::UPSTREAM
            ))
        })?;

        let length = serving.track_variation.length_in_seconds.max(0.0) as u64;
        // The URL is signed; only the scrubbed form is ever logged.
        tracing::info!(
            track = %serving.track.name,
            length,
            url = %serving.track_variation.loggable_url(),
            "starting playback"
        );

        self.send(Command::Load {
            url: serving.track_variation.tokened_url.clone(),
            length: Duration::from_secs(length),
        })?;

        self.mental_state = state.to_owned();
        self.activity = activity_id.clone();
        self.playback = "buffering";
        self.position = 0;
        self.track = Some(TrackInfo {
            id: serving.track.id.clone(),
            name: serving.track.name.clone(),
            length,
            neural_effect: serving.track_variation.neural_effect_level.unwrap_or(0.0),
        });
        let _ = self.state_changed(&emitter).await;
        let _ = self.mental_state_changed(&emitter).await;
        let _ = self.activity_changed(&emitter).await;
        let _ = self.track_changed(&emitter).await;
        let _ = self.timer_changed(&emitter).await;
        let _ = Self::session_started(&emitter, &serving.track.id).await;
        let _ = Self::track_started(&emitter, &serving.track.name, length).await;

        self.hooks.fire(&hooks::Event::SessionStart {
            state: state.to_owned(),
            activity: activity_id,
        });
        self.hooks.fire(&hooks::Event::TrackStart {
            track: serving.track.name.clone(),
            length,
        });
        Ok(serving.track.id)
    }

    pub async fn stop_session(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let _ = self.send(Command::Stop);
        self.playback = "idle";
        self.track = None;
        self.timer = None;
        self.position = 0;

        let _ = self.state_changed(&emitter).await;
        let _ = self.track_changed(&emitter).await;
        let _ = self.timer_changed(&emitter).await;
        let _ = Self::session_ended(&emitter, "stopped").await;
        self.hooks.fire(&hooks::Event::SessionEnd {
            reason: "stopped".into(),
        });
        Ok(())
    }

    pub async fn pause(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if self.playback == "playing" {
            self.send(Command::Pause)?;
            self.playback = "paused";
            let _ = self.state_changed(&emitter).await;
        }
        Ok(())
    }

    pub async fn resume(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        if self.playback == "paused" {
            self.send(Command::Play)?;
            self.playback = "playing";
            let _ = self.state_changed(&emitter).await;
        }
        Ok(())
    }

    pub async fn skip_track(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.require_auth()?;
        let state = self.mental_state.clone();
        if state.is_empty() {
            return Err(zbus::fdo::Error::Failed("no session is running".into()));
        }

        let (servings, rot) = self.api.new_serving(&state).await.map_err(api_error)?;
        self.persist_rotation(&rot);

        let serving = servings
            .into_iter()
            .next()
            .ok_or_else(|| zbus::fdo::Error::Failed("no next track available".into()))?;
        let length = serving.track_variation.length_in_seconds.max(0.0) as u64;

        self.send(Command::Load {
            url: serving.track_variation.tokened_url.clone(),
            length: Duration::from_secs(length),
        })?;
        self.track = Some(TrackInfo {
            id: serving.track.id.clone(),
            name: serving.track.name.clone(),
            length,
            neural_effect: serving.track_variation.neural_effect_level.unwrap_or(0.0),
        });
        self.position = 0;

        let _ = self.track_changed(&emitter).await;
        let _ = Self::track_started(&emitter, &serving.track.name, length).await;
        self.hooks.fire(&hooks::Event::TrackStart {
            track: serving.track.name.clone(),
            length,
        });
        Ok(())
    }

    /// `"50m"`, `"25m/5m"`, `"25m/5m x4"`, or `""` to clear.
    ///
    /// Works without a session: a timer is useful on its own, and keeping it
    /// independent lets a plugin drive one without touching playback.
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

    // ---- catalog ----

    async fn list_mental_states(&mut self) -> zbus::fdo::Result<Vec<String>> {
        self.require_auth()?;
        let (states, rot) = self.api.mental_states().await.map_err(api_error)?;
        self.persist_rotation(&rot);
        Ok(states.into_iter().map(|s| s.id).collect())
    }

    async fn list_activities(&mut self, state: &str) -> zbus::fdo::Result<Vec<(String, String)>> {
        self.require_auth()?;
        let (acts, rot) = self.api.activities(state).await.map_err(api_error)?;
        self.persist_rotation(&rot);
        Ok(acts.into_iter().map(|a| (a.id, a.display_value)).collect())
    }

    async fn list_genres(&mut self, state: &str) -> zbus::fdo::Result<Vec<String>> {
        self.require_auth()?;
        let (g, rot) = self.api.genres(state).await.map_err(api_error)?;
        self.persist_rotation(&rot);
        let mut all = g.base_genre_names;
        all.extend(g.nature_genre_names);
        Ok(all)
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
        put("authenticated", Variant::from(self.api.token().is_some()));
        put("volume", Variant::from(self.volume));
        put("timer_phase", Variant::from(self.timer_phase()));
        put("timer_remaining", Variant::from(self.timer_remaining()));
        put(
            "track",
            Variant::from(
                self.track
                    .as_ref()
                    .map(|t| t.name.clone())
                    .unwrap_or_default(),
            ),
        );
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
        self.position
    }

    #[zbus(property)]
    async fn authenticated(&self) -> bool {
        self.api.token().is_some()
    }

    #[zbus(property)]
    async fn volume(&self) -> f64 {
        self.volume
    }

    #[zbus(property)]
    pub async fn set_volume(&mut self, volume: f64) {
        self.volume = volume.clamp(0.0, 1.0);
        let _ = self.send(Command::SetVolume(self.volume as f32));
    }

    #[zbus(property)]
    async fn track(&self) -> HashMap<String, zbus::OwnedVariant> {
        let mut m: HashMap<String, zbus::OwnedVariant> = HashMap::new();
        let Some(t) = self.track.as_ref() else {
            return m;
        };
        let mut put = |k: &str, v: Variant<'_>| {
            if let Ok(owned) = zbus::OwnedVariant::try_from(v) {
                m.insert(k.to_owned(), owned);
            }
        };
        put("id", Variant::from(t.id.clone()));
        put("name", Variant::from(t.name.clone()));
        put("length", Variant::from(t.length));
        put("neural_effect", Variant::from(t.neural_effect));
        m
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
    /// Persist a rotated token. Lazy renewal means this is the only place a
    /// long-running daemon keeps its session alive.
    fn persist_rotation(&mut self, rotation: &bfm_api::TokenRotation) {
        if let Some(raw) = self.api.adopt_rotation(rotation) {
            if let Err(e) = self.store.save(&raw) {
                tracing::warn!(error = %e, "renewed the session but could not store it");
            } else {
                tracing::debug!("session token renewed and stored");
            }
        }
    }

    fn send(&self, cmd: Command) -> zbus::fdo::Result<()> {
        self.player
            .lock()
            .map_err(|_| zbus::fdo::Error::Failed("audio engine is poisoned".into()))?
            .handle(cmd)
            .map_err(|e| zbus::fdo::Error::Failed(format!("{}: {e}", error_code::AUDIO_DEVICE)))
    }

    /// Resolve an activity argument to an id: empty means the state's
    /// default, otherwise match on id or display name.
    /// Fail fast when there is no session, rather than sending an
    /// unauthenticated request and reading the rejection back.
    fn require_auth(&self) -> zbus::fdo::Result<()> {
        if self.api.token().is_none() {
            return Err(needs_login());
        }
        Ok(())
    }

    async fn resolve_activity(&mut self, state: &str, activity: &str) -> zbus::fdo::Result<String> {
        let (acts, rot) = self.api.activities(state).await.map_err(api_error)?;
        self.persist_rotation(&rot);

        if activity.trim().is_empty() {
            return acts
                .iter()
                .find(|a| a.is_default)
                .or_else(|| acts.first())
                .map(|a| a.id.clone())
                .ok_or_else(|| {
                    zbus::fdo::Error::Failed(format!("no activities for state {state:?}"))
                });
        }

        let wanted = activity.trim().to_ascii_lowercase();
        acts.iter()
            .find(|a| a.id == activity || a.display_value.to_ascii_lowercase() == wanted)
            .map(|a| a.id.clone())
            .ok_or_else(|| {
                let names: Vec<_> = acts.iter().map(|a| a.display_value.as_str()).collect();
                zbus::fdo::Error::InvalidArgs(format!(
                    "unknown activity {activity:?}; {state} has: {}",
                    names.join(", ")
                ))
            })
    }

    /// Drain the audio engine and react.
    ///
    /// This is the session orchestration: without it a track plays once and
    /// nothing follows it. Called from the daemon's tick, which is also what
    /// keeps `Position` honest.
    pub async fn pump(&mut self, emitter: &SignalEmitter<'_>) {
        let events = match self.player.lock() {
            Ok(mut p) => p.poll(),
            Err(_) => return,
        };

        for event in events {
            match event {
                Event::Started { .. } => {
                    if self.playback != "playing" {
                        self.playback = "playing";
                        let _ = self.state_changed(emitter).await;
                    }
                }

                Event::Position { secs } => {
                    let secs = secs.max(0.0) as u64;
                    // Only notify on a whole-second change; a 250ms tick
                    // would otherwise spam every client four times a second.
                    if secs != self.position {
                        self.position = secs;
                        let _ = self.position_changed(emitter).await;
                    }
                }

                // Fetch the next track and queue it behind the current one.
                // Gapless depends on this happening before the end.
                Event::NearEnd { generation } => {
                    if let Err(e) = self.queue_next().await {
                        tracing::warn!(generation, error = %e, "could not prefetch the next track");
                        let _ = Self::error(emitter, error_code::UPSTREAM, &e.to_string()).await;
                    }
                }

                Event::Ended { .. } => {
                    // The queue drained without a successor.
                    self.playback = "idle";
                    self.position = 0;
                    let _ = self.state_changed(emitter).await;
                    let _ = Self::session_ended(emitter, "track-ended").await;
                    self.hooks.fire(&hooks::Event::SessionEnd {
                        reason: "track-ended".into(),
                    });
                }

                Event::Error { message } => {
                    tracing::error!(message, "audio engine error");
                    let _ = Self::error(emitter, error_code::AUDIO_DEVICE, &message).await;
                }
            }
        }
    }

    /// Fetch and queue the next track for the running session.
    async fn queue_next(&mut self) -> Result<(), String> {
        let state = self.mental_state.clone();
        if state.is_empty() || self.api.token().is_none() {
            return Ok(());
        }

        let (servings, rot) = self
            .api
            .new_serving(&state)
            .await
            .map_err(|e| e.to_string())?;
        self.persist_rotation(&rot);

        let serving = servings.into_iter().next().ok_or("no next track")?;
        let length = serving.track_variation.length_in_seconds.max(0.0) as u64;
        tracing::info!(
            next = %serving.track.name,
            url = %serving.track_variation.loggable_url(),
            "queued next track"
        );

        self.player
            .lock()
            .map_err(|_| "audio engine is poisoned".to_string())?
            .handle(Command::Queue {
                url: serving.track_variation.tokened_url.clone(),
                length: Duration::from_secs(length),
            })
            .map_err(|e| e.to_string())
    }

    /// Fire a lifecycle hook. Used by the daemon tick for timer phases.
    pub fn fire_hook(&self, event: &hooks::Event) {
        self.hooks.fire(event);
    }

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
    pub fn tick_timer(&mut self, delta: Duration) -> Option<Phase> {
        self.timer.as_mut()?.advance(delta)
    }
}
