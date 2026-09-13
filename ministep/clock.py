"""Clock abstraction used by the sequencer.

An external MIDI-clock implementation can later implement ``StepClock`` without
changing the sequencer. It would resolve ``next_step`` from incoming clock ticks.
"""

from __future__ import annotations

import asyncio
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from time import monotonic
from typing import Protocol


def step_duration_seconds(bpm: float, step_division: int) -> float:
    """Duration of one step: division 4 is a quarter note, 16 a sixteenth."""
    if bpm <= 0:
        raise ValueError("bpm must be positive")
    if step_division <= 0:
        raise ValueError("step_division must be positive")
    return (60.0 / bpm) * (4.0 / step_division)


@dataclass(frozen=True)
class StepWindow:
    """One step's slot on the ideal transport grid.

    ``start`` is the scheduled grid time, which may already be in the past when
    the loop woke late; ``late`` says by how much. ``skipped`` counts whole grid
    slots the clock dropped to resynchronise after a stall of a step or more.
    """

    start: float
    duration: float
    late: float = 0.0
    skipped: int = 0

    @property
    def end(self) -> float:
        return self.start + self.duration


class StepClock(Protocol):
    """Produces timed step windows. Future MIDI clock slaves fit this interface."""

    def reset(self) -> None: ...

    async def next_step(self, bpm: float, step_division: int) -> StepWindow: ...

    async def wait_until(self, deadline: float) -> None: ...


class InternalClock:
    """Monotonic clock handing out deadlines from an anchored ideal grid.

    The next step always starts exactly one step after the previous *scheduled*
    start, never after the actual wake-up time, so timer overshoot and handler
    work do not accumulate into the transport period. See ``docs/TIMING.md``.

    Late policy: a wake-up less than ``max_late_steps`` steps late plays the step
    immediately and keeps the grid. A stall of ``max_late_steps`` or more drops
    the missed slots (reported via ``StepWindow.skipped``) instead of bursting
    them, so the loop position stays in phase with wall-clock time.

    ``now`` and ``sleep`` are injectable so the algorithm can be simulated
    deterministically against a fake clock.
    """

    def __init__(
        self,
        now: Callable[[], float] = monotonic,
        sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
        max_late_steps: float = 1.0,
    ) -> None:
        if max_late_steps <= 0:
            raise ValueError("max_late_steps must be positive")
        self._now = now
        self._sleep = sleep
        self.max_late_steps = max_late_steps
        self._next_start: float | None = None

    def reset(self) -> None:
        self._next_start = None

    async def next_step(self, bpm: float, step_division: int) -> StepWindow:
        duration = step_duration_seconds(bpm, step_division)
        now = self._now()
        if self._next_start is None:
            start = now
            skipped = 0
        else:
            start = self._next_start
            late = now - start
            skipped = 0
            if late >= duration * self.max_late_steps:
                skipped = int(late // duration)
                start += skipped * duration
            elif late < 0:
                # Woke early (coarse timers): hold the step until its grid time.
                await self.wait_until(start)
                now = self._now()
        self._next_start = start + duration
        return StepWindow(
            start=start, duration=duration, late=max(0.0, now - start), skipped=skipped
        )

    async def wait_until(self, deadline: float) -> None:
        remaining = deadline - self._now()
        if remaining > 0:
            await self._sleep(remaining)
