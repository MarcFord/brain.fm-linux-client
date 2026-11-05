//! Playback scheduling — pure logic, no audio dependencies.
//!
//! Everything here is decided without touching a device or the network, so it
//! is fully testable offline. The rodio adapter in [`crate::engine`] is a thin
//! shell around these decisions.
//!
//! Two things make this worth separating:
//!
//! * **Staleness.** A session change must not be resurrected by a network
//!   response that was already in flight. Every load carries a generation, and
//!   events from an older one are dropped.
//! * **Prefetch timing.** Brain.fm tracks run ~30 minutes, so the lead time
//!   before the end is much larger than a typical playlist's, and fetching the
//!   next track is itself slow. Getting this wrong means an audible gap.

use std::time::Duration;

/// How long before a track ends to fetch the next one.
///
/// Generous because a 30-minute MP3 takes real time to start streaming. The
/// cost of being early is a buffered file; the cost of being late is silence.
pub const DEFAULT_PREFETCH_LEAD: Duration = Duration::from_secs(90);

/// What the engine should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Fetch and queue the next track now.
    Prefetch { generation: u64 },
    /// Nothing to do.
    Idle,
}

#[derive(Debug, Clone, Copy)]
struct Current {
    generation: u64,
    length: Duration,
    prefetched: bool,
}

/// Tracks which load is current and when to queue the next one.
#[derive(Debug)]
pub struct Schedule {
    generation: u64,
    current: Option<Current>,
    prefetch_lead: Duration,
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new(DEFAULT_PREFETCH_LEAD)
    }
}

impl Schedule {
    pub fn new(prefetch_lead: Duration) -> Self {
        Self {
            generation: 0,
            current: None,
            prefetch_lead,
        }
    }

    /// Begin a new track, invalidating anything in flight. Returns the
    /// generation to tag the load with.
    pub fn begin(&mut self, length: Duration) -> u64 {
        self.generation += 1;
        self.current = Some(Current {
            generation: self.generation,
            length,
            prefetched: false,
        });
        self.generation
    }

