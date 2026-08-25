#!/usr/bin/env python3
"""Validate the canonical repository memory bank without external packages."""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import date
from pathlib import Path
from typing import Optional


# Chunk identifiers come in two accepted formats:
# - MEM-YYYYMMDD-xxxxxxxx (current): the allocation date plus eight hex
#   characters of the source Brain record's UUID. Needing no shared counter,
#   concurrent promotions on different machines or branches cannot collide.
# - MEM-0001 (legacy): sequential numbers once allocated from
#   `.memory-counter`. Existing chunks keep these IDs forever; nothing
#   renames them.
# The date-based alternative comes first so a filename match captures the
# whole date-based ID instead of stopping at its digit prefix.
# `.memory-counter` files may still exist on disk, but they are legacy, are
# no longer an ID source, and the validator deliberately ignores them.
ID_PATTERN = re.compile(r"^MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,})$")
FILENAME_PATTERN = re.compile(
    r"^(MEM-(?:\d{8}-[0-9a-f]{8}|\d{4,}))-[a-z0-9]+(?:-[a-z0-9]+)*\.md$"
)
ALLOWED_TYPES = {
    "architecture",
    "constraint",
    "convention",
    "decision",
    "domain",
    "integration",
    "operations",
}
ALLOWED_STATUSES = {"active", "needs-review", "superseded", "archived"}
REQUIRED_KEYS = {
    "id",
    "title",
    "type",
    "status",
    "scope",
    "tags",
    "created",
    "last_verified",
    "review_after",
    "sources",
    "supersedes",
    "superseded_by",
}
# Temporal validity. Optional, because requiring it would invalidate every
# chunk written before it existed. `superseded_by` says what replaced a chunk;
# `valid_to` says when it stopped being true. Automatically written memory
# needs the second: a wrong fact earns a boundary instead of being erased.
OPTIONAL_KEYS = {
    "valid_from",
    "valid_to",
    # Digests of the files this chunk cites, so a chunk can notice that the
    # thing it summarizes has moved on. Optional, because requiring it would
    # invalidate every chunk written before it existed - the same reason
    # `valid_from`/`valid_to` are optional.
    "source_digests",
}
SECRET_PATTERNS = {
    "private key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "GitHub token": re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b"),
    "AWS access key": re.compile(r"\bAKIA[0-9A-Z]{16}\b"),
    "OpenAI-style token": re.compile(r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}\b"),
    "Stripe secret key": re.compile(r"\bsk_(?:live|test)_[A-Za-z0-9]{16,}\b"),
    "JWT": re.compile(r"\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b"),
    "assigned credential": re.compile(
        r"\b(?:password|passwd|secret|api[_-]?key|access[_-]?token)\s*[:=]\s*[^\s<{][^\s]*",
        re.IGNORECASE,
    ),
}


class ValidationError(Exception):
    """A memory-bank contract violation."""


def parse_frontmatter(path: Path) -> dict:
    text = path.read_text(encoding="utf-8")
    if not text.startswith("---\n"):
        raise ValidationError("missing opening frontmatter delimiter")
    try:
        raw_metadata, _ = text[4:].split("\n---\n", 1)
    except ValueError as error:
        raise ValidationError("missing closing frontmatter delimiter") from error
    try:
        metadata = json.loads(raw_metadata)
    except json.JSONDecodeError as error:
        raise ValidationError(f"frontmatter is not valid JSON-compatible YAML: {error}") from error
    if not isinstance(metadata, dict):
        raise ValidationError("frontmatter must be an object")
    return metadata


def require_string_list(metadata: dict, key: str, *, allow_empty: bool = False) -> None:
    value = metadata.get(key)
    if not isinstance(value, list) or any(not isinstance(item, str) or not item for item in value):
        raise ValidationError(f"{key} must be a list of non-empty strings")
    if not allow_empty and not value:
        raise ValidationError(f"{key} must not be empty")


def require_date(metadata: dict, key: str) -> date:
    value = metadata.get(key)
    if not isinstance(value, str):
        raise ValidationError(f"{key} must be an ISO date string")
    try:
        return date.fromisoformat(value)
    except ValueError as error:
        raise ValidationError(f"{key} must use YYYY-MM-DD") from error


def terminal_chunk(metadata: dict, valid_to: object) -> bool:
    """Whether a chunk has reached the end of its life, by status or by date.

    A terminal chunk no longer answers questions, so the sources it cites are
    history rather than obligations. Deleting a cited file is routine work -
    dropping a controller, regenerating a schema - and it must not make an
    unrelated task fail validation for a chunk that was correctly archived.
    """
    if metadata.get("status") in {"superseded", "archived"}:
        return True
    return valid_to is not None and valid_to < date.today()


