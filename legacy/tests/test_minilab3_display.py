from __future__ import annotations

import mido
import pytest

from ministep.minilab3_display import MiniLab3Display


class FakePort:
    def __init__(self) -> None:
        self.messages: list[mido.Message] = []
        self.closed = False

    def send(self, message: mido.Message) -> None:
        self.messages.append(message.copy())

    def close(self) -> None:
        self.closed = True


def test_encoder_view_uses_label_value_and_refreshes() -> None:
    display_port = FakePort()
    display = MiniLab3Display(display_port)

    display.connect()
    display.show_encoder("CUTOFF", 96)
    display.refresh()

    assert len(display_port.messages) == 4
    message = display_port.messages[-1]
    assert message.type == "sysex"
    expected = (
        0x1F,
        0x03,
        0x01,
        96,
        0x00,
        0x00,
        0x01,
        *b"CUTOFF",
        0x00,
        0x02,
        *b"096",
        0x00,
    )
    assert tuple(message.data[-len(expected) :]) == expected


def test_known_knob_cc_updates_the_display() -> None:
    display_port = FakePort()
    display = MiniLab3Display(display_port)

    assert display.handle_message(mido.Message("control_change", control=87, value=55))
    assert b"CC 87" in bytes(display_port.messages[-1].data)
    assert not display.handle_message(mido.Message("control_change", control=1, value=55))


def test_pad_colours_require_eight_valid_rgb_triplets() -> None:
    display_port = FakePort()
    pad_port = FakePort()
    display = MiniLab3Display(display_port, pad_port)

    display.set_pad_colours([(1, 2, 3)] * 8)
    assert pad_port.messages[-1].type == "sysex"
    with pytest.raises(ValueError, match="eight"):
        display.set_pad_colours([(1, 2, 3)] * 7)
    with pytest.raises(ValueError, match="0 and 127"):
        display.set_pad_colours([(128, 2, 3)] * 8)


def test_step_status_marks_playhead_and_record_pad() -> None:
    display_port = FakePort()
    pad_port = FakePort()
    display = MiniLab3Display(display_port, pad_port)

    display.show_step_status(recording=True, playing=True, playhead=2, cursor=5)

    data = tuple(pad_port.messages[-1].data)
    assert data[-24:] == (
        0,
        0,
        20,
        0,
        0,
        20,
        0,
        127,
        0,
        0,
        0,
        20,
        0,
        0,
        20,
        0,
        0,
        20,
        0,
        0,
        20,
        127,
        0,
        0,
    )


def test_recording_without_playback_marks_the_current_recording_step() -> None:
    display_port = FakePort()
    pad_port = FakePort()
    display = MiniLab3Display(display_port, pad_port)

    display.show_step_status(recording=True, playing=False, playhead=0, cursor=3)

    data = tuple(pad_port.messages[-1].data)
    assert data[-24:] == (
        0,
        0,
        20,
        0,
        0,
        20,
        0,
        0,
        20,
        127,
        0,
        0,
        0,
        0,
        20,
        0,
        0,
        20,
        0,
        0,
        20,
        127,
        42,
        0,
    )
