"""Transport timing regression tests: anchored grid, late policy, diagnostics.

The scheduling algorithm runs against a fake clock, so these tests are
deterministic and instant while still exercising the real ``InternalClock`` and
``Sequencer`` code paths.
"""

from __future__ import annotations

import asyncio
import os
import random

import pytest

from ministep.clock import InternalClock, step_duration_seconds
from ministep.sequencer import Sequencer
from ministep.state import AppState, Step
from ministep.timing import TimingStats


class VirtualTime:
    """Fake monotonic clock whose ``sleep`` wakes late by a modelled overshoot."""

    def __init__(self, overshoot: float = 0.0011, seed: int = 1) -> None:
        self.t = 1000.0
        self.overshoot = overshoot
        self.rng = random.Random(seed)
        self.stalls: dict[int, float] = {}
        self.sleeps = 0

    def now(self) -> float:
        return self.t

    async def sleep(self, seconds: float) -> None:
        self.sleeps += 1
        self.t += max(0.0, seconds) + self.overshoot * self.rng.uniform(0.8, 1.4)
        self.t += self.stalls.pop(self.sleeps, 0.0)
        await asyncio.sleep(0)


class CountingSink:
    def __init__(self) -> None:
        self.events: list[tuple[str, int, float]] = []
        self.now = lambda: 0.0

    def note_on(self, note: int, velocity: int, channel: int = 0, owner: str = "sequencer") -> None:
        self.events.append(("on", note, self.now()))

    def note_off(self, note: int, channel: int = 0, owner: str = "sequencer") -> None:
        self.events.append(("off", note, self.now()))

    def control_change(self, control: int, value: int, channel: int = 0) -> None:
        pass

    def all_notes_off(self) -> None:
        pass


def build(bpm: float, vt: VirtualTime, steps: int = 16) -> tuple[Sequencer, TimingStats, AppState]:
    state = AppState(bpm=bpm, step_division=16, default_gate=0.5)
    state.sequence = [Step(60 + index % 12, gate=0.5) for index in range(steps)]
    sink = CountingSink()
    sink.now = vt.now
    stats = TimingStats(now=vt.now)
    clock = InternalClock(now=vt.now, sleep=vt.sleep)
    return Sequencer(state, sink, clock, stats=stats), stats, state


async def play_for(sequencer: Sequencer, vt: VirtualTime, seconds: float) -> None:
    sequencer.state.playing = True
    end = vt.t + seconds
    while vt.t < end:
        await sequencer.advance_one()


@pytest.mark.parametrize("bpm", [120, 150, 180, 200, 240])
async def test_sixteenths_do_not_accumulate_drift_over_one_minute(bpm: float) -> None:
    vt = VirtualTime(overshoot=0.0011)
    sequencer, stats, _ = build(bpm, vt)
    await play_for(sequencer, vt, 60.0)

    summary = stats.summary()
    assert summary is not None
    expected_steps = round(60.0 / step_duration_seconds(bpm, 16))
    # Every grid slot in the minute is emitted: no lost steps, no burst.
    assert abs(summary.steps - expected_steps) <= 1
    # Cumulative drift stays bounded by one wake-up overshoot, not N of them.
    assert abs(summary.drift_final) < 0.003
    assert summary.drift_max_abs < 0.003
    # Mean interval equals the step duration (no systematic tempo error).
    assert abs(summary.interval_mean_error) < 0.0001
    assert summary.skipped_steps == 0


async def test_step_deadlines_come_from_the_grid_not_the_wake_time() -> None:
    vt = VirtualTime(overshoot=0.0)
    clock = InternalClock(now=vt.now, sleep=vt.sleep)
    first = await clock.next_step(120, 16)  # 125 ms steps
    vt.t = first.end + 0.030  # woke 30 ms late
    second = await clock.next_step(120, 16)
    assert second.start == pytest.approx(first.end)
    assert second.late == pytest.approx(0.030)
    assert second.skipped == 0
    vt.t = second.end  # on time again: the grid never moved
    third = await clock.next_step(120, 16)
    assert third.start == pytest.approx(first.start + 2 * 0.125)
    assert third.late == pytest.approx(0.0)


async def test_stall_of_several_steps_skips_slots_and_keeps_the_grid() -> None:
    vt = VirtualTime(overshoot=0.0)
    clock = InternalClock(now=vt.now, sleep=vt.sleep)
    first = await clock.next_step(120, 16)
    vt.t = first.end + 0.125 * 2 + 0.010  # 2 whole slots plus 10 ms gone
    window = await clock.next_step(120, 16)
    assert window.skipped == 2
    assert window.start == pytest.approx(first.start + 3 * 0.125)
    assert window.late == pytest.approx(0.010)


