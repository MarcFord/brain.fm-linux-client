//! `bfm` — a thin client for `bfmd`.
//!
//! Every subcommand here is a D-Bus call. There is no API or audio code in
//! this crate by design: the daemon is the product, and this binary has no
//! privileges a third-party plugin lacks.

use bfm_proto::{BrainFM1Proxy, auth_method};
use bfm_ui_core::browser;
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

    // Adoption only parses the token; it does not prove the session is live.
    // A token scraped from a browser profile can be stale, because Chrome
    // flushes localStorage lazily. Verify now, so failure lands here with an
    // explanation rather than later on `bfm play` as a puzzle.
    match bfm.list_mental_states().await {
        Ok(_) => println!("signed in"),
        Err(e) if e.to_string().contains(bfm_proto::error_code::NEEDS_LOGIN) => {
            bail!(
                "that session is no longer valid.\n\
                 If it came from a browser profile it may be stale — Chrome writes \n\
                 localStorage to disk lazily. Sign in again in the browser, close the \n\
                 tab so it flushes, and retry."
            );
        }
        // A network blip should not make a good login look like a bad one.
        Err(e) => {
            println!("signed in (could not reach Brain.fm to verify: {e})");
        }
    }
    Ok(())
}

/// The only route for SSO accounts: let the real web app in a real browser
/// do the provider login, then adopt the session it produced. See
/// `docs/auth.md` for why we cannot do the OAuth ourselves.
///
/// The session is read straight out of the browser profile, so the user never
/// has to open DevTools or copy anything. Pasting remains as a fallback for
/// browsers we cannot read.
async fn browser_handoff(bfm: &BrainFM1Proxy<'_>) -> Result<()> {
    const URL: &str = "https://my.brain.fm/";

    // Already signed in? Then there is nothing to do but pick it up.
    if let Some(found) = browser::find_session() {
        println!("Found a Brain.fm session in {}.", found.profile);
        bfm.login(auth_method::TOKEN, "", &found.token).await?;
        return Ok(());
    }

    // `xdg-open` chats on stdout, and it would land in the middle of the
    // prompt below.
    let _ = std::process::Command::new("xdg-open")
        .arg(URL)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    println!("Opened {URL}");
    println!("Sign in with whichever provider your account uses, then come back here.");
    print!("Press Enter once you are signed in… ");
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;

    if let Some(found) = browser::find_session() {
        println!("Found a Brain.fm session in {}.", found.profile);
        bfm.login(auth_method::TOKEN, "", &found.token).await?;
        return Ok(());
    }

    // Unreadable profile (Firefox, a snap/flatpak browser, a custom data
    // dir). Fall back to the manual route.
    println!();
    println!("Could not read a session from any local browser profile.");
    println!("Falling back to copy-and-paste. In that tab, open DevTools (F12) and run:");
    println!();
    println!("    JSON.parse(JSON.parse(localStorage['persist:auth']).token)");
    println!();
    println!("Copy the RESULT — a long string of three dot-separated parts starting");
    println!("with `eyJ` — not the command itself.");

    let token =
        rpassword::prompt_password("session token: ").wrap_err("could not read the token")?;
    let token = token.trim().trim_matches('"').trim();
    validate_pasted_token(token)?;
    bfm.login(auth_method::TOKEN, "", token).await?;
    Ok(())
}

/// Check a pasted value looks like a JWT before handing it to the daemon.
///
/// Without this the daemon answers with a base64 error naming an offset,
/// which tells the user nothing about what they actually did wrong.
fn validate_pasted_token(token: &str) -> Result<()> {
    if token.is_empty() {
        bail!("no token entered");
    }
    if token.contains('(') || token.contains(' ') || token.starts_with("JSON") {
        bail!(
            "that looks like the JavaScript snippet, not its result.\n\
             Run it in the DevTools console and copy what it prints — a long \n\
             string beginning with `eyJ`."
        );
    }
    if token.split('.').count() != 3 {
        bail!(
            "that is not a session token: expected three dot-separated parts, found {}",
            token.split('.').count()
        );
    }
    if !token.starts_with("eyJ") {
        bail!("that is not a session token: it should begin with `eyJ`");
    }
    Ok(())
}

