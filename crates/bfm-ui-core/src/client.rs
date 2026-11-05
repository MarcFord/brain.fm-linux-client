//! Talking to `bfmd`, without imposing a runtime on the front-end.
//!
//! GTK and Qt each own their main loop and neither will host a tokio
//! runtime. So the bus lives on a background thread with its own runtime, and
//! the two sides meet over `async_channel`, which is runtime-agnostic and can
//! be awaited from `glib::spawn_future_local` or polled from a Qt timer.
//!
//! The front-end therefore never writes async D-Bus code; it sends
//! [`Command`]s and renders [`Update`]s.

use std::collections::HashMap;

use bfm_proto::{BrainFM1Proxy, auth_method};
use futures_util::StreamExt as _;

use crate::state::{Playback, State, timer_from, track_from};

/// Something the user asked for.
#[derive(Debug, Clone)]
pub enum Command {
    StartSession {
        state: String,
        activity: String,
        genres: Vec<String>,
        effort: String,
        timer: String,
    },
    Pause,
    Resume,
    Skip,
    Stop,
    SetTimer(String),
    SetVolume(f64),
    LoginFromBrowserToken(String),
    Logout,
    /// Re-read everything, for a manual refresh.
    Refresh,
}

/// Something the front-end should react to.
#[derive(Debug, Clone)]
pub enum Update {
    /// Full state. Sent on connect and after every change.
    State(Box<State>),
    /// The daemon could not be reached; the UI should say so rather than
    /// showing a convincing but frozen idle screen.
    Disconnected(String),
    /// A `code` from `bfm_proto::error_code`, plus a human message.
    Failed { code: String, message: String },
    /// The catalog, fetched lazily because it needs authentication.
    Catalog {
        states: Vec<String>,
        activities: Vec<(String, String)>,
        genres: Vec<String>,
    },
}

/// Handle to the background bus thread.
pub struct Client {
    commands: async_channel::Sender<Command>,
    updates: async_channel::Receiver<Update>,
}

impl Client {
    /// Start the bus thread. Returns immediately; the first [`Update`]
    /// arrives once the daemon answers.
    pub fn spawn() -> Self {
        let (ctx, crx) = async_channel::unbounded::<Command>();
        let (utx, urx) = async_channel::unbounded::<Update>();

        std::thread::Builder::new()
            .name("bfm-bus".into())
            .spawn(move || {
                // Multi-threaded on purpose: zbus keeps background tasks
                // alive between calls, and a current-thread runtime would
                // stall them the moment we stop blocking on one.
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = utx.send_blocking(Update::Disconnected(e.to_string()));
                        return;
                    }
                };
                rt.block_on(run(crx, utx));
            })
            .expect("spawn bus thread");

        Self {
            commands: ctx,
            updates: urx,
        }
    }

    /// Queue a command. Never blocks; failures surface as an [`Update`].
    pub fn send(&self, cmd: Command) {
        if self.commands.try_send(cmd).is_err() {
            tracing::warn!("bus thread is gone; command dropped");
        }
    }

    /// Stream of updates, for the front-end's own loop to await.
    pub fn updates(&self) -> async_channel::Receiver<Update> {
        self.updates.clone()
    }
}