def validate_metadata(
    path: Path,
    metadata: dict,
    repository_root: Path,
    warnings: Optional[list[str]] = None,
) -> None:
    """Raise on anything that makes a chunk unusable.

    ``warnings`` opts a caller into the non-fatal class: a terminal chunk
    whose cited source has since been deleted is reported there instead of
    raised. Without a sink the behaviour is unchanged, which is what keeps
    the retrieval gate in `active_memory` strict.
    """
    missing = REQUIRED_KEYS - metadata.keys()
    extra = metadata.keys() - REQUIRED_KEYS - OPTIONAL_KEYS
    if missing:
        raise ValidationError(f"missing metadata keys: {', '.join(sorted(missing))}")
    if extra:
        raise ValidationError(f"unexpected metadata keys: {', '.join(sorted(extra))}")

    filename_match = FILENAME_PATTERN.fullmatch(path.name)
    if filename_match is None:
        raise ValidationError(
            "filename must be MEM-YYYYMMDD-xxxxxxxx-short-slug.md "
            "(or legacy MEM-0001-short-slug.md)"
        )
    memory_id = metadata["id"]
    if not isinstance(memory_id, str) or ID_PATTERN.fullmatch(memory_id) is None:
        raise ValidationError(
            "id must use MEM-YYYYMMDD-xxxxxxxx (or legacy MEM-0001) format"
        )
    if filename_match.group(1) != memory_id:
        raise ValidationError("frontmatter id does not match filename id")
    if not isinstance(metadata["title"], str) or not metadata["title"].strip():
        raise ValidationError("title must be a non-empty string")
    if not isinstance(metadata["type"], str) or metadata["type"] not in ALLOWED_TYPES:
        raise ValidationError(f"type must be one of: {', '.join(sorted(ALLOWED_TYPES))}")
    if not isinstance(metadata["status"], str) or metadata["status"] not in ALLOWED_STATUSES:
        raise ValidationError(f"status must be one of: {', '.join(sorted(ALLOWED_STATUSES))}")

    require_string_list(metadata, "scope")
    require_string_list(metadata, "tags")
    require_string_list(metadata, "sources")
    require_string_list(metadata, "supersedes", allow_empty=True)
    created = require_date(metadata, "created")
    last_verified = require_date(metadata, "last_verified")
    review_after = require_date(metadata, "review_after")
    if created > last_verified:
        raise ValidationError("created must not be later than last_verified")
    if last_verified > date.today():
        raise ValidationError("last_verified must not be in the future")
    if review_after < last_verified:
        raise ValidationError("review_after must not be earlier than last_verified")
    if metadata["status"] == "active" and review_after < date.today():
        raise ValidationError("active chunk is overdue for review")

    valid_from = require_date(metadata, "valid_from") if "valid_from" in metadata else None
    valid_to = None
    if metadata.get("valid_to") is not None:
        valid_to = require_date(metadata, "valid_to")
    # valid_from may precede `created`: knowledge is often true well before
    # anyone writes it down.
    if valid_to is not None:
        if valid_from is not None and valid_to < valid_from:
            raise ValidationError("valid_to must not be earlier than valid_from")
        # Matches the review_after rule above: knowledge that has stopped being
        # true may not keep calling itself active.
        if metadata["status"] == "active" and valid_to < date.today():
            raise ValidationError("active chunk is past its valid_to date")

    digests = metadata.get("source_digests")
    if digests is not None:
        if not isinstance(digests, list):
            raise ValidationError("source_digests must be a list")
        for entry in digests:
            if not isinstance(entry, dict) or set(entry) != {"path", "sha256"}:
                raise ValidationError(
                    "each source_digests entry must be {path, sha256}"
                )
            if not isinstance(entry["path"], str) or not entry["path"]:
                raise ValidationError("source_digests path must be a non-empty string")
            if not isinstance(entry["sha256"], str) or not re.fullmatch(
                r"[0-9a-f]{64}", entry["sha256"]
            ):
                raise ValidationError("source_digests sha256 must be a hex digest")
        cited = {
            source.split("#", 1)[0]
            for source in metadata["sources"]
            if not source.startswith(("https://", "http://"))
        }
        if {entry["path"] for entry in digests} - cited:
            raise ValidationError(
                "source_digests may only cover paths listed in sources"
            )

    replacement = metadata["superseded_by"]
    if replacement is not None and (
        not isinstance(replacement, str) or ID_PATTERN.fullmatch(replacement) is None
    ):
        raise ValidationError("superseded_by must be null or a memory ID")
    if metadata["status"] == "superseded" and replacement is None:
        raise ValidationError("superseded chunks require superseded_by")
    if metadata["status"] != "superseded" and replacement is not None:
        raise ValidationError("only superseded chunks may set superseded_by")

    for source in metadata["sources"]:
        if source.startswith(("https://", "http://")):
            continue
        source_path = source.split("#", 1)[0]
        resolved_source = (repository_root / source_path).resolve()
        try:
            resolved_source.relative_to(repository_root.resolve())
        except ValueError as error:
            raise ValidationError(f"source path escapes the repository: {source_path}") from error
        if not resolved_source.exists():
            # The containment check above stays fatal in every status: it is a
            # boundary guarantee, not a freshness one.
            if warnings is not None and terminal_chunk(metadata, valid_to):
                warnings.append(
                    f"{path}: cited source no longer exists: {source_path} "
                    f"(chunk is {metadata['status']} and no longer retrievable)"
                )
                continue
            raise ValidationError(f"source path does not exist: {source_path}")