/// Did a daemon actually answer?
///
/// Every property read in `print_status` falls back to a default on error,
/// which is right for a daemon that is up but has nothing playing, and wrong
/// for one that is not there at all: a dead daemon rendered as a healthy
/// `idle`, so a broken install looked identical to a working one. One cheap
/// probe separates the two.
enum Reach {
    Up,
    Down(zbus::Error),
}

async fn status(
    conn: &zbus::Connection,
    bfm: &BrainFM1Proxy<'_>,
    json: bool,
    watch: bool,
) -> Result<()> {
    // `--json` and `--watch` both want an absent daemon reported in-band
    // rather than as a failed command: waybar renders nothing at all if the
    // process exits, and a watcher should recover when the daemon returns.
    // A plain one-shot `bfm status` is the opposite — it is what a person or
    // a script runs to find out whether this works, so it must fail loudly.
    let reach = print_status(bfm, json, json || watch).await?;
    if let Reach::Down(e) = reach
        && !watch
        && !json
    {
        // Deliberately not `bail!`: an absent daemon is an expected state of
        // the user's machine, not a fault in this binary, and color_eyre
        // would dress it up as a crash with a source location and a
        // backtrace hint. That framing buries the one thing the reader
        // needs, which is what to go and check.
        eprintln!(
            "no bfm daemon answered on the session bus.\n\
             \x20 {e}\n\n\
             bfmd is normally D-Bus activated, so this usually means the \
             service file is missing or the daemon failed to start:\n\
             \x20 is it installed?  ls \"${{XDG_DATA_HOME:-$HOME/.local/share}}/dbus-1/services/dev.marcford.bfm.service\"\n\
             \x20 does it run?      bfmd --version\n\
             \x20 stale bus?        busctl --user call org.freedesktop.DBus \
             /org/freedesktop/DBus org.freedesktop.DBus ReloadConfig\n\
             \x20 what did it say?  journalctl --user -u bfmd -n 20"
        );
        std::process::exit(1);
    }
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

    let mut prop_changes = props.receive_properties_changed().await?;

    // Watching property changes alone leaves a watcher silently wrong in both
    // directions, because a daemon that is not running emits nothing:
    //
    //   * Started with no daemon, it printed one `unavailable` and then stayed
    //     silent forever. Since it never exits, nothing retried it.
    //   * If the daemon died mid-stream it printed nothing at all, so a bar
    //     went on showing `playing` indefinitely for a daemon that was gone.
    //
    // NameOwnerChanged is the missing signal: it fires when the daemon takes
    // the bus name and when it drops it, including a crash.
    let dbus = zbus::fdo::DBusProxy::new(conn).await?;
    let mut owner_changes = dbus.receive_name_owner_changed().await?;

    loop {
        tokio::select! {
            change = prop_changes.next() => {
                if change.is_none() { break; }
                print_status(bfm, json, true).await?;
            }
            signal = owner_changes.next() => {
                let Some(signal) = signal else { break };
                let args = signal.args()?;
                if args.name.as_str() != bfm_proto::BUS_NAME {
                    continue;
                }
                if args.new_owner.is_some() {
                    // Back. Reading properties is safe here: the daemon
                    // already owns the name, so this cannot activate anything.
                    print_status(bfm, json, true).await?;
                } else {
                    // Gone. Deliberately does *not* read a property. The name
                    // is D-Bus activatable, so a read would start a fresh
                    // daemon merely because something was watching — a status
                    // bar must not resurrect the service it is reporting on.
                    print_unavailable(json);
                }
            }
        }
    }
    Ok(())
}

/// The one place the "no daemon" line is defined, so the stream and the
/// one-shot path cannot describe an outage differently.
fn print_unavailable(json: bool) {
    if json {
        let obj = serde_json::json!({
            "text": "unavailable",
            "alt": "unavailable",
            "tooltip": "no bfm daemon on the session bus",
            "class": "unavailable",
        });
        println!("{obj}");
    } else {
        println!("unavailable");
    }
}

/// `render_down` decides whether an absent daemon is written to stdout at
/// all. A one-shot plain `bfm status` must print nothing and fail instead,
/// or a script reading stdout would treat the outage as a state.
async fn print_status(bfm: &BrainFM1Proxy<'_>, json: bool, render_down: bool) -> Result<Reach> {
    if let Err(e) = bfm.interface_version().await {
        if render_down {
            print_unavailable(json);
        }
        return Ok(Reach::Down(e));
    }

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
    Ok(Reach::Up)
}
