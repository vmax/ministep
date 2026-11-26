"""Named pattern persistence: PatternStore, format tolerance, runtime and TUI wiring."""

from __future__ import annotations

import json
import os
import stat
from pathlib import Path

import pytest

from ministep.patterns import (
    DRUMS,
    KINDS,
    MELODIC,
    PatternError,
    PatternStore,
    sanitize_pattern_name,
)
from ministep.picker import PatternPicker, fuzzy_rank
from ministep.runtime import MiniStepRuntime
from ministep.state import AppState, Step
from ministep.tui import MiniStepApp


def rich_state() -> AppState:
    state = AppState(
        bpm=133.5,
        step_division=8,
        loop_length=16,
        default_gate=0.35,
        default_velocity=90,
        transpose=-3,
        octave=1,
        output_channel=4,
    )
    state.append_note(60, 111)
    state.append_hold()  # ties step 1 into a gate-1.0 copy
    state.append_rest()
    state.append_note(67, 42)
    state.sequence[-1].accent = True
    state.sequence[-1].enabled = False
    return state


def musical_view(state: AppState) -> dict:
    data = state.to_dict()
    data.pop("version")
    return data


# --- serialisation -----------------------------------------------------------


def test_serialize_deserialize_roundtrip() -> None:
    state = rich_state()
    restored = AppState.from_dict(json.loads(json.dumps(state.to_dict())))
    assert musical_view(restored) == musical_view(state)
    assert restored.sequence == state.sequence


def test_notes_and_rests_survive() -> None:
    restored = AppState.from_dict(rich_state().to_dict())
    assert [step.note for step in restored.sequence] == [60, 60, None, 67]


def test_hold_state_survives() -> None:
    restored = AppState.from_dict(rich_state().to_dict())
    assert [(step.tie, step.gate) for step in restored.sequence[:2]] == [(True, 0.35), (False, 1.0)]


def test_velocity_survives() -> None:
    restored = AppState.from_dict(rich_state().to_dict())
    assert [step.velocity for step in restored.sequence] == [111, 111, 90, 42]
    assert restored.default_velocity == 90


def test_bpm_division_gate_survive() -> None:
    restored = AppState.from_dict(rich_state().to_dict())
    assert (restored.bpm, restored.step_division, restored.default_gate) == (133.5, 8, 0.35)


def test_transpose_octave_root_survive() -> None:
    restored = AppState.from_dict(rich_state().to_dict())
    assert (restored.transpose, restored.octave) == (-3, 1)
    # Root is derived from the first pitched step, so it follows the notes.
    assert restored.sequence_root() == 60


def test_loop_length_and_flags_survive() -> None:
    restored = AppState.from_dict(rich_state().to_dict())
    assert restored.loop_length == 16
    assert (restored.sequence[3].accent, restored.sequence[3].enabled) == (True, False)


def test_transient_state_is_not_serialized() -> None:
    state = rich_state()
    state.playing = True
    state.playhead = 2
    state.cursor = 1
    state.recording = True
    state.selected_input = "MiniLab3"
    state.set_last_note(70, 99)
    data = state.to_dict(name="x")
    for key in ("playing", "playhead", "cursor", "recording", "selected_input", "last_note_played"):
        assert key not in data


def test_missing_optional_fields_get_defaults() -> None:
    restored = AppState.from_dict({"version": 1, "steps": [{"note": 62}]})
    assert restored.bpm == 128.0
    assert restored.step_division == 16
    assert restored.loop_length is None
    assert restored.octave == 0
    assert restored.sequence == [Step(62)]


def test_unknown_fields_are_ignored() -> None:
    data = rich_state().to_dict(name="future")
    data["swing"] = 0.6
    data["melodic"]["steps"][0]["probability"] = 0.5
    restored = AppState.from_dict(data)
    assert restored.sequence[0].note == 60


def test_unsupported_version_is_a_readable_error() -> None:
    with pytest.raises(ValueError, match="Unsupported pattern version 99"):
        AppState.from_dict({"version": 99, "steps": []})


