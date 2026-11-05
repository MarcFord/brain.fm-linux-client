//! The rodio + stream-download adapter.
//!
//! **Every** third-party audio call in this project is in this file. The
//! decisions it carries out live in [`crate::schedule`], which has no audio
//! dependencies and is tested offline. If a backend API changes, the blast
//! radius is this file alone.
//!
//! Shape: one dedicated thread owns the output device and a current-thread
//! tokio runtime (needed because `StreamDownload` opens the HTTP stream
//! asynchronously). The daemon talks to it only over channels.

use std::{
    io::BufReader,
    sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
    thread,
    time::{Duration, Instant},
};

use rodio::{Decoder, Player as RodioPlayer};
use stream_download::{Settings, StreamDownload, storage::temp::TempStorageProvider};

use crate::{
    Command, Event, Player, PlayerError,
    schedule::{Decision, PositionClock, Schedule, clamp_volume},
};

/// How often to re-evaluate position when no command arrives.
const TICK: Duration = Duration::from_millis(250);

/// Handle to the audio thread. Dropping it stops the thread.
pub struct AudioEngine {
    commands: Sender<Command>,
    events: Receiver<Event>,
}

impl AudioEngine {
    /// Start the audio thread and open the default output device.
    pub fn spawn() -> Result<Self, PlayerError> {
        let (ctx, crx) = mpsc::channel::<Command>();
        let (etx, erx) = mpsc::channel::<Event>();
        let (readytx, readyrx) = mpsc::channel::<Result<(), PlayerError>>();

        thread::Builder::new()
            .name("bfm-audio".into())
            .spawn(move || run(crx, etx, readytx))
            .map_err(|e| PlayerError::Decode(e.to_string()))?;

        // Surface device-open failures to the caller rather than losing them
        // on a detached thread.
        match readyrx.recv() {
            Ok(Ok(())) => Ok(Self {
                commands: ctx,
                events: erx,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(PlayerError::NoDevice),
        }
    }
}

impl Player for AudioEngine {
    fn handle(&mut self, cmd: Command) -> Result<(), PlayerError> {
        self.commands
            .send(cmd)
            .map_err(|_| PlayerError::Decode("audio thread has stopped".into()))
    }

    fn poll(&mut self) -> Vec<Event> {
        self.events.try_iter().collect()
    }
}

fn run(commands: Receiver<Command>, events: Sender<Event>, ready: Sender<Result<(), PlayerError>>) {
    // Opening the device can fail (no PipeWire/Pulse/ALSA, or none free).
    let device = match rodio::DeviceSinkBuilder::open_default_sink() {
        Ok(d) => d,
        Err(e) => {
            let _ = ready.send(Err(PlayerError::NoDevice));
            tracing::error!(error = %e, "could not open an audio output device");
            return;
        }
    };
    let player = RodioPlayer::connect_new(device.mixer());

    // StreamDownload opens the HTTP stream asynchronously; this thread owns a
    // single-threaded runtime for that and nothing else.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = ready.send(Err(PlayerError::Decode(e.to_string())));
            return;
        }
    };
    let _ = ready.send(Ok(()));

    let mut schedule = Schedule::default();
    let mut clock = PositionClock::new();

    loop {
        match commands.recv_timeout(TICK) {
            Ok(cmd) => {
                if !apply(cmd, &player, &rt, &mut schedule, &mut clock, &events) {
                    return; // channel closed, or Stop with nobody listening
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        tick(&player, &mut schedule, &clock, &events);
    }
}

/// Returns false when the thread should exit.
fn apply(
    cmd: Command,
    player: &RodioPlayer,
    rt: &tokio::runtime::Runtime,
    schedule: &mut Schedule,
    clock: &mut PositionClock,
    events: &Sender<Event>,
) -> bool {
    match cmd {
        Command::Load { url, length } => {
            player.stop();
            clock.reset();
            let generation = schedule.begin(length);
            match open(&url, rt) {
                Ok(source) => {
                    player.append(source);
                    player.play();
                    clock.resume_at(Instant::now());
                    events.send(Event::Started { generation }).is_ok()
                }
                Err(e) => emit_error(events, e),
            }
        }

        Command::Queue { url, .. } => match open(&url, rt) {
            // Appending behind the current source is what makes playback
            // gapless; rodio plays the queue in order.
            Ok(source) => {
                player.append(source);
                true
            }
            Err(e) => emit_error(events, e),
        },

        Command::Play => {
            player.play();
            clock.resume_at(Instant::now());
            true
        }

        Command::Pause => {
            player.pause();
            clock.pause_at(Instant::now());
            true
        }

        Command::Stop => {
            player.stop();
            clock.reset();
            schedule.stop();
            true
        }

        Command::SetVolume(v) => {
            player.set_volume(clamp_volume(v));
            true
        }
    }
}

fn tick(
    player: &RodioPlayer,
    schedule: &mut Schedule,
    clock: &PositionClock,
    events: &Sender<Event>,
) {
    if !clock.is_running() {
        return;
    }
    let position = clock.position_at(Instant::now());
    let _ = events.send(Event::Position {
        secs: position.as_secs_f64(),
    });

    if let Decision::Prefetch { generation } = schedule.on_position(position) {
        let _ = events.send(Event::NearEnd { generation });
    }

    // rodio empties the queue when the last source finishes.
    if player.empty() {
        let generation = schedule.generation();
        schedule.stop();
        let _ = events.send(Event::Ended { generation });
    }
}

/// Open a signed audio URL as a decodable source.
///
/// Disk-backed storage, deliberately: a 30-minute MP3 is 30-60 MB and we may
/// hold two at once across a gapless transition.
fn open(
    url: &str,
    rt: &tokio::runtime::Runtime,
) -> Result<Decoder<BufReader<StreamDownload<TempStorageProvider>>>, PlayerError> {
    let parsed = url
        .parse()
        .map_err(|_| PlayerError::Decode("audio URL did not parse".into()))?;

    let reader = rt
        .block_on(StreamDownload::new_http(
            parsed,
            TempStorageProvider::new(),
            Settings::default(),
        ))
        // The URL is signed; its query string must never reach a log or an
        // error message.
        .map_err(|_| PlayerError::Decode("could not open the audio stream".into()))?;

    // rodio decodes from a BufReader, not a bare reader.
    Decoder::try_from(BufReader::new(reader)).map_err(|e| PlayerError::Decode(e.to_string()))
}

fn emit_error(events: &Sender<Event>, e: PlayerError) -> bool {
    events
        .send(Event::Error {
            message: e.to_string(),
        })
        .is_ok()
}
