"""CLI entry point and MIDI-port startup selection."""

from __future__ import annotations

import argparse
import asyncio
import sys
from contextlib import suppress
from pathlib import Path

import mido

from .controller import DEFAULT_MIDI_MAPPING_PATH, Command, CommandName, SimpleMapping
from .midi import MidiManager, MidoOutput, find_port, input_ports, output_ports
from .minilab3_display import MiniLab3Display
from .profiles import ProfileError, ProfileRouter, load_profile
from .runtime import MiniStepRuntime
from .state import AppState
from .tui import DEFAULT_SEQUENCE_PATH, MiniStepApp


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="MiniStep MIDI step recorder/sequencer")
    parser.add_argument("--input", help="MIDI input name (exact or unambiguous partial match)")
    parser.add_argument("--output", help="MIDI output name (exact or unambiguous partial match)")
    parser.add_argument(
        "--list-midi", action="store_true", help="List MIDI input and output ports, then exit"
    )
    parser.add_argument("--load", type=Path, help="Load a JSON sequence on startup")
    parser.add_argument(
        "--mapping",
        choices=("none", "minilab3"),
        default="none",
        help="Enable persisted MiniLab CC MIDI Learn mappings",
    )
    parser.add_argument(
        "--minilab-display",
        action="store_true",
        help="Show MiniStep knob values on a connected MiniLab 3 OLED display",
    )
    parser.add_argument("--profile", type=Path, help="YAML synth-control profile")
    parser.add_argument(
        "--control-output",
        help="MIDI output for profile CCs; overrides the profile's output field",
    )
    return parser


def print_ports(inputs: list[str], outputs: list[str]) -> None:
    print("MIDI inputs:")
    print(*(f"  {name}" for name in inputs), sep="\n") if inputs else print("  (none)")
    print("MIDI outputs:")
    print(*(f"  {name}" for name in outputs), sep="\n") if outputs else print("  (none)")


def choose_port(kind: str, requested: str | None, ports: list[str]) -> str | None:
    if requested is not None:
        match = find_port(requested, ports)
        if match is None:
            raise ValueError(f"No unambiguous {kind} port matches {requested!r}")
        return match
    if not ports:
        return None
    if len(ports) == 1 or not sys.stdin.isatty():
        return ports[0] if len(ports) == 1 else None
    print(f"Select MIDI {kind} (Enter for none):")
    for index, name in enumerate(ports, start=1):
        print(f"  {index}. {name}")
    answer = input("> ").strip()
    if not answer:
        return None
    try:
        return ports[int(answer) - 1]
    except (ValueError, IndexError) as error:
        raise ValueError(f"Invalid {kind} selection") from error


def display_command(command: Command) -> tuple[str, str | None]:
    """Return a concise OLED label and formatted value for a learned control."""
    labels = {
        CommandName.SET_BPM: "BPM",
        CommandName.SET_GATE: "GATE",
        CommandName.SET_TRANSPOSE: "TRANSPOSE",
        CommandName.SET_OCTAVE: "OCTAVE",
    }
    label = labels.get(command.name, command.name.name.replace("_", " "))
    if command.value is None:
        return label, None
    if command.name == CommandName.SET_GATE:
        return label, f"{float(command.value):.2f}"
    return label, str(command.value)


