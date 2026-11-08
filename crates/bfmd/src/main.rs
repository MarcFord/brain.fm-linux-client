//! The bfm daemon.
//!
//! The daemon is the product: it owns the API session, the audio engine and
//! the timer, and exposes all of it on the session bus as
//! `dev.marcford.BrainFM1`. The CLI, a status-bar module and any third-party
//! plugin are equal clients of that interface — none is privileged.
//!
//! See `docs/architecture.md`.

use bfmd::{hooks, iface, mpris, store};

use std::time::Duration;

use bfm_proto::{BUS_NAME, INTERFACE_VERSION, OBJECT_PATH};
use clap::Parser;
use color_eyre::{Result, eyre::WrapErr};
use zbus::connection;

use crate::iface::BrainFm;

/// Tick resolution. The audio engine reports position about four times a
/// second, so draining it on a one-second cadence keeps `Position` accurate
/// to the second without waking the CPU needlessly.
const TICK: Duration = Duration::from_secs(1);

/// Even a daemon nobody invokes by hand needs these.
///
/// Without argument parsing `bfmd --version` started the daemon and blocked
/// forever, and an unknown flag was ignored rather than reported — both
/// surprising for something a distro package puts on PATH.
#[derive(Parser)]
#[command(
    name = "bfmd",
    version,
    about = "Brain.fm daemon (unofficial). Normally started on demand by D-Bus."
)]
struct Cli {}

#[tokio::main]
async fn main() -> Result<()> {
    Cli::parse();
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BFM_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    // In zbus 6 only `build` is fallible; the rest of the chain returns Self.
    // An unusable audio device must not stop the daemon from serving: the
    // control plane, timers and `bfm login` all still work without sound, and
    // the failure surfaces on the bus instead of at startup.
    let player: Box<dyn bfm_player::Player> = match bfm_player::AudioEngine::spawn() {
        Ok(engine) => Box::new(engine),
        Err(e) => {
            tracing::error!(error = %e, "no audio output; playback will fail");
            Box::new(bfm_player::NullPlayer::default())
        }
    };

    let conn = connection::Builder::session()
        .name(BUS_NAME)
        .serve_at(OBJECT_PATH, BrainFm::new(store::open(), player))
        // MPRIS buys media keys, playerctl and lock-screen widgets for state
        // the daemon already has. A connection can own several names.
        .name(bfm_proto::MPRIS_BUS_NAME)
        .serve_at(mpris::MPRIS_PATH, mpris::Root)
        .serve_at(mpris::MPRIS_PATH, mpris::Player)
        .build()
        .await
        .wrap_err(
            "could not take dev.marcford.bfm on the session bus \
             (no desktop session, or another bfmd is already running)",
        )?;

    tracing::info!(
        bus = BUS_NAME,
        path = OBJECT_PATH,
        interface = bfm_proto::INTERFACE,
        version = INTERFACE_VERSION,
        mpris = bfm_proto::MPRIS_BUS_NAME,
        "serving"
    );

    tick_timer_forever(&conn).await
}

/// Drain the audio engine and advance the timer once a second, emitting
/// signals only on real changes.
async fn tick_timer_forever(conn: &zbus::Connection) -> Result<()> {
    let iface_ref = conn
        .object_server()
        .interface::<_, BrainFm>(OBJECT_PATH)
        .await
        .wrap_err("interface vanished from the object server")?;

    let mut ticker = tokio::time::interval(TICK);
    loop {
        ticker.tick().await;

        let emitter = iface_ref.signal_emitter();
        let mut iface = iface_ref.get_mut().await;

        // Drain the audio engine first: it drives position, track changes and
        // the gapless prefetch.
        iface.pump(emitter).await;

        let Some(phase) = iface.tick_timer(TICK) else {
            // Still mid-phase. Refresh the property so a status bar counting
            // down stays honest, but emit no signal.
            if iface.timer.is_some() {
                let _ = iface.timer_changed(emitter).await;
            }
            continue;
        };

        let remaining = iface.timer_remaining();
        tracing::debug!(phase = phase.as_str(), remaining, "timer phase changed");

        if let Err(e) = BrainFm::timer_phase_changed(emitter, phase.as_str(), remaining).await {
            tracing::warn!(error = %e, "could not emit TimerPhaseChanged");
        }
        iface.fire_hook(&hooks::Event::TimerPhase {
            phase: phase.as_str().to_owned(),
            remaining,
            shape: iface.timer_shape(),
        });

        // A finished timer ends the session - that is the whole point of
        // setting one.
        if iface.timer.as_ref().is_some_and(|t| t.is_done()) {
            iface.playback = "idle";
            iface.timer = None;
            let _ = BrainFm::session_ended(emitter, "timer-elapsed").await;
            iface.fire_hook(&hooks::Event::SessionEnd {
                reason: "timer-elapsed".into(),
            });
            let _ = iface.state_changed(emitter).await;
        }
        let _ = iface.timer_changed(emitter).await;
    }
}
