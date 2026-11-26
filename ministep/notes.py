"""MIDI note naming in Ableton Live's convention: MIDI 60 is ``C3``."""

from __future__ import annotations

import re

NOTE_NAMES = ("C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B")
_NOTE_NAME = re.compile(r"([A-Ga-g])([#b]?)(-?\d+)")


def note_name(note: int) -> str:
    """Return an Ableton Live-style note name (MIDI 60 is C3)."""
    if not 0 <= note <= 127:
        raise ValueError("MIDI note must be between 0 and 127")
    return f"{NOTE_NAMES[note % 12]}{note // 12 - 2}"


def parse_note_name(name: str) -> int:
    """Inverse of :func:`note_name`: ``"C3"`` is 60, ``"F#1"`` is 42, ``"Bb1"`` is 46."""
    match = _NOTE_NAME.fullmatch(name.strip())
    if match is None:
        raise ValueError(f"Unknown note name {name!r}")
    letter, accidental, octave = match.groups()
    pitch = NOTE_NAMES.index(letter.upper())
    pitch += {"#": 1, "b": -1, "": 0}[accidental]
    note = pitch + (int(octave) + 2) * 12
    if not 0 <= note <= 127:
        raise ValueError(f"Note {name!r} is outside the MIDI range")
    return note
