from __future__ import annotations

import asyncio

import mido

from ministep.controller import Command, CommandName
from ministep.runtime import MiniStepRuntime
from ministep.state import AppState, Step


async def test_record_mode_commits_each_physical_note_without_using_wall_clock() -> None:
    state = AppState()
    runtime = MiniStepRuntime(state, output=None)

    await runtime.execute(Command(CommandName.RECORD_TOGGLE))
    assert state.recording is True
    assert state.playing is False

    runtime.handle_midi_message(mido.Message("note_on", note=60, velocity=91))
    await asyncio.sleep(0.001)
    runtime.handle_midi_message(mido.Message("note_on", note=63, velocity=102))
    await asyncio.sleep(0.002)
    runtime.handle_midi_message(mido.Message("note_on", note=67, velocity=113))

    assert [(step.note, step.velocity) for step in state.sequence] == [
        (60, 91),
        (63, 102),
        (67, 113),
    ]
    assert not hasattr(state.sequence[0], "timestamp")


async def test_note_off_does_not_record_and_record_off_restores_explicit_commit() -> None:
    state = AppState()
    runtime = MiniStepRuntime(state, output=None)
    await runtime.execute(Command(CommandName.RECORD, 1))

    runtime.handle_midi_message(mido.Message("note_on", note=60, velocity=90))
    runtime.handle_midi_message(mido.Message("note_off", note=60))
    assert [step.note for step in state.sequence] == [60]

    await runtime.execute(Command(CommandName.RECORD_TOGGLE))
    runtime.handle_midi_message(mido.Message("note_on", note=64, velocity=101))
    assert [step.note for step in state.sequence] == [60]
    await runtime.execute(Command(CommandName.COMMIT))
    assert [(step.note, step.velocity) for step in state.sequence] == [(60, 90), (64, 101)]


async def test_armed_root_uses_next_midi_key_without_recording_it() -> None:
    state = AppState(sequence=[Step(60), Step(63)], recording=True)
    runtime = MiniStepRuntime(state, output=None)
    await runtime.execute(Command(CommandName.ARM_ROOT_CAPTURE))

    runtime.handle_midi_message(mido.Message("note_on", note=62, velocity=100))

    assert state.root_capture_armed is False
    assert state.transpose == 0
    assert [step.note for step in state.sequence] == [62, 65]
