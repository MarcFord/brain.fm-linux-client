//! Native GTK4 + libadwaita front-end.
//!
//! This is a client of `dev.marcford.BrainFM1` and nothing else — no API
//! calls, no audio, no duplicated state. That is the pay-off of putting the
//! daemon at the centre: a native GUI costs no backend work, and it has
//! exactly the privileges any third-party app has.
//!
//! `--layer` renders the same model as a Wayland layer-shell overlay, which
//! is the shape Hyprland users actually want: bound to a key, anchored to a
//! corner, no window management.

mod app;

use clap::Parser;

#[derive(Parser)]
#[command(name = "bfm-gtk", version, about = "Brain.fm for Linux (GTK)")]
struct Cli {
    /// Show as a Wayland layer-shell overlay instead of a normal window.
    #[arg(long)]
    layer: bool,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BFM_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let cli = Cli::parse();
    let app = relm4::RelmApp::new("dev.marcford.bfm.gtk");
    app.run::<app::App>(app::Init { layer: cli.layer });
}
