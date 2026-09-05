from __future__ import annotations

from ministep.controller import Command, CommandName
from ministep.main import display_command


def test_display_command_uses_parameter_names_and_scaled_values() -> None:
    assert display_command(Command(CommandName.SET_BPM, 142)) == ("BPM", "142")
    assert display_command(Command(CommandName.SET_GATE, 0.65)) == ("GATE", "0.65")
    assert display_command(Command(CommandName.SET_TRANSPOSE, -12)) == ("TRANSPOSE", "-12")
