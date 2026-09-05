"""Clock abstraction used by the sequencer.

An external MIDI-clock implementation can later implement ``StepClock`` without
changing the sequencer. It would resolve ``next_step`` from incoming clock ticks.
"""

from __future__ import annotations

import asyncio
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
    start: float
    duration: float

    @property
    def end(self) -> float:
        return self.start + self.duration


class StepClock(Protocol):
    """Produces timed step windows. Future MIDI clock slaves fit this interface."""

    def reset(self) -> None: ...

    async def next_step(self, bpm: float, step_division: int) -> StepWindow: ...

    async def wait_until(self, deadline: float) -> None: ...


class InternalClock:
    """Monotonic internal clock with scheduled step boundaries to limit drift."""

    def __init__(self) -> None:
        self._next_start: float | None = None

    def reset(self) -> None:
        self._next_start = None

    async def next_step(self, bpm: float, step_division: int) -> StepWindow:
        now = monotonic()
        start = now if self._next_start is None else max(now, self._next_start)
        duration = step_duration_seconds(bpm, step_division)
        self._next_start = start + duration
        return StepWindow(start=start, duration=duration)

    async def wait_until(self, deadline: float) -> None:
        remaining = deadline - monotonic()
        if remaining > 0:
            await asyncio.sleep(remaining)
