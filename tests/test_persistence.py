from ministep.state import AppState


def test_json_save_and_load_round_trip(tmp_path) -> None:  # type: ignore[no-untyped-def]
    path = tmp_path / "melody.json"
    state = AppState(bpm=99, step_division=8, loop_length=16, default_gate=0.7, transpose=-2)
    state.append_note(60, 111)
    state.append_rest()
    state.save(path)

    loaded = AppState()
    loaded.load(path)

    assert loaded.bpm == 99
    assert loaded.step_division == 8
    assert loaded.loop_length == 16
    assert loaded.default_gate == 0.7
    assert loaded.transpose == -2
    assert [(step.note, step.velocity) for step in loaded.sequence] == [(60, 111), (None, 100)]
