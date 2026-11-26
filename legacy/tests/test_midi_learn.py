from __future__ import annotations

import mido

from ministep.controller import Command, CommandName, SimpleMapping
from ministep.runtime import MiniStepRuntime
from ministep.state import AppState


def test_midi_learn_maps_button_cc_once_and_persists(tmp_path) -> None:  # type: ignore[no-untyped-def]
    path = tmp_path / "mapping.json"
    mapping = SimpleMapping(note_commands={}, cc_commands={})
    runtime = MiniStepRuntime(AppState(), output=None, mapping=mapping, mapping_path=path)
    assert runtime.arm_midi_learn(CommandName.RECORD_TOGGLE) is True

    runtime.handle_midi_message(mido.Message("control_change", control=74, value=127))
    assert path.exists()
    assert mapping.command_for(mido.Message("control_change", control=74, value=127)) is not None
    assert mapping.command_for(mido.Message("control_change", control=74, value=0)) is None
    assert (
        SimpleMapping.load(path)
        .command_for(mido.Message("control_change", control=74, value=127))
        .name
        == CommandName.RECORD_TOGGLE
    )


def test_midi_learn_uses_absolute_range_for_bpm() -> None:
    mapping = SimpleMapping(note_commands={}, cc_commands={})
    mapping.learn_cc(16, CommandName.SET_BPM)

    command = mapping.command_for(mido.Message("control_change", control=16, value=127))
    assert command is not None
    assert command.name == CommandName.SET_BPM
    assert command.value == 240


def test_midi_learn_maps_a_pad_note_only_on_its_midi_channel(tmp_path) -> None:  # type: ignore[no-untyped-def]
    path = tmp_path / "mapping.json"
    mapping = SimpleMapping(note_commands={}, cc_commands={})
    runtime = MiniStepRuntime(AppState(), output=None, mapping=mapping, mapping_path=path)
    assert runtime.arm_midi_learn(CommandName.PLAY_STOP) is True

    runtime.handle_midi_message(mido.Message("note_on", channel=9, note=40, velocity=100))

    restored = SimpleMapping.load(path)
    assert restored.command_for(
        mido.Message("note_on", channel=9, note=40, velocity=100)
    ) == Command(CommandName.PLAY_STOP)
    assert restored.command_for(mido.Message("note_on", channel=0, note=40, velocity=100)) is None
