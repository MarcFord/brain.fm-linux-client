//! The order of questions in the start wizard.
//!
//! Kept separate from the widgets because the sequence branches — choosing
//! "Infinite" skips straight to starting, while "Timer" and "Intervals" each
//! need a follow-up — and branching buried in signal handlers is both easy
//! to get wrong and impossible to test.

use crate::mode::SessionMode;

/// One question in the flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Activity,
    Effort,
    Genres,
    /// Infinite, timer, or intervals.
    LengthKind,
    /// How many minutes, once "Timer" is chosen.
    TimerLength,
    /// Work, break and cycle counts, once "Intervals" is chosen.
    IntervalSetup,
}

impl Step {
    /// The first question. Everything else follows from an answer.
    pub const FIRST: Step = Step::Activity;

    /// Heading for the page.
    pub fn title(self) -> &'static str {
        match self {
            Self::Activity => "What are you doing?",
            Self::Effort => "How strong?",
            Self::Genres => "Any preference?",
            Self::LengthKind => "How long?",
            Self::TimerLength => "For how long?",
            Self::IntervalSetup => "Work and break",
        }
    }

    /// A line under the heading, because several of these questions are not
    /// self-explanatory.
    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Activity => "Pick the activity closest to your task.",
            Self::Effort => "Neural effect strength. High is the default.",
            Self::Genres => "Leave everything off and Brain.fm will choose.",
            Self::LengthKind => "Play until you stop, or set a timer.",
            Self::TimerLength => "The session ends when the timer does.",
            Self::IntervalSetup => "Alternating focus and rest, pomodoro style.",
        }
    }

    /// Whether answering this step advances on its own.
    ///
    /// Single-choice questions advance the moment you tap an option — that
    /// is what makes it feel like clicking through rather than filling in a
    /// form. Multi-select and numeric steps cannot, so they carry a button.
    pub fn advances_on_tap(self) -> bool {
        matches!(self, Self::Activity | Self::Effort | Self::LengthKind)
    }

    /// Label for the explicit continue button, where one is needed.
    pub fn continue_label(self) -> Option<&'static str> {
        match self {
            Self::Genres => Some("Continue"),
            Self::TimerLength | Self::IntervalSetup => Some("Start"),
            _ => None,
        }
    }
}

/// What follows `step`, given the length mode chosen so far.
///
/// `None` means there is nothing left to ask and the session should start.
pub fn next(step: Step, mode: SessionMode) -> Option<Step> {
    match step {
        Step::Activity => Some(Step::Effort),
        Step::Effort => Some(Step::Genres),
        Step::Genres => Some(Step::LengthKind),
        Step::LengthKind => match mode {
            // Nothing more to ask: start.
            SessionMode::Infinite => None,
            SessionMode::Timer { .. } => Some(Step::TimerLength),
            SessionMode::Intervals { .. } => Some(Step::IntervalSetup),
        },
        Step::TimerLength | Step::IntervalSetup => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INF: SessionMode = SessionMode::Infinite;
    const TIMER: SessionMode = SessionMode::Timer { minutes: 50 };
    const IVL: SessionMode = SessionMode::Intervals {
        work: 25,
        rest: 5,
        cycles: 4,
    };

    #[test]
    fn the_common_path_is_four_questions_then_start() {
        let mut step = Step::FIRST;
        let mut seen = vec![step];
        while let Some(n) = next(step, INF) {
            step = n;
            seen.push(step);
        }
        assert_eq!(
            seen,
            vec![Step::Activity, Step::Effort, Step::Genres, Step::LengthKind]
        );
    }

    #[test]
    fn choosing_a_timer_adds_one_more_question() {
        assert_eq!(next(Step::LengthKind, TIMER), Some(Step::TimerLength));
        assert_eq!(next(Step::TimerLength, TIMER), None);
    }

    #[test]
    fn choosing_intervals_adds_its_own_question() {
        assert_eq!(next(Step::LengthKind, IVL), Some(Step::IntervalSetup));
        assert_eq!(next(Step::IntervalSetup, IVL), None);
    }

    /// Infinite is the default and should be the shortest path: four taps
    /// and it plays.
    #[test]
    fn infinite_finishes_at_the_length_question() {
        assert_eq!(next(Step::LengthKind, INF), None);
    }

    /// Every path must terminate; a wizard that loops is a trap.
    #[test]
    fn every_path_terminates() {
        for mode in [INF, TIMER, IVL] {
            let mut step = Step::FIRST;
            let mut guard = 0;
            while let Some(n) = next(step, mode) {
                step = n;
                guard += 1;
                assert!(guard < 10, "{mode:?} did not terminate");
            }
        }
    }

    /// Single-choice questions advance on tap; anything multi-select or
    /// numeric needs a button, or the user can never finish answering.
    #[test]
    fn only_single_choice_steps_advance_on_tap() {
        assert!(Step::Activity.advances_on_tap());
        assert!(Step::Effort.advances_on_tap());
        assert!(Step::LengthKind.advances_on_tap());

        assert!(!Step::Genres.advances_on_tap());
        assert!(!Step::TimerLength.advances_on_tap());
        assert!(!Step::IntervalSetup.advances_on_tap());
    }

    /// A step that does not advance on tap MUST offer a button, otherwise
    /// the flow dead-ends.
    #[test]
    fn steps_that_do_not_advance_always_offer_a_button() {
        for step in [
            Step::Activity,
            Step::Effort,
            Step::Genres,
            Step::LengthKind,
            Step::TimerLength,
            Step::IntervalSetup,
        ] {
            assert_eq!(
                step.advances_on_tap(),
                step.continue_label().is_none(),
                "{step:?} must either advance on tap or carry a button, not both or neither"
            );
        }
    }

    /// The last question's button should say Start, not Continue — it is
    /// the one that actually does something.
    #[test]
    fn terminal_steps_say_start() {
        assert_eq!(Step::TimerLength.continue_label(), Some("Start"));
        assert_eq!(Step::IntervalSetup.continue_label(), Some("Start"));
        assert_eq!(Step::Genres.continue_label(), Some("Continue"));
    }

    #[test]
    fn every_step_explains_itself() {
        for step in [
            Step::Activity,
            Step::Effort,
            Step::Genres,
            Step::LengthKind,
            Step::TimerLength,
            Step::IntervalSetup,
        ] {
            assert!(!step.title().is_empty(), "{step:?} has no title");
            assert!(!step.subtitle().is_empty(), "{step:?} has no subtitle");
        }
    }
}
