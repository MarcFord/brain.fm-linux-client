//! `bfm` — a thin client for `bfmd`.
//!
//! Every subcommand here is a D-Bus call. There is no API or audio code in
//! this crate by design: the daemon is the product, and this binary has no
//! privileges a third-party plugin lacks.

use bfm_proto::{BrainFM1Proxy, auth_method};
use clap::{Parser, Subcommand};
use color_eyre::{
    Result,
    eyre::{Context, bail},
};
use futures_util::StreamExt;

#[derive(Parser)]
#[command(name = "bfm", version, about = "Unofficial Brain.fm client for Linux")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Authenticate and store the session token.
    ///
    /// Accounts created with Google, Apple or Facebook use `--browser`: the
    /// provider login has to happen in a real browser. Because renewal is
    /// lazy, that handoff is a one-time cost.
    Login {
        /// Email + password. The password is never stored.
        #[arg(long, value_name = "ADDRESS", group = "method")]
        email: Option<String>,
        /// Sign in with any provider in your browser, then paste the session.
        #[arg(long, group = "method")]
        browser: bool,
        /// Consume a magic link. Accepts the URL or the bare token.
        #[arg(long, value_name = "URL", group = "method")]
        magic_link: Option<String>,
        /// Read a session token from stdin. For scripts and headless hosts.
        #[arg(long, group = "method")]
        token_stdin: bool,
    },
    /// Forget the stored session.
    Logout,
    /// Show whether a session is stored.
    Whoami,

    /// Start a session.
    Play {
        /// focus | relax | sleep | meditate
        state: String,
        #[arg(long)]
        activity: Option<String>,
        #[arg(long = "genre")]
        genres: Vec<String>,
        /// low | medium | high
        #[arg(long, default_value = "high")]
        effort: String,
        /// "50m", "25m/5m", "25m/5m x4"
        #[arg(long)]
        timer: Option<String>,
    },
    Pause,
    Resume,
    Skip,
    Stop,
    /// Set or clear the session timer.
    Timer {
        /// "50m", "25m/5m x4". Omit to clear.
        spec: Option<String>,
    },
    /// List what the catalog offers.
    List {
        /// states | activities | genres
        what: String,
        #[arg(long, default_value = "focus")]
        state: String,
    },
    /// Print current status.
    Status {
        /// Emit a waybar-shaped JSON object.
        #[arg(long)]
        json: bool,
        /// Stream a line per change instead of exiting.
        #[arg(long)]
        watch: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;
    let cli = Cli::parse();

    let conn = zbus::Connection::session()
        .await
        .wrap_err("no session bus available")?;
    let bfm = BrainFM1Proxy::new(&conn).await.wrap_err(
        "could not reach bfmd. Install dist/dev.marcford.bfm.service for \
         D-Bus activation, or start bfmd manually.",
    )?;

    match cli.command {
        Command::Login {
            email,
            browser,
            magic_link,
            token_stdin,
        } => login(&bfm, email, browser, magic_link, token_stdin).await?,
        Command::Logout => {
            bfm.logout().await?;
            println!("signed out");
        }
        Command::Whoami => {
            if bfm.authenticated().await? {
                println!("signed in");
            } else {
                println!("not signed in — run `bfm login`");
            }
        }

        Command::Play {
            state,
            activity,
            genres,
            effort,
            timer,
        } => {
            let genre_refs: Vec<&str> = genres.iter().map(String::as_str).collect();
            bfm.start_session(
                &state,
                activity.as_deref().unwrap_or(""),
                &genre_refs,
                &effort,
                timer.as_deref().unwrap_or(""),
            )
            .await?;
            let name = bfm
                .track()
                .await
                .unwrap_or_default()
                .get("name")
                .and_then(|v| String::try_from(v.clone()).ok())
                .unwrap_or_default();
            if name.is_empty() {
                println!("playing {state}");
            } else {
                println!("playing {state}: {name}");
            }
        }
        Command::Pause => bfm.pause().await?,
        Command::Resume => bfm.resume().await?,
        Command::Skip => bfm.skip_track().await?,
        Command::Stop => bfm.stop_session().await?,

        Command::Timer { spec } => {
            bfm.set_timer(spec.as_deref().unwrap_or("")).await?;
            match spec {
                Some(s) => println!("timer set: {s}"),
                None => println!("timer cleared"),
            }
        }

        Command::List { what, state } => match what.as_str() {
            "states" => {
                for s in bfm.list_mental_states().await? {
                    println!("{s}");
                }
            }
            "activities" => {
                for (id, name) in bfm.list_activities(&state).await? {
                    println!("{id}\t{name}");
                }
            }
            "genres" => {
                for g in bfm.list_genres(&state).await? {
                    println!("{g}");
                }
            }
            other => bail!("don't know how to list {other:?}; try states, activities or genres"),
        },

        Command::Status { json, watch } => status(&conn, &bfm, json, watch).await?,
    }
    Ok(())
}