def parse_index(path: Path) -> dict[str, dict[str, str]]:
    rows: dict[str, dict[str, str]] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith("| MEM-"):
            continue
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) != 8:
            raise ValidationError(f"invalid index row: {line}")
        memory_id, title, memory_type, scope, tags, status, verified, file_path = cells
        if memory_id in rows:
            raise ValidationError(f"duplicate index ID: {memory_id}")
        rows[memory_id] = {
            "title": title,
            "type": memory_type,
            "scope": scope,
            "tags": tags,
            "status": status,
            "last_verified": verified,
            "file": file_path.strip("[]").split("](", 1)[-1].rstrip(")"),
        }
    return rows


def validate_secret_patterns(path: Path) -> None:
    text = path.read_text(encoding="utf-8")
    for label, pattern in SECRET_PATTERNS.items():
        if pattern.search(text):
            raise ValidationError(f"possible {label} detected; value intentionally not printed")


def summarize_bank(bank_root: Path) -> str:
    chunks_dir = bank_root / "chunks"
    total = 0
    active = 0
    needs_review = 0
    expired = 0
    if chunks_dir.is_dir():
        for path in sorted(chunks_dir.iterdir()):
            if path.is_symlink() or not path.is_file() or FILENAME_PATTERN.fullmatch(path.name) is None:
                continue
            total += 1
            try:
                metadata = parse_frontmatter(path)
                status = metadata.get("status")
                if isinstance(metadata.get("valid_to"), str):
                    try:
                        if date.fromisoformat(metadata["valid_to"]) < date.today():
                            expired += 1
                    except ValueError:
                        pass
            except (OSError, ValidationError):
                continue
            active += status == "active"
            needs_review += status == "needs-review"
    summary = f"Memory bank: {total} chunks ({active} active, {needs_review} needs review"
    return summary + (f", {expired} past valid_to)." if expired else ").")


def validate_bank(bank_root: Path, source_root: Path | None = None) -> list[str]:
    """Every problem that makes the bank invalid. Kept for existing callers."""
    return validate_bank_report(bank_root, source_root)[0]


