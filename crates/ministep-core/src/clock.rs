//! Anchored transport grid (see `docs/TIMING.md`).
//!
//! The next step always starts exactly one step after the previous *scheduled*
//! start, never after the actual wake-up, so wake-up overshoot and handler work
//! never accumulate into the tempo. A stall of `max_late_steps` or more drops
//! the missed slots instead of bursting them.

use std::sync::OnceLock;
use std::time::Instant;

/// Duration of one step in seconds: division 4 is a quarter, 16 a sixteenth.
pub fn step_duration_seconds(bpm: f64, step_division: u32) -> f64 {
    assert!(bpm > 0.0 && step_division > 0, "bpm and division must be positive");
    (60.0 / bpm) * (4.0 / f64::from(step_division))
}

/// Monotonic seconds since process start.
pub fn monotonic() -> f64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN.get_or_init(Instant::now).elapsed().as_secs_f64()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StepWindow {
    pub start: f64,
    pub duration: f64,
    pub late: f64,
    pub skipped: u64,
}

impl StepWindow {
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

#[derive(Debug, Clone)]
pub struct Grid {
    next_start: Option<f64>,
    pub max_late_steps: f64,
}

impl Default for Grid {
    fn default() -> Self {
        Self { next_start: None, max_late_steps: 1.0 }
    }
}

impl Grid {
    pub fn reset(&mut self) {
        self.next_start = None;
    }

    /// Scheduled start of the next step, if the grid is anchored.
    pub fn next_start(&self) -> Option<f64> {
        self.next_start
    }

    /// Hand out the next window. Call at or after [`Grid::next_start`].
    pub fn next_step(&mut self, now: f64, bpm: f64, step_division: u32) -> StepWindow {
        let duration = step_duration_seconds(bpm, step_division);
        let (start, skipped) = match self.next_start {
            None => (now, 0),
            Some(start) => {
                let late = now - start;
                if late >= duration * self.max_late_steps {
                    let skipped = (late / duration).floor() as u64;
                    (start + skipped as f64 * duration, skipped)
                } else {
                    (start, 0)
                }
            }
        };
        self.next_start = Some(start + duration);
        StepWindow { start, duration, late: (now - start).max(0.0), skipped }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert!((step_duration_seconds(180.0, 16) - 0.083_333_333).abs() < 1e-9);
        assert!((step_duration_seconds(120.0, 4) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn late_wakeups_do_not_accumulate() {
        let mut g = Grid::default();
        let d = step_duration_seconds(120.0, 16);
        let mut now = 10.0;
        let first = g.next_step(now, 120.0, 16);
        assert_eq!(first.start, 10.0);
        for i in 1..1000 {
            now = g.next_start().unwrap() + 0.0015; // always 1.5 ms late
            let w = g.next_step(now, 120.0, 16);
            assert!((w.start - (10.0 + i as f64 * d)).abs() < 1e-9);
            assert_eq!(w.skipped, 0);
        }
    }

    #[test]
    fn stall_drops_slots() {
        let mut g = Grid::default();
        let d = step_duration_seconds(120.0, 16);
        g.next_step(0.0, 120.0, 16);
        let w = g.next_step(d + 3.5 * d, 120.0, 16);
        assert_eq!(w.skipped, 3);
        assert!((w.start - 4.0 * d).abs() < 1e-12);
    }
}
