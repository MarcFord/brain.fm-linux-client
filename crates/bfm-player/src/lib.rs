//! Audio engine.
//!
//! Brain.fm serves **progressive MP3** (no HLS), so a pure-Rust pipeline is
//! sufficient: no `mpv` subprocess, no GStreamer. Tracks run ~30 minutes, so
//! buffering a whole one in RAM would cost 30–60 MB — use bounded or
//! temp-file-backed storage.
//!
//! The engine owns the output device on its own thread and is driven entirely
//! by [`Command`], reporting back as [`Event`]. Keeping that boundary typed is
//! what lets the daemon stay testable without an audio device.

#[derive(Debug, Clone)]
pub enum Command {
    /// Begin a track. The URL is signed; never log it unscrubbed.
    Load {
        url: String,
        generation: u64,
    },
    /// Append for gapless continuation.
    Queue {
        url: String,
        generation: u64,
    },
    Play,
    Pause,
    Stop,
    SetVolume(f32),
    /// Drop any in-flight load whose generation is older than this.
    CancelOlderThan(u64),
}

#[derive(Debug, Clone)]
pub enum Event {
    Started {
        generation: u64,
    },
    Position {
        secs: f64,
    },
    /// Fired with enough lead time to fetch and queue the next track.
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

/// Records commands and emits nothing. Used by the daemon's tests.
#[derive(Debug, Default)]
pub struct NullPlayer {
    pub received: Vec<Command>,
}

impl Player for NullPlayer {
    fn handle(&mut self, cmd: Command) -> Result<(), PlayerError> {
        self.received.push(cmd);
        Ok(())
    }
    fn poll(&mut self) -> Vec<Event> {
        Vec::new()
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
}