    /// Invalidate everything in flight without starting a track.
    pub fn stop(&mut self) {
        self.generation += 1;
        self.current = None;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// True when an event tagged `generation` belongs to a superseded load and
    /// must be ignored.
    pub fn is_stale(&self, generation: u64) -> bool {
        generation != self.generation
    }

    /// Feed the current playback position. Fires [`Decision::Prefetch`] exactly
    /// once per track.
    pub fn on_position(&mut self, position: Duration) -> Decision {
        let Some(cur) = self.current.as_mut() else {
            return Decision::Idle;
        };
        if cur.prefetched {
            return Decision::Idle;
        }

        // For a track shorter than the lead time, prefetch at the midpoint
        // rather than immediately: starting two streams at once on a slow
        // connection is worse than starting the second one slightly late.
        let trigger = cur
            .length
            .checked_sub(self.prefetch_lead)
            .unwrap_or(cur.length / 2);

        if position >= trigger {
            cur.prefetched = true;
            Decision::Prefetch {
                generation: cur.generation,
            }
        } else {
            Decision::Idle
        }
    }
}

/// Clamp a requested volume into the range the output accepts.
pub fn clamp_volume(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// Linear fade multiplier in `0.0..=1.0` for `elapsed` into a fade of
/// `duration`. A zero-length fade is already complete.
pub fn fade_factor(elapsed: Duration, duration: Duration) -> f32 {
    if duration.is_zero() {
        return 1.0;
    }
    (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn generation_advances_and_supersedes() {
        let mut s = Schedule::default();
        let first = s.begin(secs(1800));
        let second = s.begin(secs(1800));

        assert_ne!(first, second);
        assert!(s.is_stale(first), "the earlier load must be ignored");
        assert!(!s.is_stale(second));
    }

    #[test]
    fn stop_invalidates_everything_in_flight() {
        let mut s = Schedule::default();
        let g = s.begin(secs(1800));
        s.stop();
        assert!(s.is_stale(g));
        assert_eq!(s.on_position(secs(1799)), Decision::Idle);
    }

    /// A 30-minute track prefetches at 28:30, not near the end of a short one.
    #[test]
    fn prefetch_fires_one_lead_before_the_end() {
        let mut s = Schedule::new(secs(90));
        let g = s.begin(secs(1800));

        assert_eq!(s.on_position(secs(1709)), Decision::Idle);
        assert_eq!(
            s.on_position(secs(1710)),
            Decision::Prefetch { generation: g }
        );
    }

    #[test]
    fn prefetch_fires_only_once_per_track() {
        let mut s = Schedule::new(secs(90));
        s.begin(secs(1800));

        assert!(matches!(
            s.on_position(secs(1750)),
            Decision::Prefetch { .. }
        ));
        assert_eq!(s.on_position(secs(1760)), Decision::Idle);
        assert_eq!(s.on_position(secs(1799)), Decision::Idle);
    }

    #[test]
    fn a_new_track_re_arms_prefetch() {
        let mut s = Schedule::new(secs(90));
        s.begin(secs(1800));
        assert!(matches!(
            s.on_position(secs(1750)),
            Decision::Prefetch { .. }
        ));

        let g2 = s.begin(secs(1800));
        assert_eq!(
            s.on_position(secs(1750)),
            Decision::Prefetch { generation: g2 }
        );
    }

    /// Lead longer than the track must not underflow into "prefetch instantly".
    #[test]
    fn short_track_prefetches_at_the_midpoint_not_immediately() {
        let mut s = Schedule::new(secs(90));
        let g = s.begin(secs(60));

        assert_eq!(s.on_position(secs(29)), Decision::Idle);
        assert_eq!(
            s.on_position(secs(30)),
            Decision::Prefetch { generation: g }
        );
    }

    #[test]
    fn position_without_a_track_is_idle() {
        let mut s = Schedule::default();
        assert_eq!(s.on_position(secs(10)), Decision::Idle);
    }

    #[test]
    fn volume_is_clamped_and_nan_safe() {
        assert_eq!(clamp_volume(0.5), 0.5);
        assert_eq!(clamp_volume(-1.0), 0.0);
        assert_eq!(clamp_volume(2.0), 1.0);
        assert_eq!(clamp_volume(f32::NAN), 0.0);
    }

    #[test]
    fn fade_ramps_from_zero_to_one() {
        let d = secs(4);
        assert_eq!(fade_factor(secs(0), d), 0.0);
        assert_eq!(fade_factor(secs(2), d), 0.5);
        assert_eq!(fade_factor(secs(4), d), 1.0);
        assert_eq!(fade_factor(secs(9), d), 1.0, "never exceeds 1");
    }

    #[test]
    fn zero_length_fade_is_complete() {
        assert_eq!(fade_factor(secs(0), Duration::ZERO), 1.0);
    }
}

/// Tracks elapsed *playing* time, excluding paused intervals.
///
/// We derive position from a monotonic clock rather than asking the audio
/// backend, so the scheduling logic stays testable and independent of which
/// decoder is underneath.
#[derive(Debug, Clone)]
pub struct PositionClock {
    accumulated: Duration,
    running_since: Option<std::time::Instant>,
}

impl Default for PositionClock {
    fn default() -> Self {
        Self::new()
    }
}

impl PositionClock {
    pub fn new() -> Self {
        Self {
            accumulated: Duration::ZERO,
            running_since: None,
        }
    }

    pub fn reset(&mut self) {
        self.accumulated = Duration::ZERO;
        self.running_since = None;
    }

    pub fn resume_at(&mut self, now: std::time::Instant) {
        if self.running_since.is_none() {
            self.running_since = Some(now);
        }
    }

    pub fn pause_at(&mut self, now: std::time::Instant) {
        if let Some(started) = self.running_since.take() {
            self.accumulated += now.saturating_duration_since(started);
        }
    }

    pub fn position_at(&self, now: std::time::Instant) -> Duration {
        match self.running_since {
            Some(started) => self.accumulated + now.saturating_duration_since(started),
            None => self.accumulated,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running_since.is_some()
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn a_fresh_clock_is_at_zero_and_stopped() {
        let c = PositionClock::new();
        assert_eq!(c.position_at(Instant::now()), Duration::ZERO);
        assert!(!c.is_running());
    }

    #[test]
    fn paused_time_does_not_count_toward_position() {
        let t0 = Instant::now();
        let mut c = PositionClock::new();

        c.resume_at(t0);
        c.pause_at(t0 + Duration::from_secs(10));
        // ten minutes pass while paused
        let later = t0 + Duration::from_secs(610);
        assert_eq!(c.position_at(later), Duration::from_secs(10));

        c.resume_at(later);
        assert_eq!(
            c.position_at(later + Duration::from_secs(5)),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn repeated_resume_does_not_double_count() {
        let t0 = Instant::now();
        let mut c = PositionClock::new();
        c.resume_at(t0);
        c.resume_at(t0 + Duration::from_secs(5));
        assert_eq!(
            c.position_at(t0 + Duration::from_secs(10)),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn pause_while_already_paused_is_a_no_op() {
        let t0 = Instant::now();
        let mut c = PositionClock::new();
        c.resume_at(t0);
        c.pause_at(t0 + Duration::from_secs(3));
        c.pause_at(t0 + Duration::from_secs(99));
        assert_eq!(
            c.position_at(t0 + Duration::from_secs(99)),
            Duration::from_secs(3)
        );
    }

    #[test]
    fn reset_clears_a_running_clock() {
        let t0 = Instant::now();
        let mut c = PositionClock::new();
        c.resume_at(t0);
        c.reset();
        assert_eq!(c.position_at(t0 + Duration::from_secs(10)), Duration::ZERO);
        assert!(!c.is_running());
    }
}
