"""Drum track: model, playback dispatch, MIDI routing, persistence v2, TUI editing."""

from __future__ import annotations

import asyncio
import json
from pathlib import Path

import mido
import pytest
from test_timing import VirtualTime

from ministep.clock import InternalClock, StepWindow, step_duration_seconds
from ministep.controller import Command, CommandName
from ministep.drums import DRUM_STEPS, DrumLane, DrumStep, DrumTrack
from ministep.patterns import DRUMS, PatternStore
from ministep.runtime import MiniStepRuntime
from ministep.sequencer import DRUM_NOTE_LENGTH_MS, Sequencer
from ministep.state import AppState, Step, note_name, parse_note_name
from ministep.tui import MiniStepApp


class FakeClock:
    """Hands out one fixed window; records every deadline waited on."""

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


async def settle() -> None:
    """Let drum note-off tasks run (they wait on the fake clock, which returns at once)."""
    for _ in range(3):
        await asyncio.sleep(0)


def hit(state: AppState, lane: int, step: int, velocity: int | None = None) -> None:
    drum = state.drums.lanes[lane].steps[step]
    drum.active = True
    if velocity is not None:
        drum.velocity = velocity


# --- model ---------------------------------------------------------------------


def test_default_drum_track_has_four_empty_lanes() -> None:
    drums = DrumTrack.default()
    assert [lane.name for lane in drums.lanes] == ["Kick", "Snare", "Closed Hat", "Open Hat"]
    assert all(len(lane.steps) == DRUM_STEPS for lane in drums.lanes)
    assert not drums.has_hits()
    assert drums.channel == 10
    # A fresh AppState carries the same empty track: no preset beat.
    assert not AppState().drums.has_hits()


def test_default_lane_notes_follow_ableton_naming_and_gm_drum_map() -> None:
    drums = DrumTrack.default()
    assert [note_name(lane.note) for lane in drums.lanes] == ["C1", "D1", "F#1", "A#1"]
    assert [lane.note for lane in drums.lanes] == [36, 38, 42, 46]
    assert parse_note_name("C1") == 36 and parse_note_name("Bb1") == 46
    with pytest.raises(ValueError):
        parse_note_name("H2")


def test_toggle_hit_and_cursor_movement() -> None:
    drums = DrumTrack.default()
    drums.move_lane(1)
    drums.move_step(3)
    assert (drums.cursor_lane, drums.cursor_step) == (1, 3)
    assert drums.toggle_hit() is True
    assert drums.lanes[1].steps[3].active is True
    assert drums.hits_at(3) == [(drums.lanes[1], drums.lanes[1].steps[3])]
    assert drums.toggle_hit() is False
    assert not drums.has_hits()
    drums.move_step(-4)  # wraps
    drums.move_lane(-2)  # wraps
    assert (drums.cursor_lane, drums.cursor_step) == (3, 15)


def test_velocity_is_stored_per_hit_and_clamped() -> None:
    drums = DrumTrack.default()
    assert drums.cursor.velocity == 100
    assert drums.adjust_velocity(-10) == 90
    assert drums.adjust_velocity(-200) == 1
    assert drums.adjust_velocity(500) == 127
    drums.move_step(1)
    assert drums.cursor.velocity == 100  # neighbour untouched
    with pytest.raises(ValueError):
        DrumStep(velocity=0)
    with pytest.raises(ValueError):
        DrumLane("short", 36, steps=[DrumStep()])


# --- playback ------------------------------------------------------------------


async def test_drum_playback_sends_note_on_then_short_note_off() -> None:
    state = AppState()
    hit(state, 0, 0, velocity=120)
    clock, output = FakeClock(), MockMidiOutput()
    sequencer = Sequencer(state, output, clock)

    assert await sequencer.advance_one() is True
    assert output.events == [("on", 36, 120, 9, "drums")]
    await settle()
    assert output.events[-1] == ("off", 36, 9, "drums")
    # The note-off waits on the same clock: grid start + drum length, then the step end.
    assert sorted(clock.waits) == [10.0 + DRUM_NOTE_LENGTH_MS / 1000, 11.0]
    assert sequencer.drum_step == 1