async fn login(
    bfm: &BrainFM1Proxy<'_>,
    email: Option<String>,
    browser: bool,
    magic_link: Option<String>,
    token_stdin: bool,
) -> Result<()> {
    if let Some(address) = email {
        // Prompted, never echoed, never written to disk. It crosses the
        // user's own session bus once so the daemon can exchange it, and is
        // not stored by either side.
        let password = rpassword::prompt_password("Brain.fm password: ")
            .wrap_err("could not read the password")?;
        bfm.login(auth_method::EMAIL, &address, &password).await?;
    } else if let Some(link) = magic_link {
        bfm.login(auth_method::MAGIC_LINK, "", &link).await?;
    } else if token_stdin {
        let mut raw = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut raw)?;
        bfm.login(auth_method::TOKEN, "", raw.trim()).await?;
    } else if browser {
        browser_handoff(bfm).await?;
    } else {
        bail!(
            "choose a method: --email <address>, --browser (Google/Apple/Facebook), \
             --magic-link <url>, or --token-stdin"
        );
    }

    println!("signed in");
    Ok(())
}

/// The only route for SSO accounts: let the real web app in a real browser do
/// the provider login, then adopt the session it produced. See `docs/auth.md`
/// for why we cannot do the OAuth ourselves.
async fn browser_handoff(bfm: &BrainFM1Proxy<'_>) -> Result<()> {
    const URL: &str = "https://my.brain.fm/";
    let _ = std::process::Command::new("xdg-open").arg(URL).spawn();

    println!("Opened {URL} — sign in with whichever provider your account uses.");
    println!();
    println!("Then, in that tab, open DevTools (F12) and run:");
    println!("    JSON.parse(JSON.parse(localStorage['persist:auth']).token)");
    println!();
    println!("Paste the value here (it will not be echoed).");

    let token =
        rpassword::prompt_password("session token: ").wrap_err("could not read the token")?;
    let token = token.trim().trim_matches('"');
    if token.is_empty() {
        bail!("no token entered");
    }
    bfm.login(auth_method::TOKEN, "", token).await?;
    Ok(())
}

async fn status(
    conn: &zbus::Connection,
    bfm: &BrainFM1Proxy<'_>,
    json: bool,
    watch: bool,
) -> Result<()> {
    print_status(bfm, json).await?;
    if !watch {
        return Ok(());
    }

    // One stream for every property on the interface, so a status bar gets a
    // line per change without polling.
    let props = zbus::fdo::PropertiesProxy::builder(conn)
        .destination(bfm_proto::BUS_NAME)
        .path(bfm_proto::OBJECT_PATH)
        .build()
        .await?;

    let mut changes = props.receive_properties_changed().await?;
    while changes.next().await.is_some() {
        print_status(bfm, json).await?;
    }
    Ok(())
}

async fn print_status(bfm: &BrainFM1Proxy<'_>, json: bool) -> Result<()> {
    let state = bfm.state().await.unwrap_or_else(|_| "idle".into());
    let track = bfm.track().await.unwrap_or_default();
    let name = track
        .get("name")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default();
    let timer = bfm.timer().await.unwrap_or_default();
    let phase = timer
        .get("phase")
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_else(|| "none".into());
    let remaining = timer
        .get("remaining")
        .and_then(|v| u64::try_from(v.clone()).ok())
        .unwrap_or(0);

    if json {
        // waybar's native shape: text, alt, tooltip, class.
        let text = if name.is_empty() {
            state.clone()
        } else {
            name.clone()
        };
        let tooltip = match phase.as_str() {
            "none" => state.clone(),
            p => format!(
                "{state} — {p} {}:{:02} left",
                remaining / 60,
                remaining % 60
            ),
        };
        let obj = serde_json::json!({
            "text": text,
            "alt": state,
            "tooltip": tooltip,
            "class": state,
        });
        println!("{obj}");
    } else if name.is_empty() {
        println!("{state}");
    } else {
        println!("{state}: {name}");
    }
    Ok(())
}
