import pytest

from ministep.clock import step_duration_seconds
from ministep.controller import Command, CommandName
from ministep.runtime import MiniStepRuntime
from ministep.state import AppState


def test_step_duration_calculations() -> None:
    assert step_duration_seconds(120, 4) == pytest.approx(0.5)
    assert step_duration_seconds(120, 8) == pytest.approx(0.25)
    assert step_duration_seconds(120, 16) == pytest.approx(0.125)
    assert step_duration_seconds(128, 16) == pytest.approx(60 / 128 / 4)


async def test_step_division_cycles_through_musical_values() -> None:
    state = AppState(step_division=16)
    runtime = MiniStepRuntime(state, output=None)

    await runtime.execute(Command(CommandName.STEP_DIVISION_UP))
    assert state.step_division == 32
    await runtime.execute(Command(CommandName.STEP_DIVISION_UP))
    assert state.step_division == 4
    await runtime.execute(Command(CommandName.STEP_DIVISION_DOWN))
    assert state.step_division == 32