async def test_multiple_hits_on_one_step_fire_together() -> None:
    state = AppState()
    hit(state, 0, 4, velocity=100)
    hit(state, 2, 4, velocity=80)
    hit(state, 3, 4, velocity=60)
    clock, output = FakeClock(), MockMidiOutput()
    sequencer = Sequencer(state, output, clock)
    sequencer.drum_step = 4

    await sequencer.advance_one()
    assert output.events == [
        ("on", 36, 100, 9, "drums"),
        ("on", 42, 80, 9, "drums"),
        ("on", 46, 60, 9, "drums"),
    ]
    await settle()
    assert [event for event in output.events if event[0] == "off"] == [
        ("off", 36, 9, "drums"),
        ("off", 42, 9, "drums"),
        ("off", 46, 9, "drums"),
    ]


async def test_melodic_playback_is_unchanged_without_drums() -> None:
    state = AppState(sequence=[Step(60, velocity=110, gate=0.5)], transpose=2)
    clock, output = FakeClock(), MockMidiOutput()
    sequencer = Sequencer(state, output, clock)

    await sequencer.advance_one()
    await settle()
    assert output.events == [("on", 62, 110, 0, "sequencer"), ("off", 62, 0, "sequencer")]
    assert clock.waits == [10.5, 11.0]
    assert sequencer._drum_releases == set()


async def test_melodic_and_drums_play_on_the_same_step() -> None:
    state = AppState(sequence=[Step(60, gate=0.5)])
    hit(state, 1, 0)
    clock, output = FakeClock(), MockMidiOutput()
    sequencer = Sequencer(state, output, clock)

    await sequencer.advance_one()
    await settle()
    on_events = [event for event in output.events if event[0] == "on"]
    assert on_events == [("on", 60, 100, 0, "sequencer"), ("on", 38, 100, 9, "drums")]
    # Melodic gate still 0.5 -> off at 10.5; drum off at 10.04; step ends at 11.
    assert sorted(clock.waits) == [10.04, 10.5, 11.0]
    assert state.playhead == 0 and sequencer.drum_step == 1


async def test_drum_bar_is_sixteen_steps_regardless_of_melodic_loop() -> None:
    state = AppState(sequence=[Step(60), Step(62), Step(64)])
    hit(state, 0, 0)
    output = MockMidiOutput()
    sequencer = Sequencer(state, output, FakeClock())
    for _ in range(DRUM_STEPS + 1):
        await sequencer.advance_one()
    kicks = [event for event in output.events if event[0] == "on" and event[1] == 36]
    assert len(kicks) == 2  # step 0 and step 16
    assert state.playhead == (DRUM_STEPS + 1) % 3


async def test_drums_use_their_own_output_and_channel() -> None:
    state = AppState(sequence=[Step(60)], output_channel=3)
    state.drums.channel = 12
    hit(state, 0, 0)
    melodic_out, drum_out = MockMidiOutput(), MockMidiOutput()
    sequencer = Sequencer(state, melodic_out, FakeClock(), drum_output=drum_out)

    await sequencer.advance_one()
    await settle()
    assert melodic_out.events == [("on", 60, 100, 2, "sequencer"), ("off", 60, 2, "sequencer")]
    assert drum_out.events == [("on", 36, 100, 11, "drums"), ("off", 36, 11, "drums")]


async def test_drums_play_when_melodic_sequence_is_empty() -> None:
    state = AppState()
    hit(state, 0, 0)
    output, clock = MockMidiOutput(), FakeClock()
    sequencer = Sequencer(state, output, clock)
    assert sequencer.has_content()
    assert await sequencer.advance_one() is True
    assert output.events == [("on", 36, 100, 9, "drums")]
    assert clock.waits[-1] == 11.0

    state.drums.clear()
    assert await sequencer.advance_one() is False


async def test_stop_releases_hanging_drum_notes() -> None:
    class HoldingClock(FakeClock):
        """Never resolves a note-off wait, as if Stop arrived inside the drum gate."""

        async def wait_until(self, deadline: float) -> None:
            self.waits.append(deadline)
            if deadline < 11.0:
                await asyncio.Event().wait()

    state = AppState()
    hit(state, 0, 0)
    hit(state, 1, 0)
    output = MockMidiOutput()
    sequencer = Sequencer(state, output, HoldingClock())
    await sequencer.advance_one()
    await settle()
    assert [event for event in output.events if event[0] == "off"] == []
    assert len(sequencer._drum_releases) == 1

    await sequencer.stop()
    await settle()
    assert sorted(event for event in output.events if event[0] == "off") == [
        ("off", 36, 9, "drums"),
        ("off", 38, 9, "drums"),
    ]
    assert sequencer._drum_releases == set() and sequencer._sounding_drums == set()
    # Stopping twice is harmless.
    await sequencer.stop()
    assert len([event for event in output.events if event[0] == "off"]) == 2


