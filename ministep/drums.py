"""Drum track model: fixed 16-step lanes that fire short MIDI notes.

Kept separate from the melodic :class:`~ministep.state.Step` on purpose: a drum
hit has no pitch, tie, or gate; it is an on/off flag with a velocity. The
module holds no MIDI or UI code and never imports :mod:`ministep.state`.
"""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any

from .notes import note_name, parse_note_name

DRUM_STEPS = 16
DEFAULT_DRUM_VELOCITY = 100
DEFAULT_DRUM_CHANNEL = 10  # one-based, as shown to the user
# Default lanes as (name, MIDI note). Notes follow the General MIDI drum map,
# which is also Ableton Drum Rack's default pad layout: C1 = 36 in MiniStep's
# Ableton-style naming (MIDI 60 is C3).
DEFAULT_DRUM_LANES: tuple[tuple[str, int], ...] = (
    ("Kick", 36),  # C1
    ("Snare", 38),  # D1
    ("Closed Hat", 42),  # F#1
    ("Open Hat", 46),  # A#1
)


@dataclass
class DrumStep:
    active: bool = False
    velocity: int = DEFAULT_DRUM_VELOCITY

    def __post_init__(self) -> None:
        if not 1 <= self.velocity <= 127:
            raise ValueError("drum velocity must be between 1 and 127")


@dataclass
class DrumLane:
    name: str
    note: int
    steps: list[DrumStep] = field(default_factory=lambda: [DrumStep() for _ in range(DRUM_STEPS)])

    def __post_init__(self) -> None:
        if not 0 <= self.note <= 127:
            raise ValueError("MIDI note must be between 0 and 127")
        if len(self.steps) != DRUM_STEPS:
            raise ValueError(f"a drum lane has exactly {DRUM_STEPS} steps")