async def test_sequencer_advances_playhead_past_skipped_slots() -> None:
    vt = VirtualTime(overshoot=0.0)
    sequencer, stats, state = build(120, vt, steps=8)
    state.playing = True
    await sequencer.advance_one()  # step 0
    assert state.playhead == 1
    vt.stalls[vt.sleeps + 2] = 0.125 * 3  # stall 3 slots during step 1's final wait
    await sequencer.advance_one()  # step 1 plays normally, stall happens at its end
    await sequencer.advance_one()  # slots 2,3,4 were missed: play slot 5
    notes = [note for kind, note, _ in sequencer.output.events if kind == "on"]  # type: ignore[attr-defined]
    assert notes == [60, 61, 65]
    assert state.playhead == 6
    assert stats.skipped_steps == 3


async def test_tempo_change_reanchors_from_the_current_step_boundary() -> None:
    vt = VirtualTime(overshoot=0.0005)
    sequencer, stats, state = build(120, vt)
    state.playing = True
    for _ in range(4):
        await sequencer.advance_one()
    boundary = sequencer.clock._next_start  # type: ignore[attr-defined]
    state.set_bpm(180)
    window_ahead = await sequencer.clock.next_step(state.bpm, state.step_division)
    assert window_ahead.start == pytest.approx(boundary)
    assert window_ahead.duration == pytest.approx(step_duration_seconds(180, 16))


async def test_gate_and_note_off_stay_on_the_grid_when_late() -> None:
    vt = VirtualTime(overshoot=0.0)
    sequencer, _, state = build(120, vt, steps=2)
    state.playing = True
    await sequencer.advance_one()
    vt.stalls[vt.sleeps + 2] = 0.020  # wake 20 ms late for step 1... during its gate wait
    await sequencer.advance_one()
    events = sequencer.output.events  # type: ignore[attr-defined]
    on0, off0, on1, off1 = events
    # Note-offs land on gate deadlines derived from the grid, not from wake time.
    assert off0[2] - on0[2] == pytest.approx(0.0625)
    assert on1[2] == pytest.approx(on0[2] + 0.125)


def test_repeated_float_addition_is_negligible_for_transport() -> None:
    # One hour of 1/32 steps at 240 BPM (the fastest musical setting) via
    # repeated addition, as the clock does, versus exact multiplication.
    duration = step_duration_seconds(240, 32)
    origin = 3_000_000.0  # a large monotonic value, as after weeks of uptime
    accumulated = origin
    steps = int(3600 / duration)
    for _ in range(steps):
        accumulated += duration
    assert abs(accumulated - (origin + steps * duration)) < 1e-6


def test_timing_stats_summary_reports_drift_and_percentiles() -> None:
    stats = TimingStats()
    duration = 0.1
    for index in range(100):
        late = 0.001 if index % 10 else 0.010
        stats.record(
            scheduled=index * duration,
            actual=index * duration + late,
            handler=0.0001,
            duration=duration,
        )
    summary = stats.summary()
    assert summary is not None
    assert summary.steps == 100
    assert summary.lateness_p50 == pytest.approx(0.001)
    assert summary.lateness_max == pytest.approx(0.010)
    assert summary.drift_final == pytest.approx(0.001)
    # 10 late wakes over 99 intervals: -9 ms x10, +9 ms x9 -> tiny negative mean.
    assert summary.interval_mean_error == pytest.approx(-0.009 / 99)
    assert summary.interval_jitter_max == pytest.approx(0.009)


@pytest.mark.skipif(
    not os.environ.get("MINISTEP_REALTIME_TESTS"),
    reason="set MINISTEP_REALTIME_TESTS=1 to run wall-clock timing checks",
)
async def test_realtime_sixteenths_at_180_bpm_hold_the_grid() -> None:
    state = AppState(bpm=180, step_division=16)
    state.sequence = [Step(60 + index) for index in range(16)]
    stats = TimingStats()
    sequencer = Sequencer(state, CountingSink(), InternalClock(), stats=stats)
    sequencer.start()
    await asyncio.sleep(10.0)
    await sequencer.stop()
    summary = stats.summary()
    assert summary is not None
    assert summary.skipped_steps == 0
    assert abs(summary.drift_final) < 0.010
    assert summary.lateness_p95 < 0.010
