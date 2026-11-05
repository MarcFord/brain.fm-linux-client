//! Session timers.
//!
//! Brain.fm content is already state-and-duration shaped, so a timer belongs
//! in the daemon rather than in any one client. Putting it here is what makes
//! `SetTimer("25m/5m x4")` and `TimerPhaseChanged` available to the CLI, a
//! status bar and a desktop plugin at once.
//!
//! Pure logic: the clock is supplied by the caller, so every case below is
//! testable without waiting.

use std::{fmt, time::Duration};

/// What a timer is counting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Work,
    Break,
    Done,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "work",
            Self::Break => "break",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimerSpec {
    pub work: Duration,
    /// `None` is a plain countdown rather than a work/break cycle.
    pub rest: Option<Duration>,
    /// How many work periods. Always at least 1.
    pub cycles: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Parse a duration like `90s`, `50m`, `1h`, `1h30m`.
fn parse_duration(s: &str) -> Result<Duration, ParseError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ParseError("empty duration".into()));
    }

    let mut total = 0u64;
    let mut digits = String::new();
    let mut saw_unit = false;

    for ch in s.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        let n: u64 = digits
            .parse()
            .map_err(|_| ParseError(format!("expected a number before '{ch}' in {s:?}")))?;
        digits.clear();
        saw_unit = true;
        total += match ch {
            'h' => n * 3600,
            'm' => n * 60,
            's' => n,
            other => return Err(ParseError(format!("unknown unit '{other}' in {s:?}"))),
        };
    }

    if !digits.is_empty() {
        return Err(ParseError(format!(
            "{s:?} is missing a unit (use h, m or s)"
        )));
    }
    if !saw_unit || total == 0 {
        return Err(ParseError(format!("{s:?} is not a usable duration")));
    }
    Ok(Duration::from_secs(total))
}

impl TimerSpec {
    /// Parse `"50m"`, `"25m/5m"`, or `"25m/5m x4"`.
    pub fn parse(spec: &str) -> Result<Self, ParseError> {
        let spec = spec.trim();

        let (body, cycles) = match spec.split_once('x') {
            Some((b, c)) => {
                let n: u32 = c
                    .trim()
                    .parse()
                    .map_err(|_| ParseError(format!("{:?} is not a cycle count", c.trim())))?;
                if n == 0 {
                    return Err(ParseError("cycle count must be at least 1".into()));
                }
                (b.trim(), n)
            }
            None => (spec, 1),
        };

        let (work, rest) = match body.split_once('/') {
            Some((w, r)) => (parse_duration(w)?, Some(parse_duration(r)?)),
            None => (parse_duration(body)?, None),
        };

        Ok(Self { work, rest, cycles })
    }

    /// Total time including breaks. The final break is not counted: the timer
    /// is done when the last work period ends.
    pub fn total(&self) -> Duration {
        let work = self.work * self.cycles;
        match self.rest {
            Some(r) if self.cycles > 1 => work + r * (self.cycles - 1),
            _ => work,
        }
    }
}

/// A running timer.
#[derive(Debug, Clone)]
pub struct Timer {
    spec: TimerSpec,
    phase: Phase,
    /// 1-based index of the work period in progress.
    cycle: u32,
    elapsed_in_phase: Duration,
}

