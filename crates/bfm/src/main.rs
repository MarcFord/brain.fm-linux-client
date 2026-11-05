//! `bfm` — a thin client for `bfmd`.
//!
//! Every subcommand here is a D-Bus call. There is no API or audio code in
//! this crate, by design.

use clap::{Parser, Subcommand};
use color_eyre::Result;

#[derive(Parser)]
#[command(name = "bfm", version, about = "Unofficial Brain.fm client for Linux")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Authenticate and store the session token in the keyring.
    Login,
    /// Start a session.
    Play {
        /// focus | relax | sleep | meditate
        state: String,
        #[arg(long)]
        activity: Option<String>,
        #[arg(long)]
        genre: Vec<String>,
        /// low | medium | high
        #[arg(long, default_value = "high")]
        effort: String,
        /// e.g. "50m", "25m/5m", "until 17:00"
        #[arg(long)]
        timer: Option<String>,
    },
    /// Pause playback.
    Pause,
    /// Resume playback.
    Resume,
    /// Skip the current track.
    Skip,
    /// Stop the session.
    Stop,
    /// Print current status.
    Status {
        /// Emit a waybar-shaped JSON object.
        #[arg(long)]
        json: bool,
        /// Stream a line per state change instead of exiting.
        #[arg(long)]
        watch: bool,
    },
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();
    let verb = match cli.command {
        Command::Login => "Login",
        Command::Play { .. } => "StartSession",
        Command::Pause => "Pause",
        Command::Resume => "Resume",
        Command::Skip => "SkipTrack",
        Command::Stop => "StopSession",
        Command::Status { .. } => "GetStatus",
    };
    println!(
        "bfm is a stub until milestone 4; this would call {}.{} on {}",
        bfm_proto::INTERFACE,
        verb,
        bfm_proto::BUS_NAME
    );
    Ok(())
}
