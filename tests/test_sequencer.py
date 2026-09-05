from __future__ import annotations

from ministep.clock import StepWindow
from ministep.sequencer import Sequencer
from ministep.state import AppState, Step


class FakeClock:
    def __init__(self) -> None:
        self.waits: list[float] = []

    def reset(self) -> None:
        pass

    async def next_step(self, bpm: float, step_division: int) -> StepWindow:
        return StepWindow(start=10.0, duration=1.0)

    async def wait_until(self, deadline: float) -> None:
        self.waits.append(deadline)


class MockMidiOutput:
    def __init__(self) -> None:
        self.events: list[tuple] = []

    def note_on(self, note: int, velocity: int, channel: int = 0, owner: str = "sequencer") -> None:
        self.events.append(("on", note, velocity, channel, owner))

    def note_off(self, note: int, channel: int = 0, owner: str = "sequencer") -> None:
        self.events.append(("off", note, channel, owner))

    def control_change(self, control: int, value: int, channel: int = 0) -> None:
        self.events.append(("cc", control, value, channel))

    def all_notes_off(self) -> None:
        self.events.append(("all_off",))


async def test_playback_orders_note_on_then_off_with_gate_and_transpose() -> None:
    state = AppState(sequence=[Step(60, velocity=110, gate=0.5)], transpose=2)
    clock = FakeClock()
    output = MockMidiOutput()
    sequencer = Sequencer(state, output, clock)

    assert await sequencer.advance_one() is True
    assert output.events == [("on", 62, 110, 0, "sequencer"), ("off", 62, 0, "sequencer")]
    assert clock.waits == [10.5, 11.0]
    assert state.playhead == 0


async def test_rest_only_waits_a_full_step() -> None:
    state = AppState(sequence=[Step(None)])
    clock = FakeClock()
    output = MockMidiOutput()

    await Sequencer(state, output, clock).advance_one()
    assert output.events == []
    assert clock.waits == [11.0]


async def test_hold_ties_note_across_an_extra_full_step() -> None:
    state = AppState(sequence=[Step(60, gate=0.5, tie=True), Step(60, gate=1.0)])
    clock = FakeClock()
    output = MockMidiOutput()
    sequencer = Sequencer(state, output, clock)

    await sequencer.advance_one()
    await sequencer.advance_one()

    assert output.events == [("on", 60, 100, 0, "sequencer"), ("off", 60, 0, "sequencer")]
    assert clock.waits == [11.0, 11.0]


async def test_loop_length_wraps_playback_before_later_sequence_steps() -> None:
    state = AppState(sequence=[Step(60 + index) for index in range(10)], loop_length=8, playhead=7)
    clock = FakeClock()
    output = MockMidiOutput()
    sequencer = Sequencer(state, output, clock)

    await sequencer.advance_one()
    await sequencer.advance_one()

    assert [event[1] for event in output.events if event[0] == "on"] == [67, 60]
    assert state.playhead == 1
