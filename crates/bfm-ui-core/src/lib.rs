//! Shared client core for bfm front-ends.
//!
//! A GUI is just another `bfm-proto` client: the daemon owns the session, the
//! audio and the timers, and a front-end renders state and sends commands.
//! That is the whole reason a native GUI costs no backend work.
//!
//! This crate holds the parts neither GTK nor Qt should re-invent — the state
//! model, the formatting, and the bus plumbing — so each front-end is only
//! views.

pub mod browser;
pub mod client;
pub mod filters;
pub mod mode;
pub mod state;

pub use browser::{Found, find_session};
pub use client::{Client, Command, Update, needs_login, split_error};
pub use filters::{Catalog, Effort, Filters};
pub use mode::{SessionLength, SessionMode, fallback_lengths};
pub use state::{IDLE_LABEL, Playback, State, Timer, Track, format_duration};
