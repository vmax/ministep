# MiniStep

A hardware-style MIDI step recorder: audition one note at a time, commit the notes you want, then play a perfectly clocked sequence.

```text
play C
COMMIT

play Eb
COMMIT

REST

play G
COMMIT

PLAY

        ↓

C | Eb | -- | G | ...
```

Take five seconds or five minutes to find the next note. MiniStep records steps, not the wall-clock timing between them.

It is a small, macOS-first MIDI controller/sequencer—not a DAW and not an audio processor. It works with standard MIDI input/output ports, and has a deeper Arturia MiniLab 3 integration for learned controls, OLED feedback, RGB pads, and synth-control pages.

```text
MIDI keyboard / MiniLab 3
             │
             ▼
         MiniStep
             │  MIDI only: audition pass-through + sequencer playback
             ▼
       IAC "MiniStep"
          ├──► Ableton Live ──► instrument plug-in
          └──► standalone MIDI synth
```

> A short demo belongs here: slowly enter notes, watch the MiniLab feedback and TUI update, then press PLAY.

## What it does

- **Audition, then commit.** Incoming notes are immediately forwarded to the selected MIDI output so you can hear them. `COMMIT` appends the last auditioned note as the next step, preserving its velocity.
- **REC step mode.** Toggle REC and each physical Note On commits one step automatically. Note Off events do not create steps; REC never captures performance timing or starts transport.
- **Sequencing tools.** Add rests, ties/holds, undo the final step, clear, restart, edit a selected step, transpose live, or set a new pitch-class root.
- **Clocked playback.** Internal monotonic timing on an anchored grid plays the sequence at a selectable BPM, 1/4, 1/8, 1/16, or 1/32 division, with a global gate. Late wake-ups never shift the transport; see `docs/TIMING.md`. Pass `--timing-log PATH` to append timing summaries whenever playback stops.
- **Non-destructive loop spans.** Choose `FULL → 8 → 16 → 32 → 64`; selecting `16`, for example, loops the first 16 stored steps without truncating later ones.
- **Persistence.** Load JSON at startup and save/load the default sequence from the TUI.
- **MIDI-safe pass-through.** Notes from live input and sequencer playback are ownership-tracked, so one source's Note Off does not cut off the other source's same-pitch note.

## Installation

