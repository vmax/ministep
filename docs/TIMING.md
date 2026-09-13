# MiniStep transport timing model

This note describes how playback timing works, what was wrong before, and how
to measure it. Read it before touching `ministep/clock.py` or
`Sequencer.advance_one`.

## Grid, not stopwatch

Playback runs on an **anchored ideal grid**. `InternalClock` keeps one number,
`_next_start`: the scheduled start of the next step. Every call to
`next_step()` hands out that scheduled start and then advances it by exactly
one step duration:

```text
step_duration = (60 / bpm) * (4 / step_division)
start_n       = start_{n-1} + step_duration      # never "now + step_duration"
```

The actual wake-up time is **never** written back into the grid. If the loop
wakes 1.2 ms late, the step plays 1.2 ms late, and the *next* wait is 1.2 ms
shorter. Lateness is therefore a per-step offset, not a permanent tempo error.

Within a step the sequencer waits on absolute deadlines derived from the same
grid: note-off at `start + duration * gate`, then `start + duration`. Tied steps
skip the note-off. Rests only wait for the end.

At 180 BPM the deadlines are:

| unit          | duration   |
| ------------- | ---------- |
| quarter (1/4) | 333.333 ms |
| eighth (1/8)  | 166.667 ms |
| 16th (1/16)   | 83.333 ms  |
| 32nd (1/32)   | 41.667 ms  |

There is no PPQN tick; the grid is per step at the selected division.

## Tempo and division changes

`state.bpm` and `state.step_division` are read at every `next_step()`. A change
takes effect at the next step boundary: the new duration is added to the
current boundary, so the grid re-anchors from a musically meaningful point
without any jump.

## Late policy

`next_step()` compares `now` with the scheduled start:

* **late by less than one step** – play immediately. `StepWindow.start` stays
  on the grid, so gate and next deadline stay on the grid. `StepWindow.late`
  carries the lateness for diagnostics. A step later than its gate deadline
  produces a near-zero-length note; this is only reachable after a stall of
  tens of milliseconds and is preferable to moving the grid.
* **late by one step or more** (machine suspended, giant GC pause, terminal
  frozen) – the missed grid slots are **dropped**, not burst. `StepWindow.skipped`
  reports how many; the sequencer advances the playhead by that count so the
  loop position stays in phase with wall-clock time. Bursting would fire the
  missed notes on top of each other; shifting the grid would permanently
  desynchronise the sequencer from anything else running to the same tempo.

Trade-off: a dropped slot is an audible gap, but the pattern lands back on the
beat immediately and stays there. A musician following a DAW or drum machine
hears a hiccup, not a slow tempo. A mode that shifts the grid would sound
smooth in isolation and wrong against everything else.

## Where the time goes (measured, Apple silicon macOS, Python 3.12)

| source                                        | cost                        |
| --------------------------------------------- | --------------------------- |
| `asyncio.sleep()` wake overshoot (kqueue)     | ~1.1 ms p50, ~1.4 ms p95    |
| `mido` `port.send()` to IAC                   | ~0.01 ms                    |
| IAC delivery send→receive                     | ~0.26 ms p50, 0.6 ms max    |
| step handler (note selection + note-on)       | ~0.01 ms                    |

The wake overshoot is the dominant component and is a property of the runtime
timer, not of MiniStep. It shows up as a constant ~1 ms latency on every step,
which is musically irrelevant *as long as it does not accumulate*.

## The bug that was fixed

The original clock did:

```python
start = now if self._next_start is None else max(now, self._next_start)
self._next_start = start + duration
```

`max(now, …)` rewrote the grid to the wake time whenever the loop woke late,
which on this runtime is *every* step. Each step's period became
`duration + overshoot`. Measured: +1.9 ms per step at every tempo, i.e. the
sequencer ran ~1.5 % slow at 120 BPM and ~2.3 % slow at 180 BPM 1/16
(~0.9 s of drift per minute, 10 of 720 steps lost per minute). Faster tempos
have more steps per minute, so the drift per minute grows with tempo; that is
why 180 BPM felt unstable while 120 BPM was tolerable.

