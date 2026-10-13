//! How much the screen moves on its own. A wide or narrow window is lively: a quick spinner, queue marks that
//! pulse. The compact layout is calm: it is read over a phone link, where every repaint is paid for, so a spinner
//! steps slowly and a screen at rest draws the same frame again.
use std::time::Duration;

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const LIVELY_FRAME: Duration = Duration::from_millis(80);
/// Four steps a second still read as alive.
const CALM_FRAME: Duration = Duration::from_millis(250);
const LIVELY_TICK: Duration = Duration::from_millis(100);
const PULSE: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Lively,
    Calm,
}

impl Motion {
    fn spinner_frame(self) -> Duration {
        match self {
            Self::Lively => LIVELY_FRAME,
            Self::Calm => CALM_FRAME,
        }
    }

    /// How soon a screen that just changed is looked at again; calm, once per spinner step, since nothing on it
    /// moves faster.
    pub fn tick(self) -> Duration {
        match self {
            Self::Lively => LIVELY_TICK,
            Self::Calm => CALM_FRAME,
        }
    }

    /// The spinner of something loading, `elapsed` after the start.
    pub fn spinner(self, elapsed: Duration) -> &'static str {
        SPINNER[(elapsed.as_millis() / self.spinner_frame().as_millis()) as usize % SPINNER.len()]
    }

    /// The badge of a running pipeline: it may run for minutes, so calm keeps it on one step.
    pub fn running(self, elapsed: Duration) -> &'static str {
        match self {
            Self::Lively => self.spinner(elapsed),
            Self::Calm => SPINNER[0],
        }
    }

    /// Whether a mark that asks for attention is lit: lively, it breathes, bright one second and faded the next.
    pub fn lit(self, elapsed: Duration) -> bool {
        match self {
            Self::Lively => (elapsed.as_millis() / PULSE.as_millis()).is_multiple_of(2),
            Self::Calm => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn steps_a_second(motion: Motion) -> usize {
        let at = |ms: u64| motion.spinner(Duration::from_millis(ms));
        (1..=1000).filter(|&ms| at(ms) != at(ms - 1)).count()
    }

    #[test]
    fn the_spinner_steps_every_80_ms_when_lively_and_four_times_a_second_when_calm() {
        assert_eq!(steps_a_second(Motion::Lively), 12);
        assert_eq!(Motion::Lively.spinner(Duration::from_millis(79)), "⠋");
        assert_eq!(Motion::Lively.spinner(Duration::from_millis(80)), "⠙");
        assert_eq!(steps_a_second(Motion::Calm), 4);
    }

    #[test]
    fn a_changed_screen_is_looked_at_after_100_ms_when_lively_and_once_per_step_when_calm() {
        assert_eq!(Motion::Lively.tick(), Duration::from_millis(100));
        assert_eq!(Motion::Calm.tick(), Duration::from_millis(250));
        let drawn: Vec<&str> = (0..=20).map(|tick| Motion::Calm.spinner(Motion::Calm.tick() * tick)).collect();
        assert!(drawn.windows(2).all(|pair| pair[0] != pair[1]), "no tick draws the step already on screen: {drawn:?}");
        let turn: BTreeSet<&str> = drawn[..10].iter().copied().collect();
        assert_eq!(turn.len(), 10, "and none of the ten steps is skipped: {drawn:?}");
    }

    #[test]
    fn marks_breathe_once_a_second_and_a_running_badge_spins_only_when_lively() {
        let at = Duration::from_millis;
        assert_eq!([0, 999, 1000, 1999, 2000].map(|ms| Motion::Lively.lit(at(ms))), [true, true, false, false, true]);
        assert!((0..5000).step_by(250).all(|ms| Motion::Calm.lit(at(ms))));
        assert_ne!(Motion::Lively.running(at(0)), Motion::Lively.running(at(80)));
        assert!((0..5000).step_by(250).all(|ms| Motion::Calm.running(at(ms)) == "⠋"));
    }
}