async def run(args: argparse.Namespace) -> int:
    inputs, outputs = input_ports(), output_ports()
    if args.list_midi:
        print_ports(inputs, outputs)
        return 0
    print_ports(inputs, outputs)
    try:
        selected_input = choose_port("input", args.input, inputs)
        selected_output = choose_port("output", args.output, outputs)
    except ValueError as error:
        print(f"ministep: {error}", file=sys.stderr)
        return 2

    state = AppState(selected_input=selected_input, selected_output=selected_output)
    if args.load is not None:
        try:
            state.load(args.load)
        except (OSError, ValueError) as error:
            print(f"ministep: cannot load {args.load}: {error}", file=sys.stderr)
            return 2

    loop = asyncio.get_running_loop()
    runtime: MiniStepRuntime | None = None
    display: MiniLab3Display | None = None
    display_task: asyncio.Task[None] | None = None
    profile_router: ProfileRouter | None = None
    profile_output: MidoOutput | None = None

    def receive(message) -> None:  # type: ignore[no-untyped-def]
        if display is not None and runtime is not None and runtime.handle_main_encoder(message):
            if profile_router is not None:
                profile_router.select_page(runtime.control_page_index)
            frame = runtime.menu_screen()
            display.show_text(*(frame or ("MINISTEP", "READY")))
            return
        if profile_router is not None:
            routed = profile_router.handle_message(message)
            if routed is not None:
                if display is not None:
                    display.show_encoder(f"{routed.page} {routed.label}", routed.value)
                return
            if profile_router.is_knob_message(message):
                return
        if display is not None:
            command = mapping.command_for(message) if mapping is not None else None
            label, value_text = display_command(command) if command is not None else (None, None)
            display.handle_message(message, label, value_text)
        if runtime is not None:
            runtime.handle_midi_message(message)

    manager = MidiManager(loop, receive)
    try:
        output = manager.open_output(selected_output) if selected_output else None
        mapping: SimpleMapping | None = None
        if args.mapping == "minilab3":
            try:
                mapping = SimpleMapping.load(DEFAULT_MIDI_MAPPING_PATH)
            except FileNotFoundError:
                mapping = SimpleMapping(note_commands={}, cc_commands={})
            except (OSError, ValueError, KeyError) as error:
                print(f"ministep: cannot load MIDI mapping: {error}", file=sys.stderr)
                return 2
        profile = None
        if args.profile is not None:
            try:
                profile = load_profile(args.profile)
            except ProfileError as error:
                print(f"ministep: cannot load control profile: {error}", file=sys.stderr)
                return 2
            requested_output = args.control_output or profile.output or selected_output
            selected_control_output = (
                find_port(requested_output, outputs) if requested_output is not None else None
            )
            if selected_control_output is None:
                print("ministep: profile needs a valid control MIDI output.", file=sys.stderr)
                return 2
            if selected_control_output == selected_output and output is not None:
                profile_router = ProfileRouter(profile, output)
            else:
                profile_output = MidoOutput(mido.open_output(selected_control_output))
                profile_router = ProfileRouter(profile, profile_output)
        runtime = MiniStepRuntime(
            state,
            output,
            mapping,
            DEFAULT_MIDI_MAPPING_PATH,
            control_pages=profile.page_names if profile is not None else (),
        )
        if selected_input:
            manager.open_input(selected_input)
        if args.minilab_display:
            try:
                display = MiniLab3Display.open()
                display.connect()
                display.show_text("MINISTEP", "READY")
            except (OSError, RuntimeError) as error:
                print(f"ministep: cannot open MiniLab display: {error}", file=sys.stderr)
                return 2

            async def refresh_display() -> None:
                last_step_status: tuple[bool, bool, int, int] | None = None
                while True:
                    display.refresh()
                    step_status = (
                        runtime.state.recording,
                        runtime.state.playing,
                        runtime.state.playhead,
                        runtime.state.cursor,
                    )
                    if step_status != last_step_status:
                        display.show_step_status(
                            recording=step_status[0],
                            playing=step_status[1],
                            playhead=step_status[2],
                            cursor=step_status[3],
                        )
                        last_step_status = step_status
                    await asyncio.sleep(0.1)

            display_task = asyncio.create_task(refresh_display())
        app = MiniStepApp(runtime, DEFAULT_SEQUENCE_PATH)
        await app.run_async()
    except KeyboardInterrupt:
        pass
    finally:
        if display_task is not None:
            display_task.cancel()
            with suppress(asyncio.CancelledError):
                await display_task
        if display is not None:
            display.close()
        if profile_output is not None:
            profile_output.close()
        if runtime is not None:
            await runtime.shutdown()
        manager.close()
    return 0


def main() -> None:
    args = build_parser().parse_args()
    raise SystemExit(asyncio.run(run(args)))


if __name__ == "__main__":
    main()