def validate_bank_report(
    bank_root: Path, source_root: Path | None = None
) -> tuple[list[str], list[str]]:
    """Errors and warnings, separately.

    The split exists because two different situations produced the same red
    output: a chunk that is wrong, and a chunk that correctly reached the end
    of its life while a file it cites was deleted. Only the first is something
    the engineer must fix, and only the first should fail a task that has
    nothing to do with the bank.
    """
    errors: list[str] = []
    warnings: list[str] = []
    # Chunks cite the project's own files, which sit beside the bank once it is
    # published. During generation the bank is staged apart from the project it
    # describes, so the caller passes the tree those citations resolve against;
    # without it a correctly seeded bank fails purely for being staged.
    repository_root = (source_root or bank_root.parent).resolve()
    index_path = bank_root / "INDEX.md"
    chunks_dir = bank_root / "chunks"
    # `.memory-counter` is intentionally absent from the required files and
    # from every check below: identifiers are date+UUID based now, so the
    # counter is a retired legacy artifact that may or may not exist on disk.
    for required in (bank_root / "README.md", index_path):
        if not required.is_file():
            errors.append(f"{required}: required file is missing")
    if errors:
        return errors, warnings

    try:
        index = parse_index(index_path)
    except ValidationError as error:
        errors.append(f"{index_path}: {error}")
        index = {}

    chunk_paths: list[Path] = []
    if chunks_dir.is_dir():
        for entry in sorted(chunks_dir.iterdir()):
            if entry.is_symlink() or not entry.is_file() or FILENAME_PATTERN.fullmatch(entry.name) is None:
                errors.append(
                    f"{entry}: unexpected chunk entry; expected a direct MEM-0001-short-slug.md file"
                )
                continue
            chunk_paths.append(entry)

    chunks: dict[str, tuple[Path, dict]] = {}
    # Every id that exists on disk, including chunks that failed validation.
    # The index cross-check below distinguishes "the file is gone" from "the
    # file is here and broken", and a chunk can fail before its id is
    # readable, so the filename is the fallback.
    on_disk: set[str] = set()
    for path in chunk_paths:
        filename_match = FILENAME_PATTERN.fullmatch(path.name)
        if filename_match is not None:
            on_disk.add(filename_match.group(1))
        try:
            metadata = parse_frontmatter(path)
            validate_metadata(path, metadata, repository_root, warnings)
            validate_secret_patterns(path)
            memory_id = metadata["id"]
            on_disk.add(memory_id)
            if memory_id in chunks:
                raise ValidationError(f"duplicate chunk ID: {memory_id}")
            chunks[memory_id] = (path, metadata)
        except (OSError, ValidationError) as error:
            errors.append(f"{path}: {error}")

    for memory_id, (path, metadata) in chunks.items():
        row = index.get(memory_id)
        if row is None:
            errors.append(f"{path}: chunk is missing from INDEX.md")
            continue
        expected_file = str(path.relative_to(bank_root))
        if row["file"] != expected_file:
            errors.append(f"{index_path}: {memory_id} file must be {expected_file}")
        for index_key, metadata_key in (
            ("title", "title"),
            ("type", "type"),
            ("status", "status"),
            ("last_verified", "last_verified"),
        ):
            if row[index_key] != str(metadata[metadata_key]):
                errors.append(f"{index_path}: {memory_id} {index_key} differs from chunk metadata")
        for index_key, metadata_key in (("scope", "scope"), ("tags", "tags")):
            expected = ", ".join(metadata[metadata_key])
            if row[index_key] != expected:
                errors.append(f"{index_path}: {memory_id} {index_key} differs from chunk metadata")

    for memory_id, row in index.items():
        if memory_id in chunks:
            continue
        if memory_id in on_disk:
            # The chunk is on disk and already reported its own error above.
            # Saying it is missing sends the reader looking for a deleted file
            # that is sitting right there.
            warnings.append(
                f"{index_path}: {memory_id} index row retained for invalid chunk "
                f"({row['file']})"
            )
        else:
            errors.append(
                f"{index_path}: {memory_id} points to a missing chunk ({row['file']})"
            )

    for memory_id, (_, metadata) in chunks.items():
        if memory_id in metadata["supersedes"] or metadata["superseded_by"] == memory_id:
            errors.append(f"{memory_id}: chunk cannot supersede itself")
        referenced = [*metadata["supersedes"]]
        if metadata["superseded_by"] is not None:
            referenced.append(metadata["superseded_by"])
        for referenced_id in referenced:
            if referenced_id not in chunks:
                errors.append(f"{memory_id}: replacement link points to missing {referenced_id}")

        for superseded_id in metadata["supersedes"]:
            if superseded_id not in chunks:
                continue
            superseded_metadata = chunks[superseded_id][1]
            if superseded_metadata["status"] != "superseded":
                errors.append(f"{memory_id}: {superseded_id} must have superseded status")
            if superseded_metadata["superseded_by"] != memory_id:
                errors.append(f"{memory_id}: {superseded_id} must point back with superseded_by")

        replacement_id = metadata["superseded_by"]
        if replacement_id in chunks and memory_id not in chunks[replacement_id][1]["supersedes"]:
            errors.append(f"{memory_id}: {replacement_id} must include this ID in supersedes")

    for start_id in chunks:
        visited: set[str] = set()
        current_id: str | None = start_id
        while current_id in chunks:
            if current_id in visited:
                errors.append(f"{start_id}: supersession cycle detected")
                break
            visited.add(current_id)
            current_id = chunks[current_id][1]["superseded_by"]

    return errors, warnings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "bank",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="Memory-bank directory (default: directory containing this script)",
    )
    parser.add_argument(
        "--summary",
        action="store_true",
        help="print status counts from chunk frontmatter without printing chunk contents",
    )
    parser.add_argument(
        "--source-root",
        type=Path,
        default=None,
        help=(
            "tree the chunk sources resolve against (default: the bank's parent). "
            "Pass the real project root when validating a staged bank."
        ),
    )
    args = parser.parse_args()
    bank_root = args.bank.resolve()
    if args.summary:
        print(summarize_bank(bank_root))
        return 0

    errors, warnings = validate_bank_report(
        bank_root, args.source_root.resolve() if args.source_root else None
    )
    if warnings:
        # Printed on both paths and never fatal: a warning describes a chunk
        # that is doing what it promised, so it must not fail a task that has
        # nothing to do with the bank.
        print(f"Memory bank validation warnings ({len(warnings)}):", file=sys.stderr)
        for warning in warnings:
            print(f"- {warning}", file=sys.stderr)
    if errors:
        print(f"Memory bank validation failed ({len(errors)} error(s)):", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1
    print("Memory bank validation passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
