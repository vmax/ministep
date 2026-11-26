"""Named pattern persistence: one JSON file per pattern, atomic writes.

Melodic and drum patterns are independent files in two sub-directories of the
patterns directory (``melodic/`` and ``drums/``). This layer knows nothing about
the TUI. It turns a display name into a safe file, writes through a temporary
file plus ``os.replace`` so a crash never leaves a half-written pattern, and
applies a loaded file to exactly one track of the running :class:`AppState`.
Every failure surfaces as :class:`PatternError` with a message fit for the
status line.

Older MiniStep builds wrote one combined file per pattern straight into the
patterns directory. Those keep working: they are listed for both kinds and
loading one takes only the section that matches the active track.
"""

from __future__ import annotations

import json
import os
import re
import tempfile
from pathlib import Path
from typing import Any

from .state import AppState

DEFAULT_PATTERNS_DIR = Path.home() / ".config" / "ministep" / "patterns"
PATTERN_SUFFIX = ".json"
MAX_NAME_LENGTH = 64
# Pattern kinds double as the sub-directory names and the "type" field in files.
MELODIC = "melodic"
DRUMS = "drums"
KINDS = (MELODIC, DRUMS)
KIND_LABELS = {MELODIC: "melodic pattern", DRUMS: "drum pattern"}

_UNSAFE = re.compile(r"[^A-Za-z0-9._-]+")


class PatternError(ValueError):
    """A user-facing persistence failure (no traceback needed).

    Subclasses ``ValueError`` so callers that already handle invalid sequence
    files keep working unchanged.
    """


def sanitize_pattern_name(name: str) -> str:
    """Reduce a display name to a filename stem that cannot leave the directory.

    Whitespace and other unsafe characters become ``_``; path separators and
    leading dots are removed so ``../x`` or ``.hidden`` cannot occur.
    """
    stem = _UNSAFE.sub("_", name.strip().replace("/", "_").replace("\\", "_"))
    stem = stem.strip("._")
    stem = re.sub(r"_+", "_", stem)
    if stem.endswith(PATTERN_SUFFIX):
        stem = stem[: -len(PATTERN_SUFFIX)].rstrip("._")
    if not stem:
        raise PatternError("Pattern name must contain a letter or digit")
    return stem[:MAX_NAME_LENGTH]


def write_json_atomic(path: Path, data: dict[str, Any]) -> None:
    """Write JSON to ``path`` via a sibling temp file and ``os.replace``."""
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        handle = tempfile.NamedTemporaryFile(
            "w", encoding="utf-8", dir=path.parent, prefix=f".{path.name}.", delete=False
        )
    except OSError as error:
        raise PatternError(f"Cannot write {path.name}: {error.strerror or error}") from error
    temp_path = Path(handle.name)
    try:
        with handle:
            json.dump(data, handle, indent=2)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temp_path, path)
    except OSError as error:
        temp_path.unlink(missing_ok=True)
        raise PatternError(f"Cannot write {path.name}: {error.strerror or error}") from error


def read_json(path: Path) -> Any:
    try:
        text = path.read_text(encoding="utf-8")
    except FileNotFoundError as error:
        raise PatternError(f"Pattern not found: {path.stem}") from error
    except OSError as error:
        raise PatternError(f"Cannot read {path.name}: {error.strerror or error}") from error
    try:
        return json.loads(text)
    except ValueError as error:
        raise PatternError(f"{path.name} is not valid JSON: {error}") from error


def _check_kind(kind: str) -> str:
    if kind not in KINDS:
        raise PatternError(f"Unknown pattern kind {kind!r}")
    return kind


def extract_section(kind: str, data: Any, source: str = "pattern") -> Any:
    """Return the ``kind`` section of any pattern file MiniStep has ever written.

    Accepts a per-track file (``"type": "melodic"|"drums"``), a combined
    snapshot (version 2 with ``melodic``/``drums`` objects) or a flat version 1
    file (melodic only). Raises :class:`PatternError` when the file has no
    such section or an unknown version.
    """
    _check_kind(kind)
    if not isinstance(data, dict):
        raise PatternError(f"{source} must contain a JSON object")
    file_type = data.get("type")
    version = data.get("version", 1)
    if file_type is not None:
        if file_type not in KINDS:
            raise PatternError(f"{source}: unknown pattern type {file_type!r}")
        if file_type != kind:
            raise PatternError(f"{source} is a {KIND_LABELS[file_type]}, not a {KIND_LABELS[kind]}")
        if version != AppState.TRACK_FORMAT_VERSION:
            raise PatternError(
                f"{source}: unsupported {KIND_LABELS[kind]} version {version!r} "
                f"(this MiniStep reads version {AppState.TRACK_FORMAT_VERSION})"
            )
        return data
    # Legacy combined files.
    if version == 1:
        if kind == DRUMS:
            raise PatternError(f"{source} has no drum section (old melodic-only file)")
        return data
    if version == AppState.FORMAT_VERSION:
        section = data.get(kind)
        if section is None:
            raise PatternError(f"{source} has no {kind} section")
        return section
    raise PatternError(f"{source}: unsupported pattern version {version!r}")


