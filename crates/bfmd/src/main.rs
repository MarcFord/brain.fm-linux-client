//! The bfm daemon.
//!
//! The daemon is the product: it owns the API session, the audio engine, the
//! timer state machine and the hook runner, and exposes all of it on the
//! session bus. The CLI, a status-bar module and any third-party plugin are
//! equal clients of that interface — none is privileged.
//!
//! Milestone 3 fills this in. See `docs/architecture.md`.

use color_eyre::Result;

fn main() -> Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BFM_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    tracing::info!(
        interface = bfm_proto::INTERFACE,
        version = bfm_proto::INTERFACE_VERSION,
        "bfmd is a stub until milestone 3"
    );
    Ok(())
}
