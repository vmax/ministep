"""Named pattern persistence: one JSON file per pattern, atomic writes.

This layer knows nothing about the TUI. It turns a display name into a safe
file inside the patterns directory, writes :meth:`AppState.to_dict` output
through a temporary file plus ``os.replace`` so a crash never leaves a
half-written pattern, and reads files back through :meth:`AppState.from_dict`.
Every failure surfaces as :class:`PatternError` with a message fit for the
status line.
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


class PatternStore:
    """Save and load named patterns inside one directory."""

    def __init__(self, directory: Path = DEFAULT_PATTERNS_DIR) -> None:
        self.directory = directory

    def path_for(self, name: str) -> Path:
        path = (self.directory / f"{sanitize_pattern_name(name)}{PATTERN_SUFFIX}").resolve()
        if path.parent != self.directory.resolve():
            raise PatternError("Pattern name escapes the patterns directory")
        return path

    def list_names(self) -> list[str]:
        try:
            return sorted(p.stem for p in self.directory.glob(f"*{PATTERN_SUFFIX}") if p.is_file())
        except OSError:
            return []

    def exists(self, name: str) -> bool:
        return self.path_for(name).is_file()

    def save(self, state: AppState, name: str) -> str:
        """Write ``state`` as ``name``; return the sanitized stored name."""
        path = self.path_for(name)
        write_json_atomic(path, state.to_dict(name=name.strip() or path.stem))
        return path.stem

    def load(self, name: str) -> AppState:
        path = self.path_for(name)
        data = read_json(path)
        try:
            return AppState.from_dict(data)
        except ValueError as error:
            raise PatternError(f"{path.name}: {error}") from error

    def delete(self, name: str) -> None:
        path = self.path_for(name)
        try:
            path.unlink()
        except FileNotFoundError as error:
            raise PatternError(f"Pattern not found: {path.stem}") from error
        except OSError as error:
            raise PatternError(f"Cannot delete {path.name}: {error.strerror or error}") from error

    def rename(self, old: str, new: str) -> str:
        source, target = self.path_for(old), self.path_for(new)
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
