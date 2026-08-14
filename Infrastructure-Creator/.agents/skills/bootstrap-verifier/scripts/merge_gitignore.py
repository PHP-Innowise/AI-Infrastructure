#!/usr/bin/env python3
"""Build a deterministic staged .gitignore from exact requirement entries."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import sys
import unicodedata
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence


BLOCK_VERSION = 1
BEGIN_MARKER = f"# BEGIN Infrastructure-Creator .gitignore v{BLOCK_VERSION}"
END_MARKER = f"# END Infrastructure-Creator .gitignore v{BLOCK_VERSION}"
MARKER_PATTERN = re.compile(
    r"^# (?:BEGIN|END) Infrastructure-Creator \.gitignore v\d+$"
)


class GitignoreMergeError(ValueError):
    """Raised when input cannot be merged without risking team-owned bytes."""


@dataclass(frozen=True)
class MergeResult:
    """The bytes to stage and their deterministic proposal metadata."""

    staged_bytes: bytes
    metadata: dict[str, Any]


def _sha256(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _decode_existing(existing: bytes | None) -> tuple[bytes, str, bool]:
    raw = existing if existing is not None else b""
    if b"\x00" in raw:
        raise GitignoreMergeError("existing .gitignore contains NUL")
    has_bom = raw.startswith(b"\xef\xbb\xbf")
    payload = raw[3:] if has_bom else raw
    try:
        text = payload.decode("utf-8")
    except UnicodeDecodeError as error:
        raise GitignoreMergeError(
            "existing .gitignore must be valid UTF-8"
        ) from error
    return raw, text, has_bom


def _normalize_requirements(value: Any) -> list[str]:
    if isinstance(value, Mapping):
        unknown = set(value) - {"requirements"}
        if unknown or "requirements" not in value:
            raise GitignoreMergeError(
                "requirement JSON object must contain only 'requirements'"
            )
        value = value["requirements"]
    if not isinstance(value, Sequence) or isinstance(value, (str, bytes, bytearray)):
        raise GitignoreMergeError(
            "requirement JSON must be an array or {'requirements': [...]}"
        )

    normalized: set[str] = set()
    for index, entry in enumerate(value):
        if not isinstance(entry, str):
            raise GitignoreMergeError(f"requirement {index} must be a string")
        if not entry:
            raise GitignoreMergeError(f"requirement {index} must not be empty")
        if entry.startswith("#"):
            raise GitignoreMergeError(
                f"requirement {index} must be an entry, not a comment"
            )
        if any(unicodedata.category(character).startswith("C") for character in entry):
            raise GitignoreMergeError(
                f"requirement {index} contains a control character"
            )
        normalized.add(entry)
    return sorted(normalized)


def load_requirements_json(raw: bytes | str) -> list[str]:
    """Parse and validate requirement JSON from UTF-8 bytes or text."""
    if isinstance(raw, bytes):
        if b"\x00" in raw:
            raise GitignoreMergeError("requirement JSON contains NUL")
        try:
            text = raw.decode("utf-8-sig")
        except UnicodeDecodeError as error:
            raise GitignoreMergeError(
                "requirement JSON must be valid UTF-8"
            ) from error
    else:
        text = raw
    try:
        value = json.loads(text)
    except json.JSONDecodeError as error:
        raise GitignoreMergeError(f"invalid requirement JSON: {error}") from error
    return _normalize_requirements(value)


def combine_requirement_documents(documents: Sequence[bytes | str]) -> list[str]:
    """Validate and union forge documents independently of completion order."""
    if not documents:
        raise GitignoreMergeError("at least one requirement document is required")
    combined: set[str] = set()
    for document in documents:
        combined.update(load_requirements_json(document))
    return sorted(combined)


def _newline_policy(text: str) -> str:
    newline_tokens = re.findall(r"\r\n|\r|\n", text)
    if newline_tokens and all(token == "\r\n" for token in newline_tokens):
        return "\r\n"
    return "\n"


def _line_value(line: str) -> str:
    return line[:-2] if line.endswith("\r\n") else line[:-1] if line.endswith(("\r", "\n")) else line


def _managed_span(lines: Sequence[str]) -> tuple[int, int] | None:
    markers = [
        (index, _line_value(line))
        for index, line in enumerate(lines)
        if MARKER_PATTERN.fullmatch(_line_value(line))
    ]
    if not markers:
        return None
    expected = [(BEGIN_MARKER, END_MARKER)]
    values = [value for _, value in markers]
    if len(markers) != 2 or values != list(expected[0]):
        raise GitignoreMergeError(
            "existing .gitignore has malformed, duplicate, or unsupported managed markers"
        )
    start, end = markers[0][0], markers[1][0]
    if end <= start:
        raise GitignoreMergeError("managed .gitignore markers are out of order")
    return start, end


def _outside_entries(lines: Sequence[str], span: tuple[int, int] | None) -> set[str]:
    entries: set[str] = set()
    for index, line in enumerate(lines):
        if span is not None and span[0] <= index <= span[1]:
            continue
        value = _line_value(line)
        if value and not value.startswith("#"):
            entries.add(value)
    return entries


def _block(requirements: Sequence[str], newline: str) -> str:
    values = [BEGIN_MARKER, *requirements, END_MARKER]
    return newline.join(values) + newline


def _append_block(text: str, block: str, newline: str) -> str:
    if not text:
        return block
    separator = "" if text.endswith(("\r", "\n")) else newline
    if text.endswith((newline + newline, "\r\r")):
        return text + separator + block
    return text + separator + newline + block


def merge_gitignore(
    existing: bytes | None,
    requirements: (
        bytes
        | str
        | Sequence[str]
        | Mapping[str, Sequence[str]]
    ),
) -> MergeResult:
    """Return staged bytes and metadata from JSON or validated requirement values."""
    original, text, has_bom = _decode_existing(existing)
    normalized = (
        load_requirements_json(requirements)
        if isinstance(requirements, (bytes, str))
        else _normalize_requirements(requirements)
    )
    newline = _newline_policy(text)
    lines = text.splitlines(keepends=True)
    span = _managed_span(lines)
    outside = _outside_entries(lines, span)
    managed_requirements = [entry for entry in normalized if entry not in outside]

    if span is None and not managed_requirements:
        proposal_text = text
        strategy = "unchanged"
    elif span is None:
        proposal_text = _append_block(
            text, _block(managed_requirements, newline), newline
        )
        strategy = "append-managed-block"
    else:
        start, end = span
        proposal_text = (
            "".join(lines[:start])
            + _block(managed_requirements, newline)
            + "".join(lines[end + 1 :])
        )
        strategy = (
            "unchanged"
            if proposal_text == text
            else "replace-managed-block"
        )

    bom = b"\xef\xbb\xbf" if has_bom else b""
    staged = bom + proposal_text.encode("utf-8")
    metadata = {
        "schema_version": 1,
        "path": ".gitignore",
        "origin": "Infrastructure-Creator",
        "strategy": strategy,
        "block_version": BLOCK_VERSION,
        "newline": "crlf" if newline == "\r\n" else "lf",
        "changed": staged != original,
        "original_sha256": _sha256(original),
        "proposal_sha256": _sha256(staged),
        "resolved_sha256": _sha256(staged),
        "requirements": normalized,
    }
    return MergeResult(staged_bytes=staged, metadata=metadata)


def _read_optional(path: str | None) -> bytes | None:
    if path is None:
        return None
    candidate = Path(path)
    try:
        return candidate.read_bytes()
    except OSError as error:
        raise GitignoreMergeError(
            f"existing .gitignore unreadable: {candidate}: {error}"
        ) from error


def _atomic_write(path: Path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    try:
        temporary.write_bytes(content)
        os.replace(temporary, path)
    finally:
        try:
            temporary.unlink()
        except FileNotFoundError:
            pass


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--existing", help="optional existing .gitignore (omit when absent)"
    )
    parser.add_argument(
        "--requirements",
        required=True,
        action="append",
        help="requirement JSON path; repeat once per forge",
    )
    parser.add_argument(
        "--output",
        default="-",
        help="staged .gitignore path, or '-' for stdout (default)",
    )
    parser.add_argument(
        "--metadata",
        help="proposal metadata JSON path (stdout mode writes metadata to stderr)",
    )
    args = parser.parse_args(argv)

    try:
        requirement_documents = [
            Path(path).read_bytes() for path in args.requirements
        ]
        requirements = combine_requirement_documents(requirement_documents)
        result = merge_gitignore(_read_optional(args.existing), requirements)
        metadata_bytes = (
            json.dumps(result.metadata, indent=2, sort_keys=True) + "\n"
        ).encode("utf-8")

        if args.output == "-":
            sys.stdout.buffer.write(result.staged_bytes)
            if args.metadata:
                _atomic_write(Path(args.metadata), metadata_bytes)
            else:
                sys.stderr.write(metadata_bytes.decode("utf-8"))
        else:
            _atomic_write(Path(args.output), result.staged_bytes)
            if args.metadata:
                _atomic_write(Path(args.metadata), metadata_bytes)
            else:
                sys.stdout.buffer.write(metadata_bytes)
    except (OSError, GitignoreMergeError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
