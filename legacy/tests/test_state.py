from ministep.state import AppState, Step, note_name


def test_note_name_uses_ableton_live_octave_numbering() -> None:
    assert note_name(60) == "C3"
    assert note_name(61) == "C#3"
    assert note_name(0) == "C-2"


def test_sequence_append_rest_and_undo() -> None:
    state = AppState()
    state.last_note_played = (60, 111)

    assert state.commit_last_note() is True
    state.append_rest()
    assert [(step.note, step.velocity) for step in state.sequence] == [(60, 111), (None, 100)]

    removed = state.undo()
    assert removed == Step(note=None)
    assert [step.note for step in state.sequence] == [60]


def test_hold_extends_last_note_and_undo_restores_its_release() -> None:
    state = AppState()
    state.append_note(60, 111)

    held = state.append_hold()
    assert held is not None
    assert [(step.note, step.gate, step.tie) for step in state.sequence] == [
        (60, 0.5, True),
        (60, 1.0, False),
    ]

    state.undo()
    assert [(step.note, step.tie) for step in state.sequence] == [(60, False)]


def test_commit_requires_an_auditioned_note() -> None:
    state = AppState()
    assert state.commit_last_note() is False
    assert state.sequence == []
    assert "Nothing to commit" in state.status_message


def test_auditioned_note_is_exposed_before_it_is_committed() -> None:
    state = AppState()
    state.note_held(61, 107)

    assert state.last_note_played == (61, 107)
    assert "Auditioning C#3" in state.status_message
    assert state.sequence == []


def test_arrow_editing_changes_selected_pitched_step() -> None:
    state = AppState(sequence=[Step(60), Step(None)])
    assert state.adjust_cursor_note(1) is True
    assert state.sequence[0].note == 61
    state.move_cursor(1)
    assert state.adjust_cursor_note(1) is False
    assert state.sequence[1].note is None


def test_set_root_edits_notes_using_nearest_pitch_class_transposition() -> None:
    state = AppState(sequence=[Step(60), Step(63), Step(67)])
    assert state.set_root_from_note(70) is True  # Bb
    assert state.transpose == 0
    assert [step.note for step in state.sequence] == [58, 61, 65]


def test_loop_length_limits_playback_without_truncating_the_sequence() -> None:
    state = AppState(sequence=[Step(60)] * 12, loop_length=8)

    assert state.playback_length() == 8
    assert len(state.sequence) == 12
