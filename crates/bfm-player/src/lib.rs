//! Audio engine.
//!
//! Brain.fm serves **progressive MP3** (no HLS), so a pure-Rust pipeline is
//! sufficient: no `mpv` subprocess, no GStreamer. Tracks run ~30 minutes, so
//! buffering is disk-backed rather than in memory.
//!
//! The crate is split so that almost all of it is testable without an audio
//! device:
//!
//! * [`schedule`] — generation tracking, prefetch timing, fades, the position
//!   clock. Pure logic, no dependencies, fully tested offline.
//! * [`engine`] — the rodio and stream-download adapter. Every third-party
//!   audio call lives there and nowhere else.

pub mod engine;
pub mod schedule;

pub use engine::AudioEngine;
pub use schedule::{DEFAULT_PREFETCH_LEAD, Decision, PositionClock, Schedule};

use std::time::Duration;

/// Drop the signing parameters from an audio URL so it is safe to log.
pub fn scrub(url: &str) -> &str {
    match url.find('?') {
        Some(i) => &url[..i],
        None => url,
    }
}

/// Instructions to the audio thread.
///
/// Generations are assigned internally by [`schedule::Schedule`], so callers
/// never manage them; they only need to honour the generation reported back on
/// [`Event::NearEnd`] when fetching the next track.
#[derive(Debug, Clone)]
pub enum Command {
    /// Replace whatever is playing with this track.
    ///
    /// The URL is signed — never log it unscrubbed.
    Load {
        url: String,
        length: Duration,
    },
    /// Append for gapless continuation behind the current track.
    Queue {
        url: String,
        length: Duration,
    },
    Play,
    Pause,
    Stop,
    SetVolume(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Started {
        generation: u64,
    },
    Position {
        secs: f64,
    },
    /// Time to fetch and [`Command::Queue`] the next track. Carries the
    /// generation so a late response can be discarded if the session moved on.
    NearEnd {
        generation: u64,
    },
    Ended {
        generation: u64,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("no audio output device available")]
    NoDevice,
    #[error("decode failed: {0}")]
    Decode(String),
}

/// Abstracts the backend so the daemon can be tested without real audio, and
/// so the decoder can be swapped without touching session logic.
pub trait Player: Send {
    fn handle(&mut self, cmd: Command) -> Result<(), PlayerError>;
    fn poll(&mut self) -> Vec<Event>;
}

/// Records commands and emits whatever it is told to. Used by the daemon's
/// tests, which must not open a device.
#[derive(Debug, Default)]
pub struct NullPlayer {
    pub received: Vec<Command>,
    pub pending: Vec<Event>,
}

impl NullPlayer {
    /// Queue an event for the next [`Player::poll`].
    pub fn push_event(&mut self, e: Event) {
        self.pending.push(e);
    }
}

impl Player for NullPlayer {
    fn handle(&mut self, cmd: Command) -> Result<(), PlayerError> {
        self.received.push(cmd);
        Ok(())
    }
    fn poll(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_player_records_commands() {
        let mut p = NullPlayer::default();
        p.handle(Command::Play).unwrap();
        p.handle(Command::SetVolume(0.5)).unwrap();
        assert_eq!(p.received.len(), 2);
        assert!(p.poll().is_empty());
    }

    #[test]
    fn null_player_drains_queued_events_once() {
        let mut p = NullPlayer::default();
        p.push_event(Event::Started { generation: 1 });

        assert_eq!(p.poll(), vec![Event::Started { generation: 1 }]);
        assert!(p.poll().is_empty(), "events are drained, not replayed");
    }

    /// A signed URL reaching a log is the realistic leak, so make the risk
    /// visible at the type level: Command's Debug carries the URL, and the
    /// daemon must scrub before logging.
    #[test]
    fn load_carries_the_track_length_for_prefetch_timing() {
        let c = Command::Load {
            url: "https://audio2.brain.fm/x.mp3?token=secret".into(),
            length: Duration::from_secs(1800),
        };
        match c {
            Command::Load { length, .. } => assert_eq!(length.as_secs(), 1800),
            _ => unreachable!(),
        }
    }
}
