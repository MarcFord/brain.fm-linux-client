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
    ///
    /// With no flags, picks the right method interactively. Accounts created
    /// with Google, Apple or Facebook use `--browser`: the provider login has
    /// to happen in a real browser, and `bfm` adopts the resulting session.
    /// Because renewal is lazy, that handoff is a one-time cost.
    Login {
        /// Email + password. The password is never stored.
        #[arg(long, value_name = "ADDRESS", group = "method")]
        email: Option<String>,

        /// Open the system browser, sign in with any provider, hand the
        /// session back. The only route for Google / Apple / Facebook.
        #[arg(long, group = "method")]
        browser: bool,

        /// Consume a magic link you were emailed. Accepts the URL or the
        /// bare token.
        #[arg(long, value_name = "URL", group = "method")]
        magic_link: Option<String>,

        /// Read a session token from stdin. For scripts and headless hosts.
        #[arg(long, group = "method")]
        token_stdin: bool,
    },

    /// Forget the stored session token.
    Logout,

    /// Show who is signed in and by what method.
    Whoami,
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
        Command::Login { .. } => "Login",
        Command::Logout => "Logout",
        Command::Whoami => "Whoami",
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
