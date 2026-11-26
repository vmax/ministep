//! In-memory transport timing diagnostics (same definitions as the Python build):
//! lateness = actual - scheduled; interval error vs step duration; drift against an
//! ideal grid anchored at the first step and re-anchored on tempo/division change.

#[derive(Debug, Clone, Copy)]
pub struct StepSample {
    pub scheduled: f64,
    pub actual: f64,
    pub handler: f64,
    pub duration: f64,
    pub ideal: f64,
}

impl StepSample {
    pub fn lateness(&self) -> f64 {
        self.actual - self.scheduled
    }
    pub fn drift(&self) -> f64 {
        self.actual - self.ideal
    }
}

#[derive(Debug, Clone, Default)]
pub struct TimingSummary {
    pub steps: usize,
    pub duration: f64,
    pub lateness_p50: f64,
    pub lateness_p95: f64,
    pub lateness_p99: f64,
    pub lateness_max: f64,
    pub handler_p50: f64,
    pub handler_max: f64,
    pub interval_mean_error: f64,
    pub interval_jitter_p95: f64,
    pub interval_jitter_p99: f64,
    pub interval_jitter_max: f64,
    pub drift_final: f64,
    pub drift_max_abs: f64,
    pub skipped_steps: u64,
    pub elapsed: f64,
}

impl TimingSummary {
    pub fn format(&self, label: &str) -> String {
        let ms = 1000.0;
        let head = if label.is_empty() { String::new() } else { format!("[{label}] ") };
        format!(
            "{head}steps={} step={:.3}ms elapsed={:.1}s skipped={}\n  lateness ms  p50={:.3} p95={:.3} p99={:.3} max={:.3}\n  handler  ms  p50={:.3} max={:.3}\n  interval ms  mean_err={:+.4} jitter p95={:.3} p99={:.3} max={:.3}\n  drift    ms  final={:+.3} max_abs={:.3}",
            self.steps,
            self.duration * ms,
            self.elapsed,
            self.skipped_steps,
            self.lateness_p50 * ms,
            self.lateness_p95 * ms,
            self.lateness_p99 * ms,
            self.lateness_max * ms,
            self.handler_p50 * ms,
            self.handler_max * ms,
            self.interval_mean_error * ms,
            self.interval_jitter_p95 * ms,
            self.interval_jitter_p99 * ms,
            self.interval_jitter_max * ms,
            self.drift_final * ms,
            self.drift_max_abs * ms,
        )
    }
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = ((fraction * (sorted.len() - 1) as f64).round() as usize).min(sorted.len() - 1);
    sorted[pos]
}

#[derive(Debug, Clone)]
pub struct TimingStats {
    pub capacity: usize,
    pub samples: Vec<StepSample>,
    pub skipped_steps: u64,
    origin: Option<f64>,
    grid_index: u64,
    duration: Option<f64>,
}

impl Default for TimingStats {
    fn default() -> Self {
        Self { capacity: 200_000, samples: Vec::new(), skipped_steps: 0, origin: None, grid_index: 0, duration: None }
    }
}

impl TimingStats {
    pub fn reset(&mut self) {
        *self = Self { capacity: self.capacity, ..Self::default() };
    }

    pub fn record(&mut self, scheduled: f64, actual: f64, handler: f64, duration: f64, skipped: u64) {
        if self.origin.is_none() || self.duration != Some(duration) {
            self.origin = Some(scheduled);
            self.grid_index = 0;
            self.duration = Some(duration);
        }
        self.grid_index += skipped;
        self.skipped_steps += skipped;
        let ideal = self.origin.unwrap_or(scheduled) + self.grid_index as f64 * duration;
        self.grid_index += 1;
        if self.samples.len() < self.capacity {
            self.samples.push(StepSample { scheduled, actual, handler, duration, ideal });
        }
    }

    pub fn summary(&self) -> Option<TimingSummary> {
        let s = &self.samples;
        let last = s.last()?;
        let sorted = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v
        };
        let lateness = sorted(s.iter().map(StepSample::lateness).collect());
        let handler = sorted(s.iter().map(|x| x.handler).collect());
        let errors: Vec<f64> = s
            .windows(2)
            .filter(|w| w[0].duration == w[1].duration)
            .map(|w| (w[1].actual - w[0].actual) - w[1].duration)
            .collect();
        let jitter = sorted(errors.iter().map(|e| e.abs()).collect());
        Some(TimingSummary {
            steps: s.len(),
            duration: last.duration,
            lateness_p50: percentile(&lateness, 0.5),
            lateness_p95: percentile(&lateness, 0.95),
            lateness_p99: percentile(&lateness, 0.99),
            lateness_max: *lateness.last().unwrap_or(&0.0),
            handler_p50: percentile(&handler, 0.5),
            handler_max: *handler.last().unwrap_or(&0.0),
            interval_mean_error: if errors.is_empty() { 0.0 } else { errors.iter().sum::<f64>() / errors.len() as f64 },
            interval_jitter_p95: percentile(&jitter, 0.95),
            interval_jitter_p99: percentile(&jitter, 0.99),
            interval_jitter_max: jitter.last().copied().unwrap_or(0.0),
            drift_final: last.drift(),
            drift_max_abs: s.iter().map(|x| x.drift().abs()).fold(0.0, f64::max),
            skipped_steps: self.skipped_steps,
            elapsed: last.actual - s[0].actual,
        })
    }
}