def test_structurally_broken_steps_are_readable_errors() -> None:
    with pytest.raises(ValueError, match="Invalid pattern data"):
        AppState.from_dict({"version": 1, "steps": [{"velocity": 3}]})
    with pytest.raises(ValueError, match="Invalid pattern data"):
        AppState.from_dict({"version": 1, "steps": [{"note": 400}]})
    with pytest.raises(ValueError, match="JSON object"):
        AppState.from_dict([1, 2])  # type: ignore[arg-type]


# --- names and paths ---------------------------------------------------------


@pytest.mark.parametrize(
    ("raw", "expected"),
    [
        ("bite_test", "bite_test"),
        ("Egypt 01", "Egypt_01"),
        ("  spaced  out ", "spaced_out"),
        ("../../etc/passwd", "etc_passwd"),
        ("..\\..\\win", "win"),
        ("/abs/path", "abs_path"),
        (".hidden", "hidden"),
        ("name.json", "name"),
        ("ünïcödé!!x", "n_c_d_x"),
    ],
)
def test_sanitize_pattern_name(raw: str, expected: str) -> None:
    assert sanitize_pattern_name(raw) == expected


@pytest.mark.parametrize("raw", ["", "   ", "..", "///", "!!!"])
def test_sanitize_rejects_empty_names(raw: str) -> None:
    with pytest.raises(PatternError):
        sanitize_pattern_name(raw)


def test_path_traversal_stays_inside_directory(tmp_path: Path) -> None:
    store = PatternStore(tmp_path / "patterns")
    for kind in KINDS:
        for raw in ("../escape", "../../etc/passwd", "a/../../b", "/tmp/evil"):
            path = store.path_for(kind, raw)
            assert path.parent == (tmp_path / "patterns" / kind).resolve()
            assert path.suffix == ".json"


# --- store -------------------------------------------------------------------


def melodic_view(state: AppState) -> dict:
    return state.melodic_to_dict()


def drum_view(state: AppState) -> dict:
    return state.drums.to_dict()


def test_melodic_save_writes_only_into_melodic_directory(tmp_path: Path) -> None:
    store = PatternStore(tmp_path / "nested" / "patterns")
    assert store.save(MELODIC, rich_state(), "bite test") == "bite_test"
    path = tmp_path / "nested" / "patterns" / "melodic" / "bite_test.json"
    data = json.loads(path.read_text())
    assert data["version"] == 1 and data["type"] == "melodic"
    assert data["name"] == "bite test"
    assert "bpm" not in data and "step_division" not in data and "drums" not in data
    assert data["steps"][0]["note"] == 60 and data["transpose"] == -3
    assert store.list_names(MELODIC) == ["bite_test"]
    assert store.list_names(DRUMS) == []
    assert not (tmp_path / "nested" / "patterns" / "drums").exists()
    assert not [p for p in path.parent.iterdir() if p.name.startswith(".")]  # no temp leftovers