Repeated floating-point addition of the step duration is **not** a meaningful
contributor: one hour of 1/32 steps at 240 BPM accumulates well under a
microsecond of error (see `tests/test_timing.py`). It was not "fixed".

## Before / after (real time, this machine, 1/16 steps, 30 s runs)

| tempo | old drift after 30 s | old period error | new drift after 30 s | new period error |
| ----- | -------------------- | ---------------- | -------------------- | ---------------- |
| 120   | +441 ms (237/240 steps) | +1.87 ms/step | +2.1 ms (240 steps) | +0.009 ms/step |
| 150   | +578 ms (295/300)    | +1.97 ms/step    | +2.2 ms (300)        | +0.007 ms/step   |
| 180   | +680 ms (352/360)    | +1.94 ms/step    | +2.2 ms (360)        | +0.006 ms/step   |
| 200   | +759 ms (392/400)    | +1.94 ms/step    | +2.2 ms (400)        | +0.006 ms/step   |
| 240   | +922 ms (466/480)    | +1.98 ms/step    | +2.2 ms (480)        | +0.005 ms/step   |

The "new drift" is the offset of the very first step and does not grow: a
3-minute run at 180 BPM with the TUI, the IAC MIDI port and the MiniLab display
all active emitted exactly 2160 steps with a final drift of +1.2 ms, no
skipped slots, lateness p99 1.6 ms and a worst case of 4.8 ms.

## UI and playback separation

The Textual TUI (`set_interval(0.05, refresh_view)`) and the MiniLab display
loop (10 Hz) observe `AppState` on their own cadence; they never drive the
transport. They do share the asyncio thread, so any render work delays a step
wake-up by however long it blocks. Measured at 180 BPM 1/16:

| configuration                              | lateness p95 | p99     | max      |
| ------------------------------------------ | ------------ | ------- | -------- |
| sequencer only                             | 2.3 ms       | 2.5 ms  | 5.3 ms   |
| + MiniLab display loop                     | 1.6 ms       | 2.3 ms  | 3.8 ms   |
| + Textual TUI, updating widgets every tick | 4.0 ms       | 6.5 ms  | 17.9 ms  |
| + Textual TUI, change-detected updates     | 1.3–1.6 ms   | 3.4–3.9 ms | 26–38 ms (single outlier per run) |
| + Textual TUI, change-detected + GC tuning | 1.3 ms       | 2.1 ms  | 5.3 ms   |

`refresh_view()` itself costs ~0.4 ms; the cost is Textual's layout/render pass
that every `Static.update` schedules. The TUI now only pushes text to a widget
when it changed (`MiniStepApp._update`), which removed the steady multi-ms
jitter.

The remaining tens-of-millisecond outliers were garbage collection: with the
TUI running, `gc.callbacks` tracing showed five full (gen2) collections per
minute pausing the loop for up to 15 ms each, and none at all without the TUI.
`tune_gc_for_playback()` in `main.py` freezes the start-up object graph and
raises the gen2 threshold; the same 60 s run then had zero gen2 collections
and a 5.3 ms worst case. Whatever outliers remain never move the grid, so the
following step lands back on time.

Keep anything expensive out of `Sequencer.advance_one` and out of `MidiSink`
implementations. If lower jitter is ever needed, the next step is to run the
sequencer on its own thread with its own event loop; measure first.

## Measuring

Run the diagnostics in the real app:

```bash
uv run ministep --input "Minilab3 MIDI" --output MiniStep --timing-log /tmp/ministep-timing.txt
```

Each time playback stops, a summary (lateness p50/p95/p99/max, handler time,
interval error, jitter, cumulative drift, skipped steps) is appended to the
file. Nothing is printed while playing.

Benchmarks and simulation:

```bash
.venv/bin/python tools/timing_bench.py simulate --clock legacy   # the old algorithm
.venv/bin/python tools/timing_bench.py simulate --clock current
.venv/bin/python tools/timing_bench.py realtime --seconds 60
.venv/bin/python tools/timing_bench.py realtime --bpm 180 --ui textual --midi MiniStep
.venv/bin/python tools/timing_bench.py loopback --port MiniStep
```

Regression tests live in `tests/test_timing.py` and run the real clock and
sequencer against a fake clock with modelled wake overshoot, so they are
deterministic and fast.
