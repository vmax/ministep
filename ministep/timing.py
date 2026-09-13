"""In-memory transport timing diagnostics.

``TimingStats`` records one sample per emitted step and produces a summary on
demand. Nothing is printed while playing: printing from the timing path would
itself perturb the measurement. Enable it in the app with ``--timing-log PATH``.

Definitions (all seconds, monotonic clock):

* ``scheduled``  ideal deadline the clock handed out for the step (``StepWindow.start``)
* ``actual``     when the sequencer actually ran the step handler
* ``lateness``   ``actual - scheduled``; wake-up latency relative to the deadline
* ``interval``   ``actual_i - actual_{i-1}``; compared against the step duration
* ``drift``      ``actual_i - (origin + i * duration)``; cumulative error against
                 an ideal grid anchored at the first step. The grid is re-anchored
                 whenever the step duration changes (tempo/division change).
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from time import monotonic


@dataclass(frozen=True)
class StepSample:
    index: int
    scheduled: float
    actual: float
    handler: float
    duration: float
    ideal: float

    @property
    def lateness(self) -> float:
        return self.actual - self.scheduled

    @property
    def drift(self) -> float:
        return self.actual - self.ideal


def _percentile(sorted_values: list[float], fraction: float) -> float:
    if not sorted_values:
        return 0.0
    position = min(len(sorted_values) - 1, int(round(fraction * (len(sorted_values) - 1))))
    return sorted_values[position]


@dataclass
class TimingSummary:
    steps: int
    duration: float
    lateness_p50: float
    lateness_p95: float
    lateness_p99: float
    lateness_max: float
    handler_p50: float
    handler_max: float
    interval_mean_error: float
    interval_jitter_p95: float
    interval_jitter_p99: float
    interval_jitter_max: float
    drift_final: float
    drift_max_abs: float
    skipped_steps: int = 0
    elapsed: float = 0.0

    def format(self, label: str = "") -> str:
        ms = 1000.0
        head = f"[{label}] " if label else ""
        return (
            f"{head}steps={self.steps} step={self.duration * ms:.3f}ms "
            f"elapsed={self.elapsed:.1f}s skipped={self.skipped_steps}\n"
            f"  lateness ms  p50={self.lateness_p50 * ms:.3f} p95={self.lateness_p95 * ms:.3f} "
            f"p99={self.lateness_p99 * ms:.3f} max={self.lateness_max * ms:.3f}\n"
            f"  handler  ms  p50={self.handler_p50 * ms:.3f} max={self.handler_max * ms:.3f}\n"
            f"  interval ms  mean_err={self.interval_mean_error * ms:+.4f} "
            f"jitter p95={self.interval_jitter_p95 * ms:.3f} "
            f"p99={self.interval_jitter_p99 * ms:.3f} max={self.interval_jitter_max * ms:.3f}\n"
            f"  drift    ms  final={self.drift_final * ms:+.3f} "
            f"max_abs={self.drift_max_abs * ms:.3f}"
        )


@dataclass
class TimingStats:
    """Collect per-step timing samples; ``summary()`` reduces them."""

    capacity: int = 200_000
    # Injectable so simulations can measure against a fake clock.
    now: Callable[[], float] = monotonic
    samples: list[StepSample] = field(default_factory=list)
    skipped_steps: int = 0
    _origin: float | None = None
    _grid_index: int = 0
    _duration: float | None = None
    _last_actual: float | None = None

    def reset(self) -> None:
        self.samples.clear()
        self.skipped_steps = 0
        self._origin = None
        self._grid_index = 0
        self._duration = None
        self._last_actual = None

    def record(
        self,
        *,
        scheduled: float,
        actual: float,
        handler: float,
        duration: float,
        skipped: int = 0,
    ) -> StepSample:
        if self._origin is None or self._duration != duration:
            # First sample, or tempo/division changed: anchor a fresh ideal grid.
            self._origin = scheduled
            self._grid_index = 0
            self._duration = duration
        self._grid_index += skipped
        self.skipped_steps += skipped
        ideal = self._origin + self._grid_index * duration
        sample = StepSample(
            index=len(self.samples),
            scheduled=scheduled,
            actual=actual,
            handler=handler,
            duration=duration,
            ideal=ideal,
        )
        self._grid_index += 1
        self._last_actual = actual
        if len(self.samples) < self.capacity:
            self.samples.append(sample)
        return sample

    def summary(self) -> TimingSummary | None:
        if not self.samples:
            return None
        samples = self.samples
        duration = samples[-1].duration
        lateness = sorted(s.lateness for s in samples)
        handler = sorted(s.handler for s in samples)
        interval_errors = [
            (b.actual - a.actual) - b.duration
            for a, b in zip(samples, samples[1:], strict=False)
            if a.duration == b.duration
        ]
        jitter = sorted(abs(e) for e in interval_errors)
        drifts = [s.drift for s in samples]
        return TimingSummary(
            steps=len(samples),
            duration=duration,
            lateness_p50=_percentile(lateness, 0.50),
            lateness_p95=_percentile(lateness, 0.95),
            lateness_p99=_percentile(lateness, 0.99),
            lateness_max=lateness[-1],
            handler_p50=_percentile(handler, 0.50),
            handler_max=handler[-1],
            interval_mean_error=(
                sum(interval_errors) / len(interval_errors) if interval_errors else 0.0
            ),
            interval_jitter_p95=_percentile(jitter, 0.95),
            interval_jitter_p99=_percentile(jitter, 0.99),
            interval_jitter_max=jitter[-1] if jitter else 0.0,
            drift_final=drifts[-1],
            drift_max_abs=max(abs(d) for d in drifts),
            skipped_steps=self.skipped_steps,
            elapsed=samples[-1].actual - samples[0].actual,
        )
