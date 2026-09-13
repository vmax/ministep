"""Remote control over a Unix domain socket, newline-delimited JSON.

Any local client (the MacroPad daemon, a script, `nc -U`) connects to
``~/.config/ministep/control.sock`` and receives a full state snapshot on
connect, then a fresh snapshot whenever something changes (polled on the
event loop at 20 Hz, the same cadence the TUI uses). Commands are one
object per line:

    {"cmd": "PLAY_STOP"}
    {"cmd": "BPM_UP", "value": 5}
    {"cmd": "CURSOR", "value": -1}
    {"cmd": "SAVE"}

``cmd`` is any :class:`~ministep.controller.CommandName` member name, or
one of the editor extras below. Unknown or malformed lines get an
``{"type": "error"}`` reply and are otherwise ignored. Everything runs on
the asyncio loop, so state mutations are ordered with MIDI and TUI input.
"""

from __future__ import annotations

import asyncio
import contextlib
import json
import os
from pathlib import Path
from typing import Any

from .controller import Command, CommandName
from .runtime import MiniStepRuntime
from .state import LOOP_LENGTHS, note_name

DEFAULT_SOCKET_PATH = Path.home() / ".config" / "ministep" / "control.sock"
POLL_INTERVAL_S = 0.05
MAX_LINE = 4096

# Editor operations that are not controller commands.
EXTRA_COMMANDS = ("SAVE", "LOAD", "CURSOR", "CURSOR_NOTE", "REPLACE", "DELETE", "LOOP_CYCLE")


def snapshot(runtime: MiniStepRuntime) -> dict[str, Any]:
    """Serialisable view of everything a remote display might show."""
    state = runtime.state
    cursor_note: str | None = None
    if state.sequence:
        step = state.sequence[min(state.cursor, len(state.sequence) - 1)]
        cursor_note = "--" if step.note is None else note_name(step.note)
    last = None if state.last_note_played is None else note_name(state.last_note_played[0])
    return {
        "type": "state",
        "bpm": state.bpm,
        "division": state.step_division,
        "loop": state.loop_length,
        "playing": state.playing,
        "recording": state.recording,
        "playhead": state.playhead,
        "length": len(state.sequence),
        "cursor": state.cursor,
        "cursor_note": cursor_note,
        "last": last,
        "transpose": state.transpose,
        "status": state.status_message,
        "input": state.selected_input,
        "output": state.selected_output,
    }


def _int(value: Any, default: int) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


async def apply(
    runtime: MiniStepRuntime, message: dict[str, Any], sequence_path: Path
) -> str | None:
    """Apply one command message. Returns an error string or None."""
    cmd = message.get("cmd")
    if not isinstance(cmd, str):
        return "missing cmd"
    value = message.get("value")
    state = runtime.state
    if cmd in CommandName.__members__:
        await runtime.execute(Command(CommandName[cmd], value))
        return None
    if cmd == "SAVE":
        try:
            await runtime.save(sequence_path)
        except OSError as error:
            return f"save failed: {error}"
        return None
    if cmd == "LOAD":
        try:
            await runtime.load(sequence_path)
        except (OSError, ValueError) as error:
            return f"load failed: {error}"
        return None
    if cmd == "CURSOR":
        state.move_cursor(_int(value, 1))
        return None
    if cmd == "CURSOR_NOTE":
        state.adjust_cursor_note(_int(value, 1))
        return None
    if cmd == "REPLACE":
        state.replace_cursor_with_last_note()
        return None
    if cmd == "DELETE":
        state.delete_cursor()
        return None
    if cmd == "LOOP_CYCLE":
        current = LOOP_LENGTHS.index(state.loop_length)
        state.set_loop_length(LOOP_LENGTHS[(current + _int(value, 1)) % len(LOOP_LENGTHS)])
        state.status_message = (
            "Loop length: FULL"
            if state.loop_length is None
            else f"Loop length: {state.loop_length}"
        )
        return None
    return f"unknown cmd {cmd!r}"


def encode(message: dict[str, Any]) -> bytes:
    return (json.dumps(message, separators=(",", ":")) + "\n").encode("utf-8")


class RemoteServer:
    def __init__(self, runtime: MiniStepRuntime, path: Path, sequence_path: Path) -> None:
        self.runtime = runtime
        self.path = path
        self.sequence_path = sequence_path
        self._server: asyncio.base_events.Server | None = None
        self._clients: set[asyncio.StreamWriter] = set()
        self._poll_task: asyncio.Task[None] | None = None
        self._last: dict[str, Any] | None = None

    async def start(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with contextlib.suppress(FileNotFoundError):
            self.path.unlink()
        self._server = await asyncio.start_unix_server(self._serve, path=str(self.path))
        os.chmod(self.path, 0o600)
        self._poll_task = asyncio.create_task(self._poll())

    async def close(self) -> None:
        if self._poll_task is not None:
            self._poll_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await self._poll_task
        for writer in list(self._clients):
            writer.close()
        self._clients.clear()
        if self._server is not None:
            self._server.close()
            await self._server.wait_closed()
        with contextlib.suppress(FileNotFoundError):
            self.path.unlink()

    @property
    def client_count(self) -> int:
        return len(self._clients)

    async def _serve(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        self._clients.add(writer)
        try:
            writer.write(encode(snapshot(self.runtime)))
            await writer.drain()
            while True:
                try:
                    line = await reader.readline()
                except (asyncio.LimitOverrunError, ValueError):
                    writer.write(encode({"type": "error", "msg": "line too long"}))
                    continue
                if not line:
                    break
                if len(line) > MAX_LINE:
                    writer.write(encode({"type": "error", "msg": "line too long"}))
                    continue
                try:
                    message = json.loads(line)
                except ValueError:
                    writer.write(encode({"type": "error", "msg": "bad json"}))
                    continue
                if not isinstance(message, dict):
                    writer.write(encode({"type": "error", "msg": "expected object"}))
                    continue
                error = await apply(self.runtime, message, self.sequence_path)
                if error is not None:
                    writer.write(encode({"type": "error", "msg": error}))
                await self._broadcast(force=True)
        except (ConnectionError, OSError):
            pass
        finally:
            self._clients.discard(writer)
            writer.close()

    async def _poll(self) -> None:
        while True:
            await asyncio.sleep(POLL_INTERVAL_S)
            await self._broadcast()

    async def _broadcast(self, force: bool = False) -> None:
        current = snapshot(self.runtime)
        if not force and current == self._last:
            return
        self._last = current
        data = encode(current)
        for writer in list(self._clients):
            try:
                writer.write(data)
                await writer.drain()
            except (ConnectionError, OSError):
                self._clients.discard(writer)
                writer.close()
