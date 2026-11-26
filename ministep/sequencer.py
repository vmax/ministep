"""Async step playback with no dependency on mido, Textual, or MIDI hardware."""

from __future__ import annotations

import asyncio

from .clock import InternalClock, StepClock
from .drums import DRUM_STEPS
from .midi import MidiSink
from .state import AppState, Step
from .timing import TimingStats

# Drum hits are fixed-length triggers, independent of the melodic gate. Drum
# Racks and samplers only need the note-on; 40 ms is short enough to never
# overlap the next 1/32 step at any sane tempo and long enough for every host.
DRUM_NOTE_LENGTH_MS = 40


class Sequencer:
    """Plays the melodic steps and the drum track from one anchored clock.

    Both tracks are advanced inside :meth:`advance_one`, so they can never
    drift apart. Melodic note-offs are awaited on the grid as before; drum
    note-offs run as small side tasks that wait on the same clock, so the
    transport loop is never blocked by them.
    """

    def __init__(
        self,
        state: AppState,
        output: MidiSink,
        clock: StepClock | None = None,
        stats: TimingStats | None = None,
        drum_output: MidiSink | None = None,
    ) -> None:
        self.state = state
        self.output = output
        # Drums default to the melodic port; a separate port (e.g. an IAC bus
        # into Ableton) is passed by the runtime when configured.
        self.drum_output = drum_output if drum_output is not None else output
        self.clock = clock or InternalClock()
        # Optional in-memory diagnostics; None keeps the timing path free of bookkeeping.
        self.stats = stats
        self._task: asyncio.Task[None] | None = None
        self._active_tied_note: int | None = None
        # Drum position is its own 16-step counter: the melodic loop span can
        # differ from 16 and the drum bar must not follow it.
        self.drum_step = 0
        self._drum_releases: set[asyncio.Task[None]] = set()
        self._sounding_drums: set[tuple[int, int]] = set()

    def start(self, *, restart: bool = True) -> None:
        if restart:
            self.state.playhead = 0
            self.drum_step = 0
            self.clock.reset()
        if self._task is not None and not self._task.done():
            self.state.playing = True
            return
        # A fresh playback task must not inherit a stale grid from an earlier run,
        # or the anchored clock would treat the gap as a stall and skip slots.
        self.clock.reset()
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
        self._release_all_drums()

    async def restart(self) -> None:
        await self.stop()
        self.start(restart=True)

    async def _run(self) -> None:
        try:
            while self.state.playing:
                if not self.has_content():
                    await asyncio.sleep(0.05)
                    self.clock.reset()
                    continue
                await self.advance_one()
        except asyncio.CancelledError:
            raise
        finally:
            self.state.playing = False

    def has_content(self) -> bool:
        """Whether either track has anything to play."""
        return bool(self.state.sequence) or self.state.drums.has_hits()

    async def advance_one(self) -> bool:
        """Play exactly one grid step on both tracks; public for deterministic tests."""
        melodic = bool(self.state.sequence)
        if not melodic and not self.state.drums.has_hits():
            return False
        loop_length = self.state.playback_length()
        window = await self.clock.next_step(self.state.bpm, self.state.step_division)
        woke = self.stats.now() if self.stats is not None else 0.0
        # After a stall the clock drops whole grid slots; move both positions past
        # them so the loops stay in phase with the grid instead of replaying late.
        self.drum_step = (self.drum_step + window.skipped) % DRUM_STEPS
        if not melodic:
            self._fire_drums(window.start)
            if self.stats is not None:
                self.stats.record(
                    scheduled=window.start,
                    actual=woke,
                    handler=self.stats.now() - woke,
                    duration=window.duration,
                    skipped=window.skipped,
                )
            await self.clock.wait_until(window.end)
            self.drum_step = (self.drum_step + 1) % DRUM_STEPS
            return True
        self.state.playhead = (self.state.playhead + window.skipped) % loop_length
        step = self.state.sequence[self.state.playhead]
        note = self._playback_note(step)

        if self._active_tied_note is not None and note != self._active_tied_note:
            self._release_tie()

        is_continuing_tie = note is not None and note == self._active_tied_note
        should_sound = step.enabled and note is not None
        if should_sound and not is_continuing_tie:
            self.output.note_on(note, step.velocity, self._channel(), owner="sequencer")
        self._fire_drums(window.start)
        if self.stats is not None:
            self.stats.record(
                scheduled=window.start,
                actual=woke,
                handler=self.stats.now() - woke,
                duration=window.duration,
                skipped=window.skipped,
            )

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
        self.drum_step = (self.drum_step + 1) % DRUM_STEPS
        return True

    # --- drums ------------------------------------------------------------------

    def _fire_drums(self, grid_start: float) -> None:
        """Send every active hit on the current drum step and schedule its note-off."""
        hits = self.state.drums.hits_at(self.drum_step)
        if not hits:
            return
        channel = self._drum_channel()
        notes: list[int] = []
        for lane, hit in hits:
            self.drum_output.note_on(lane.note, hit.velocity, channel, owner="drums")
            self._sounding_drums.add((lane.note, channel))
            notes.append(lane.note)
        deadline = grid_start + DRUM_NOTE_LENGTH_MS / 1000.0
        task = asyncio.create_task(
            self._release_drums(notes, channel, deadline), name="ministep-drum-release"
        )
        self._drum_releases.add(task)
        task.add_done_callback(self._drum_releases.discard)

    async def _release_drums(self, notes: list[int], channel: int, deadline: float) -> None:
        # Waits on the sequencer clock, not wall time, so simulations stay exact.
        await self.clock.wait_until(deadline)
        for note in notes:
            self.drum_output.note_off(note, channel, owner="drums")
            self._sounding_drums.discard((note, channel))

    def _release_all_drums(self) -> None:
        """Cancel pending drum note-offs and silence anything still sounding."""
        for task in list(self._drum_releases):
            task.cancel()
        self._drum_releases.clear()
        for note, channel in sorted(self._sounding_drums):
            self.drum_output.note_off(note, channel, owner="drums")
        self._sounding_drums.clear()

    def _playback_note(self, step: Step) -> int | None:
        if not step.enabled or step.note is None:
            return None
        return min(127, max(0, step.note + self.state.transpose + self.state.octave * 12))

    def _channel(self) -> int:
        return self.state.output_channel - 1

    def _drum_channel(self) -> int:
        return self.state.drums.channel - 1

    def _release_tie(self) -> None:
        if self._active_tied_note is not None:
            self.output.note_off(self._active_tied_note, self._channel(), owner="sequencer")
            self._active_tied_note = None
