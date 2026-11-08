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
    /// How many work periods, or `None` for intervals that repeat until the
    /// user stops them. `Some` is always at least 1.
    ///
    /// Unbounded exists because the web player's intervals mode simply keeps
    /// going, and nothing here could express that: a missing count means one
    /// cycle, and `x0` is rejected, so a client could only approximate it with
    /// an absurd count like `x99`.
    pub cycles: Option<u32>,
}

/// Which shape a spec describes. Exposed on the `Timer` property because a
/// countdown and a one-cycle interval are otherwise indistinguishable -- both
/// report `cycles=1, phase="work"` -- so a client could not restore what the
/// user actually chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Countdown,
    Intervals,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Countdown => "countdown",
            Mode::Intervals => "intervals",
        }
    }
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
        // Case-insensitive: `7H` and `7h` differing was a gratuitous
        // inconsistency, and `7h` is how one naturally writes sleep's
        // seven-hour suggestion.
        total += match ch.to_ascii_lowercase() {
            'h' => n * 3600,
            'm' => n * 60,
            's' => n,
            _ => return Err(ParseError(format!("unknown unit '{ch}' in {s:?}"))),
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
    /// Parse `"50m"`, `"25m/5m"`, `"25m/5m x4"`, or `"25m/5m x*"`.
    ///
    /// Units are `h`, `m`, `s`, in any case, and may be compounded
    /// (`"1h30m"`). A count of `*` or `inf` means repeat until stopped.
    ///
    /// A missing count still means **one** cycle rather than endless, even
    /// though endless is arguably the more natural reading of `"25m/5m"`.
    /// Changing that would silently alter what every existing client gets from
    /// a spec it already sends, so endless has to be asked for explicitly.
    pub fn parse(spec: &str) -> Result<Self, ParseError> {
        let spec = spec.trim();

        let (body, cycles) = match spec.rfind(['x', 'X']) {
            Some(i) => {
                let (b, c) = spec.split_at(i);
                let c = c[1..].trim();
                let cycles = match c.to_ascii_lowercase().as_str() {
                    "*" | "inf" | "infinite" => None,
                    _ => {
                        let n: u32 = c
                            .parse()
                            .map_err(|_| ParseError(format!("{c:?} is not a cycle count")))?;
                        if n == 0 {
                            return Err(ParseError(
                                "cycle count must be at least 1, or '*' for endless".into(),
                            ));
                        }
                        Some(n)
                    }
                };
                (b.trim(), cycles)
            }
            None => (spec, Some(1)),
        };

        let (work, rest) = match body.split_once('/') {
            Some((w, r)) => (parse_duration(w)?, Some(parse_duration(r)?)),
            None => (parse_duration(body)?, None),
        };

        if cycles.is_none() && rest.is_none() {
            return Err(ParseError(
                "an endless count needs a work/break pair, as in \"25m/5m x*\"".into(),
            ));
        }

        Ok(Self { work, rest, cycles })
    }

    /// Total time including breaks. The final break is not counted: the timer
    /// is done when the last work period ends.
    ///
    /// Zero for an endless spec, which has no total by definition. Clients
    /// should read `unbounded` rather than inferring anything from a zero.
    pub fn total(&self) -> Duration {
        let Some(cycles) = self.cycles else {
            return Duration::ZERO;
        };
        let work = self.work * cycles;
        match self.rest {
            Some(r) if cycles > 1 => work + r * (cycles - 1),
            _ => work,
        }
    }

    pub fn mode(&self) -> Mode {
        match self.rest {
            Some(_) => Mode::Intervals,
            None => Mode::Countdown,
        }
    }

    pub fn is_unbounded(&self) -> bool {
        self.cycles.is_none()
    }
}

/// A running timer.
#[derive(Debug, Clone)]
pub struct Timer {
    spec: TimerSpec,
    /// The spec exactly as the client sent it, so it can be read back. A
    /// client that cannot see what it set cannot restore the user's choice
    /// after a restart.
    text: String,
    phase: Phase,
    /// 1-based index of the work period in progress.
    cycle: u32,
    elapsed_in_phase: Duration,
}

impl Timer {
    pub fn start(spec: TimerSpec) -> Self {
        Self::start_with_text(spec, String::new())
    }