def test_drum_save_writes_only_into_drums_directory(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    state = rich_state()
    state.drums.lanes[0].steps[0].active = True
    state.drums.lanes[0].steps[0].velocity = 110
    assert store.save(DRUMS, state, "electro 01") == "electro_01"
    path = tmp_path / "drums" / "electro_01.json"
    data = json.loads(path.read_text())
    assert data["version"] == 1 and data["type"] == "drums" and data["name"] == "electro 01"
    assert "bpm" not in data and "steps" not in data and "transpose" not in data
    assert data["lanes"][0] == {
        "name": "Kick",
        "note": "C1",
        "steps": [{"active": True, "velocity": 110}] + [{"active": False, "velocity": 100}] * 15,
    }
    assert store.list_names(DRUMS) == ["electro_01"]
    assert store.list_names(MELODIC) == []
    assert not (tmp_path / "melodic").exists()


def test_loading_melodic_leaves_drums_and_tempo_alone(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    store.save(MELODIC, rich_state(), "melody")
    state = AppState(bpm=99, step_division=8, selected_drum_output="IAC")
    state.drums.lanes[1].steps[3].active = True
    state.drums.lanes[1].steps[3].velocity = 77
    state.drums.channel = 12
    state.drums.lanes[0].note = 35
    before_drums = drum_view(state)
    state.active_track = "drums"

    assert store.load(MELODIC, state, "melody") == "melody"
    assert melodic_view(state) == melodic_view(rich_state())
    assert drum_view(state) == before_drums
    assert (state.bpm, state.step_division) == (99, 8)
    assert state.selected_drum_output == "IAC" and state.active_track == "drums"
    assert state.playhead == 0 and state.cursor == 0


def test_loading_drums_leaves_melodic_state_alone(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    beat = AppState()
    beat.drums.lanes[2].steps[0].active = True
    beat.drums.lanes[2].steps[0].velocity = 45
    beat.drums.channel = 11
    store.save(DRUMS, beat, "beat")

    state = rich_state()
    state.set_bpm(77)
    state.drums.cursor_step = 9
    state.drums.audition[0] = 100
    before = melodic_view(state)
    assert store.load(DRUMS, state, "beat") == "beat"
    assert melodic_view(state) == before
    assert state.sequence == rich_state().sequence
    assert state.bpm == 77
    assert drum_view(state) == drum_view(beat)
    assert state.drums.cursor_step == 0 and state.drums.audition == {}


def test_overwrite_is_atomic_for_both_kinds(tmp_path: Path, monkeypatch) -> None:  # type: ignore[no-untyped-def]
    store = PatternStore(tmp_path)
    first = AppState(transpose=1)
    first.append_note(60)
    first.drums.lanes[0].steps[0].active = True
    second = AppState(transpose=2)
    second.append_note(72)
    second.drums.lanes[3].steps[15].active = True
    targets = ((MELODIC, tmp_path / "melodic" / "p.json"), (DRUMS, tmp_path / "drums" / "p.json"))

    for kind, path in targets:
        store.save(kind, first, "p")
        inode_before = path.stat().st_ino
        store.save(kind, second, "p")
        data = json.loads(path.read_text())
        if kind == MELODIC:
            assert data["transpose"] == 2 and data["steps"][0]["note"] == 72
        else:
            assert data["lanes"][3]["steps"][15]["active"] is True
            assert data["lanes"][0]["steps"][0]["active"] is False
        # os.replace swaps in a new inode; the old file was never partially written.
        assert path.stat().st_ino != inode_before

    # A failure mid-write leaves the existing pattern intact and no temp file.
    def boom(src, dst):  # type: ignore[no-untyped-def]
        raise OSError(28, "No space left on device")

    monkeypatch.setattr(os, "replace", boom)
    for kind, path in targets:
        with pytest.raises(PatternError, match="No space left"):
            store.save(kind, first, "p")
        assert json.loads(path.read_text())["name"] == "p"
        assert sorted(p.name for p in path.parent.iterdir()) == ["p.json"]


def test_corrupted_files_are_pattern_errors_for_both_kinds(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    (tmp_path / "melodic").mkdir()
    (tmp_path / "drums").mkdir()
    (tmp_path / "melodic" / "bad.json").write_text("{not json")
    (tmp_path / "drums" / "bad.json").write_text("[1, 2]")
    (tmp_path / "drums" / "broken.json").write_text(
        json.dumps({"version": 1, "type": "drums", "lanes": [{"name": "x"}]})
    )
    state = rich_state()
    before = (melodic_view(state), drum_view(state))
    with pytest.raises(PatternError, match="not valid JSON"):
        store.load(MELODIC, state, "bad")
    with pytest.raises(PatternError, match="JSON object"):
        store.load(DRUMS, state, "bad")
    with pytest.raises(PatternError, match="needs a 'note'"):
        store.load(DRUMS, state, "broken")
    assert (melodic_view(state), drum_view(state)) == before


def test_wrong_kind_and_unsupported_version_are_readable_errors(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    store.save(DRUMS, AppState(), "beat")
    store.save(MELODIC, AppState(), "tune")
    (tmp_path / "drums" / "future.json").write_text(
        json.dumps({"version": 9, "type": "drums", "lanes": []})
    )
    (tmp_path / "melodic" / "alien.json").write_text(json.dumps({"version": 1, "type": "bass"}))
    state = AppState()
    # A drum file dropped into melodic/ (or vice versa) is refused, not misread.
    (tmp_path / "melodic" / "beat.json").write_text((tmp_path / "drums" / "beat.json").read_text())
    with pytest.raises(PatternError, match="is a drum pattern, not a melodic pattern"):
        store.load(MELODIC, state, "beat")
    with pytest.raises(PatternError, match="unsupported drum pattern version 9"):
        store.load(DRUMS, state, "future")
    with pytest.raises(PatternError, match="unknown pattern type 'bass'"):
        store.load(MELODIC, state, "alien")
    with pytest.raises(PatternError, match="Unknown pattern kind"):
        store.list_names("bass")


def test_missing_pattern_and_missing_directory(tmp_path: Path) -> None:
    store = PatternStore(tmp_path / "nope")
    assert store.list_names(MELODIC) == [] and store.list_names(DRUMS) == []
    with pytest.raises(PatternError, match="Pattern not found: ghost"):
        store.load(MELODIC, AppState(), "ghost")
    with pytest.raises(PatternError, match="Pattern not found: ghost"):
        store.load(DRUMS, AppState(), "ghost")


@pytest.mark.skipif(os.geteuid() == 0, reason="root ignores permission bits")
def test_permission_denied_is_a_pattern_error(tmp_path: Path) -> None:
    locked = tmp_path / "locked"
    locked.mkdir()
    locked.chmod(stat.S_IRUSR | stat.S_IXUSR)
    try:
        for kind in KINDS:
            with pytest.raises(PatternError, match="Cannot write"):
                PatternStore(locked).save(kind, AppState(), "x")
    finally:
        locked.chmod(stat.S_IRWXU)


def test_delete_and_rename(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    store.save(MELODIC, rich_state(), "old")
    store.save(DRUMS, rich_state(), "old")
    assert store.rename(MELODIC, "old", "new name") == "new_name"
    assert store.list_names(MELODIC) == ["new_name"]
    assert store.list_names(DRUMS) == ["old"]  # the drum namespace is independent
    assert json.loads((tmp_path / "melodic" / "new_name.json").read_text())["name"] == "new name"
    with pytest.raises(PatternError, match="not found"):
        store.rename(MELODIC, "old", "x")
    store.delete(MELODIC, "new name")
    assert store.list_names(MELODIC) == []
    with pytest.raises(PatternError, match="not found"):
        store.delete(MELODIC, "new name")
    assert store.list_names(DRUMS) == ["old"]


# --- legacy combined files ---------------------------------------------------


def legacy_combined(tmp_path: Path) -> AppState:
    """Write a pre-split combined v2 file straight into the patterns root."""
    state = rich_state()
    state.drums.lanes[0].steps[0].active = True
    state.drums.lanes[0].steps[0].velocity = 123
    state.drums.channel = 13
    (tmp_path / "combo.json").write_text(json.dumps(state.to_dict(name="combo")))
    return state


def test_old_combined_pattern_loads_only_melodic_in_melodic_mode(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    old = legacy_combined(tmp_path)
    assert store.list_names(MELODIC) == ["combo"] and store.list_names(DRUMS) == ["combo"]
    state = AppState(bpm=60)
    state.drums.lanes[3].steps[7].active = True
    before_drums = drum_view(state)
    store.load(MELODIC, state, "combo")
    assert melodic_view(state) == melodic_view(old)
    assert drum_view(state) == before_drums
    assert state.bpm == 60  # legacy bpm is not applied


def test_old_combined_pattern_loads_only_drums_in_drums_mode(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    old = legacy_combined(tmp_path)
    state = AppState(bpm=60)
    state.append_note(40)
    before = melodic_view(state)
    store.load(DRUMS, state, "combo")
    assert drum_view(state) == drum_view(old)
    assert melodic_view(state) == before and state.bpm == 60


def test_old_flat_v1_file_is_melodic_only(tmp_path: Path) -> None:
    (tmp_path / "flat.json").write_text(
        json.dumps({"version": 1, "bpm": 140, "transpose": 3, "steps": [{"note": 61}]})
    )
    store = PatternStore(tmp_path)
    state = AppState()
    store.load(MELODIC, state, "flat")
    assert [step.note for step in state.sequence] == [61] and state.transpose == 3
    with pytest.raises(PatternError, match="no drum section"):
        store.load(DRUMS, state, "flat")


def test_saving_after_legacy_load_writes_split_files_and_keeps_the_old_one(
    tmp_path: Path,
) -> None:
    store = PatternStore(tmp_path)
    legacy_combined(tmp_path)
    state = AppState()
    store.load(MELODIC, state, "combo")
    store.save(MELODIC, state, "combo")
    store.load(DRUMS, state, "combo")
    store.save(DRUMS, state, "combo")
    assert (tmp_path / "combo.json").is_file()  # never deleted automatically
    assert json.loads((tmp_path / "melodic" / "combo.json").read_text())["type"] == "melodic"
    assert json.loads((tmp_path / "drums" / "combo.json").read_text())["type"] == "drums"
    # The split file now shadows the legacy one for both kinds.
    assert store.list_names(MELODIC) == ["combo"]
    assert store.exists(DRUMS, "combo")


# --- runtime integration -----------------------------------------------------


async def test_runtime_save_modify_load_restores_melodic_only(tmp_path: Path) -> None:
    state = rich_state()
    state.selected_input, state.selected_output = "IN", "OUT"
    runtime = MiniStepRuntime(state, output=None, pattern_store=PatternStore(tmp_path))
    expected = melodic_view(state)

    assert runtime.save_pattern("egypt_01") is True
    assert state.status_message == "Saved melodic pattern: egypt_01"

    state.clear()
    state.set_bpm(60)
    state.transpose = 5
    state.octave = -2
    state.set_loop_length(8)
    state.append_note(40)
    state.drums.lanes[0].steps[0].active = True
    assert melodic_view(state) != expected

    assert await runtime.load_pattern("egypt_01") is True
    assert state.status_message == "Loaded melodic pattern: egypt_01"
    assert melodic_view(state) == expected
    assert (state.selected_input, state.selected_output) == ("IN", "OUT")
    assert state.playing is False and state.playhead == 0
    assert state.bpm == 60 and state.drums.lanes[0].steps[0].active is True


async def test_runtime_failed_load_keeps_current_state(tmp_path: Path) -> None:
    (tmp_path / "melodic").mkdir()
    (tmp_path / "melodic" / "bad.json").write_text("nope")
    state = rich_state()
    before = (melodic_view(state), drum_view(state))
    runtime = MiniStepRuntime(state, output=None, pattern_store=PatternStore(tmp_path))
    assert await runtime.load_pattern("bad") is False
    assert state.status_message.startswith("Failed to load: bad.json")
    assert await runtime.load_pattern("missing") is False
    assert state.status_message == "Failed to load: Pattern not found: missing"
    assert runtime.save_pattern("...") is False
    assert state.status_message.startswith("Failed to save:")
    assert (melodic_view(state), drum_view(state)) == before


def test_fuzzy_rank_orders_exact_prefix_substring_subsequence() -> None:
    names = ["c_bass_machine", "c_tritone_bite", "egypt_phrygian", "bite", "abc"]
    assert fuzzy_rank("", names) == sorted(names)
    assert fuzzy_rank("bite", names) == ["bite", "c_tritone_bite"]
    assert fuzzy_rank("egy", names) == ["egypt_phrygian"]
    assert fuzzy_rank("cbm", names) == ["c_bass_machine"]
    assert fuzzy_rank("BASS", names) == ["c_bass_machine"]
    assert fuzzy_rank("zzz", names) == []


async def test_tui_picker_filters_and_selects_with_arrows(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    for name, transpose in (("alpha", 1), ("beta", 2), ("bass_two", 3)):
        store.save(MELODIC, AppState(transpose=transpose), name)
    store.save(DRUMS, AppState(), "drum_only")  # must never show up in the melodic picker
    state = AppState()
    runtime = MiniStepRuntime(state, output=None, pattern_store=store)
    app = MiniStepApp(runtime)
    async with app.run_test() as pilot:
        await pilot.press("L")
        picker = app.query_one("#prompt", PatternPicker)
        assert picker.matches == ["alpha", "bass_two", "beta"]
        await pilot.press("b")
        assert picker.matches == ["bass_two", "beta"]
        assert picker.highlighted_name == "bass_two"
        await pilot.press("down")
        assert picker.highlighted_name == "beta"
        await pilot.press("enter")
        assert state.status_message == "Loaded melodic pattern: beta" and state.transpose == 2

        # Save: picking an existing name overwrites it; typing a new one creates it.
        state.transpose = 9
        await pilot.press("S", "down", "enter")
        assert state.status_message == "Saved melodic pattern: bass_two"
        probe = AppState()
        store.load(MELODIC, probe, "bass_two")
        assert probe.transpose == 9
        await pilot.press("S", *"gamma", "enter")
        assert state.status_message == "Saved melodic pattern: gamma"
        assert sorted(store.list_names(MELODIC)) == ["alpha", "bass_two", "beta", "gamma"]
        assert store.list_names(DRUMS) == ["drum_only"]


async def test_tui_shift_s_and_shift_l_prompt_for_pattern_name(tmp_path: Path) -> None:
    state = rich_state()
    runtime = MiniStepRuntime(state, output=None, pattern_store=PatternStore(tmp_path))
    expected = melodic_view(state)
    app = MiniStepApp(runtime)
    async with app.run_test() as pilot:
        await pilot.press("S")
        assert app._prompt_mode == "save"
        assert "SAVE MELODIC PATTERN AS" in app._rendered["status"]
        # Letters that are normally shortcuts (h=hold, r=rest, c=clear) go to the name.
        await pilot.press(*"birch", "enter")
        assert state.status_message == "Saved melodic pattern: birch"
        assert len(state.sequence) == 4
        assert (tmp_path / "melodic" / "birch.json").is_file()

        await pilot.press("c")  # clear
        assert state.sequence == []
        await pilot.press("L")
        assert app._prompt_mode == "load"
        assert "LOAD MELODIC PATTERN" in app._rendered["status"]
        assert app.query_one("#prompt", PatternPicker).matches == ["birch"]
        await pilot.press(*"birch", "enter")
        assert state.status_message == "Loaded melodic pattern: birch"
        assert melodic_view(state) == expected

        await pilot.press("L", "escape")
        assert app._prompt_mode is None
        assert state.status_message == "Cancelled."
        assert melodic_view(state) == expected


async def test_tui_drum_track_picker_shows_and_saves_only_drum_patterns(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    store.save(MELODIC, AppState(), "melody_only")
    beat = AppState()
    beat.drums.lanes[1].steps[4].active = True
    store.save(DRUMS, beat, "electro")
    state = rich_state()
    melody_before = melodic_view(state)
    runtime = MiniStepRuntime(state, output=None, pattern_store=store)
    app = MiniStepApp(runtime)
    async with app.run_test() as pilot:
        await pilot.press("tab", "L")
        picker = app.query_one("#prompt", PatternPicker)
        assert picker.matches == ["electro"]
        assert "LOAD DRUM PATTERN" in app._rendered["status"]
        await pilot.press("enter")
        assert state.status_message == "Loaded drum pattern: electro"
        assert state.drums.lanes[1].steps[4].active is True
        assert melodic_view(state) == melody_before

        await pilot.press("right", "enter")  # kick on step 2, then save as a new drum pattern
        await pilot.press("S")
        assert "SAVE DRUM PATTERN AS" in app._rendered["status"]
        await pilot.press(*"broken", "enter")
        assert state.status_message == "Saved drum pattern: broken"
        assert (tmp_path / "drums" / "broken.json").is_file()
        assert not (tmp_path / "melodic" / "broken.json").exists()
        assert store.list_names(MELODIC) == ["melody_only"]

        # Back on the melodic track the picker is melodic again.
        await pilot.press("tab", "L")
        assert app.query_one("#prompt", PatternPicker).matches == ["melody_only"]
        await pilot.press("escape")


def test_fuzzy_search_is_independent_per_namespace(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    for name in ("bite_01", "egypt_01"):
        store.save(MELODIC, AppState(), name)
    for name in ("electro_01", "broken_01"):
        store.save(DRUMS, AppState(), name)
    assert fuzzy_rank("e", store.list_names(MELODIC)) == ["egypt_01", "bite_01"]
    assert fuzzy_rank("e", store.list_names(DRUMS)) == ["electro_01", "broken_01"]
    assert fuzzy_rank("bite", store.list_names(DRUMS)) == []
    assert fuzzy_rank("brk", store.list_names(DRUMS)) == ["broken_01"]