class PatternStore:
    """Save and load named melodic and drum patterns under one directory."""

    def __init__(self, directory: Path = DEFAULT_PATTERNS_DIR) -> None:
        self.directory = directory

    def kind_directory(self, kind: str) -> Path:
        return self.directory / _check_kind(kind)

    def path_for(self, kind: str, name: str) -> Path:
        directory = self.kind_directory(kind)
        path = (directory / f"{sanitize_pattern_name(name)}{PATTERN_SUFFIX}").resolve()
        if path.parent != directory.resolve():
            raise PatternError("Pattern name escapes the patterns directory")
        return path

    def _legacy_path(self, name: str) -> Path | None:
        """A pre-split combined file in the root directory, if one exists."""
        try:
            path = (self.directory / f"{sanitize_pattern_name(name)}{PATTERN_SUFFIX}").resolve()
        except PatternError:
            return None
        if path.parent != self.directory.resolve() or not path.is_file():
            return None
        return path

    @staticmethod
    def _stems(directory: Path) -> set[str]:
        try:
            return {p.stem for p in directory.glob(f"*{PATTERN_SUFFIX}") if p.is_file()}
        except OSError:
            return set()

    def list_names(self, kind: str) -> list[str]:
        """Names saved as ``kind`` plus legacy combined files (listed for both kinds)."""
        names = self._stems(self.kind_directory(kind)) | self._stems(self.directory)
        return sorted(names)

    def exists(self, kind: str, name: str) -> bool:
        return self.path_for(kind, name).is_file() or self._legacy_path(name) is not None

    def save(self, kind: str, state: AppState, name: str) -> str:
        """Write ``state``'s ``kind`` track as ``name``; return the sanitized stored name."""
        path = self.path_for(kind, name)
        data: dict[str, Any] = {
            "version": AppState.TRACK_FORMAT_VERSION,
            "type": kind,
            "name": name.strip() or path.stem,
        }
        section = state.melodic_to_dict() if kind == MELODIC else state.drums.to_dict()
        data.update(section)
        write_json_atomic(path, data)
        return path.stem

    def read_section(self, kind: str, name: str) -> Any:
        """Locate ``name`` for ``kind`` (own directory first, then legacy) and extract it."""
        path = self.path_for(kind, name)
        if not path.is_file():
            legacy = self._legacy_path(name)
            if legacy is None:
                raise PatternError(f"Pattern not found: {path.stem}")
            path = legacy
        return extract_section(kind, read_json(path), source=path.name)

    def load(self, kind: str, state: AppState, name: str) -> str:
        """Apply the named ``kind`` pattern to that one track of ``state``.

        Nothing else in ``state`` changes: the other track, tempo, ports,
        transport, and the active track stay as they were. The state is
        untouched when the file is missing, corrupt, or of the wrong type.
        """
        section = self.read_section(kind, name)
        try:
            if kind == MELODIC:
                state.apply_melodic(section)
            else:
                state.apply_drums(section)
        except ValueError as error:
            raise PatternError(f"{name}: {error}") from error
        return self.path_for(kind, name).stem

    def delete(self, kind: str, name: str) -> None:
        path = self.path_for(kind, name)
        try:
            path.unlink()
        except FileNotFoundError as error:
            raise PatternError(f"Pattern not found: {path.stem}") from error
        except OSError as error:
            raise PatternError(f"Cannot delete {path.name}: {error.strerror or error}") from error

    def rename(self, kind: str, old: str, new: str) -> str:
        source, target = self.path_for(kind, old), self.path_for(kind, new)
        if not source.is_file():
            raise PatternError(f"Pattern not found: {source.stem}")
        if target.exists():
            raise PatternError(f"Pattern already exists: {target.stem}")
        try:
            data = read_json(source)
            if isinstance(data, dict):
                data["name"] = new.strip() or target.stem
            write_json_atomic(target, data)
            source.unlink()
        except OSError as error:
            raise PatternError(f"Cannot rename {source.name}: {error.strerror or error}") from error
        return target.stem