@dataclass
class DrumTrack:
    lanes: list[DrumLane] = field(default_factory=list)
    # One-based MIDI channel, like ``AppState.output_channel``.
    channel: int = DEFAULT_DRUM_CHANNEL
    # Edit cursor; UI state, not persisted.
    cursor_lane: int = 0
    cursor_step: int = 0
    # Hits auditioned from a MIDI keyboard but not yet committed: lane index ->
    # velocity. Playing the same lane again just updates its velocity.
    audition: dict[int, int] = field(default_factory=dict, compare=False)

    def __post_init__(self) -> None:
        self.channel = min(16, max(1, int(self.channel)))

    @classmethod
    def default(cls) -> DrumTrack:
        """Four empty lanes: Kick, Snare, Closed Hat, Open Hat. No preset beat."""
        return cls(lanes=[DrumLane(name, note) for name, note in DEFAULT_DRUM_LANES])

    # --- playback queries ---------------------------------------------------

    def has_hits(self) -> bool:
        return any(step.active for lane in self.lanes for step in lane.steps)

    def hits_at(self, index: int) -> list[tuple[DrumLane, DrumStep]]:
        """Active (lane, step) pairs on grid position ``index`` (0..15)."""
        position = index % DRUM_STEPS
        return [(lane, lane.steps[position]) for lane in self.lanes if lane.steps[position].active]

    # --- audition / commit (the melodic PLAY -> COMMIT -> STEP workflow) ---------

    def lane_for_note(self, note: int) -> int | None:
        return next((index for index, lane in enumerate(self.lanes) if lane.note == note), None)

    def audition_note(self, note: int, velocity: int) -> DrumLane | None:
        """Remember a played drum note for the next commit; None if no lane owns it."""
        index = self.lane_for_note(note)
        if index is None:
            return None
        self.audition[index] = min(127, max(1, velocity))
        self.cursor_lane = index
        return self.lanes[index]

    def commit_audition(self) -> list[DrumLane]:
        """Write every auditioned hit into the cursor step, then move to the next step."""
        if not self.audition:
            return []
        committed: list[DrumLane] = []
        for index, velocity in sorted(self.audition.items()):
            step = self.lanes[index].steps[self.cursor_step]
            step.active = True
            step.velocity = velocity
            committed.append(self.lanes[index])
        self.audition.clear()
        self.move_step(1)
        return committed

    def clear_audition(self) -> None:
        self.audition.clear()

    def skip_step(self) -> None:
        """Drum "rest": leave the cursor step as it is, drop the audition, advance."""
        self.audition.clear()
        self.move_step(1)

    # --- editing --------------------------------------------------------------

    @property
    def cursor(self) -> DrumStep:
        return self.lanes[self.cursor_lane].steps[self.cursor_step]

    @property
    def cursor_lane_obj(self) -> DrumLane:
        return self.lanes[self.cursor_lane]

    def move_step(self, delta: int) -> None:
        self.cursor_step = (self.cursor_step + delta) % DRUM_STEPS

    def move_lane(self, delta: int) -> None:
        if self.lanes:
            self.cursor_lane = (self.cursor_lane + delta) % len(self.lanes)

    def toggle_hit(self) -> bool:
        """Flip the hit under the cursor; return its new ``active`` value."""
        step = self.cursor
        step.active = not step.active
        return step.active

    def clear_hit(self) -> None:
        self.cursor.active = False

    def adjust_velocity(self, delta: int) -> int:
        step = self.cursor
        step.velocity = min(127, max(1, step.velocity + delta))
        return step.velocity

    def clear(self) -> None:
        for lane in self.lanes:
            for step in lane.steps:
                step.active = False
                step.velocity = DEFAULT_DRUM_VELOCITY

    # --- persistence ------------------------------------------------------------

    def to_dict(self) -> dict[str, Any]:
        """Musical content only: lanes, notes (as names), hits, velocities, channel."""
        return {
            "channel": self.channel,
            "lanes": [
                {
                    "name": lane.name,
                    "note": note_name(lane.note),
                    "steps": [asdict(step) for step in lane.steps],
                }
                for lane in self.lanes
            ],
        }

    @classmethod
    def from_dict(cls, raw: Any) -> DrumTrack:
        """Build a track from :meth:`to_dict` output; ``ValueError`` on bad structure.

        Notes may be names (``"C1"``) or numbers, short lanes are padded with
        silent steps, and a missing velocity is the default 100.
        """
        if not isinstance(raw, dict):
            raise ValueError("drums must be an object")
        raw_lanes = raw.get("lanes", [])
        if not isinstance(raw_lanes, list):
            raise ValueError("drum lanes must be a list")
        lanes: list[DrumLane] = []
        for raw_lane in raw_lanes:
            if not isinstance(raw_lane, dict) or "note" not in raw_lane:
                raise ValueError("each drum lane needs a 'note'")
            note = raw_lane["note"]
            note = parse_note_name(note) if isinstance(note, str) else int(note)
            raw_steps = raw_lane.get("steps", [])
            if not isinstance(raw_steps, list) or len(raw_steps) > DRUM_STEPS:
                raise ValueError(f"drum lane steps must be a list of at most {DRUM_STEPS}")
            steps = [
                DrumStep(
                    active=bool(step.get("active", False)),
                    velocity=int(step.get("velocity", DEFAULT_DRUM_VELOCITY)),
                )
                if isinstance(step, dict)
                else DrumStep()
                for step in raw_steps
            ]
            steps.extend(DrumStep() for _ in range(DRUM_STEPS - len(steps)))
            name = str(raw_lane.get("name", note_name(note)))
            lanes.append(DrumLane(name=name, note=note, steps=steps))
        return cls(lanes=lanes, channel=int(raw.get("channel", DEFAULT_DRUM_CHANNEL)))

    def replace_content(self, other: DrumTrack) -> None:
        """Take ``other``'s lanes and channel; reset the editing cursor and audition."""
        self.lanes = other.lanes
        self.channel = other.channel
        self.cursor_lane = 0
        self.cursor_step = 0
        self.audition.clear()
