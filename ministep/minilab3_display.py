"""Small, self-contained display and pad-colour client for Arturia MiniLab 3.

The SysEx format is empirically derived.  The MiniLab firmware can overwrite its
OLED with its own status, so call :meth:`refresh` periodically while a custom
screen must remain visible.
"""

from __future__ import annotations

from collections.abc import Iterable, Sequence
from typing import Protocol

import mido

MIDI_PORT = "Minilab3 MIDI"
ALV_PORT = "Minilab3 ALV"
_HEADER = (0x00, 0x20, 0x6B, 0x7F, 0x42)
_ARTURIA_CONNECT = (0x04, 0x01, 0x60, 0x01, 0x00, 0x02, 0x00)
_DAW_CONNECT = (0x02, 0x02, 0x40, 0x6A, 0x21)

# The first row is Arturia/User mode; the second is DAW mode.
KNOB_CONTROLS = {
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


class MidiOutput(Protocol):
    def send(self, message: mido.Message) -> None: ...

    def close(self) -> None: ...


def _sysex(body: Iterable[int]) -> mido.Message:
    return mido.Message("sysex", data=_HEADER + tuple(body))


def _text(value: str) -> bytes:
    """Return a screen-safe, zero-terminated-text payload (maximum 30 bytes)."""
    return value.encode("ascii", errors="replace")[:30]


class MiniLab3Display:
    """Control a MiniLab 3 OLED screen and, optionally, its RGB pads.

    Use :meth:`open` for the normal two-port setup, then call :meth:`connect`.
    A display state is retained locally; :meth:`refresh` re-sends that state.
    """

    def __init__(
        self,
        display_port: MidiOutput,
        pad_port: MidiOutput | None = None,
        *,
        close_ports: bool = False,
    ) -> None:
        self._display_port = display_port
        self._pad_port = pad_port
        self._close_ports = close_ports
        self._screen: mido.Message | None = None

    @classmethod
    def open(cls, display_port: str = ALV_PORT, pad_port: str = MIDI_PORT) -> MiniLab3Display:
        """Open MiniLab's ALV display port and normal MIDI pad-colour port."""
        return cls(
            mido.open_output(display_port),
            mido.open_output(pad_port),
            close_ports=True,
        )

    def connect(self) -> None:
        """Claim the Arturia/DAW display channel before drawing."""
        self._display_port.send(_sysex(_ARTURIA_CONNECT))
        self._display_port.send(_sysex(_DAW_CONNECT))

    def show_text(self, line1: str, line2: str = "") -> None:
        """Set a two-line OLED display buffer."""
        self._screen = _sysex(
            (
                0x04,
                0x02,
                0x60,
                0x1F,
                0x02,
                0x01,
                0x00,
                0x01,
                *_text(line1),
                0x00,
                0x02,
                *_text(line2),
                0x00,
            )
        )
        self.refresh()

    def show_encoder(self, label: str, value: int, value_text: str | None = None) -> None:
        """Set the encoder OLED view with a 0–127 value bar."""
        if not 0 <= value <= 127:
            raise ValueError("MiniLab encoder value must be between 0 and 127")
        self._screen = _sysex(
            (
                0x04,
                0x02,
                0x60,
                0x1F,
                0x03,
                0x01,
                value,
                0x00,
                0x00,
                0x01,
                *_text(label),
                0x00,
                0x02,
                *_text(value_text if value_text is not None else f"{value:03}"),
                0x00,
            )
        )
        self.refresh()

    def handle_message(
        self,
        event: mido.Message,
        label: str | None = None,
        value_text: str | None = None,
    ) -> bool:
        """Render a recognised physical knob event; return whether it was handled."""
        if event.type != "control_change" or event.control not in KNOB_CONTROLS:
            return False
        self.show_encoder(label or f"CC {event.control}", event.value, value_text)
        return True

    def refresh(self) -> None:
        """Re-send the most recent screen to counter MiniLab status redraws."""
        if self._screen is not None:
            self._display_port.send(self._screen)

    def set_pad_colours(self, colours: Sequence[Sequence[int]]) -> None:
        """Set eight temporary RGB pad colours, each component in the 0–127 range."""
        if self._pad_port is None:
            raise RuntimeError("A MIDI pad output port was not configured")
        if len(colours) != 8 or any(len(colour) != 3 for colour in colours):
            raise ValueError("MiniLab needs exactly eight RGB pad colours")
        values = tuple(component for colour in colours for component in colour)
        if any(not 0 <= component <= 127 for component in values):
            raise ValueError("MiniLab RGB components must be between 0 and 127")
        self._pad_port.send(_sysex((0x04, 0x02, 0x16, 0x00, *values)))

    def show_step_status(
        self,
        *,
        recording: bool,
        playing: bool,
        playhead: int,
        cursor: int,
    ) -> None:
        """Show MiniStep transport state across the eight MiniLab pads.

        The active playback step moves in green. While stopped recording, the
        selected/last recorded step moves in red. Pad 8 is amber as a recording
        status indicator, or red while recording during playback.
        """
        colours = [(0, 0, 20)] * 8
        active_pad = playhead % 8
        if playing:
            colours[active_pad] = (0, 127, 0)
        elif recording:
            active_pad = cursor % 8
            colours[active_pad] = (127, 0, 0)
        if recording and playing:
            colours[7] = (127, 42, 0) if playing and active_pad == 7 else (127, 0, 0)
        elif recording and active_pad != 7:
            colours[7] = (127, 42, 0)
        self.set_pad_colours(colours)

    def close(self) -> None:
        """Close only ports opened by :meth:`open`."""
        if self._close_ports:
            self._display_port.close()
            if self._pad_port is not None and self._pad_port is not self._display_port:
                self._pad_port.close()