impl Timer {
    pub fn start(spec: TimerSpec) -> Self {
        Self {
            spec,
            phase: Phase::Work,
            cycle: 1,
            elapsed_in_phase: Duration::ZERO,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn cycle(&self) -> u32 {
        self.cycle
    }

    pub fn cycles(&self) -> u32 {
        self.spec.cycles
    }

    /// Total configured length, breaks included.
    pub fn total(&self) -> Duration {
        self.spec.total()
    }

    pub fn is_done(&self) -> bool {
        self.phase == Phase::Done
    }

    pub fn remaining(&self) -> Duration {
        match self.phase {
            Phase::Done => Duration::ZERO,
            Phase::Work => self.spec.work.saturating_sub(self.elapsed_in_phase),
            Phase::Break => self
                .spec
                .rest
                .unwrap_or_default()
                .saturating_sub(self.elapsed_in_phase),
        }
    }

    /// Advance by `delta`. Returns the new phase if it changed, so the caller
    /// can emit `TimerPhaseChanged` exactly on transitions.
    pub fn advance(&mut self, delta: Duration) -> Option<Phase> {
        if self.phase == Phase::Done {
            return None;
        }
        self.elapsed_in_phase += delta;

        let mut changed = None;
        // A loop, because a long delta can cross more than one boundary.
        loop {
            let limit = match self.phase {
                Phase::Work => self.spec.work,
                Phase::Break => self.spec.rest.unwrap_or_default(),
                Phase::Done => break,
            };
            if self.elapsed_in_phase < limit {
                break;
            }
            self.elapsed_in_phase -= limit;

            self.phase = match self.phase {
                Phase::Work => {
                    if self.cycle >= self.spec.cycles {
                        Phase::Done
                    } else {
                        match self.spec.rest {
                            Some(_) => Phase::Break,
                            // No break configured: straight into the next
                            // work period.
                            None => {
                                self.cycle += 1;
                                Phase::Work
                            }
                        }
                    }
                }
                Phase::Break => {
                    self.cycle += 1;
                    Phase::Work
                }
                Phase::Done => Phase::Done,
            };
            changed = Some(self.phase);

            if self.phase == Phase::Done {
                self.elapsed_in_phase = Duration::ZERO;
                break;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mins(m: u64) -> Duration {
        Duration::from_secs(m * 60)
    }

    #[test]
    fn parses_single_units() {
        assert_eq!(
            TimerSpec::parse("90s").unwrap().work,
            Duration::from_secs(90)
        );
        assert_eq!(TimerSpec::parse("50m").unwrap().work, mins(50));
        assert_eq!(TimerSpec::parse("2h").unwrap().work, mins(120));
    }

    #[test]
    fn parses_compound_durations() {
        assert_eq!(TimerSpec::parse("1h30m").unwrap().work, mins(90));
    }

    #[test]
    fn parses_work_break_pairs() {
        let s = TimerSpec::parse("25m/5m").unwrap();
        assert_eq!(s.work, mins(25));
        assert_eq!(s.rest, Some(mins(5)));
        assert_eq!(s.cycles, 1);
    }

    #[test]
    fn parses_cycle_counts() {
        let s = TimerSpec::parse("25m/5m x4").unwrap();
        assert_eq!(s.cycles, 4);
        // Four work periods, three breaks between them.
        assert_eq!(s.total(), mins(25 * 4 + 5 * 3));
    }

    #[test]
    fn a_plain_countdown_has_no_break_in_its_total() {
        assert_eq!(TimerSpec::parse("50m").unwrap().total(), mins(50));
    }

    #[test]
    fn rejects_nonsense() {
        for bad in [
            "",
            "   ",
            "50",
            "50x",
            "abc",
            "25m/",
            "/5m",
            "25m/5m x0",
            "5y",
        ] {
            assert!(TimerSpec::parse(bad).is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn work_only_timer_finishes_after_one_period() {
        let mut t = Timer::start(TimerSpec::parse("10m").unwrap());
        assert_eq!(t.phase(), Phase::Work);
        assert_eq!(t.remaining(), mins(10));

        assert_eq!(t.advance(mins(9)), None, "no transition mid-phase");
        assert_eq!(t.remaining(), mins(1));

        assert_eq!(t.advance(mins(1)), Some(Phase::Done));
        assert!(t.is_done());
        assert_eq!(t.remaining(), Duration::ZERO);
    }

    #[test]
    fn pomodoro_alternates_work_and_break() {
        let mut t = Timer::start(TimerSpec::parse("25m/5m x2").unwrap());

        assert_eq!(t.advance(mins(25)), Some(Phase::Break));
        assert_eq!(t.cycle(), 1);
        assert_eq!(t.remaining(), mins(5));

        assert_eq!(t.advance(mins(5)), Some(Phase::Work));
        assert_eq!(t.cycle(), 2, "second work period");

        assert_eq!(
            t.advance(mins(25)),
            Some(Phase::Done),
            "last work period ends it"
        );
        assert!(t.is_done());
    }

    /// The daemon ticks every 250ms but can be starved; a long delta must not
    /// skip a phase.
    #[test]
    fn a_long_delta_crosses_several_boundaries() {
        let mut t = Timer::start(TimerSpec::parse("10m/5m x3").unwrap());
        // Jump past work, break, work, break in one go.
        assert_eq!(t.advance(mins(31)), Some(Phase::Work));
        assert_eq!(t.cycle(), 3);
    }

    #[test]
    fn a_delta_past_the_end_lands_on_done() {
        let mut t = Timer::start(TimerSpec::parse("10m/5m x2").unwrap());
        assert_eq!(t.advance(mins(999)), Some(Phase::Done));
        assert!(t.is_done());
    }

    #[test]
    fn advancing_a_finished_timer_does_nothing() {
        let mut t = Timer::start(TimerSpec::parse("1m").unwrap());
        t.advance(mins(1));
        assert_eq!(t.advance(mins(10)), None);
        assert!(t.is_done());
    }

    /// Without a break, cycles run back to back rather than stalling.
    #[test]
    fn cycles_without_a_break_chain_straight_through() {
        let mut t = Timer::start(TimerSpec::parse("10m x2").unwrap());
        assert_eq!(t.advance(mins(10)), Some(Phase::Work));
        assert_eq!(t.cycle(), 2);
        assert_eq!(t.advance(mins(10)), Some(Phase::Done));
    }
}