    pub fn start_with_text(spec: TimerSpec, text: String) -> Self {
        Self {
            spec,
            text,
            phase: Phase::Work,
            cycle: 1,
            elapsed_in_phase: Duration::ZERO,
        }
    }

    /// The spec as set, for clients restoring their own UI state.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn mode(&self) -> Mode {
        self.spec.mode()
    }

    pub fn is_unbounded(&self) -> bool {
        self.spec.is_unbounded()
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn cycle(&self) -> u32 {
        self.cycle
    }

    /// Configured cycle count, or 0 when endless. Pair it with
    /// `is_unbounded` rather than reading a 0 as "none".
    pub fn cycles(&self) -> u32 {
        self.spec.cycles.unwrap_or(0)
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
                    // `None` is endless, so the work period never completes
                    // the timer; it always rolls into the next break.
                    if self.spec.cycles.is_some_and(|n| self.cycle >= n) {
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
        assert_eq!(s.cycles, Some(1));
    }

    #[test]
    fn parses_cycle_counts() {
        let s = TimerSpec::parse("25m/5m x4").unwrap();
        assert_eq!(s.cycles, Some(4));
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
    fn endless_intervals_parse_and_never_finish() {
        for spec in ["25m/5m x*", "25m/5m xinf", "25m/5m X*"] {
            let s = TimerSpec::parse(spec).unwrap_or_else(|e| panic!("{spec:?}: {e}"));
            assert!(s.is_unbounded(), "{spec:?} should be endless");
            assert_eq!(s.total(), Duration::ZERO, "{spec:?} has no total");
        }

        // The point of endless: a long run never reaches Done. Twenty hours
        // of 25/5 cycles is 40 cycles, which any bounded spec would finish.
        let mut t = Timer::start(TimerSpec::parse("25m/5m x*").unwrap());
        t.advance(Duration::from_secs(20 * 3600));
        assert!(!t.is_done(), "an endless timer must never finish");
        assert!(t.cycle() > 10, "it should have cycled many times");
        assert_eq!(t.cycles(), 0, "cycles reads 0 when endless");
        assert!(t.is_unbounded());
    }

    #[test]
    fn endless_needs_a_break_to_cycle_through() {
        // `50m x*` would be an endless single phase, which is just "no timer"
        // wearing a costume.
        assert!(TimerSpec::parse("50m x*").is_err());
    }

    #[test]
    fn a_countdown_and_a_one_cycle_interval_are_distinguishable() {
        // Both report cycles=1 and phase=work, which is exactly why `mode`
        // had to exist: a client could not tell them apart.
        let countdown = TimerSpec::parse("50m").unwrap();
        let interval = TimerSpec::parse("25m/5m x1").unwrap();
        assert_eq!(countdown.cycles, interval.cycles);
        assert_eq!(countdown.mode(), Mode::Countdown);
        assert_eq!(interval.mode(), Mode::Intervals);
        assert_eq!(countdown.mode().as_str(), "countdown");
        assert_eq!(interval.mode().as_str(), "intervals");
    }

    #[test]
    fn units_and_the_cycle_marker_are_case_insensitive() {
        assert_eq!(
            TimerSpec::parse("7H").unwrap(),
            TimerSpec::parse("7h").unwrap()
        );
        assert_eq!(
            TimerSpec::parse("25m/5m X4").unwrap(),
            TimerSpec::parse("25m/5m x4").unwrap()
        );
        assert_eq!(
            TimerSpec::parse("1H30M").unwrap().work,
            Duration::from_secs(5400)
        );
    }

    #[test]
    fn a_timer_remembers_the_spec_it_was_given() {
        let t = Timer::start_with_text(
            TimerSpec::parse("25m/5m x4").unwrap(),
            "25m/5m x4".to_owned(),
        );
        assert_eq!(t.text(), "25m/5m x4");
    }

    #[test]
    fn cycles_without_a_break_chain_straight_through() {
        let mut t = Timer::start(TimerSpec::parse("10m x2").unwrap());
        assert_eq!(t.advance(mins(10)), Some(Phase::Work));
        assert_eq!(t.cycle(), 2);
        assert_eq!(t.advance(mins(10)), Some(Phase::Done));
    }
}
