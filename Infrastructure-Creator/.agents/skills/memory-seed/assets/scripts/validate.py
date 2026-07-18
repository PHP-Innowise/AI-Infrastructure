#!/usr/bin/env python3
"""Validate a project memory bank without external packages.

This validator is bundled by Infrastructure-Creator's memory-seed skill and is
copied verbatim into every generated target's memory-bank/scripts/. It is
dependency-free (standard library only) and stack-agnostic.

Usage:
    python3 validate.py [--summary] [--bank PATH]

Exit code 0 = valid, non-zero = one or more errors (printed to stderr).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import date
from pathlib import Path

ID_PATTERN = re.compile(r"^MEM-(\d{4,})$")
FILENAME_PATTERN = re.compile(r"^(MEM-\d{4,})-[a-z0-9]+(?:-[a-z0-9]+)*\.md$")

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
    pass


def parse_frontmatter(text: str) -> dict:
    if not text.startswith("---\n"):
        raise ValidationError("file must start with '---' frontmatter")
    end = text.find("\n---\n", 4)
    if end == -1:
        raise ValidationError("frontmatter is not closed with '---'")
    block = text[4:end]
    try:
        data = json.loads(block)
    except json.JSONDecodeError as exc:
        raise ValidationError(f"frontmatter is not valid JSON: {exc}")
    if not isinstance(data, dict):
        raise ValidationError("frontmatter must be a JSON object")
    return data


def _parse_date(value: str, field: str) -> date:
    try:
        return date.fromisoformat(value)
    except (ValueError, TypeError):
        raise ValidationError(f"{field} must be an ISO date (YYYY-MM-DD)")


def _is_nonempty_str_list(value) -> bool:
    return (
        isinstance(value, list)
        and len(value) > 0
        and all(isinstance(x, str) and x.strip() for x in value)
    )


def validate_metadata(meta: dict, filename: str, bank_root: Path, errors: list) -> None:
    def err(msg: str) -> None:
        errors.append(f"{filename}: {msg}")

    keys = set(meta.keys())
    missing = REQUIRED_KEYS - keys
    unexpected = keys - REQUIRED_KEYS
    if missing:
        err(f"missing metadata keys: {sorted(missing)}")
    if unexpected:
        err(f"unexpected metadata keys: {sorted(unexpected)}")
    if missing:
        return

    fn_match = FILENAME_PATTERN.match(filename)
    if not fn_match:
        err("filename must match MEM-NNNN-short-slug.md")
    id_match = ID_PATTERN.match(str(meta["id"]))
    if not id_match:
        err("id must match ^MEM-\\d{4,}$")
    if fn_match and id_match and fn_match.group(1) != meta["id"]:
        err("filename id must match frontmatter id")

    if not (isinstance(meta["title"], str) and meta["title"].strip()):
        err("title must be a non-empty string")
    if meta["type"] not in ALLOWED_TYPES:
        err(f"type must be one of {sorted(ALLOWED_TYPES)}")
    if meta["status"] not in ALLOWED_STATUSES:
        err(f"status must be one of {sorted(ALLOWED_STATUSES)}")
    for field in ("scope", "tags", "sources"):
        if not _is_nonempty_str_list(meta[field]):
            err(f"{field} must be a non-empty list of strings")
    if not isinstance(meta["supersedes"], list) or not all(
        isinstance(x, str) for x in meta["supersedes"]
    ):
        err("supersedes must be a list of memory IDs")

    try:
        created = _parse_date(meta["created"], "created")
        last_verified = _parse_date(meta["last_verified"], "last_verified")
        review_after = _parse_date(meta["review_after"], "review_after")
        today = date.today()
        if created > last_verified:
            err("created must be on or before last_verified")
        if last_verified > today:
            err("last_verified must not be in the future")
        if review_after < last_verified:
            err("review_after must be on or after last_verified")
        if meta["status"] == "active" and review_after < today:
            err("active chunk is overdue for review")
    except ValidationError as exc:
        err(str(exc))

    superseded_by = meta["superseded_by"]
    if superseded_by is not None and not ID_PATTERN.match(str(superseded_by)):
        err("superseded_by must be null or a valid memory ID")
    if meta["status"] == "superseded" and not superseded_by:
        err("superseded status requires superseded_by")
    if meta["status"] != "superseded" and superseded_by:
        err("only superseded chunks may set superseded_by")

    if isinstance(meta["sources"], list):
        for src in meta["sources"]:
            if not isinstance(src, str):
                continue
            if src.startswith("http://") or src.startswith("https://"):
                continue
            clean = src.split("#", 1)[0]
            try:
                resolved = (bank_root.parent / clean).resolve()
                repo_root = bank_root.parent.resolve()
                if repo_root not in resolved.parents and resolved != repo_root:
                    err(f"source path escapes the repository: {src}")
                elif not resolved.exists():
                    err(f"source path does not exist: {src}")
            except (OSError, ValueError):
                err(f"source path could not be resolved: {src}")


def validate_secret_patterns(text: str, filename: str, errors: list) -> None:
    for label, pattern in SECRET_PATTERNS.items():
        if pattern.search(text):
            errors.append(
                f"{filename}: possible {label} detected; value intentionally not printed"
            )


def parse_index(index_text: str) -> dict:
    rows = {}
    for line in index_text.splitlines():
        if not line.startswith("| MEM-"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) != 8:
            raise ValidationError(f"invalid index row (expected 8 cells): {line}")
        file_cell = cells[7].strip("[]").split("](", 1)[-1].rstrip(")")
        rows[cells[0]] = {
            "id": cells[0],
            "title": cells[1],
            "type": cells[2],
            "scope": cells[3],
            "tags": cells[4],
            "status": cells[5],
            "last_verified": cells[6],
            "file": file_cell,
        }
    return rows


def validate_bank(bank: Path, errors: list) -> dict:
    counts = {"chunks": 0, "active": 0, "superseded": 0}
    for required in ("README.md", "INDEX.md", ".memory-counter"):
        if not (bank / required).exists():
            errors.append(f"missing required file: memory-bank/{required}")
    if errors:
        return counts

    counter_raw = (bank / ".memory-counter").read_text(encoding="utf-8").strip()
    if not counter_raw.isdigit() or int(counter_raw) <= 0:
        errors.append(".memory-counter must be a positive integer")
        counter = None
    else:
        counter = int(counter_raw)

    chunks_dir = bank / "chunks"
    chunk_meta = {}
    max_id = 0
    if chunks_dir.exists():
        for entry in sorted(chunks_dir.iterdir()):
            if entry.is_symlink() or not entry.is_file() or entry.suffix != ".md":
                errors.append(f"unexpected chunk entry: {entry.name}")
                continue
            if not FILENAME_PATTERN.match(entry.name):
                errors.append(f"unexpected chunk entry: {entry.name}")
                continue
            text = entry.read_text(encoding="utf-8")
            validate_secret_patterns(text, entry.name, errors)
            try:
                meta = parse_frontmatter(text)
            except ValidationError as exc:
                errors.append(f"{entry.name}: {exc}")
                continue
            validate_metadata(meta, entry.name, bank, errors)
            cid = meta.get("id")
            if cid in chunk_meta:
                errors.append(f"duplicate chunk id: {cid}")
            chunk_meta[cid] = meta
            counts["chunks"] += 1
            if meta.get("status") == "active":
                counts["active"] += 1
            elif meta.get("status") == "superseded":
                counts["superseded"] += 1
            m = ID_PATTERN.match(str(cid))
            if m:
                max_id = max(max_id, int(m.group(1)))

    if counter is not None and counter <= max_id:
        errors.append("counter must be greater than every allocated ID")

    index_rows = parse_index((bank / "INDEX.md").read_text(encoding="utf-8"))
    for cid, meta in chunk_meta.items():
        if cid not in index_rows:
            errors.append(f"chunk is missing from INDEX.md: {cid}")
            continue
        row = index_rows[cid]
        expected_file = f"chunks/{cid}-" if cid else ""
        if not row["file"].startswith("chunks/"):
            errors.append(f"INDEX file column for {cid} must point under chunks/")
        for field in ("title", "type", "status", "last_verified"):
            if row[field] != str(meta.get(field)):
                errors.append(f"INDEX {field} mismatch for {cid}")
        if row["scope"] != ", ".join(meta.get("scope", [])):
            errors.append(f"INDEX scope mismatch for {cid}")
        if row["tags"] != ", ".join(meta.get("tags", [])):
            errors.append(f"INDEX tags mismatch for {cid}")
    for cid in index_rows:
        if cid not in chunk_meta:
            errors.append(f"INDEX.md points to a missing chunk: {cid}")

    # Supersession graph integrity.
    for cid, meta in chunk_meta.items():
        for target in meta.get("supersedes", []):
            if target == cid:
                errors.append(f"{cid}: chunk cannot supersede itself")
            elif target not in chunk_meta:
                errors.append(f"{cid}: supersedes unknown chunk {target}")
            else:
                tgt = chunk_meta[target]
                if tgt.get("status") != "superseded" or tgt.get("superseded_by") != cid:
                    errors.append(
                        f"{cid}: supersession target {target} must be superseded and point back"
                    )
        sb = meta.get("superseded_by")
        if sb and sb not in chunk_meta:
            errors.append(f"{cid}: superseded_by references unknown chunk {sb}")

    # Cycle detection over superseded_by chains.
    for start in chunk_meta:
        seen = set()
        cur = start
        while cur:
            if cur in seen:
                errors.append(f"supersession cycle detected involving {start}")
                break
            seen.add(cur)
            cur = chunk_meta.get(cur, {}).get("superseded_by")

    return counts


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate a project memory bank.")
    parser.add_argument("--summary", action="store_true", help="print counts only")
    parser.add_argument("--bank", default="memory-bank", help="path to the memory bank")
    args = parser.parse_args()

    bank = Path(args.bank)
    if not bank.exists():
        print(f"memory bank not found: {bank}", file=sys.stderr)
        return 2

    errors: list = []
    counts = validate_bank(bank, errors)

    if args.summary:
        print(
            f"chunks={counts['chunks']} active={counts['active']} "
            f"superseded={counts['superseded']} errors={len(errors)}"
        )

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        return 1

    if not args.summary:
        print("memory bank OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
