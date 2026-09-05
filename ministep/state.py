"""Sequence data and application state, independent from MIDI and the UI."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class Step:
    """One sequencer step. ``note=None`` represents a rest."""

    note: int | None
    velocity: int = 100
    gate: float = 0.5
    enabled: bool = True
    tie: bool = False
    accent: bool = False

    def __post_init__(self) -> None:
        if self.note is not None and not 0 <= self.note <= 127:
            raise ValueError("MIDI note must be between 0 and 127")
        if not 0 <= self.velocity <= 127:
            raise ValueError("MIDI velocity must be between 0 and 127")
        if not 0.0 <= self.gate <= 1.0:
            raise ValueError("gate must be between 0.0 and 1.0")


LOOP_LENGTHS = (None, 8, 16, 32, 64)


@dataclass
class AppState:
    """Mutable state owned by the application coordinator."""

    bpm: float = 128.0
    step_division: int = 16
    sequence: list[Step] = field(default_factory=list)
    # None preserves historical behaviour: loop through the complete sequence.
    loop_length: int | None = None
    playhead: int = 0
    playing: bool = False
    # Record means "COMMIT held": it is deliberately off at startup.
    recording: bool = False
    last_note_played: tuple[int, int] | None = None
    held_notes: dict[int, int] = field(default_factory=dict)
    output_channel: int = 1
    default_gate: float = 0.5
    default_velocity: int = 100
    transpose: int = 0
    octave: int = 0
    selected_input: str | None = None
    selected_output: str | None = None
    cursor: int = 0
    root_capture_armed: bool = False
    status_message: str = "Ready: audition a note, then commit it."

    def __post_init__(self) -> None:
        self.set_step_division(self.step_division)
        self.set_bpm(self.bpm)
        self.set_loop_length(self.loop_length)
        self.output_channel = min(16, max(1, self.output_channel))

    def set_bpm(self, bpm: float) -> None:
        self.bpm = min(999.0, max(1.0, float(bpm)))

    def set_step_division(self, division: int) -> None:
        if division not in (1, 2, 4, 8, 16, 32, 64):
            raise ValueError("step_division must be a power-of-two note division")
        self.step_division = division

    def set_loop_length(self, length: int | None) -> None:
        if length not in LOOP_LENGTHS:
            raise ValueError("loop_length must be None, 8, 16, 32, or 64")
        self.loop_length = length

    def playback_length(self) -> int:
        """Return the active loop span, never longer than the sequence itself."""
        if self.loop_length is None:
            return len(self.sequence)
        return min(len(self.sequence), self.loop_length)

    def set_default_gate(self, gate: float, *, apply_existing: bool = True) -> None:
        """Set v1's global gate; existing steps follow it unless edited in future UI."""
        self.default_gate = min(1.0, max(0.0, float(gate)))
        if apply_existing:
            for step in self.sequence:
                step.gate = self.default_gate

    def append_note(self, note: int, velocity: int | None = None) -> Step:
        step = Step(
            note=note,
            velocity=self.default_velocity if velocity is None else velocity,
            gate=self.default_gate,
        )
        self.sequence.append(step)
        self.cursor = len(self.sequence) - 1
        return step

    def commit_last_note(self) -> bool:
        if self.last_note_played is None:
            self.status_message = "Nothing to commit: audition a MIDI note first."
            return False
        note, velocity = self.last_note_played
        self.append_note(note, velocity)
        self.status_message = f"Committed {note_name(note)} as step {len(self.sequence)}."
        return True

    def append_rest(self) -> Step:
        step = Step(note=None, velocity=self.default_velocity, gate=self.default_gate)
        self.sequence.append(step)
        self.cursor = len(self.sequence) - 1
        self.status_message = f"Added rest as step {len(self.sequence)}."
        return step

    def append_hold(self) -> Step | None:
        """Extend the immediately preceding pitched step by one full grid step.

        The preceding step is tied into a copied terminal step. The copy's gate
        is 1.0 so a hold is exactly one extra step long, independent of the
        normal note gate.
        """
        if not self.sequence or self.sequence[-1].note is None:
            self.status_message = "Hold needs a note immediately before it."
            return None
        previous = self.sequence[-1]
        previous.tie = True
        step = Step(
            note=previous.note,
            velocity=previous.velocity,
            gate=1.0,
            enabled=previous.enabled,
            tie=False,
            accent=previous.accent,
        )
        self.sequence.append(step)
        self.cursor = len(self.sequence) - 1
        self.status_message = f"Held {note_name(step.note)} for one more step."
        return step

    def undo(self) -> Step | None:
        if not self.sequence:
            self.status_message = "Nothing to undo."
            return None
        step = self.sequence.pop()
        # A v1 hold ties the preceding copied note only to the step removed here.
        # Restore normal release semantics when that extension is undone.
        if self.sequence and step.note is not None:
            previous = self.sequence[-1]
            if previous.note == step.note and previous.tie:
                previous.tie = False
        self.cursor = max(0, min(self.cursor, len(self.sequence) - 1))
        self.playhead = min(self.playhead, max(0, len(self.sequence) - 1))
        self.status_message = "Removed last step."
        return step

    def clear(self) -> None:
        self.sequence.clear()
        self.cursor = 0
        self.playhead = 0
        self.status_message = "Sequence cleared."

    def replace_cursor_with_last_note(self) -> bool:
        if not self.sequence:
            self.status_message = "There is no step to replace."
            return False
        if self.last_note_played is None:
            self.status_message = "Audition a MIDI note before replacing a step."
            return False
        note, velocity = self.last_note_played
        old = self.sequence[self.cursor]
        self.sequence[self.cursor] = Step(
            note=note,
            velocity=velocity,
            gate=old.gate,
            enabled=old.enabled,
            tie=old.tie,
            accent=old.accent,
        )
        self.status_message = f"Replaced step {self.cursor + 1} with {note_name(note)}."
        return True

    def delete_cursor(self) -> Step | None:
        if not self.sequence:
            self.status_message = "There is no step to delete."
            return None
        step = self.sequence.pop(self.cursor)
        self.cursor = max(0, min(self.cursor, len(self.sequence) - 1))
        self.playhead = min(self.playhead, max(0, len(self.sequence) - 1))
        self.status_message = "Deleted selected step."
        return step

    def move_cursor(self, delta: int) -> None:
        if self.sequence:
            self.cursor = (self.cursor + delta) % len(self.sequence)

    def adjust_cursor_note(self, semitones: int) -> bool:
        """Move the selected pitched step without destructively changing its other data."""
        if not self.sequence:
            self.status_message = "There is no step to edit."
            return False
        step = self.sequence[self.cursor]
        if step.note is None:
            self.status_message = "A rest has no pitch to change. Use E to replace it."
            return False
        step.note = min(127, max(0, step.note + semitones))
        self.status_message = f"Step {self.cursor + 1}: {note_name(step.note)}."
        return True

    def sequence_root(self) -> int | None:
        """The first pitched step is the v1 sequence root reference."""
        return next((step.note for step in self.sequence if step.note is not None), None)

    def set_root_from_note(self, target_note: int) -> bool:
        """Destructively move pitched steps so the first has target's pitch class.

        Root selection intentionally ignores octave and chooses the nearest signed
        interval: a C sequence targeted at Bb moves down two semitones, not up ten.
        Any prior live transpose is baked into the steps, so the grid and saved
        JSON immediately show the actual new MIDI notes.
        """
        root = self.sequence_root()
        if root is None:
            self.status_message = "Cannot set root: the sequence has no notes."
            return False
        audible_root = min(127, max(0, root + self.transpose))
        semitones = (target_note % 12 - audible_root % 12) % 12
        if semitones > 6:
            semitones -= 12
        total_shift = self.transpose + semitones
        for step in self.sequence:
            if step.note is not None:
                step.note = min(127, max(0, step.note + total_shift))
        self.transpose = 0
        self.status_message = (
            f"Root changed to {note_name(target_note)} ({total_shift:+d} semitones)."
        )
        return True

    def set_last_note(self, note: int, velocity: int) -> None:
        self.last_note_played = (note, velocity)

    def note_held(self, note: int, velocity: int) -> None:
        self.held_notes[note] = self.held_notes.get(note, 0) + 1
        self.set_last_note(note, velocity)
        self.status_message = f"Auditioning {note_name(note)} (velocity {velocity})."

    def note_released(self, note: int) -> None:
        count = self.held_notes.get(note, 0)
        if count <= 1:
            self.held_notes.pop(note, None)
        else:
            self.held_notes[note] = count - 1

    def to_dict(self) -> dict[str, Any]:
        return {
            "version": 1,
            "bpm": self.bpm,
            "step_division": self.step_division,
            "loop_length": self.loop_length,
            "default_gate": self.default_gate,
            "default_velocity": self.default_velocity,
            "transpose": self.transpose,
            "output_channel": self.output_channel,
            "steps": [asdict(step) for step in self.sequence],
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> AppState:
        if data.get("version", 1) != 1:
            raise ValueError(f"Unsupported sequence format version: {data.get('version')}")
        state = cls(
            bpm=float(data.get("bpm", 128)),
            step_division=int(data.get("step_division", 16)),
            loop_length=data.get("loop_length"),
            default_gate=float(data.get("default_gate", 0.5)),
            default_velocity=int(data.get("default_velocity", 100)),
            transpose=int(data.get("transpose", 0)),
            output_channel=int(data.get("output_channel", 1)),
        )
        state.sequence = [Step(**raw) for raw in data.get("steps", [])]
        return state

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(self.to_dict(), indent=2) + "\n", encoding="utf-8")
        self.status_message = f"Saved {len(self.sequence)} steps to {path}."

    def load(self, path: Path) -> None:
        loaded = self.from_dict(json.loads(path.read_text(encoding="utf-8")))
        selected_input, selected_output = self.selected_input, self.selected_output
        self.__dict__.update(loaded.__dict__)
        self.selected_input, self.selected_output = selected_input, selected_output
        self.status_message = f"Loaded {len(self.sequence)} steps from {path}."


NOTE_NAMES = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")
PLAYBACK_DIVISIONS = (4, 8, 16, 32)


def note_name(note: int) -> str:
    """Return an Ableton Live-style note name (MIDI 60 is C3)."""
    if not 0 <= note <= 127:
        raise ValueError("MIDI note must be between 0 and 127")
    return f"{NOTE_NAMES[note % 12]}{note // 12 - 2}"
