"""Fuzzy pattern picker: an Input that filters an OptionList of saved names.

Typing narrows the list (subsequence match, best hits first), Up/Down move the
highlight, Enter picks the highlighted name or, when nothing matches, the typed
text itself so a new name can be saved. Esc is handled by the App. The widget
knows nothing about files: it emits :class:`PatternPicker.Picked` and the App
decides what to do with the name.
"""

from __future__ import annotations

from textual.app import ComposeResult
from textual.containers import Vertical
from textual.message import Message
from textual.widgets import Input, OptionList


def fuzzy_rank(query: str, names: list[str]) -> list[str]:
    """Return ``names`` matching ``query`` as a case-insensitive subsequence.

    Order: exact, prefix, substring, then scattered subsequence; ties keep the
    earliest match position and then alphabetical order. An empty query keeps
    every name in alphabetical order.
    """
    needle = query.strip().lower()
    if not needle:
        return sorted(names)
    ranked: list[tuple[int, int, str]] = []
    for name in names:
        hay = name.lower()
        if hay == needle:
            ranked.append((0, 0, name))
        elif hay.startswith(needle):
            ranked.append((1, 0, name))
        elif needle in hay:
            ranked.append((2, hay.index(needle), name))
        else:
            position = _subsequence_position(needle, hay)
            if position is not None:
                ranked.append((3, position, name))
    return [name for _, _, name in sorted(ranked)]


def _subsequence_position(needle: str, hay: str) -> int | None:
    """Index of the first needle character if needle is a subsequence, else None."""
    start = -1
    first = None
    for char in needle:
        start = hay.find(char, start + 1)
        if start < 0:
            return None
        if first is None:
            first = start
    return first


class PatternPicker(Vertical):
    """Input + filtered OptionList; closed by the App, never by itself."""

    DEFAULT_CSS = """
    PatternPicker { height: auto; max-height: 12; margin: 0 1; }
    PatternPicker OptionList { height: auto; max-height: 8; }
    """

    class Picked(Message):
        def __init__(self, name: str, existing: bool) -> None:
            super().__init__()
            self.name = name
            self.existing = existing

    def __init__(self, id: str | None = None) -> None:
        super().__init__(id=id)
        self.names: list[str] = []
        self.matches: list[str] = []
        self.allow_new = False

    def compose(self) -> ComposeResult:
        yield Input(placeholder="pattern name")
        yield OptionList()

    def open(self, names: list[str], *, allow_new: bool, placeholder: str) -> None:
        self.names = list(names)
        self.allow_new = allow_new
        field = self.query_one(Input)
        field.placeholder = placeholder
        field.value = ""
        self._refilter("")
        field.focus()

    @property
    def highlighted_name(self) -> str | None:
        option_list = self.query_one(OptionList)
        if option_list.highlighted is None or not self.matches:
            return None
        return self.matches[option_list.highlighted]

    def move(self, delta: int) -> None:
        option_list = self.query_one(OptionList)
        if not self.matches:
            return
        current = option_list.highlighted if option_list.highlighted is not None else -1
        option_list.highlighted = (current + delta) % len(self.matches)

    def _refilter(self, query: str) -> None:
        self.matches = fuzzy_rank(query, self.names)
        option_list = self.query_one(OptionList)
        option_list.clear_options()
        option_list.add_options(self.matches)
        option_list.highlighted = 0 if self.matches else None
        option_list.display = bool(self.matches)

    def on_input_changed(self, event: Input.Changed) -> None:
        event.stop()
        self._refilter(event.value)

    def on_input_submitted(self, event: Input.Submitted) -> None:
        event.stop()
        typed = event.value.strip()
        highlighted = self.highlighted_name
        # Typed text that exactly names a pattern wins over the fuzzy highlight,
        # and typing a new name for Save must not be hijacked by a near match.
        if typed and typed in self.names:
            self.post_message(self.Picked(typed, existing=True))
        elif self.allow_new and typed and highlighted != typed:
            self.post_message(self.Picked(typed, existing=False))
        elif highlighted is not None:
            self.post_message(self.Picked(highlighted, existing=True))
        elif typed:
            self.post_message(self.Picked(typed, existing=False))

    def on_option_list_option_selected(self, event: OptionList.OptionSelected) -> None:
        event.stop()
        self.post_message(self.Picked(self.matches[event.option_index], existing=True))