Requires Python 3.12+, macOS MIDI services, and [uv](https://docs.astral.sh/uv/).

```bash
git clone git@github.com:vmax/ministep.git
cd ministep
uv sync

uv run ministep --help
uv run ministep --list-midi
```

Start with explicit port names (exact names or an unambiguous substring both work):

```bash
uv run ministep --input "Minilab3 MIDI" --output "MiniStep"
```

Without `--input` or `--output`, MiniStep lists available ports and, in a terminal, offers a picker. Use `--load melody.json` to open a saved sequence at startup.

## macOS IAC routing

MiniStep opens real MIDI ports; it does not create a virtual port itself. On macOS, use the built-in IAC Driver:

1. Open **Audio MIDI Setup**.
2. Choose **Window → Show MIDI Studio**.
3. Double-click **IAC Driver**.
4. Enable **Device is online**.
5. Create or enable a bus named `MiniStep`.

Choose that bus as MiniStep's `--output`. Restart a DAW or synth if it does not immediately expose the new bus.

### Ableton Live

```text
MiniLab 3 → MiniStep → IAC "MiniStep" → Ableton MIDI track → VST/AU synth
```

In Live's MIDI preferences, enable **Track** for the `MiniStep` IAC input. Put an instrument on a MIDI track, select `MiniStep` as its input, and set monitoring to `In` (or arm and monitor it). Do not also monitor the MiniLab directly on that track unless you want doubled audition notes.

### Miniverse

The included profile targets Cherry Audio Miniverse, but MiniStep itself is MIDI-based and is not tied to it. Enable the `MiniStep` IAC bus as Miniverse's MIDI input, then use that same bus as MiniStep's output.

## Controls

The Textual TUI shows selected MIDI ports, tempo, division, gate, transpose, loop span, transport/REC state, auditioned/held notes, and a numbered step grid. MIDI note names follow Ableton's convention: MIDI 60 is `C3`.

| Computer key | Action |
| --- | --- |
| `Space` | Play / stop |
| `Enter` | Commit the last auditioned note |
| `M` | Toggle REC auto-commit |
| `R` | Append a rest |
| `H` | Add a tied one-step hold to the final pitched step |
| `U` or `Backspace` | Undo the final step |
| `C` | Clear the sequence |
| `Home` | Restart from step 1 |
| `Left` / `Right` | Move the selected step cursor |
| `Up` / `Down` | Raise / lower the selected pitched step one semitone |
| `Shift+Up` / `Shift+Down` | Raise / lower it one octave |
| `E` | Replace the selected step with the last auditioned note |
| `X` or `Delete` | Delete the selected step |
| `,` / `.` | BPM − / + |
| `PageUp` / `PageDown` | Next / previous step division |
| `[` / `]` | Global gate − / + 0.05 |
| `T`, then a MIDI key | Move the sequence root to that key's pitch class |
| `K` | MIDI Learn (with `--mapping minilab3`) |
| `S` / `L` | Save / load `~/.config/ministep/sequence.json` |
| `Shift+S` / `Shift+L` | Save as / load a named pattern in `~/.config/ministep/patterns/` (type a name, `Enter`; `Esc` cancels) |
| `Q` or `Ctrl+C` | Quit cleanly |

`T` is a destructive root edit: it shifts all pitched steps by the nearest pitch-class interval and clears any live transpose. `H` ties the preceding note into a copied full-length step. Changing the global gate also updates the gates of existing steps.

## Arturia MiniLab 3

Run the MiniLab display integration explicitly:

```bash
uv run ministep \
  --input "Minilab3 MIDI" \
  --output "MiniStep" \
  --mapping minilab3 \
  --minilab-display
```

`--minilab-display` opens the hard-coded MiniLab ports `Minilab3 ALV` (OLED) and `Minilab3 MIDI` (pad colour). The OLED connection and drawing SysEx are empirically derived rather than a published Arturia protocol; firmware may redraw its own screen, so MiniStep re-sends its display state while running.

### Hardware feedback

- The OLED shows MiniStep's ready/menu state and recognised knob values.
- The eight RGB pads show a green playback playhead.
- While REC is on but stopped, the selected/latest recorded step is red and pad 8 is amber as a REC indicator. During REC playback, pad 8 remains a record-status indicator.
- The main encoder opens the OLED menu: **press** to open, press again to edit the selected value, press again to stop editing; **turn** to select or change a value; **Shift-press** to close. The menu contains BPM, gate, division, transpose, loop length, and—when a profile is loaded—control page.

The implementation recognises the MiniLab main encoder and eight knob CCs from each of its Arturia/User and DAW control programs. It does not provide DAW transport integration or claim that every DAW-mode pad assignment is preserved.

### Learned command controls

The supplied MiniLab mapping deliberately starts empty. This keeps keyboard notes and pads musical until you assign a command.

Start MiniStep with `--mapping minilab3`, press `K`, choose a command with arrow keys, press `Enter`, then send a pad Note On or a CC. Learned mappings are stored in:

```text
~/.config/ministep/minilab3-mapping.json
```

Pads are matched by both MIDI channel and note. A learned pad/control is consumed as a MiniStep command; unlearned messages continue through as MIDI. Button-style CC commands fire only on a non-zero value, avoiding a second action on release. BPM, gate, transpose, and octave use absolute CC mappings; their learned ranges are 40–240 BPM, 0.05–1.0 gate, −24–+24 semitones, and −3–+3 octaves.

## Synth-control profiles

A YAML profile turns the MiniLab's eight knobs into outgoing CC controls arranged in pages. Use it with the display/menu path:

```bash
uv run ministep \
  --input "Minilab3 MIDI" \
  --output "MiniStep" \
  --minilab-display \
  --profile profiles/miniverse.yaml
```

The profile can use the sequencer output, declare its own `output`, or be overridden with `--control-output`. Profile knob events are routed to the active profile page and are not passed through as normal MIDI controls. Select **CONTROL PAGE** from the MiniLab main-encoder menu to change pages.

The included `profiles/miniverse.yaml` has these page bindings:

| Page | Knob → outgoing CC |
| --- | --- |
| `OSCILLATOR BANK` | 1 → 16 `OSC2 FREQUENCY` |
| `MIXER` | 1 → 82 `OSC1 VOLUME`; 2 → 83 `OSC2 VOLUME`; 3 → 85 `OSC3 VOLUME` |
| `MODIFIERS` | 1 → 20 `CUTOFF`; 2 → 21 `EMPHASIS`; 3 → 22 `CONTOUR`; 4 → 93 `FILTER ATTACK` |

`CONTROLLERS` and `OUTPUT` are present as empty pages. In Miniverse, use its MIDI Learn UI to assign those outgoing CCs to parameters.

To create a profile for another CC-capable synth, use this shape:

```yaml
name: My Synth
output: MiniStep       # optional; --control-output wins
channel: 1             # optional; 1–16
pages:
  FILTER:
    knobs:
      1: { cc: 20, label: CUTOFF }
      2: { cc: 21, label: RESONANCE, channel: 2 }
```

The profile must contain at least one named page; individual pages may leave knobs unbound. Knob numbers are 1–8, and CCs are 0–127.

## Remote control socket

MiniStep opens `~/.config/ministep/control.sock` (mode 0600) unless started with `--no-remote`; `--remote-socket PATH` moves it. The protocol is newline-delimited JSON. A client receives a full state snapshot on connect and another whenever anything changes; it sends commands as `{"cmd": NAME, "value": …}` where `NAME` is any controller command (`PLAY_STOP`, `COMMIT`, `REST`, `HOLD`, `UNDO`, `CLEAR`, `RESTART`, `RECORD_TOGGLE`, `BPM_UP`, `STEP_DIVISION_DOWN`, `TRANSPOSE_UP`, …) or an editor extra: `SAVE`, `LOAD`, `SAVE_PATTERN name`, `LOAD_PATTERN name`, `CURSOR ±n`, `CURSOR_NOTE ±n`, `REPLACE`, `DELETE`, `LOOP_CYCLE ±n`.

```bash
printf '{"cmd":"REST"}\n{"cmd":"PLAY_STOP"}\n' | nc -U ~/.config/ministep/control.sock
```

This is how the [MacroPad desk terminal](../../PERSONAL/macropad) drives MiniStep from physical keys.

## Sequence and pattern files

`S` and `L` use `~/.config/ministep/sequence.json`. `Shift+S` saves the current pattern under a name you type, one file per pattern in `~/.config/ministep/patterns/<name>.json`; `Shift+L` lists the saved names in the status line and loads the one you type. `--load path.json` loads any compatible JSON file. Names are sanitised to `[A-Za-z0-9._-]` for the filename (`Egypt 01` → `Egypt_01.json`); the name as typed is kept in the file. Writes go through a temporary file and an atomic rename, so an interrupted save never damages an existing pattern.

A pattern holds only musical state: MIDI port names, playhead, play/record state, cursor, and the audition note are never saved. Files are human-readable and versioned:

```json
{
  "version": 1,
  "name": "bite_test",
  "bpm": 128.0,
  "step_division": 16,
  "loop_length": 16,
  "default_gate": 0.5,
  "default_velocity": 100,
  "transpose": 0,
  "octave": 0,
  "output_channel": 1,
  "steps": [
    {"note": 60, "velocity": 110, "gate": 0.5, "enabled": true, "tie": false, "accent": false},
    {"note": null, "velocity": 100, "gate": 0.5, "enabled": true, "tie": false, "accent": false}
  ]
}
```

`note: null` is a rest. `loop_length: null` means `FULL`; otherwise it is one of 8, 16, 32, or 64. Stored note values are unchanged by live `transpose`; the value is applied during playback. `tie` carries a note over to its following matching note. `enabled` and `accent` are preserved in the format, although this TUI has no controls to edit them. Unknown keys are ignored and missing optional keys take defaults; a file with another `version` is refused with a status message instead of loading.

## Architecture

```text
MIDI backend ──► controller mapping / profile router ──► runtime + state
     │                                                       │
     └────────────────── MIDI pass-through ──────────────────┤
                                                             ▼
                                                   sequencer + internal clock
                                                             │
                                                             ▼
                                                        MIDI output

MiniLab OLED / pad SysEx ◄────────────────────────────── runtime state
```

- `state.py` owns typed steps, edits, settings, note names, and JSON (de)serialisation.
- `patterns.py` stores named patterns as one JSON file each with atomic writes; the TUI and remote socket only call it.
- `sequencer.py` and `clock.py` perform async playback and timing without hardware/UI dependencies.
- `midi.py` discovers and opens mido ports, moves callback input into asyncio, passes MIDI through, and tracks note ownership.
- `controller.py` and `mappings/` translate learned hardware messages into abstract commands.
- `profiles.py` routes MiniLab knob values through YAML CC pages; `minilab3_display.py` contains the MiniLab-specific SysEx client.
- `runtime.py` coordinates state, commands, playback, MIDI input, and the encoder menu; `tui.py` is the Textual interface; `main.py` wires the CLI and ports.

Controller-specific behaviour stays outside the sequencer core. The current clock source is internal; the `StepClock` abstraction leaves room for an external MIDI-clock implementation.

## Development

```bash
uv sync
uv run pytest
uv run ruff check .
uv run ruff format --check .
```

The test suite covers timing calculations, playback order/gates/ties/loop spans, record mode, editing and persistence, mappings/MIDI Learn, profiles, MiniLab display packets, encoder menus, and TUI rendering.

## Limitations and roadmap

- macOS-first: the documented virtual-routing path uses the IAC Driver.
- MIDI only: no audio processing and no live-performance timing capture.
- Playback uses MiniStep's internal clock; external MIDI clock is not implemented.
- MiniLab OLED/RGB support depends on its named ports and empirically derived SysEx behaviour.

Likely next additions: more controller adapters and synth profiles, external MIDI clock, and step-level features such as probability, velocity/gate variation, swing, direction modes, or pattern slots. They are not current features.

## Contributing

Contributions are welcome, especially controller adapters, synth profiles, bug fixes, and macOS MIDI hardware testing. Keep controller-specific code separate from the sequencer core where possible, and run the checks above before opening a change.

## License

MIT. See [LICENSE](LICENSE).