async def test_runtime_stop_sends_all_notes_off_to_separate_drum_output() -> None:
    class Owned(MockMidiOutput):
        pass

    state = AppState()
    hit(state, 0, 0)
    melodic_out, drum_out = Owned(), Owned()
    runtime = MiniStepRuntime(state, melodic_out, drum_output=drum_out)  # type: ignore[arg-type]
    runtime.play()  # only drums have content
    assert state.playing is True
    await runtime.stop()
    assert ("all_off",) in melodic_out.events and ("all_off",) in drum_out.events
    assert state.playing is False


async def test_drum_note_offs_do_not_drift_the_grid() -> None:
    """Heavy drum traffic must leave scheduled starts on the exact anchored grid."""
    bpm = 174.0
    vt = VirtualTime(overshoot=0.0011)
    state = AppState(bpm=bpm, step_division=16, sequence=[Step(60, gate=0.5) for _ in range(16)])
    for lane in range(4):
        for step in range(DRUM_STEPS):
            hit(state, lane, step)
    times: list[tuple[str, int, float]] = []

    class Sink(MockMidiOutput):
        def note_on(self, note, velocity, channel=0, owner="sequencer"):  # type: ignore[no-untyped-def]
            times.append(("on", note, vt.now()))

        def note_off(self, note, channel=0, owner="sequencer"):  # type: ignore[no-untyped-def]
            times.append(("off", note, vt.now()))

    clock = InternalClock(now=vt.now, sleep=vt.sleep)
    sequencer = Sequencer(state, Sink(), clock)
    steps = 64
    for _ in range(steps):
        await sequencer.advance_one()
    await sequencer.stop()

    duration = step_duration_seconds(bpm, 16)
    melodic_ons = [t for kind, note, t in times if kind == "on" and note == 60]
    assert len(melodic_ons) == steps
    first = melodic_ons[0]
    # Each step's note-on lands within one timer overshoot of its ideal grid slot,
    # and the *last* step is no later than the first: nothing accumulated.
    lateness = [t - (first + index * duration) for index, t in enumerate(melodic_ons)]
    assert max(lateness) < 0.003, max(lateness)
    assert min(lateness) >= 0.0
    # Every drum note-off happened, roughly DRUM_NOTE_LENGTH_MS after its note-on.
    drum_ons = [t for kind, note, t in times if kind == "on" and note != 60]
    drum_offs = [t for kind, note, t in times if kind == "off" and note != 60]
    assert len(drum_ons) == len(drum_offs) == steps * 4
    gap = DRUM_NOTE_LENGTH_MS / 1000
    pairs = zip(drum_ons, drum_offs, strict=True)
    assert all(gap - 0.001 <= off - on <= gap + 0.01 for on, off in pairs)


# --- persistence -----------------------------------------------------------------


def beat_state() -> AppState:
    state = AppState(bpm=123, step_division=16)
    state.append_note(60, 111)
    state.append_rest()
    hit(state, 0, 0, velocity=127)
    hit(state, 0, 8, velocity=90)
    hit(state, 1, 4)
    hit(state, 2, 2, velocity=64)
    state.drums.channel = 11
    return state


def test_pattern_v2_roundtrip() -> None:
    state = beat_state()
    data = json.loads(json.dumps(state.to_dict(name="bite_with_drums")))
    assert data["version"] == 2
    assert data["bpm"] == 123 and data["step_division"] == 16
    assert data["melodic"]["steps"][0]["note"] == 60
    assert data["drums"]["channel"] == 11
    assert [lane["note"] for lane in data["drums"]["lanes"]] == ["C1", "D1", "F#1", "A#1"]
    assert data["drums"]["lanes"][0]["steps"][0] == {"active": True, "velocity": 127}
    assert "selected_drum_output" not in data and "active_track" not in data

    restored = AppState.from_dict(data)
    assert restored.sequence == state.sequence
    assert restored.drums.lanes == state.drums.lanes
    assert restored.drums.channel == 11