async fn run(commands: async_channel::Receiver<Command>, updates: async_channel::Sender<Update>) {
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            let _ = updates
                .send(Update::Disconnected(format!("no session bus: {e}")))
                .await;
            return;
        }
    };

    // D-Bus activation starts bfmd on this first call, so there is nothing
    // for the front-end to launch.
    let bfm = match BrainFM1Proxy::new(&conn).await {
        Ok(p) => p,
        Err(e) => {
            let _ = updates
                .send(Update::Disconnected(format!("could not reach bfmd: {e}")))
                .await;
            return;
        }
    };

    let _ = updates
        .send(Update::State(Box::new(read_state(&bfm).await)))
        .await;

    let props = match zbus::fdo::PropertiesProxy::builder(&conn)
        .destination(bfm_proto::BUS_NAME)
        .path(bfm_proto::OBJECT_PATH)
        .build()
        .await
    {
        Ok(p) => p,
        Err(e) => {
            let _ = updates.send(Update::Disconnected(e.to_string())).await;
            return;
        }
    };
    let mut changes = match props.receive_properties_changed().await {
        Ok(s) => s,
        Err(e) => {
            let _ = updates.send(Update::Disconnected(e.to_string())).await;
            return;
        }
    };

    loop {
        tokio::select! {
            // Any property change re-reads the whole state. Simpler than
            // patching fields, and the daemon only signals on real changes.
            Some(_) = changes.next() => {
                let _ = updates.send(Update::State(Box::new(read_state(&bfm).await))).await;
            }

            cmd = commands.recv() => {
                let Ok(cmd) = cmd else { return };  // front-end dropped
                if let Err(e) = apply(&bfm, cmd).await {
                    let (code, message) = split_error(&e);
                    let _ = updates.send(Update::Failed { code, message }).await;
                }
                let _ = updates.send(Update::State(Box::new(read_state(&bfm).await))).await;
            }

            else => return,
        }
    }
}

async fn apply(bfm: &BrainFM1Proxy<'_>, cmd: Command) -> zbus::Result<()> {
    match cmd {
        Command::StartSession {
            state,
            activity,
            genres,
            effort,
            timer,
        } => {
            let refs: Vec<&str> = genres.iter().map(String::as_str).collect();
            bfm.start_session(&state, &activity, &refs, &effort, &timer)
                .await?;
        }
        Command::Pause => bfm.pause().await?,
        Command::Resume => bfm.resume().await?,
        Command::Skip => bfm.skip_track().await?,
        Command::Stop => bfm.stop_session().await?,
        Command::SetTimer(spec) => bfm.set_timer(&spec).await?,
        Command::SetVolume(v) => bfm.set_volume(v).await?,
        Command::LoginFromBrowserToken(t) => bfm.login(auth_method::TOKEN, "", &t).await?,
        Command::Logout => bfm.logout().await?,
        Command::Refresh => {}
    }
    Ok(())
}

async fn read_state(bfm: &BrainFM1Proxy<'_>) -> State {
    let empty: HashMap<String, zbus::OwnedVariant> = HashMap::new();
    let track_map = bfm.track().await.unwrap_or_else(|_| empty.clone());
    let timer_map = bfm.timer().await.unwrap_or(empty);

    State {
        connected: true,
        authenticated: bfm.authenticated().await.unwrap_or(false),
        playback: Playback::from_wire(&bfm.state().await.unwrap_or_default()),
        mental_state: bfm.mental_state().await.unwrap_or_default(),
        activity: bfm.activity().await.unwrap_or_default(),
        track: track_from(&track_map),
        position: bfm.position().await.unwrap_or(0),
        timer: timer_from(&timer_map),
        volume: bfm.volume().await.unwrap_or(1.0),
    }
}

/// Split the daemon's `"<code>: <message>"` convention back apart.
///
/// The codes in `bfm_proto::error_code` are stable, so a front-end can branch
/// on them — most usefully to show a sign-in prompt instead of an error.
pub fn split_error(e: &zbus::Error) -> (String, String) {
    let full = e.to_string();
    for code in [
        bfm_proto::error_code::NEEDS_LOGIN,
        bfm_proto::error_code::SCHEMA_CHANGED,
        bfm_proto::error_code::UPSTREAM,
        bfm_proto::error_code::AUDIO_DEVICE,
    ] {
        if let Some(i) = full.find(code) {
            let rest = full[i + code.len()..]
                .trim_start_matches([':', ' '])
                .to_owned();
            return (code.to_owned(), rest);
        }
    }
    (String::new(), full)
}

/// Helper for front-ends: should this failure prompt a sign-in?
pub fn needs_login(code: &str) -> bool {
    code == bfm_proto::error_code::NEEDS_LOGIN
}
