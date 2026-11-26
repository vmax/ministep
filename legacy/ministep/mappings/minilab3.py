"""Safe starter mapping for Arturia MiniLab 3.

It intentionally maps no MiniLab messages to MiniStep commands. Keyboard notes
and pads can then always be auditioned as musical MIDI without collisions. Add
controller commands here later only after assigning unique CCs in Arturia MIDI
Control Center (never reuse keyboard note values for commands).
"""

from __future__ import annotations

from ministep.controller import SimpleMapping

MINILAB3_MAPPING = SimpleMapping(note_commands={}, cc_commands={})