def test_loading_v1_pattern_creates_empty_drums() -> None:
    v1 = {
        "version": 1,
        "bpm": 99,
        "step_division": 8,
        "loop_length": 16,
        "default_gate": 0.7,
        "transpose": -2,
        "output_channel": 4,
        "steps": [{"note": 60, "velocity": 111}, {"note": None}],
    }
    state = AppState.from_dict(v1)
    assert (state.bpm, state.step_division, state.loop_length) == (99, 8, 16)
    assert (state.default_gate, state.transpose, state.output_channel) == (0.7, -2, 4)
    assert [(step.note, step.velocity) for step in state.sequence] == [(60, 111), (None, 100)]
    assert [lane.name for lane in state.drums.lanes] == ["Kick", "Snare", "Closed Hat", "Open Hat"]
    assert not state.drums.has_hits()
    assert state.drums.channel == 10


def test_v1_and_v2_of_the_same_melody_play_identically() -> None:
    v1 = {"version": 1, "bpm": 140, "transpose": 3, "steps": [{"note": 60, "gate": 0.25}]}
    from_v1 = AppState.from_dict(v1)
    from_v2 = AppState.from_dict(from_v1.to_dict())
    assert from_v2.sequence == from_v1.sequence
    assert (from_v2.bpm, from_v2.transpose) == (140, 3)


