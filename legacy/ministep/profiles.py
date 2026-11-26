"""Configurable MIDI CC control pages for hardware-controller profiles."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

import mido
import yaml

# MiniLab's eight small encoders in Arturia/User and DAW programs respectively.
MINILAB3_KNOB_CONTROLS = {
    74: 1,
    71: 2,
    76: 3,
    77: 4,
    93: 5,
    18: 6,
    19: 7,
    16: 8,
    86: 1,
    87: 2,
    89: 3,
    90: 4,
    110: 5,
    111: 6,
    116: 7,
    117: 8,
}


class ProfileError(ValueError):
    """Raised when a control-profile YAML file does not match the schema."""


class MidiControlOutput(Protocol):
    def control_change(self, control: int, value: int, channel: int = 0) -> None: ...


@dataclass(frozen=True)
class KnobBinding:
    cc: int
    label: str
    channel: int


@dataclass(frozen=True)
class ControlPage:
    name: str
    knobs: dict[int, KnobBinding]


@dataclass(frozen=True)
class ControlProfile:
    name: str
    pages: tuple[ControlPage, ...]
    output: str | None = None

    @property
    def page_names(self) -> tuple[str, ...]:
        return tuple(page.name for page in self.pages)


@dataclass(frozen=True)
class RoutedControl:
    page: str
    knob: int
    label: str
    value: int


class ProfileRouter:
    """Route physical MiniLab knob events through the active profile page."""

    def __init__(self, profile: ControlProfile, output: MidiControlOutput) -> None:
        self.profile = profile
        self.output = output
        self.page_index = 0

    @property
    def page(self) -> ControlPage:
        return self.profile.pages[self.page_index]

    def select_page(self, index: int) -> None:
        self.page_index = index % len(self.profile.pages)

    def is_knob_message(self, event: mido.Message) -> bool:
        """Return whether an event came from one of MiniLab's eight knobs."""
        return event.type == "control_change" and event.control in MINILAB3_KNOB_CONTROLS

    def handle_message(self, event: mido.Message) -> RoutedControl | None:
        if not self.is_knob_message(event):
            return None
        knob = MINILAB3_KNOB_CONTROLS[event.control]
        binding = self.page.knobs.get(knob)
        if binding is None:
            return None
        self.output.control_change(binding.cc, event.value, binding.channel - 1)
        return RoutedControl(self.page.name, knob, binding.label, event.value)


def load_profile(path: Path) -> ControlProfile:
    """Load and validate a controller profile from YAML."""
    try:
        raw = yaml.safe_load(path.read_text(encoding="utf-8"))
    except OSError as error:
        raise ProfileError(f"cannot read {path}: {error}") from error
    except yaml.YAMLError as error:
        raise ProfileError(f"invalid YAML in {path}: {error}") from error
    if not isinstance(raw, dict):
        raise ProfileError("profile must be a YAML mapping")

    name = _string(raw.get("name"), "name")
    output = raw.get("output")
    if output is not None:
        output = _string(output, "output")
    channel = _channel(raw.get("channel", 1), "channel")
    pages_raw = raw.get("pages")
    if not isinstance(pages_raw, dict) or not pages_raw:
        raise ProfileError("pages must be a non-empty mapping")

    pages = tuple(
        _page(str(page_name), page_raw, channel) for page_name, page_raw in pages_raw.items()
    )
    return ControlProfile(name=name, pages=pages, output=output)


def _page(name: str, raw: object, default_channel: int) -> ControlPage:
    if not isinstance(raw, dict):
        raise ProfileError(f"page {name!r} must be a mapping")
    knobs_raw = raw.get("knobs", {})
    if not isinstance(knobs_raw, dict):
        raise ProfileError(f"page {name!r}.knobs must be a mapping")
    knobs: dict[int, KnobBinding] = {}
    for raw_knob, raw_binding in knobs_raw.items():
        try:
            knob = int(raw_knob)
        except (TypeError, ValueError) as error:
            raise ProfileError(f"page {name!r} has a non-numeric knob key") from error
        if not 1 <= knob <= 8:
            raise ProfileError(f"page {name!r} knob must be between 1 and 8")
        if not isinstance(raw_binding, dict):
            raise ProfileError(f"page {name!r} knob {knob} must be a mapping")
        cc = _cc(raw_binding.get("cc"), f"page {name!r} knob {knob}.cc")
        label = _string(raw_binding.get("label"), f"page {name!r} knob {knob}.label")
        knob_channel = _channel(raw_binding.get("channel", default_channel), "channel")
        knobs[knob] = KnobBinding(cc=cc, label=label, channel=knob_channel)
    return ControlPage(name=name, knobs=knobs)


def _string(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ProfileError(f"{field} must be a non-empty string")
    return value


def _cc(value: object, field: str) -> int:
    if not isinstance(value, int) or not 0 <= value <= 127:
        raise ProfileError(f"{field} must be an integer between 0 and 127")
    return value


def _channel(value: object, field: str) -> int:
    if not isinstance(value, int) or not 1 <= value <= 16:
        raise ProfileError(f"{field} must be an integer between 1 and 16")
    return value
