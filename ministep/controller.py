"""Translate controller-specific MIDI messages into application commands."""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import Enum, auto
from pathlib import Path
from typing import Protocol


class CommandName(Enum):
    COMMIT = auto()
    HOLD = auto()
    RECORD = auto()
    RECORD_TOGGLE = auto()
    REST = auto()
    UNDO = auto()
    CLEAR = auto()
    PLAY = auto()
    STOP = auto()
    PLAY_STOP = auto()
    RESTART = auto()
    BPM_UP = auto()
    BPM_DOWN = auto()
    STEP_DIVISION_UP = auto()
    STEP_DIVISION_DOWN = auto()
    SET_BPM = auto()
    SET_GATE = auto()
    OCTAVE_UP = auto()
    OCTAVE_DOWN = auto()
    TRANSPOSE_UP = auto()
    TRANSPOSE_DOWN = auto()
    ARM_ROOT_CAPTURE = auto()
    SET_TRANSPOSE = auto()
    SET_OCTAVE = auto()


@dataclass(frozen=True)
class Command:
    name: CommandName
    value: float | int | None = None


class ControllerMapping(Protocol):
    def command_for(self, message: object) -> Command | None: ...


@dataclass(frozen=True)
class AbsoluteCC:
    """Map a CC value 0..127 linearly to an application command value."""

    control: int
    command: CommandName
    minimum: float
    maximum: float
    integer: bool = False

    def translate(self, value: int) -> Command:
        scaled = self.minimum + (self.maximum - self.minimum) * (value / 127)
        return Command(self.command, round(scaled) if self.integer else scaled)


@dataclass
class SimpleMapping:
    """Editable mapping for pad notes and absolute CC controls.

    Relative encoder support belongs here later, leaving command dispatch unchanged.
    """

    note_commands: dict[tuple[int, int], Command]
    cc_commands: dict[int, Command]
    absolute_cc: tuple[AbsoluteCC, ...] = ()

    def command_for(self, message: object) -> Command | None:
        msg_type = getattr(message, "type", None)
        velocity = getattr(message, "velocity", 0)
        if msg_type == "note_on" and velocity > 0:
            key = (getattr(message, "channel", -1), getattr(message, "note", -1))
            return self.note_commands.get(key)
        if msg_type == "control_change":
            control = getattr(message, "control", -1)
            if control in self.cc_commands:
                # CC pads often emit 127 on press and 0 on release. Commands
                # should run only once, while continuous controls use rules below.
                return self.cc_commands[control] if getattr(message, "value", 0) > 0 else None
            value = getattr(message, "value", 0)
            for rule in self.absolute_cc:
                if rule.control == control:
                    return rule.translate(value)
        return None

    def learn_cc(self, control: int, command: CommandName) -> None:
        """Learn one safe CC control, replacing any old action on that CC."""
        self.cc_commands.pop(control, None)
        self.absolute_cc = tuple(rule for rule in self.absolute_cc if rule.control != control)
        if command in ABSOLUTE_CC_RANGES:
            minimum, maximum, integer = ABSOLUTE_CC_RANGES[command]
            self.absolute_cc += (AbsoluteCC(control, command, minimum, maximum, integer),)
        else:
            self.cc_commands[control] = Command(command)

    def learn_note(self, channel: int, note: int, command: CommandName) -> None:
        """Map one note-on source, such as a MiniLab pad, to a button command."""
        if command in ABSOLUTE_CC_RANGES:
            raise ValueError(f"{command.name} needs a continuous CC control")
        self.note_commands[(channel, note)] = Command(command)

    def to_dict(self) -> dict[str, object]:
        return {
            "version": 1,
            "cc_commands": [
                {"control": control, "command": command.name.name}
                for control, command in sorted(self.cc_commands.items())
            ],
            "note_commands": [
                {"channel": channel, "note": note, "command": command.name.name}
                for (channel, note), command in sorted(self.note_commands.items())
            ],
            "absolute_cc": [
                {
                    "control": rule.control,
                    "command": rule.command.name,
                    "minimum": rule.minimum,
                    "maximum": rule.maximum,
                    "integer": rule.integer,
                }
                for rule in self.absolute_cc
            ],
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> SimpleMapping:
        if data.get("version", 1) != 1:
            raise ValueError("Unsupported MIDI mapping version")
        cc_commands = {
            int(item["control"]): Command(CommandName[item["command"]])
            for item in data.get("cc_commands", [])  # type: ignore[union-attr]
        }
        note_commands = {
            (int(item["channel"]), int(item["note"])): Command(CommandName[item["command"]])
            for item in data.get("note_commands", [])  # type: ignore[union-attr]
        }
        absolute_cc = tuple(
            AbsoluteCC(
                control=int(item["control"]),
                command=CommandName[item["command"]],
                minimum=float(item["minimum"]),
                maximum=float(item["maximum"]),
                integer=bool(item["integer"]),
            )
            for item in data.get("absolute_cc", [])  # type: ignore[union-attr]
        )
        return cls(note_commands=note_commands, cc_commands=cc_commands, absolute_cc=absolute_cc)

    @classmethod
    def load(cls, path: Path) -> SimpleMapping:
        return cls.from_dict(json.loads(path.read_text(encoding="utf-8")))

    def save(self, path: Path) -> None:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(self.to_dict(), indent=2) + "\n", encoding="utf-8")


# These commands need an absolute encoder rather than a button/pad CC.
ABSOLUTE_CC_RANGES: dict[CommandName, tuple[float, float, bool]] = {
    CommandName.SET_BPM: (40, 240, True),
    CommandName.SET_GATE: (0.05, 1.0, False),
    CommandName.SET_TRANSPOSE: (-24, 24, True),
    CommandName.SET_OCTAVE: (-3, 3, True),
}

DEFAULT_MIDI_MAPPING_PATH = Path.home() / ".config" / "ministep" / "minilab3-mapping.json"
