from __future__ import annotations

import asyncio
import json
import shutil
import tempfile
from pathlib import Path

import mido

from ministep import remote
from ministep.runtime import MiniStepRuntime
from ministep.state import AppState


def make_runtime() -> MiniStepRuntime:
    return MiniStepRuntime(AppState(), output=None)


async def test_snapshot_shape() -> None:
    runtime = make_runtime()
    runtime.handle_midi_message(mido.Message("note_on", note=63, velocity=100))
    runtime.state.commit_last_note()
    snap = remote.snapshot(runtime)
    assert snap["type"] == "state"
    assert snap["bpm"] == 128.0 and snap["division"] == 16 and snap["loop"] is None
    assert snap["length"] == 1 and snap["cursor_note"] == "D#3" and snap["last"] == "D#3"
    assert snap["playing"] is False and snap["recording"] is False
    json.dumps(snap)


async def test_apply_controller_commands_and_extras(tmp_path: Path) -> None:
    runtime = make_runtime()
    seq = tmp_path / "seq.json"
    assert await remote.apply(runtime, {"cmd": "BPM_UP", "value": 5}, seq) is None
    assert runtime.state.bpm == 133.0
    assert await remote.apply(runtime, {"cmd": "RECORD_TOGGLE"}, seq) is None
    assert runtime.state.recording is True
    assert await remote.apply(runtime, {"cmd": "REST"}, seq) is None
    assert await remote.apply(runtime, {"cmd": "REST"}, seq) is None
    assert await remote.apply(runtime, {"cmd": "CURSOR", "value": -1}, seq) is None
    assert runtime.state.cursor == 0
    assert await remote.apply(runtime, {"cmd": "DELETE"}, seq) is None
    assert len(runtime.state.sequence) == 1
    assert await remote.apply(runtime, {"cmd": "LOOP_CYCLE", "value": 1}, seq) is None
    assert runtime.state.loop_length == 8
    assert await remote.apply(runtime, {"cmd": "SAVE"}, seq) is None
    assert seq.exists()
    assert await remote.apply(runtime, {"cmd": "CLEAR"}, seq) is None
    assert await remote.apply(runtime, {"cmd": "LOAD"}, seq) is None
    assert len(runtime.state.sequence) == 1


async def test_apply_rejects_bad_messages(tmp_path: Path) -> None:
    runtime = make_runtime()
    assert "missing" in (await remote.apply(runtime, {}, tmp_path / "x.json") or "")
    assert "unknown" in (await remote.apply(runtime, {"cmd": "DANCE"}, tmp_path / "x.json") or "")
    assert "load failed" in (
        await remote.apply(runtime, {"cmd": "LOAD"}, tmp_path / "none.json") or ""
    )


async def read_json(reader: asyncio.StreamReader) -> dict:
    return json.loads(await asyncio.wait_for(reader.readline(), 2.0))


async def test_server_round_trip() -> None:
    runtime = make_runtime()
    # macOS caps AF_UNIX paths at 104 bytes; pytest's tmp_path is longer.
    short_dir = Path(tempfile.mkdtemp(prefix="ms", dir="/tmp"))
    sock = short_dir / "control.sock"
    server = remote.RemoteServer(runtime, sock, short_dir / "seq.json")
    await server.start()
    try:
        reader, writer = await asyncio.open_unix_connection(str(sock))
        first = await read_json(reader)
        assert first["type"] == "state" and first["length"] == 0

        writer.write(b'{"cmd":"REST"}\n')
        await writer.drain()
        state = await read_json(reader)
        assert state["length"] == 1

        writer.write(b"not json\n")
        await writer.drain()
        err = await read_json(reader)
        assert err["type"] == "error"

        # A change made elsewhere (TUI, MIDI) is pushed by the poll loop.
        runtime.state.set_bpm(90)
        pushed = await read_json(reader)
        while pushed.get("bpm") != 90.0:
            pushed = await read_json(reader)
        assert pushed["bpm"] == 90.0

        writer.close()
        await writer.wait_closed()
        await asyncio.sleep(0.1)
        assert server.client_count == 0
    finally:
        await server.close()
        shutil.rmtree(short_dir, ignore_errors=True)
    assert not sock.exists()
