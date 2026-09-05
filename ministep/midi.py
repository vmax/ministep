"""MIDI port discovery, asynchronous input, and safe output-note ownership."""

from __future__ import annotations

import asyncio
from collections import Counter
from collections.abc import Callable
from typing import Protocol

import mido


class MidiSink(Protocol):
    def note_on(
        self, note: int, velocity: int, channel: int = 0, owner: str = "sequencer"
    ) -> None: ...

    def note_off(self, note: int, channel: int = 0, owner: str = "sequencer") -> None: ...

    def control_change(self, control: int, value: int, channel: int = 0) -> None: ...

    def all_notes_off(self) -> None: ...


def input_ports() -> list[str]:
    return mido.get_input_names()


def output_ports() -> list[str]:
    return mido.get_output_names()


def find_port(query: str, ports: list[str]) -> str | None:
    """Find an exact, case-insensitive, or substring match for a port name."""
    if query in ports:
        return query
    query_lower = query.lower()
    for name in ports:
        if name.lower() == query_lower:
            return name
    matches = [name for name in ports if query_lower in name.lower()]
    return matches[0] if len(matches) == 1 else None


class MidoOutput:
    """A ``MidiSink`` backed by an open mido output port."""

    def __init__(self, port: mido.ports.BaseOutput) -> None:
        self.port = port

    def note_on(self, note: int, velocity: int, channel: int = 0, owner: str = "sequencer") -> None:
        self.port.send(mido.Message("note_on", note=note, velocity=velocity, channel=channel))

    def note_off(self, note: int, channel: int = 0, owner: str = "sequencer") -> None:
        self.port.send(mido.Message("note_off", note=note, velocity=0, channel=channel))

    def control_change(self, control: int, value: int, channel: int = 0) -> None:
        self.port.send(
            mido.Message("control_change", control=control, value=value, channel=channel)
        )

    def send_raw(self, message: mido.Message, channel: int | None = None) -> None:
        if channel is not None and hasattr(message, "channel"):
            message = message.copy(channel=channel)
        self.port.send(message)

    def all_notes_off(self) -> None:
        for channel in range(16):
            self.control_change(123, 0, channel)

    def close(self) -> None:
        self.port.close()


class OwnedMidiOutput:
    """Reference-count output notes by source to avoid cross-source stuck notes.

    Input pass-through and sequencer playback can emit the same pitch. A physical
    note-off must not turn off a currently owned sequencer note, and vice versa.
    """

    def __init__(self, output: MidoOutput) -> None:
        self.output = output
        self._owners: Counter[tuple[str, int, int]] = Counter()

    def _total(self, note: int, channel: int) -> int:
        return sum(
            count
            for (_, saved_note, saved_channel), count in self._owners.items()
            if saved_note == note and saved_channel == channel
        )

    def note_on(self, note: int, velocity: int, channel: int = 0, owner: str = "sequencer") -> None:
        was_sounding = self._total(note, channel) > 0
        self._owners[(owner, note, channel)] += 1
        if not was_sounding:
            self.output.note_on(note, velocity, channel, owner)

    def note_off(self, note: int, channel: int = 0, owner: str = "sequencer") -> None:
        key = (owner, note, channel)
        if self._owners[key] <= 0:
            return
        self._owners[key] -= 1
        if self._owners[key] == 0:
            del self._owners[key]
        if self._total(note, channel) == 0:
            self.output.note_off(note, channel, owner)

    def control_change(self, control: int, value: int, channel: int = 0) -> None:
        self.output.control_change(control, value, channel)

    def send_raw(self, message: mido.Message, channel: int | None = None) -> None:
        self.output.send_raw(message, channel)

    def release_owner(self, owner: str) -> None:
        keys = [key for key in self._owners if key[0] == owner]
        for _, note, channel in keys:
            while (owner, note, channel) in self._owners:
                self.note_off(note, channel, owner)

    def all_notes_off(self) -> None:
        self._owners.clear()
        self.output.all_notes_off()

    def close(self) -> None:
        self.all_notes_off()
        self.output.close()


class MidiManager:
    """Owns mido ports and marshals callback-thread MIDI into the asyncio loop."""

    def __init__(
        self, loop: asyncio.AbstractEventLoop, on_message: Callable[[mido.Message], None]
    ) -> None:
        self.loop = loop
        self.on_message = on_message
        self.input: mido.ports.BaseInput | None = None
        self.output: OwnedMidiOutput | None = None

    def open_input(self, name: str) -> None:
        self.input = mido.open_input(name, callback=self._on_mido_message)

    def open_output(self, name: str) -> OwnedMidiOutput:
        self.output = OwnedMidiOutput(MidoOutput(mido.open_output(name)))
        return self.output

    def _on_mido_message(self, message: mido.Message) -> None:
        self.loop.call_soon_threadsafe(self.on_message, message.copy())

    def close(self) -> None:
        if self.input is not None:
            self.input.close()
            self.input = None
        if self.output is not None:
            self.output.close()
            self.output = None
