"""Transport timing benchmark for MiniStep.

Modes
-----
realtime   Run the real ``Sequencer`` on the real asyncio loop for a fixed duration
           at each tempo. Optional real MIDI output, headless Textual UI, and the
           MiniLab display refresh loop, so UI-induced jitter can be measured.
simulate   Run the same ``Sequencer`` and clock algorithm against a fake clock:
           deterministic, instant, seeded. Shows the algorithmic drift independent
           of the runtime.
loopback   Measure IAC MIDI delivery latency (send CC -> receive on same bus).

Examples
--------
  .venv/bin/python tools/timing_bench.py simulate --clock legacy
  .venv/bin/python tools/timing_bench.py simulate --clock current
  .venv/bin/python tools/timing_bench.py realtime --seconds 30 --bpm 180
  .venv/bin/python tools/timing_bench.py realtime --seconds 30 --bpm 180 --ui textual
  .venv/bin/python tools/timing_bench.py realtime --seconds 30 --bpm 180 --midi MiniStep
  .venv/bin/python tools/timing_bench.py loopback --port MiniStep
"""

from __future__ import annotations

import argparse
import asyncio
import contextlib
import functools
import gc
import random
import statistics
import sys
import time
from collections.abc import Awaitable, Callable
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from ministep.clock import InternalClock, StepWindow, step_duration_seconds  # noqa: E402
from ministep.sequencer import Sequencer  # noqa: E402
from ministep.state import AppState, Step  # noqa: E402
from ministep.timing import TimingStats  # noqa: E402

DEFAULT_BPMS = (120, 150, 180, 200, 240)


class LegacyClock:
    """The original MiniStep clock (initial commit) kept for before/after comparison.

    Bug under test: ``max(now, next_start)`` re-anchors the grid to the actual
    wake time whenever the loop wakes late, so every sleep overshoot is added to
    the transport period permanently.
    """

    def __init__(
        self,
        now: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], Awaitable[None]] = asyncio.sleep,
    ) -> None:
        self._now = now
        self._sleep = sleep
        self._next_start: float | None = None

    def reset(self) -> None:
        self._next_start = None

    async def next_step(self, bpm: float, step_division: int) -> StepWindow:
        now = self._now()
        start = now if self._next_start is None else max(now, self._next_start)
        duration = step_duration_seconds(bpm, step_division)
        self._next_start = start + duration
        return StepWindow(start=start, duration=duration)

    async def wait_until(self, deadline: float) -> None:
        remaining = deadline - self._now()
        if remaining > 0:
            await self._sleep(remaining)


CLOCKS = {"legacy": LegacyClock, "current": InternalClock}


class NullSink:
    def __init__(self, work: Callable[[], None] | None = None) -> None:
        self.events = 0
        self._work = work

    def note_on(self, note: int, velocity: int, channel: int = 0, owner: str = "sequencer") -> None:
        self.events += 1
        if self._work:
            self._work()

    def note_off(self, note: int, channel: int = 0, owner: str = "sequencer") -> None:
        self.events += 1
        if self._work:
            self._work()

    def control_change(self, control: int, value: int, channel: int = 0) -> None:
        pass

    def all_notes_off(self) -> None:
        pass


def make_state(bpm: float, division: int, steps: int = 16, gate: float = 0.5) -> AppState:
    state = AppState(bpm=bpm, step_division=division, default_gate=gate)
    for index in range(steps):
        note = 48 + (index * 5) % 24
        state.sequence.append(Step(note if index % 4 != 3 else None, gate=gate))
    return state


# --------------------------------------------------------------------------- simulate
class VirtualTime:
    """Fake monotonic clock. ``sleep`` advances time by the request plus modelled latency."""

    def __init__(self, seed: int, overshoot_ms: float, ui_ms: float, ui_hz: float) -> None:
        self.t = 1000.0
        self.rng = random.Random(seed)
        self.overshoot = overshoot_ms / 1000.0
        self.ui_cost = ui_ms / 1000.0
        self.ui_period = 1.0 / ui_hz if ui_hz > 0 else None
        self.next_ui = self.t

    def now(self) -> float:
        return self.t

    async def sleep(self, seconds: float) -> None:
        wake = self.t + max(0.0, seconds)
        # Timer overshoot: kqueue/asyncio wakes late by a roughly constant amount plus noise.
        wake += self.overshoot * self.rng.uniform(0.8, 1.4)
        # A periodic UI task blocking the single-threaded loop delays the wake-up if it
        # was running when the timer expired.
        if self.ui_period is not None:
            while self.next_ui <= wake:
                ui_end = self.next_ui + self.ui_cost
                if ui_end > wake:
                    wake = ui_end
                self.next_ui += self.ui_period
        self.t = wake
        await asyncio.sleep(0)

    def work(self, seconds: float) -> None:
        self.t += seconds


