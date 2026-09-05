"""Async step playback with no dependency on mido, Textual, or MIDI hardware."""

from __future__ import annotations

import asyncio

from .clock import InternalClock, StepClock
from .midi import MidiSink
from .state import AppState, Step


class Sequencer:
    def __init__(self, state: AppState, output: MidiSink, clock: StepClock | None = None) -> None:
        self.state = state
        self.output = output
        self.clock = clock or InternalClock()
        self._task: asyncio.Task[None] | None = None
        self._active_tied_note: int | None = None

    def start(self, *, restart: bool = True) -> None:
        if restart:
            self.state.playhead = 0
            self.clock.reset()
        if self._task is not None and not self._task.done():
            self.state.playing = True
            return
        self.state.playing = True
        self._task = asyncio.create_task(self._run(), name="ministep-playback")

    async def stop(self) -> None:
        self.state.playing = False
        if self._task is not None and self._task is not asyncio.current_task():
            self._task.cancel()
            try:
                await self._task
            except asyncio.CancelledError:
                pass
        self._task = None
        self._release_tie()

    async def restart(self) -> None:
        await self.stop()
        self.start(restart=True)

    async def _run(self) -> None:
        try:
            while self.state.playing:
                if not self.state.sequence:
                    await asyncio.sleep(0.05)
                    self.clock.reset()
                    continue
                await self.advance_one()
        except asyncio.CancelledError:
            raise
        finally:
            self.state.playing = False

    async def advance_one(self) -> bool:
        """Play exactly one state step; public to make deterministic tests possible."""
        if not self.state.sequence:
            return False
        loop_length = self.state.playback_length()
        self.state.playhead %= loop_length
        step = self.state.sequence[self.state.playhead]
        window = await self.clock.next_step(self.state.bpm, self.state.step_division)
        note = self._playback_note(step)

        if self._active_tied_note is not None and note != self._active_tied_note:
            self._release_tie()

        is_continuing_tie = note is not None and note == self._active_tied_note
        should_sound = step.enabled and note is not None
        if should_sound and not is_continuing_tie:
            self.output.note_on(note, step.velocity, self._channel(), owner="sequencer")

        if should_sound and step.tie:
            self._active_tied_note = note
            await self.clock.wait_until(window.end)
        elif should_sound:
            await self.clock.wait_until(window.start + window.duration * step.gate)
            self.output.note_off(note, self._channel(), owner="sequencer")
            self._active_tied_note = None
            if step.gate < 1.0:
                await self.clock.wait_until(window.end)
        else:
            await self.clock.wait_until(window.end)

        if loop_length:
            self.state.playhead = (self.state.playhead + 1) % loop_length
        return True

    def _playback_note(self, step: Step) -> int | None:
        if not step.enabled or step.note is None:
            return None
        return min(127, max(0, step.note + self.state.transpose + self.state.octave * 12))

    def _channel(self) -> int:
        return self.state.output_channel - 1

    def _release_tie(self) -> None:
        if self._active_tied_note is not None:
            self.output.note_off(self._active_tied_note, self._channel(), owner="sequencer")
            self._active_tied_note = None