def test_store_save_and_load_restores_drum_steps(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    state = beat_state()
    store.save(DRUMS, state, "beat")
    on_disk = json.loads((tmp_path / "drums" / "beat.json").read_text())
    assert on_disk["version"] == 1 and on_disk["type"] == "drums"
    loaded = AppState()
    store.load(DRUMS, loaded, "beat")
    assert loaded.drums.lanes == state.drums.lanes
    assert loaded.drums.lanes[0].steps[0].velocity == 127
    assert loaded.drums.lanes[2].steps[2].velocity == 64
    assert loaded.drums.lanes[3].steps == [DrumStep() for _ in range(DRUM_STEPS)]


def test_v2_drum_lanes_tolerate_partial_and_numeric_data() -> None:
    data = {
        "version": 2,
        "melodic": {"steps": []},
        "drums": {"lanes": [{"name": "Clap", "note": 39, "steps": [{"active": True}]}]},
    }
    state = AppState.from_dict(data)
    assert len(state.drums.lanes) == 1
    assert state.drums.lanes[0].note == 39
    assert state.drums.lanes[0].steps[0] == DrumStep(active=True, velocity=100)
    assert len(state.drums.lanes[0].steps) == DRUM_STEPS
    with pytest.raises(ValueError, match="Invalid pattern data"):
        AppState.from_dict({"version": 2, "drums": {"lanes": [{"name": "x"}]}})
    with pytest.raises(ValueError, match="Invalid pattern data"):
        bad_velocity = {"lanes": [{"note": "C1", "steps": [{"velocity": 0}]}]}
        AppState.from_dict({"version": 2, "drums": bad_velocity})


async def test_runtime_load_pattern_keeps_drum_port_and_active_track(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    store.save(DRUMS, beat_state(), "beat")
    state = AppState(selected_output="Synth", selected_drum_output="IAC Bus")
    state.active_track = "drums"
    runtime = MiniStepRuntime(state, output=None, pattern_store=store)
    assert await runtime.load_pattern("beat") is True
    assert state.drums.has_hits()
    assert (state.selected_output, state.selected_drum_output) == ("Synth", "IAC Bus")
    assert state.active_track == "drums"


# --- TUI -------------------------------------------------------------------------


async def test_tui_tab_switches_track_and_header_shows_it() -> None:
    state = AppState()
    app = MiniStepApp(MiniStepRuntime(state, output=None))
    async with app.run_test() as pilot:
        assert state.active_track == "melodic"
        assert "TRACK: MELODIC" in app._rendered["summary"]
        await pilot.press("tab")
        assert state.active_track == "drums"
        app.refresh_view()
        assert "TRACK: DRUMS" in app._rendered["summary"]
        assert "KICK" in app._rendered["sequence"] and "CHH" in app._rendered["sequence"]
        await pilot.press("tab")
        assert state.active_track == "melodic"


async def test_tui_drum_cursor_moves_by_step_and_lane() -> None:
    state = AppState()
    app = MiniStepApp(MiniStepRuntime(state, output=None))
    async with app.run_test() as pilot:
        await pilot.press("tab", "right", "right", "down")
        assert (state.drums.cursor_lane, state.drums.cursor_step) == (1, 2)
        await pilot.press("left", "left", "left", "up", "up")
        assert (state.drums.cursor_lane, state.drums.cursor_step) == (3, 15)
        # Melodic cursor and notes untouched by drum navigation.
        assert state.cursor == 0 and state.sequence == []


async def test_tui_enter_toggles_hit_and_minus_plus_edit_velocity() -> None:
    state = AppState()
    state.append_note(60)
    app = MiniStepApp(MiniStepRuntime(state, output=None))
    async with app.run_test() as pilot:
        await pilot.press("tab", "down", "right", "right", "right", "right", "enter")
        assert state.drums.lanes[1].steps[4].active is True
        assert len(state.sequence) == 1  # Enter did not commit a melodic step
        await pilot.press("minus", "minus")
        assert state.drums.lanes[1].steps[4].velocity == 80
        await pilot.press("plus")
        assert state.drums.lanes[1].steps[4].velocity == 90
        assert state.status_message == "Velocity 90."
        await pilot.press("enter")
        assert state.drums.lanes[1].steps[4].active is False
        await pilot.press("enter", "x")
        assert state.drums.lanes[1].steps[4].active is False
        await pilot.press("enter", "c")
        assert not state.drums.has_hits()
        assert len(state.sequence) == 1  # C on the drum track never clears the melody


async def test_tui_melodic_keys_still_work_after_switching_back() -> None:
    state = AppState()
    state.append_note(60)
    state.append_note(62)
    app = MiniStepApp(MiniStepRuntime(state, output=None))
    async with app.run_test() as pilot:
        await pilot.press("tab", "tab", "right", "up")
        assert state.cursor == 0 and state.sequence[0].note == 61


async def test_tui_pattern_picker_still_works_and_persists_drums(tmp_path: Path) -> None:
    state = AppState()
    state.append_note(60)
    runtime = MiniStepRuntime(state, output=None, pattern_store=PatternStore(tmp_path))
    app = MiniStepApp(runtime)
    async with app.run_test() as pilot:
        await pilot.press("tab", "enter")  # kick on step 1
        assert state.drums.lanes[0].steps[0].active
        await pilot.press("S")
        assert app._prompt_mode == "save"
        await pilot.press("tab")  # Tab inside the prompt must not switch tracks
        assert state.active_track == "drums"
        await pilot.press(*"groove", "enter")
        assert state.status_message == "Saved drum pattern: groove"
        assert json.loads((tmp_path / "drums" / "groove.json").read_text())["type"] == "drums"

        await pilot.press("c")  # clears drums (drum track active)
        assert not state.drums.has_hits()
        await pilot.press("L", *"groove", "enter")
        assert state.status_message == "Loaded drum pattern: groove"
        assert state.drums.lanes[0].steps[0].active
        assert len(state.sequence) == 1


# --- MIDI audition -> commit -> step ------------------------------------------------


def drum_runtime(
    drum_out: MockMidiOutput | None = None,
) -> tuple[AppState, MiniStepRuntime, MockMidiOutput, MockMidiOutput]:
    state = AppState(active_track="drums")
    melodic_out = MockMidiOutput()
    drums_out = drum_out if drum_out is not None else MockMidiOutput()
    runtime = MiniStepRuntime(state, melodic_out, drum_output=drums_out)  # type: ignore[arg-type]
    return state, runtime, melodic_out, drums_out


def note_on(note: int, velocity: int = 100) -> mido.Message:
    return mido.Message("note_on", note=note, velocity=velocity)


@pytest.mark.parametrize(
    ("note", "lane_index", "lane_name"),
    [(36, 0, "Kick"), (38, 1, "Snare"), (42, 2, "Closed Hat"), (46, 3, "Open Hat")],
)
def test_drum_midi_note_auditions_its_lane_on_the_drum_output(
    note: int, lane_index: int, lane_name: str
) -> None:
    state, runtime, melodic_out, drums_out = drum_runtime()
    runtime.handle_midi_message(note_on(note, 93))
    assert state.drums.audition == {lane_index: 93}
    assert state.drums.cursor_lane == lane_index
    assert drums_out.events == [("on", note, 93, 9, "input")]
    assert melodic_out.events == []
    assert lane_name in state.status_message and "93" in state.status_message
    # Nothing is written until commit; the melodic audition is untouched.
    assert not state.drums.has_hits()
    assert state.last_note_played is None and state.held_notes == {}

    runtime.handle_midi_message(mido.Message("note_off", note=note))
    assert drums_out.events[-1] == ("off", note, 9, "input")


def test_unknown_midi_note_creates_no_drum_hit_and_passes_through() -> None:
    state, runtime, melodic_out, drums_out = drum_runtime()
    runtime.handle_midi_message(note_on(60, 80))
    assert state.drums.audition == {}
    assert drums_out.events == []
    # Non-drum keys still reach the synth as ordinary melodic audition.
    assert melodic_out.events == [("on", 60, 80, 0, "input")]
    assert state.last_note_played == (60, 80)
    assert runtime.commit() is False
    assert not state.drums.has_hits()


def test_commit_writes_auditioned_hit_with_midi_velocity_and_advances_step() -> None:
    state, runtime, _, _ = drum_runtime()
    runtime.handle_midi_message(note_on(42, 93))
    assert runtime.commit() is True
    hit = state.drums.lanes[2].steps[0]
    assert (hit.active, hit.velocity) == (True, 93)
    assert state.drums.cursor_step == 1
    assert state.drums.audition == {}
    assert state.status_message == "Committed Closed Hat on step 1."


def test_kick_plus_hat_audition_commits_both_and_repeat_updates_velocity() -> None:
    state, runtime, _, _ = drum_runtime()
    runtime.handle_midi_message(note_on(36, 120))
    runtime.handle_midi_message(note_on(42, 70))
    runtime.handle_midi_message(note_on(42, 95))  # same lane again: velocity only
    assert state.drums.audition == {0: 120, 2: 95}
    assert runtime.commit() is True
    assert state.drums.lanes[0].steps[0] == DrumStep(True, 120)
    assert state.drums.lanes[2].steps[0] == DrumStep(True, 95)
    assert state.drums.lanes[1].steps[0].active is False
    assert state.drums.cursor_step == 1 and state.drums.audition == {}
    assert state.status_message == "Committed Kick + Closed Hat on step 1."


async def test_rest_skips_a_drum_step_and_drops_the_audition() -> None:
    state, runtime, _, _ = drum_runtime()
    runtime.handle_midi_message(note_on(38, 100))
    await runtime.execute(Command(CommandName.REST))
    assert state.drums.audition == {} and state.drums.cursor_step == 1
    assert not state.drums.has_hits()
    assert state.sequence == []  # no melodic rest was appended


def test_midi_recorded_velocities_survive_persistence(tmp_path: Path) -> None:
    state, runtime, _, _ = drum_runtime()
    runtime.patterns = PatternStore(tmp_path)
    runtime.handle_midi_message(note_on(36, 127))
    runtime.commit()
    runtime.handle_midi_message(note_on(46, 33))
    runtime.handle_midi_message(note_on(36, 64))
    runtime.commit()
    assert runtime.save_pattern("live") is True
    assert (tmp_path / "drums" / "live.json").is_file()
    loaded = AppState()
    runtime.patterns.load(DRUMS, loaded, "live")
    assert loaded.drums.lanes[0].steps[0].velocity == 127
    assert loaded.drums.lanes[0].steps[1].velocity == 64
    assert loaded.drums.lanes[3].steps[1].velocity == 33
    assert loaded.drums.audition == {}


def test_melodic_midi_workflow_is_unchanged_when_melodic_track_is_active() -> None:
    state, runtime, melodic_out, drums_out = drum_runtime()
    state.active_track = "melodic"
    runtime.handle_midi_message(note_on(36, 90))  # C1 is just a note on the melodic track
    assert melodic_out.events == [("on", 36, 90, 0, "input")]
    assert drums_out.events == [] and state.drums.audition == {}
    assert state.last_note_played == (36, 90)
    assert runtime.commit() is True
    assert [step.note for step in state.sequence] == [36]
    assert not state.drums.has_hits()


# --- TUI polish -----------------------------------------------------------------------


async def test_tui_enter_commits_pending_audition_else_toggles() -> None:
    state, runtime, _, _ = drum_runtime()
    app = MiniStepApp(runtime)
    async with app.run_test() as pilot:
        runtime.handle_midi_message(note_on(36, 111))
        runtime.handle_midi_message(note_on(42, 77))
        app.refresh_view()
        assert "AUDITION: KICK C1 + CHH F#1" in app._rendered["summary"]
        assert "velocity 111, 77" in app._rendered["summary"]
        assert "+" in app._rendered["sequence"]  # pending marker on the cursor cell
        await pilot.press("enter")
        assert state.drums.lanes[0].steps[0] == DrumStep(True, 111)
        assert state.drums.lanes[2].steps[0] == DrumStep(True, 77)
        assert state.drums.cursor_step == 1
        # No audition pending: Enter is the plain grid toggle again.
        await pilot.press("2", "enter")
        assert state.drums.lanes[1].steps[1].active is True
        # Esc drops an audition without writing anything.
        runtime.handle_midi_message(note_on(46, 50))
        await pilot.press("escape")
        assert state.drums.audition == {} and state.status_message == "Audition dropped."
        assert state.drums.lanes[3].steps[1].active is False


async def test_tui_digits_select_lanes_and_arrows_still_work() -> None:
    state = AppState(active_track="drums")
    app = MiniStepApp(MiniStepRuntime(state, output=None))
    async with app.run_test() as pilot:
        await pilot.press("4")
        assert state.drums.cursor_lane == 3
        await pilot.press("1", "down")
        assert state.drums.cursor_lane == 1
        await pilot.press("9")  # no such lane: ignored
        assert state.drums.cursor_lane == 1
        await pilot.press("right", "enter", "minus")
        assert state.drums.lanes[1].steps[1] == DrumStep(True, 90)


async def test_tui_drum_header_hides_melodic_only_state() -> None:
    state = AppState(active_track="drums", transpose=5, octave=1)
    state.append_note(60)
    state.selected_drum_output = "IAC Driver Drums"
    app = MiniStepApp(MiniStepRuntime(state, output=None))
    async with app.run_test():
        summary = app._rendered["summary"]
        assert "TRACK: DRUMS" in summary
        assert "DRUM OUT: IAC Driver Drums  (ch 10)" in summary
        assert "STEP 1/16" in summary
        for melodic_only in ("transpose", "octave", "ROOT", "gate", "MIDI OUT", "loop"):
            assert melodic_only not in summary, melodic_only
        status = app._rendered["status"]
        for melodic_key in ("semitone", "H hold", "set root", "MIDI Learn"):
            assert melodic_key not in status
        assert "Enter commit/toggle" in status


async def test_tui_drum_grid_groups_steps_in_fours_with_beat_marks() -> None:
    state = AppState(active_track="drums")
    hit(state, 0, 0)
    hit(state, 0, 4)
    app = MiniStepApp(MiniStepRuntime(state, output=None))
    async with app.run_test():
        grid = app._rendered["sequence"]
        header, kick, snare = grid.split("\n")[:3]
        assert header.count("|") == 3 and kick.count("|") == 3 and snare.count("|") == 3
        assert "[bold]  1[/]" in header and "[bold]  5[/]" in header and "[dim]  2[/]" in header
        # Silent beat starts use a heavier dot; other silent steps a plain one.
        assert snare.count("·") == 4 and snare.count(".") == 12
        # Cursor cell is the strongest highlight; nothing else uses that style.
        assert grid.count("on yellow") == 1
        assert "[bold black on yellow]  x[/]" in kick


async def test_tui_track_switch_keeps_melodic_header_and_playback_unchanged() -> None:
    state = AppState(sequence=[Step(60, gate=0.5)])
    hit(state, 1, 0)
    clock, output = FakeClock(), MockMidiOutput()
    runtime = MiniStepRuntime(state, output)  # type: ignore[arg-type]
    runtime.sequencer.clock = clock  # type: ignore[union-attr]
    app = MiniStepApp(runtime)
    async with app.run_test() as pilot:
        assert "transpose +0" in app._rendered["summary"]
        await pilot.press("tab")
        runtime.handle_midi_message(note_on(36, 100))  # pending audition, never committed
        await runtime.sequencer.advance_one()  # type: ignore[union-attr]
        await settle()
        await pilot.press("tab")
        app.refresh_view()
        assert "TRACK: MELODIC" in app._rendered["summary"]
        # Sequencer plays only committed hits: the melodic note and the snare, not the kick.
        played = [event for event in output.events if event[0] == "on" and event[4] != "input"]
        assert played == [("on", 60, 100, 0, "sequencer"), ("on", 38, 100, 9, "drums")]
        assert sorted(clock.waits) == [10.04, 10.5, 11.0]