async def run_simulation(args: argparse.Namespace) -> None:
    for bpm in args.bpm:
        vt = VirtualTime(args.seed, args.overshoot_ms, args.ui_ms, args.ui_hz)
        clock = CLOCKS[args.clock](now=vt.now, sleep=vt.sleep)
        stats = TimingStats(now=vt.now)
        state = make_state(bpm, args.division)
        work_seconds = args.work_ms / 1000.0
        sink = NullSink(work=functools.partial(vt.work, work_seconds))
        sequencer = Sequencer(state, sink, clock, stats=stats)
        state.playing = True
        end = vt.t + args.seconds
        while vt.t < end:
            await sequencer.advance_one()
        state.playing = False
        summary = stats.summary()
        assert summary is not None
        print(summary.format(f"simulate {args.clock} {bpm} BPM 1/{args.division}"))


# --------------------------------------------------------------------------- realtime
class GcTrace:
    """Record garbage-collection pause durations per generation via gc.callbacks."""

    def __init__(self) -> None:
        self.pauses: dict[int, list[float]] = {0: [], 1: [], 2: []}
        self._start = 0.0

    def __call__(self, phase: str, info: dict[str, int]) -> None:
        if phase == "start":
            self._start = time.perf_counter()
        else:
            self.pauses[info["generation"]].append(time.perf_counter() - self._start)

    def report(self) -> str:
        parts = []
        for generation, pauses in self.pauses.items():
            if pauses:
                parts.append(
                    f"gen{generation}: n={len(pauses)} max={max(pauses) * 1000:.3f}ms "
                    f"total={sum(pauses) * 1000:.1f}ms"
                )
        return "  gc pauses   " + (" | ".join(parts) if parts else "none")


async def run_realtime(args: argparse.Namespace) -> None:
    output = None
    port = None
    if args.midi:
        import mido

        from ministep.midi import MidoOutput, OwnedMidiOutput

        port = mido.open_output(args.midi)
        output = OwnedMidiOutput(MidoOutput(port))
    if args.gc_tune:
        from ministep.main import tune_gc_for_playback

        tune_gc_for_playback()
    trace = None
    if args.gc_trace:
        trace = GcTrace()
        gc.callbacks.append(trace)
    try:
        for bpm in args.bpm:
            await _realtime_once(args, bpm, output)
            if trace is not None:
                print(trace.report(), flush=True)
                trace = GcTrace()
                gc.callbacks[:] = [trace]
    finally:
        if output is not None:
            output.close()
        if args.gc_trace:
            gc.callbacks.clear()


async def _realtime_once(args: argparse.Namespace, bpm: float, output) -> None:  # type: ignore[no-untyped-def]
    state = make_state(bpm, args.division)
    sink = output if output is not None else NullSink()
    clock = CLOCKS[args.clock]()
    stats = TimingStats()
    sequencer = Sequencer(state, sink, clock, stats=stats)
    label = (
        f"realtime {args.clock} {bpm} BPM 1/{args.division} ui={args.ui} midi={args.midi or 'none'}"
    )
    if args.display:
        label += " display"

    display_task = None
    display = None
    if args.display:
        from ministep.minilab3_display import MiniLab3Display

        display = MiniLab3Display.open()
        display.connect()
        display.show_text("MINISTEP", "BENCH")

        async def refresh_display() -> None:
            last = None
            while True:
                display.refresh()
                status = (False, state.playing, state.playhead, state.cursor)
                if status != last:
                    display.show_step_status(
                        recording=False, playing=status[1], playhead=status[2], cursor=status[3]
                    )
                    last = status
                await asyncio.sleep(0.1)

        display_task = asyncio.create_task(refresh_display())

    async def play() -> None:
        sequencer.start(restart=True)
        await asyncio.sleep(args.seconds)
        await sequencer.stop()

    try:
        if args.ui == "textual":
            from ministep.runtime import MiniStepRuntime
            from ministep.tui import MiniStepApp

            runtime = MiniStepRuntime(state, output=None)
            runtime.sequencer = sequencer
            app = MiniStepApp(runtime, Path("/dev/null"))
            async with app.run_test(size=(120, 40)) as pilot:
                await pilot.pause()
                await play()
        else:
            await play()
    finally:
        if display_task is not None:
            display_task.cancel()
            with contextlib.suppress(asyncio.CancelledError):
                await display_task
        if display is not None:
            display.close()
    summary = stats.summary()
    assert summary is not None
    print(summary.format(label), flush=True)


