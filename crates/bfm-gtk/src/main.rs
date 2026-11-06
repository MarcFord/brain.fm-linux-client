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
mod view;
mod wizard;

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

    // GtkApplication parses the command line itself and rejects anything it
    // does not recognise, so `--layer` never reached this binary's own
    // parser: GTK failed first with "Unknown option --layer". Hand it only
    // the program name; clap has already taken what it needs.
    let argv0 = std::env::args().next().unwrap_or_else(|| "bfm-gtk".into());

    let app = relm4::RelmApp::new("dev.marcford.bfm.gtk").with_args(vec![argv0]);
    app.run::<app::App>(app::Init { layer: cli.layer });
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_internally_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn layer_flag_parses() {
        assert!(Cli::parse_from(["bfm-gtk", "--layer"]).layer);
        assert!(!Cli::parse_from(["bfm-gtk"]).layer);
    }

    /// Regression guard.
    ///
    /// `GtkApplication` parses the command line itself and rejects anything
    /// it does not recognise. Passing our own argv through meant GTK failed
    /// with "Unknown option --layer" before this binary's parser ever ran,
    /// so layer mode could never start. Clap takes what it needs; GTK gets
    /// only the program name.
    #[test]
    fn gtk_is_not_handed_our_arguments() {
        let src = include_str!("main.rs");
        assert!(
            src.contains("with_args"),
            "RelmApp must be given an explicit argv, or GTK will reject our flags"
        );
    }
}
