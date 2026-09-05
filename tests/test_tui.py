from ministep.runtime import MiniStepRuntime
from ministep.state import AppState, Step
from ministep.tui import MiniStepApp


def test_step_cell_marks_cursor_and_current_playhead() -> None:
    state = AppState(sequence=[Step(60)], playing=True, playhead=0, cursor=0)
    app = MiniStepApp(MiniStepRuntime(state, output=None))

    assert "▶●C3" in app._render_step(state.sequence[0], 0)


def test_step_cell_marks_selected_step_without_changing_note_display() -> None:
    state = AppState(sequence=[Step(61)])
    app = MiniStepApp(MiniStepRuntime(state, output=None))

    assert "▶ C#3" in app._render_step(state.sequence[0], 0)
