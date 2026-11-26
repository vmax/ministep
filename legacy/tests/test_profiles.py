from __future__ import annotations

from pathlib import Path

import mido
import pytest

from ministep.profiles import ProfileError, ProfileRouter, load_profile


class FakeOutput:
    def __init__(self) -> None:
        self.events: list[tuple[int, int, int]] = []

    def control_change(self, control: int, value: int, channel: int = 0) -> None:
        self.events.append((control, value, channel))


def test_profile_routes_knobs_by_active_page(tmp_path) -> None:  # type: ignore[no-untyped-def]
    path = tmp_path / "profile.yaml"
    path.write_text(
        """name: Test Synth
channel: 2
pages:
  FILTER:
    knobs:
      1: { cc: 20, label: CUTOFF }
  OSC:
    knobs:
      1: { cc: 82, label: OSC1 }
""",
        encoding="utf-8",
    )
    output = FakeOutput()
    router = ProfileRouter(load_profile(path), output)

    routed = router.handle_message(mido.Message("control_change", control=74, value=96))
    assert output.events == [(20, 96, 1)]
    assert routed is not None
    assert (routed.page, routed.label, routed.value) == ("FILTER", "CUTOFF", 96)

    router.select_page(1)
    router.handle_message(mido.Message("control_change", control=74, value=31))
    assert output.events[-1] == (82, 31, 1)
    assert router.is_knob_message(mido.Message("control_change", control=74, value=31))
    assert router.handle_message(mido.Message("control_change", control=71, value=31)) is None


def test_profile_rejects_invalid_knob_cc(tmp_path) -> None:  # type: ignore[no-untyped-def]
    path = tmp_path / "bad.yaml"
    path.write_text(
        "name: Broken\npages:\n  FILTER:\n    knobs:\n      1: { cc: 128, label: CUTOFF }\n",
        encoding="utf-8",
    )

    with pytest.raises(ProfileError, match="between 0 and 127"):
        load_profile(path)


def test_bundled_miniverse_profile_loads() -> None:
    profile_path = Path(__file__).parents[1] / "profiles" / "miniverse.yaml"
    profile = load_profile(profile_path)

    assert profile.name == "Miniverse"
    assert profile.page_names == (
        "CONTROLLERS",
        "OSCILLATOR BANK",
        "MIXER",
        "MODIFIERS",
        "OUTPUT",
    )
