"""Small Textual surface; it observes state and never owns timing or MIDI I/O."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import VerticalScroll
from textual.css.query import NoMatches
from textual.widgets import Footer, Header, Static

from .controller import Command, CommandName
from .drums import DRUM_STEPS, DrumStep
from .patterns import KIND_LABELS
from .picker import PatternPicker
from .runtime import MiniStepRuntime
from .state import Step, note_name

DEFAULT_SEQUENCE_PATH = Path.home() / ".config" / "ministep" / "sequence.json"
LEARNABLE_COMMANDS = (
    CommandName.COMMIT,
    CommandName.REST,
    CommandName.HOLD,
    CommandName.UNDO,
    CommandName.CLEAR,
    CommandName.PLAY_STOP,
    CommandName.RESTART,
    CommandName.RECORD_TOGGLE,
    CommandName.ARM_ROOT_CAPTURE,
    CommandName.STEP_DIVISION_UP,
    CommandName.STEP_DIVISION_DOWN,
    CommandName.SET_BPM,
    CommandName.SET_GATE,
    CommandName.SET_TRANSPOSE,
    CommandName.SET_OCTAVE,
)


class MiniStepApp(App[None]):
    CSS = """
    Screen { layout: vertical; }
    #summary { height: 7; padding: 0 1; }
    #sequence { height: 1fr; padding: 0 1; }
    #status { height: 2; padding: 0 1; color: $text-muted; }
    #prompt { display: none; }
    #prompt.active { display: block; }
    .playing { color: $success; }
    """
    BINDINGS = [
        ("space", "toggle", "Play/stop"),
        ("enter", "commit", "Commit"),
        ("h", "hold", "Hold"),
        ("m", "record_toggle", "Record"),
        ("k", "midi_learn", "MIDI Learn"),
        ("t", "arm_root", "Set root"),
        ("comma", "bpm_down", "BPM -"),
        ("full_stop", "bpm_up", "BPM +"),
        ("pageup", "division_up", "Division +"),
        ("pagedown", "division_down", "Division -"),
        ("r", "rest", "Rest"),
        ("u", "undo", "Undo"),
        ("backspace", "undo", "Undo"),
        ("c", "clear", "Clear"),
        ("home", "restart", "Restart"),
        ("s", "save", "Save"),
        ("l", "load", "Load"),
        ("S", "save_pattern", "Save as"),
        ("L", "load_pattern", "Load pattern"),
        # Footer label only; the key itself is handled synchronously in on_key.
        Binding("tab", "switch_track", "Track", priority=True),
        ("q", "quit", "Quit"),
    ]

    def __init__(
        self, runtime: MiniStepRuntime, sequence_path: Path = DEFAULT_SEQUENCE_PATH
    ) -> None:
        super().__init__()
        self.runtime = runtime
        self.sequence_path = sequence_path
        self._learn_selection: int | None = None
        # "save" or "load" while the pattern-name prompt is open, else None,
        # and the pattern kind ("melodic"/"drums") the prompt was opened for.
        self._prompt_mode: str | None = None
        self._prompt_kind: str | None = None
        self._rendered: dict[str, str] = {}

    def compose(self) -> ComposeResult:
        yield Header(show_clock=False)
        with VerticalScroll():
            yield Static(id="summary")
            yield Static(id="sequence")
            yield Static(id="status")
        yield PatternPicker(id="prompt")
        yield Footer()

    def on_mount(self) -> None:
        self.set_interval(0.05, self.refresh_view)
        self.refresh_view()

    def _update(self, widget_id: str, text: str) -> None:
        """Push text to a Static only when it changed.

        Every ``Static.update`` schedules a Textual layout/render pass on the shared
        asyncio loop, which can delay a sequencer wake-up by several milliseconds.
        Skipping unchanged widgets keeps the UI's share of the loop minimal.
        """
        if self._rendered.get(widget_id) == text:
            return
        try:
            widget = self.query_one(f"#{widget_id}", Static)
        except NoMatches:
            # A key can arrive before compose has mounted the Statics; the
            # periodic refresh will draw this frame a few milliseconds later.
            return
        self._rendered[widget_id] = text
        widget.update(text)

    def refresh_view(self) -> None:
        state = self.runtime.state
        last = "--" if state.last_note_played is None else note_name(state.last_note_played[0])
        last_velocity = "--" if state.last_note_played is None else str(state.last_note_played[1])
        held = ", ".join(note_name(note) for note in sorted(state.held_notes)) or "--"
        status = "[class=playing]PLAYING[/]" if state.playing else "STOPPED"
        record = "[bold red]REC ●[/]" if state.recording else "REC"
        source_root = state.sequence_root()
        root = (
            "--"
            if source_root is None
            else note_name(min(127, max(0, source_root + state.transpose)))
        )
        root_status = (
            "[bold yellow]ROOT? Play MIDI note[/]" if state.root_capture_armed else f"ROOT: {root}"
        )
        loop = "FULL" if state.loop_length is None else str(state.loop_length)
        drums = state.drums
        drum_out = state.selected_drum_output or state.selected_output or "not selected"
        if state.active_track == "drums":
            summary = self._drum_summary(status)
        else:
            summary = (
                f"MIDI IN: {state.selected_input or 'not selected'}\n"
                f"MIDI OUT: {state.selected_output or 'not selected'}\n"
                f"DRUM OUT: {drum_out}  (ch {drums.channel})\n"
                f"BPM {state.bpm:g}  |  1/{state.step_division}  |  gate {state.default_gate:.2f}  "
                f"|  transpose {state.transpose:+d}  |  octave {state.octave:+d}  |  loop {loop}\n"
                f"[bold]TRACK: MELODIC[/]  |  {record}  |  {root_status}  |  {status}\n"
                f"[bold]AUDITION: {last}[/]  |  velocity {last_velocity}  |  Held: {held}  "
                f"|  Steps: {len(state.sequence)}"
            )
        self._update("summary", summary)
        grid = self._drum_grid() if state.active_track == "drums" else self._sequence_grid()
        self._update("sequence", grid)
        if self._learn_selection is not None:
            first = (
                f"MIDI LEARN: ←/→ {LEARNABLE_COMMANDS[self._learn_selection].name}; "
                "Enter: arm CC/pad; Esc: cancel"
            )
        elif self._prompt_mode is not None:
            first = self._prompt_help()
        else:
            first = state.status_message
        if state.active_track == "drums":
            help_line = (
                "Tab track  •  MIDI key audition  •  Enter commit/toggle  •  R skip step  "
                "•  Esc drop audition  •  ←/→ step  •  ↑/↓ or 1-4 lane  •  -/+ velocity  "
                "•  X clear  •  C clear all"
            )
        else:
            help_line = (
                "Tab: drums  •  ←/→: select  •  ↑/↓: semitone  •  Shift+↑/↓: octave  •  H: hold  "
                "•  T: set root  •  ,/.: BPM  •  PgUp/PgDn: division  •  K: MIDI Learn  "
                "•  Shift+S/L: save/load pattern"
            )
        self._update("status", f"{first}\n{help_line}")

    def _drum_summary(self, status: str) -> str:
        """Header for the drum track: only what affects drums, no melodic-only state."""
        state = self.runtime.state
        drums = state.drums
        drum_out = state.selected_drum_output or state.selected_output or "not selected"
        if drums.audition:
            parts = []
            for index in sorted(drums.audition):
                lane = drums.lanes[index]
                parts.append(f"{self._lane_label(lane.name)} {note_name(lane.note)}")
            velocities = ", ".join(str(drums.audition[index]) for index in sorted(drums.audition))
            audition = f"[bold]AUDITION: {' + '.join(parts)}[/]  |  velocity {velocities}"
        else:
            audition = "[bold]AUDITION: --[/]  |  play a drum key, then Enter"
        lane = drums.cursor_lane_obj
        hit = drums.cursor
        cursor = (
            f"STEP {drums.cursor_step + 1}/{DRUM_STEPS}  |  lane {lane.name} "
            f"{note_name(lane.note)}  |  hit {'ON' if hit.active else 'off'}  "
            f"|  velocity {hit.velocity}"
        )
        return (
            f"MIDI IN: {state.selected_input or 'not selected'}\n"
            f"DRUM OUT: {drum_out}  (ch {drums.channel})\n"
            f"BPM {state.bpm:g}  |  1/{state.step_division}  |  [bold]TRACK: DRUMS[/]  "
            f"|  {status}\n"
            f"{audition}\n"
            f"{cursor}"
        )

    def _prompt_help(self) -> str:
        label = KIND_LABELS[self._prompt_kind or self.runtime.pattern_kind].upper()
        if self._prompt_mode == "save":
            return (
                f"SAVE {label} AS: type a new name or pick one to overwrite  "
                "•  ↑/↓ select  •  Enter  •  Esc"
            )
        return f"LOAD {label}: type to filter  •  ↑/↓ select  •  Enter  •  Esc"

    def _sequence_grid(self) -> str:
        steps = self.runtime.state.sequence
        if not steps:
            return "Sequence is empty. Play a note, listen, then press Enter (or your COMMIT pad)."
        cell_width = 7
        # Keep KeyStep-style numbered cells aligned without wrapping within a
        # narrow terminal; render up to the requested 16 cells when space permits.
        steps_per_row = max(4, min(16, (self.size.width - 2) // cell_width))
        lines: list[str] = []
        for start in range(0, len(steps), steps_per_row):
            chunk = steps[start : start + steps_per_row]
            numbers = "".join(f"{start + index + 1:^{cell_width}}" for index in range(len(chunk)))
            values = "".join(
                self._render_step(step, start + index) for index, step in enumerate(chunk)
            )
            lines.extend((numbers, values, ""))
        return "\n".join(lines)

    def _drum_grid(self) -> str:
        """Lane-per-row grid in four beat groups: ``x`` is a hit, ``.`` a silent step.

        Cursor cell: yellow background (strongest). Playback column: green
        text. Beat starts (1, 5, 9, 13): bold step numbers and ``·`` instead of
        ``.`` so groups read at a glance without extra colour.
        """
        state = self.runtime.state
        drums = state.drums
        if not drums.lanes:
            return "Drum track has no lanes."
        name_width = max(len(self._lane_label(lane.name)) for lane in drums.lanes) + 2
        playhead = self.runtime.sequencer.drum_step if self.runtime.sequencer else None
        playing = state.playing

        def row(cell: Callable[[int], str]) -> str:
            groups = []
            for group in range(0, DRUM_STEPS, 4):
                groups.append("".join(cell(index) for index in range(group, group + 4)))
            return " [dim]|[/]".join(groups)

        def number(index: int) -> str:
            text = f"{index + 1:>3}"
            if playing and index == playhead:
                return f"[bold green]{text}[/]"
            return f"[bold]{text}[/]" if index % 4 == 0 else f"[dim]{text}[/]"

        lines = [" " * name_width + row(number)]
        for lane_index, lane in enumerate(drums.lanes):
            label = self._lane_label(lane.name)
            if lane_index == drums.cursor_lane:
                label = f"[bold yellow]{label:<{name_width}}[/]"
            else:
                label = f"{label:<{name_width}}"
            cells = row(
                lambda index, lane=lane, lane_index=lane_index: self._render_drum_cell(
                    lane.steps[index],
                    index,
                    selected=lane_index == drums.cursor_lane and index == drums.cursor_step,
                    playing=playing and index == playhead,
                    auditioned=lane_index in drums.audition,
                )
            )
            lines.append(f"{label}{cells}")
        return "\n".join(lines)

    @staticmethod
    def _lane_label(name: str) -> str:
        words = name.upper().split()
        if len(words) > 1:
            # "Closed Hat" -> "CHH", "Open Hat" -> "OHH": initials plus the last word's initial.
            return "".join(word[0] for word in words) + words[-1][0]
        return name.upper()[:6]

    @staticmethod
    def _render_drum_cell(
        step: DrumStep, index: int, *, selected: bool, playing: bool, auditioned: bool
    ) -> str:
        if step.active:
            mark = "X" if step.velocity >= 115 else "x"
        else:
            mark = "·" if index % 4 == 0 else "."
        if selected and auditioned:
            mark = "+"  # a pending hit that Enter will write here
        cell = f"  {mark}"
        if selected:
            return f"[bold black on yellow]{cell}[/]"
        if playing:
            return f"[bold green]{cell}[/]"
        return cell

    def _render_step(self, step: Step, index: int) -> str:
        value = "--" if step.note is None else note_name(step.note)
        if not step.enabled:
            value = f"({value})"
        if step.tie:
            value = f"{value}~"
        value = value[:4]
        cursor = "▶" if index == self.runtime.state.cursor else " "
        playing = (
            "●" if self.runtime.state.playing and index == self.runtime.state.playhead else " "
        )
        cell = f" {cursor}{playing}{value:<4}"
        if playing == "●":
            return f"[bold black on green]{cell}[/]"
        if cursor == "▶":
            return f"[bold yellow]{cell}[/]"
        return cell

    async def action_toggle(self) -> None:
        await self.runtime.toggle_play()

    async def action_commit(self) -> None:
        if self._learn_selection is not None:
            target = LEARNABLE_COMMANDS[self._learn_selection]
            self._learn_selection = None
            self.runtime.arm_midi_learn(target)
            return
        if self._drums_active():
            if self.runtime.state.drums.audition:
                self.runtime.commit()
            else:
                self._drum_toggle_hit()
            return
        await self.runtime.execute(Command(CommandName.COMMIT))

    async def action_switch_track(self) -> None:
        """Reached only via the footer click; the key press is consumed in on_key."""
        self.runtime.state.toggle_track()
        self.refresh_view()

    def _drums_active(self) -> bool:
        return self.runtime.state.active_track == "drums"

    def _drum_toggle_hit(self) -> None:
        drums = self.runtime.state.drums
        active = drums.toggle_hit()
        lane = drums.cursor_lane_obj
        self.runtime.state.status_message = (
            f"{lane.name} step {drums.cursor_step + 1}: {'hit' if active else 'off'} "
            f"(velocity {drums.cursor.velocity})."
        )

    async def action_hold(self) -> None:
        await self.runtime.execute(Command(CommandName.HOLD))

    async def action_record_toggle(self) -> None:
        await self.runtime.execute(Command(CommandName.RECORD_TOGGLE))

    async def action_midi_learn(self) -> None:
        if self.runtime.mapping is None:
            self.runtime.state.status_message = "Restart with --mapping minilab3 to use MIDI Learn."
            return
        self._learn_selection = 0

    async def action_bpm_down(self) -> None:
        await self.runtime.execute(Command(CommandName.BPM_DOWN))

    async def action_bpm_up(self) -> None:
        await self.runtime.execute(Command(CommandName.BPM_UP))

    async def action_division_up(self) -> None:
        await self.runtime.execute(Command(CommandName.STEP_DIVISION_UP))

    async def action_division_down(self) -> None:
        await self.runtime.execute(Command(CommandName.STEP_DIVISION_DOWN))

    async def action_arm_root(self) -> None:
        await self.runtime.execute(Command(CommandName.ARM_ROOT_CAPTURE))

    async def action_rest(self) -> None:
        await self.runtime.execute(Command(CommandName.REST))

    async def action_undo(self) -> None:
        await self.runtime.execute(Command(CommandName.UNDO))

    async def action_clear(self) -> None:
        if self._drums_active():
            self.runtime.state.drums.clear()
            self.runtime.state.status_message = "Drum track cleared."
            return
        await self.runtime.execute(Command(CommandName.CLEAR))

    async def action_restart(self) -> None:
        await self.runtime.restart()

    async def action_save(self) -> None:
        try:
            await self.runtime.save(self.sequence_path)
        except OSError as error:
            self.runtime.state.status_message = f"Save failed: {error}"

    async def action_load(self) -> None:
        try:
            await self.runtime.load(self.sequence_path)
        except (OSError, ValueError) as error:
            self.runtime.state.status_message = f"Load failed: {error}"

    async def action_save_pattern(self) -> None:
        self._open_prompt("save")

    async def action_load_pattern(self) -> None:
        kind = self.runtime.pattern_kind
        if not self.runtime.patterns.list_names(kind):
            self.runtime.state.status_message = (
                f"No saved {KIND_LABELS[kind]}s in {self.runtime.patterns.kind_directory(kind)}"
            )
            return
        self._open_prompt("load")

    def check_action(self, action: str, parameters: tuple[object, ...]) -> bool | None:
        # While naming a pattern, every other shortcut is parked so typing a
        # name cannot trigger transport or edit commands.
        return self._prompt_mode is None

    def _open_prompt(self, mode: str) -> None:
        # The kind is fixed when the prompt opens so a Tab mid-prompt (blocked
        # anyway) could never redirect the save or load to the other track.
        kind = self.runtime.pattern_kind
        self._prompt_mode = mode
        self._prompt_kind = kind
        label = KIND_LABELS[kind]
        picker = self.query_one("#prompt", PatternPicker)
        picker.add_class("active")
        picker.open(
            self.runtime.patterns.list_names(kind),
            allow_new=mode == "save",
            placeholder=f"save {label} as…" if mode == "save" else f"load {label}: filter",
        )
        self.refresh_bindings()
        self.refresh_view()

    def _close_prompt(self) -> None:
        self._prompt_mode = None
        self._prompt_kind = None
        picker = self.query_one("#prompt", PatternPicker)
        picker.remove_class("active")
        self.set_focus(None)
        self.refresh_bindings()
        self.refresh_view()

    async def on_pattern_picker_picked(self, event: PatternPicker.Picked) -> None:
        mode, kind = self._prompt_mode, self._prompt_kind
        self._close_prompt()
        if mode == "save":
            self.runtime.save_pattern(event.name, kind)
        elif mode == "load":
            await self.runtime.load_pattern(event.name, kind)

    async def action_quit(self) -> None:
        await self.runtime.shutdown()
        self.exit()

    async def on_key(self, event) -> None:  # type: ignore[no-untyped-def]
        key = event.key
        if self._prompt_mode is not None:
            # The focused Input owns text keys; Esc and Up/Down reach here.
            if key == "escape":
                self._close_prompt()
                self.runtime.state.status_message = "Cancelled."
                event.stop()
            elif key in ("up", "down"):
                self.query_one("#prompt", PatternPicker).move(1 if key == "down" else -1)
                event.stop()
            return
        if self._learn_selection is not None:
            if key in ("left", "up"):
                self._learn_selection = (self._learn_selection - 1) % len(LEARNABLE_COMMANDS)
            elif key in ("right", "down"):
                self._learn_selection = (self._learn_selection + 1) % len(LEARNABLE_COMMANDS)
            elif key == "escape":
                self._learn_selection = None
                self.runtime.state.status_message = "MIDI Learn cancelled."
            else:
                return
            event.stop()
            return
        if key == "tab":
            # Handled here, synchronously, so a following arrow key can never be
            # processed before the track has actually switched. Stopping the
            # event also keeps Textual's default focus cycling away from it.
            event.prevent_default()
            event.stop()
            self.runtime.state.toggle_track()
            self.refresh_view()
            return
        if self._drums_active():
            if self._on_drum_key(key):
                event.stop()
            return
        if key == "left":
            self.runtime.state.move_cursor(-1)
        elif key == "right":
            self.runtime.state.move_cursor(1)
        elif key in ("up", "down", "shift+up", "shift+down"):
            delta = 12 if key.startswith("shift+") else 1
            if key.endswith("down"):
                delta *= -1
            self.runtime.state.adjust_cursor_note(delta)
        elif key in ("[", "left_square_bracket"):
            self.runtime.state.set_default_gate(self.runtime.state.default_gate - 0.05)
        elif key in ("]", "right_square_bracket"):
            self.runtime.state.set_default_gate(self.runtime.state.default_gate + 0.05)
        elif key == "e":
            self.runtime.state.replace_cursor_with_last_note()
        elif key in ("x", "delete"):
            self.runtime.state.delete_cursor()
        else:
            return
        event.stop()

    def _on_drum_key(self, key: str) -> bool:
        """Drum-track editing keys; return whether ``key`` was consumed."""
        state = self.runtime.state
        drums = state.drums
        if key == "left":
            drums.move_step(-1)
        elif key == "right":
            drums.move_step(1)
        elif key == "up":
            drums.move_lane(-1)
        elif key == "down":
            drums.move_lane(1)
        elif key in ("minus", "-"):
            state.status_message = f"Velocity {drums.adjust_velocity(-10)}."
        elif key in ("plus", "+", "equals_sign", "="):
            state.status_message = f"Velocity {drums.adjust_velocity(10)}."
        elif key in ("x", "delete"):
            drums.clear_hit()
            lane = drums.cursor_lane_obj
            state.status_message = f"{lane.name} step {drums.cursor_step + 1}: off."
        elif key == "escape":
            drums.clear_audition()
            state.status_message = "Audition dropped."
        elif key.isdigit() and 1 <= int(key) <= len(drums.lanes):
            drums.cursor_lane = int(key) - 1
            lane = drums.cursor_lane_obj
            state.status_message = f"Lane {lane.name} ({note_name(lane.note)})."
        else:
            return False
        return True
