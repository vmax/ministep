"""Application coordinator joining state, MIDI input, controller commands, and playback."""

from __future__ import annotations

from pathlib import Path

import mido

from .controller import ABSOLUTE_CC_RANGES, Command, CommandName, SimpleMapping
from .midi import OwnedMidiOutput
from .sequencer import Sequencer
from .state import LOOP_LENGTHS, PLAYBACK_DIVISIONS, AppState

BASE_MENU_ITEMS = ("BPM", "GATE", "DIVISION", "TRANSPOSE", "LOOP LENGTH")
_MAIN_ENCODER_TURNS = frozenset((28, 114))
_MAIN_ENCODER_PRESSES = frozenset((115, 118))
_MAIN_ENCODER_SHIFT_PRESSES = frozenset((113, 119))


def _relative_delta(value: int) -> int:
    """Accept MiniLab's 64-centred and common 1/127 relative encodings."""
    if value == 64:
        return 0
    if 1 <= value <= 3:
        return value
    if 125 <= value <= 127:
        return value - 128
    return value - 64


class MiniStepRuntime:
    def __init__(
        self,
        state: AppState,
        output: OwnedMidiOutput | None,
        mapping: SimpleMapping | None = None,
        mapping_path: Path | None = None,
        control_pages: tuple[str, ...] = (),
    ) -> None:
        self.state = state
        self.output = output
        self.mapping = mapping
        self.mapping_path = mapping_path
        self.midi_learn_target: CommandName | None = None
        self.sequencer = Sequencer(state, output) if output is not None else None
        self.menu_open = False
        self.menu_editing = False
        self.menu_index = 0
        self.control_pages = control_pages
        self.control_page_index = 0
        self.menu_items = BASE_MENU_ITEMS + (("CONTROL PAGE",) if control_pages else ())

    async def execute(self, command: Command) -> None:
        name, value = command.name, command.value
        if name == CommandName.COMMIT:
            self.state.commit_last_note()
        elif name == CommandName.HOLD:
            self.state.append_hold()
        elif name == CommandName.RECORD:
            self.set_recording(True if value is None else bool(value))
        elif name == CommandName.RECORD_TOGGLE:
            self.set_recording(not self.state.recording)
        elif name == CommandName.REST:
            self.state.append_rest()
        elif name == CommandName.UNDO:
            self.state.undo()
        elif name == CommandName.CLEAR:
            self.state.clear()
        elif name == CommandName.PLAY:
            self.play()
        elif name == CommandName.STOP:
            await self.stop()
        elif name == CommandName.PLAY_STOP:
            await self.toggle_play()
        elif name == CommandName.RESTART:
            await self.restart()
        elif name == CommandName.BPM_UP:
            self.state.set_bpm(self.state.bpm + (value or 1))
        elif name == CommandName.BPM_DOWN:
            self.state.set_bpm(self.state.bpm - (value or 1))
        elif name == CommandName.STEP_DIVISION_UP:
            self._cycle_step_division(1)
        elif name == CommandName.STEP_DIVISION_DOWN:
            self._cycle_step_division(-1)
        elif name == CommandName.SET_BPM and value is not None:
            self.state.set_bpm(float(value))
        elif name == CommandName.SET_GATE and value is not None:
            self.state.set_default_gate(float(value))
        elif name == CommandName.OCTAVE_UP:
            self.state.octave = min(8, self.state.octave + 1)
        elif name == CommandName.OCTAVE_DOWN:
            self.state.octave = max(-8, self.state.octave - 1)
        elif name == CommandName.SET_OCTAVE and value is not None:
            self.state.octave = int(value)
        elif name == CommandName.TRANSPOSE_UP:
            self.state.transpose = min(127, self.state.transpose + int(value or 1))
        elif name == CommandName.TRANSPOSE_DOWN:
            self.state.transpose = max(-127, self.state.transpose - int(value or 1))
        elif name == CommandName.ARM_ROOT_CAPTURE:
            self.state.root_capture_armed = True
            self.state.status_message = "Root selection armed: play the desired root MIDI note."
        elif name == CommandName.SET_TRANSPOSE and value is not None:
            self.state.transpose = int(value)

    def set_recording(self, enabled: bool) -> None:
        """Toggle non-temporal step recording; this never changes transport state."""
        self.state.recording = enabled
        self.state.status_message = (
            "RECORD ON: each physical Note On adds a step."
            if enabled
            else ("RECORD OFF: audition notes, then COMMIT the ones you want.")
        )

    def _cycle_step_division(self, direction: int) -> None:
        current = PLAYBACK_DIVISIONS.index(self.state.step_division)
        division = PLAYBACK_DIVISIONS[(current + direction) % len(PLAYBACK_DIVISIONS)]
        self.state.set_step_division(division)
        self.state.status_message = f"Step division set to 1/{division}."

    def arm_midi_learn(self, command: CommandName) -> bool:
        if self.mapping is None:
            self.state.status_message = "MIDI Learn needs --mapping minilab3."
            return False
        self.midi_learn_target = command
        self.state.status_message = f"MIDI Learn {command.name}: send a CC or pad now."
        return True

    def handle_main_encoder(self, message: mido.Message) -> bool:
        """Handle MiniLab's main encoder menu; return whether the event was consumed."""
        if message.type != "control_change":
            return False
        if message.control in _MAIN_ENCODER_SHIFT_PRESSES and message.value > 0:
            self.menu_open = False
            self.menu_editing = False
            self.state.status_message = "Menu closed."
            return True
        if message.control in _MAIN_ENCODER_PRESSES and message.value > 0:
            if not self.menu_open:
                self.menu_open = True
            elif self.menu_editing:
                self.menu_editing = False
            else:
                self.menu_editing = True
            self._set_menu_status()
            return True
        if message.control not in _MAIN_ENCODER_TURNS:
            return False
        delta = _relative_delta(message.value)
        if delta == 0 or not self.menu_open:
            return self.menu_open
        if self.menu_editing:
            self._adjust_menu_value(delta)
        else:
            self.menu_index = (self.menu_index + delta) % len(self.menu_items)
        self._set_menu_status()
        return True

    def menu_screen(self) -> tuple[str, str] | None:
        """Return the current main-encoder menu frame for the MiniLab OLED."""
        if not self.menu_open:
            return None
        mode = "EDIT" if self.menu_editing else "MENU"
        return f"{mode} > {self.menu_items[self.menu_index]}", self._menu_value()

    def _adjust_menu_value(self, delta: int) -> None:
        item = self.menu_items[self.menu_index]
        if item == "BPM":
            self.state.set_bpm(self.state.bpm + delta)
        elif item == "GATE":
            self.state.set_default_gate(self.state.default_gate + 0.05 * delta)
        elif item == "DIVISION":
            self._cycle_step_division(delta)
        elif item == "TRANSPOSE":
            self.state.transpose = min(127, max(-127, self.state.transpose + delta))
        elif item == "LOOP LENGTH":
            current = LOOP_LENGTHS.index(self.state.loop_length)
            self.state.set_loop_length(LOOP_LENGTHS[(current + delta) % len(LOOP_LENGTHS)])
        elif item == "CONTROL PAGE":
            self.control_page_index = (self.control_page_index + delta) % len(self.control_pages)

    def _menu_value(self) -> str:
        item = self.menu_items[self.menu_index]
        if item == "BPM":
            return f"{self.state.bpm:g} BPM"
        if item == "GATE":
            return f"{self.state.default_gate:.2f}"
        if item == "DIVISION":
            return f"1/{self.state.step_division}"
        if item == "TRANSPOSE":
            return f"{self.state.transpose:+d} st"
        if item == "LOOP LENGTH":
            return "FULL" if self.state.loop_length is None else f"{self.state.loop_length} STEPS"
        return self.control_pages[self.control_page_index]

    def _set_menu_status(self) -> None:
        mode = "editing" if self.menu_editing else "selected"
        item = self.menu_items[self.menu_index]
        self.state.status_message = f"Menu {mode}: {item} = {self._menu_value()}"

    def play(self) -> None:
        if self.sequencer is None:
            self.state.status_message = "Choose a MIDI output before playing."
            return
        if not self.state.sequence:
            self.state.status_message = "Sequence is empty."
            return
        self.sequencer.start(restart=True)
        self.state.status_message = "Playing from step 1."

    async def stop(self) -> None:
        if self.sequencer is not None:
            await self.sequencer.stop()
        if self.output is not None:
            self.output.all_notes_off()
        self.state.held_notes.clear()
        self.state.status_message = "Stopped; sent All Notes Off."

    async def toggle_play(self) -> None:
        if self.state.playing:
            await self.stop()
        else:
            self.play()

    async def restart(self) -> None:
        if self.sequencer is None or not self.state.sequence:
            self.play()
            return
        await self.sequencer.restart()
        self.state.status_message = "Restarted from step 1."

    async def shutdown(self) -> None:
        await self.stop()

    async def save(self, path: Path) -> None:
        self.state.save(path)

    async def load(self, path: Path) -> None:
        was_playing = self.state.playing
        if was_playing:
            await self.stop()
        self.state.load(path)

    def handle_midi_message(self, message: mido.Message) -> None:
        """Called on the asyncio event loop, never directly from the mido callback."""
        if self.midi_learn_target is not None and message.type in ("control_change", "note_on"):
            if self.mapping is not None:
                command = self.midi_learn_target
                if message.type == "note_on":
                    if message.velocity == 0:
                        return
                    if command in ABSOLUTE_CC_RANGES:
                        self.state.status_message = f"{command.name} needs a continuous CC control."
                        return
                    self.mapping.learn_note(message.channel, message.note, command)
                    source = f"note {message.note} on channel {message.channel + 1}"
                else:
                    self.mapping.learn_cc(message.control, command)
                    source = f"CC {message.control}"
                if self.mapping_path is not None:
                    self.mapping.save(self.mapping_path)
                self.state.status_message = f"Learned {source} → {command.name}; mapping saved."
            self.midi_learn_target = None
            return
        if self.mapping is not None:
            command = self.mapping.command_for(message)
            if command is not None:
                # A controller record pad must take effect before a following key
                # note can arrive; do not defer this state-only command to a task.
                if command.name == CommandName.RECORD:
                    self.set_recording(True if command.value is None else bool(command.value))
                elif command.name == CommandName.RECORD_TOGGLE:
                    self.set_recording(not self.state.recording)
                else:
                    # Scheduling preserves ordering while keeping mido callback handling sync.
                    import asyncio

                    asyncio.create_task(self.execute(command))
                return

        channel = self.state.output_channel - 1
        if message.type == "note_on" and message.velocity > 0:
            self.state.note_held(message.note, message.velocity)
            if self.output is not None:
                self.output.note_on(message.note, message.velocity, channel, owner="input")
            if self.state.root_capture_armed:
                self.state.root_capture_armed = False
                self.state.set_root_from_note(message.note)
            elif self.state.recording:
                # This handler only receives selected MIDI *input* port messages.
                # Sequencer output bypasses it, so generated playback cannot record itself.
                self.state.commit_last_note()
        elif message.type in ("note_off", "note_on"):
            self.state.note_released(message.note)
            if self.output is not None:
                self.output.note_off(message.note, channel, owner="input")
        elif self.output is not None:
            # Sustain, modulation, pitch bend, and other channel messages pass through.
            self.output.send_raw(message, channel)