# --------------------------------------------------------------------------- loopback
def run_loopback(args: argparse.Namespace) -> None:
    import threading

    import mido

    received: list[float] = []
    event = threading.Event()

    def on_message(message: mido.Message) -> None:
        received.append(time.perf_counter())
        event.set()

    inp = mido.open_input(args.port, callback=on_message)
    out = mido.open_output(args.port)
    latencies = []
    try:
        for _ in range(args.count):
            event.clear()
            sent = time.perf_counter()
            # CC 119 is undefined; harmless for anything listening on the bus.
            out.send(mido.Message("control_change", control=119, value=0, channel=15))
            if not event.wait(1.0):
                print("loopback: timeout, is the IAC bus online?")
                return
            latencies.append((received[-1] - sent) * 1000)
            time.sleep(0.02)
    finally:
        inp.close()
        out.close()
    latencies.sort()
    n = len(latencies)
    print(
        f"loopback {args.port}: n={n} send->receive ms p50={latencies[n // 2]:.3f} "
        f"p95={latencies[int(n * 0.95)]:.3f} max={latencies[-1]:.3f} "
        f"mean={statistics.mean(latencies):.3f}"
    )
    send_costs = []
    out = mido.open_output(args.port)
    try:
        for _ in range(args.count):
            t0 = time.perf_counter()
            out.send(mido.Message("control_change", control=119, value=0, channel=15))
            send_costs.append((time.perf_counter() - t0) * 1000)
    finally:
        out.close()
    send_costs.sort()
    print(
        f"port.send() cost ms p50={send_costs[n // 2]:.4f} p95={send_costs[int(n * 0.95)]:.4f} "
        f"max={send_costs[-1]:.4f}"
    )


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = parser.add_subparsers(dest="mode", required=True)

    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--bpm", type=float, nargs="+", default=DEFAULT_BPMS)
    common.add_argument("--division", type=int, default=16)
    common.add_argument("--seconds", type=float, default=60.0)
    common.add_argument("--clock", choices=CLOCKS, default="current")

    sim = sub.add_parser("simulate", parents=[common])
    sim.add_argument("--seed", type=int, default=1)
    sim.add_argument("--overshoot-ms", type=float, default=1.1, help="timer wake overshoot")
    sim.add_argument("--work-ms", type=float, default=0.05, help="cost per MIDI send")
    sim.add_argument("--ui-ms", type=float, default=0.0, help="blocking UI tick cost")
    sim.add_argument("--ui-hz", type=float, default=20.0)

    rt = sub.add_parser("realtime", parents=[common])
    rt.add_argument("--ui", choices=("none", "textual"), default="none")
    rt.add_argument("--midi", help="real MIDI output port name (e.g. MiniStep)")
    rt.add_argument("--display", action="store_true", help="run MiniLab display refresh loop")
    rt.add_argument("--gc-trace", action="store_true", help="report GC pause durations")
    rt.add_argument("--gc-tune", action="store_true", help="apply the app's GC tuning")

    lb = sub.add_parser("loopback")
    lb.add_argument("--port", default="MiniStep")
    lb.add_argument("--count", type=int, default=200)

    args = parser.parse_args()
    if args.mode == "simulate":
        asyncio.run(run_simulation(args))
    elif args.mode == "realtime":
        asyncio.run(run_realtime(args))
    else:
        run_loopback(args)


if __name__ == "__main__":
    main()
