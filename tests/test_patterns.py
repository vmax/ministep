"""Named pattern persistence: PatternStore, format tolerance, runtime and TUI wiring."""

from __future__ import annotations

import json
import os
import stat
from pathlib import Path

import pytest

from ministep.patterns import PatternError, PatternStore, sanitize_pattern_name
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
    data["steps"][0]["probability"] = 0.5
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
    for raw in ("../escape", "../../etc/passwd", "a/../../b", "/tmp/evil"):
        path = store.path_for(raw)
        assert path.parent == (tmp_path / "patterns").resolve()
        assert path.suffix == ".json"


# --- store -------------------------------------------------------------------


def test_store_creates_directory_and_writes_named_json(tmp_path: Path) -> None:
    store = PatternStore(tmp_path / "nested" / "patterns")
    assert store.save(rich_state(), "bite test") == "bite_test"
    path = tmp_path / "nested" / "patterns" / "bite_test.json"
    data = json.loads(path.read_text())
    assert data["version"] == 1
    assert data["name"] == "bite test"
    assert store.list_names() == ["bite_test"]
    assert not [p for p in path.parent.iterdir() if p.name.startswith(".")]  # no temp leftovers


def test_store_load_restores_full_state(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    state = rich_state()
    store.save(state, "full")
    assert musical_view(store.load("full")) == musical_view(state)


def test_atomic_overwrite_replaces_existing_pattern(tmp_path: Path, monkeypatch) -> None:  # type: ignore[no-untyped-def]
    store = PatternStore(tmp_path)
    first = AppState(bpm=100)
    first.append_note(60)
    store.save(first, "p")
    inode_before = (tmp_path / "p.json").stat().st_ino

    second = AppState(bpm=140)
    second.append_note(72)
    store.save(second, "p")
    data = json.loads((tmp_path / "p.json").read_text())
    assert data["bpm"] == 140 and data["steps"][0]["note"] == 72
    # os.replace swaps in a new inode; the old file was never partially written.
    assert (tmp_path / "p.json").stat().st_ino != inode_before

    # A failure mid-write leaves the existing pattern intact and no temp file.
    def boom(src, dst):  # type: ignore[no-untyped-def]
        raise OSError(28, "No space left on device")

    monkeypatch.setattr(os, "replace", boom)
    with pytest.raises(PatternError, match="No space left"):
        store.save(first, "p")
    assert json.loads((tmp_path / "p.json").read_text())["bpm"] == 140
    assert sorted(p.name for p in tmp_path.iterdir()) == ["p.json"]


def test_corrupted_json_is_a_pattern_error(tmp_path: Path) -> None:
    (tmp_path / "bad.json").write_text("{not json")
    with pytest.raises(PatternError, match="not valid JSON"):
        PatternStore(tmp_path).load("bad")


def test_unsupported_version_via_store(tmp_path: Path) -> None:
    (tmp_path / "new.json").write_text(json.dumps({"version": 2, "steps": []}))
    with pytest.raises(PatternError, match="Unsupported pattern version 2"):
        PatternStore(tmp_path).load("new")


def test_missing_pattern_and_missing_directory(tmp_path: Path) -> None:
    store = PatternStore(tmp_path / "nope")
    assert store.list_names() == []
    with pytest.raises(PatternError, match="Pattern not found: ghost"):
        store.load("ghost")


@pytest.mark.skipif(os.geteuid() == 0, reason="root ignores permission bits")
def test_permission_denied_is_a_pattern_error(tmp_path: Path) -> None:
    locked = tmp_path / "locked"
    locked.mkdir()
    locked.chmod(stat.S_IRUSR | stat.S_IXUSR)
    try:
        with pytest.raises(PatternError, match="Cannot write"):
            PatternStore(locked).save(AppState(), "x")
    finally:
        locked.chmod(stat.S_IRWXU)


def test_delete_and_rename(tmp_path: Path) -> None:
    store = PatternStore(tmp_path)
    store.save(rich_state(), "old")
    assert store.rename("old", "new name") == "new_name"
    assert store.list_names() == ["new_name"]
    assert json.loads((tmp_path / "new_name.json").read_text())["name"] == "new name"
    with pytest.raises(PatternError, match="not found"):
        store.rename("old", "x")
    store.delete("new name")
    assert store.list_names() == []
    with pytest.raises(PatternError, match="not found"):
        store.delete("new name")


# --- runtime integration -----------------------------------------------------


async def test_runtime_save_modify_load_restores_everything(tmp_path: Path) -> None:
    state = rich_state()
    state.selected_input, state.selected_output = "IN", "OUT"
    runtime = MiniStepRuntime(state, output=None, pattern_store=PatternStore(tmp_path))
    expected = musical_view(state)

    assert runtime.save_pattern("egypt_01") is True
    assert state.status_message == "Saved: egypt_01"

    state.clear()
    state.set_bpm(60)
    state.transpose = 5
    state.octave = -2
    state.set_loop_length(8)
    state.append_note(40)
    assert musical_view(state) != expected

    assert await runtime.load_pattern("egypt_01") is True
    assert state.status_message == "Loaded: egypt_01"
    assert musical_view(state) == expected
    assert (state.selected_input, state.selected_output) == ("IN", "OUT")
    assert state.playing is False and state.playhead == 0


async def test_runtime_failed_load_keeps_current_state(tmp_path: Path) -> None:
    (tmp_path / "bad.json").write_text("nope")
    state = rich_state()
    before = musical_view(state)
    runtime = MiniStepRuntime(state, output=None, pattern_store=PatternStore(tmp_path))
    assert await runtime.load_pattern("bad") is False
    assert state.status_message.startswith("Failed to load: bad.json")
    assert await runtime.load_pattern("missing") is False
    assert state.status_message == "Failed to load: Pattern not found: missing"
    assert runtime.save_pattern("...") is False
    assert state.status_message.startswith("Failed to save:")
    assert musical_view(state) == before


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
    for name, bpm in (("alpha", 100), ("beta", 110), ("bass_two", 120)):
        store.save(AppState(bpm=bpm), name)
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
        assert state.status_message == "Loaded: beta" and state.bpm == 110

        # Save: picking an existing name overwrites it; typing a new one creates it.
        state.set_bpm(90)
        await pilot.press("S", "down", "enter")
        assert state.status_message == "Saved: bass_two"
        assert store.load("bass_two").bpm == 90
        await pilot.press("S", *"gamma", "enter")
        assert state.status_message == "Saved: gamma"
        assert sorted(store.list_names()) == ["alpha", "bass_two", "beta", "gamma"]


async def test_tui_shift_s_and_shift_l_prompt_for_pattern_name(tmp_path: Path) -> None:
    state = rich_state()
    runtime = MiniStepRuntime(state, output=None, pattern_store=PatternStore(tmp_path))
    expected = musical_view(state)
    app = MiniStepApp(runtime)
    async with app.run_test() as pilot:
        await pilot.press("S")
        assert app._prompt_mode == "save"
        # Letters that are normally shortcuts (h=hold, r=rest, c=clear) go to the name.
        await pilot.press(*"birch", "enter")
        assert state.status_message == "Saved: birch"
        assert len(state.sequence) == 4
        assert (tmp_path / "birch.json").is_file()

        await pilot.press("c")  # clear
        assert state.sequence == []
        await pilot.press("L")
        assert app._prompt_mode == "load"
        assert app.query_one("#prompt", PatternPicker).matches == ["birch"]
        await pilot.press(*"birch", "enter")
        assert state.status_message == "Loaded: birch"
        assert musical_view(state) == expected

        await pilot.press("L", "escape")
        assert app._prompt_mode is None
        assert state.status_message == "Cancelled."
        assert musical_view(state) == expected
