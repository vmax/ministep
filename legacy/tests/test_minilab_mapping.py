from types import SimpleNamespace

from ministep.mappings.minilab3 import MINILAB3_MAPPING


def test_starter_minilab_mapping_does_not_consume_musical_notes_or_ccs() -> None:
    assert (
        MINILAB3_MAPPING.command_for(SimpleNamespace(type="note_on", note=37, velocity=100)) is None
    )
    assert (
        MINILAB3_MAPPING.command_for(SimpleNamespace(type="control_change", control=16, value=127))
        is None
    )
