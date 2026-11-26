from __future__ import annotations

import mido

from ministep.runtime import MiniStepRuntime
from ministep.state import AppState


def test_main_encoder_browses_and_edits_the_menu() -> None:
    state = AppState(bpm=128, default_gate=0.5)
    runtime = MiniStepRuntime(state, output=None)

    assert runtime.handle_main_encoder(mido.Message("control_change", control=118, value=127))
    assert runtime.menu_screen() == ("MENU > BPM", "128 BPM")
    assert runtime.handle_main_encoder(mido.Message("control_change", control=28, value=65))
    assert runtime.menu_screen() == ("MENU > GATE", "0.50")

    assert runtime.handle_main_encoder(mido.Message("control_change", control=118, value=127))
    assert runtime.menu_screen() == ("EDIT > GATE", "0.50")
    assert runtime.handle_main_encoder(mido.Message("control_change", control=28, value=66))
    assert state.default_gate == 0.6
    assert runtime.menu_screen() == ("EDIT > GATE", "0.60")


def test_shift_click_closes_the_main_encoder_menu() -> None:
    runtime = MiniStepRuntime(AppState(), output=None)
    runtime.handle_main_encoder(mido.Message("control_change", control=115, value=127))

    assert runtime.handle_main_encoder(mido.Message("control_change", control=119, value=127))
    assert runtime.menu_screen() is None


def test_main_encoder_menu_changes_loop_length() -> None:
    state = AppState(loop_length=None)
    runtime = MiniStepRuntime(state, output=None)

    runtime.handle_main_encoder(mido.Message("control_change", control=118, value=127))
    runtime.handle_main_encoder(mido.Message("control_change", control=28, value=68))
    assert runtime.menu_screen() == ("MENU > LOOP LENGTH", "FULL")
    runtime.handle_main_encoder(mido.Message("control_change", control=118, value=127))
    runtime.handle_main_encoder(mido.Message("control_change", control=28, value=65))

    assert state.loop_length == 8
    assert runtime.menu_screen() == ("EDIT > LOOP LENGTH", "8 STEPS")


def test_main_encoder_menu_selects_a_control_profile_page() -> None:
    runtime = MiniStepRuntime(AppState(), output=None, control_pages=("FILTER", "OSC"))

    runtime.handle_main_encoder(mido.Message("control_change", control=118, value=127))
    runtime.handle_main_encoder(mido.Message("control_change", control=28, value=69))
    assert runtime.menu_screen() == ("MENU > CONTROL PAGE", "FILTER")
    runtime.handle_main_encoder(mido.Message("control_change", control=118, value=127))
    runtime.handle_main_encoder(mido.Message("control_change", control=28, value=65))

    assert runtime.control_page_index == 1
    assert runtime.menu_screen() == ("EDIT > CONTROL PAGE", "OSC")
