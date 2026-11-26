from types import SimpleNamespace

from ministep.controller import AbsoluteCC, Command, CommandName, SimpleMapping


def test_pad_note_and_absolute_cc_translate_to_commands() -> None:
    mapping = SimpleMapping(
        note_commands={(9, 36): Command(CommandName.COMMIT)},
        cc_commands={},
        absolute_cc=(AbsoluteCC(16, CommandName.SET_BPM, 40, 240, integer=True),),
    )
    assert mapping.command_for(
        SimpleNamespace(type="note_on", channel=9, note=36, velocity=100)
    ) == Command(CommandName.COMMIT)
    assert (
        mapping.command_for(SimpleNamespace(type="note_on", channel=0, note=36, velocity=100))
        is None
    )
    assert (
        mapping.command_for(SimpleNamespace(type="note_on", channel=9, note=36, velocity=0)) is None
    )
    assert mapping.command_for(
        SimpleNamespace(type="control_change", control=16, value=127)
    ) == Command(CommandName.SET_BPM, 240)
