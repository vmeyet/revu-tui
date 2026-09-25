//! A move key held down: the terminal sends it again and again, and each move grows the longer it lasts.
use super::App;
use crossterm::event::KeyCode;
use std::time::{Duration, Instant};

/// Longer than a terminal's key repeat (15 to 90 ms on macOS), shorter than two presses by hand.
const GAP: Duration = Duration::from_millis(100);
/// How long a key is held before each move covers more rows, longest first.
const STEPS: [(Duration, isize); 2] = [(Duration::from_secs(1), 4), (Duration::from_millis(300), 2)];

/// The same key pressed at short intervals since `since`, last at `last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repeat {
    code: KeyCode,
    since: Instant,
    last: Instant,
}

impl Repeat {
    /// The run after `code` pressed at `now`: the same one when it goes on soon enough, else a new one.
    pub fn after(run: Option<Self>, code: KeyCode, now: Instant) -> Self {
        match run {
            Some(run) if run.code == code && now.saturating_duration_since(run.last) < GAP => Self { last: now, ..run },
            _ => Self { code, since: now, last: now },
        }
    }

    /// How many rows one move covers at this point of the run.
    pub fn step(self) -> isize {
        let held = self.last.saturating_duration_since(self.since);
        STEPS.iter().find(|(after, _)| held >= *after).map_or(1, |(_, step)| *step)
    }
}

impl App {
    /// How many rows `j`, `k` and the arrows move now: more while one of them is held.
    pub(super) fn step(&self) -> isize {
        self.repeat.map_or(1, Repeat::step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The steps of `code` pressed at each of `times`, in milliseconds from the first.
    fn steps(code: KeyCode, times: &[u64]) -> Vec<isize> {
        let start = Instant::now();
        let mut run = None;
        times
            .iter()
            .map(|&ms| {
                let next = Repeat::after(run, code, start + Duration::from_millis(ms));
                run = Some(next);
                next.step()
            })
            .collect()
    }

    fn held(until: u64) -> Vec<u64> {
        (0..=until).step_by(30).collect()
    }

    #[test]
    fn a_held_key_moves_one_row_then_two_then_four() {
        let steps = steps(KeyCode::Char('j'), &held(1200));
        assert_eq!(steps.first(), Some(&1));
        assert_eq!(steps[300 / 30], 2);
        assert_eq!(steps.last(), Some(&4), "four is the most");
    }

    #[test]
    fn presses_by_hand_move_one_row_each() {
        assert_eq!(steps(KeyCode::Char('j'), &[0, 150, 300, 450, 600, 750, 900, 1050, 1200]), vec![1; 9]);
    }

    #[test]
    fn a_pause_starts_over() {
        let mut times = held(600);
        times.push(900);
        assert_eq!(steps(KeyCode::Char('j'), &times).last(), Some(&1));
    }

    #[test]
    fn another_key_starts_over() {
        let start = Instant::now();
        let run = held(600).iter().fold(None, |run, &ms| Some(Repeat::after(run, KeyCode::Down, start + Duration::from_millis(ms))));
        assert_eq!(run.map(Repeat::step), Some(2));
        assert_eq!(Repeat::after(run, KeyCode::Up, start + Duration::from_millis(630)).step(), 1);
    }
}
