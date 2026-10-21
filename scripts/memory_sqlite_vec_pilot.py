#!/usr/bin/env python3
"""Maintainer-only probe for a local sqlite-vec + sqlite-lembed hybrid pool.

The repository runtime stays standard-library only: it never imports this
module, loads an extension, reads a model or opens a network socket. The probe
is run by a maintainer with local assets, and every failure is data - it
returns ``available: false`` with a reason rather than falling back to a
different ranking.

Two steps can be run independently:

    python3 scripts/memory_sqlite_vec_pilot.py probe \
      --vec-extension ~/models/vec0.so \
      --lembed-extension ~/models/lembed0.so \
      --model ~/models/minilm.gguf

    python3 scripts/memory_sqlite_vec_pilot.py fuse \
      --lexical lexical.json --dense dense.json --k 60

``fuse`` is the only part with no native dependency: it combines two ranked
path lists with Reciprocal Rank Fusion, so a dense candidate pool can be
compared against the lexical one before any extension is trusted. Neither
command changes memory, the index or a capsule.
"""
from __future__ import annotations

import argparse
import json
import sqlite3
from pathlib import Path
from typing import Any, Optional


def _version(connection: sqlite3.Connection, function: str) -> Optional[str]:
    try:
        row = connection.execute(f"SELECT {function}()").fetchone()
    except sqlite3.Error:
        return None
    return str(row[0]) if row and row[0] is not None else None


def probe(
    vec_extension: Optional[str],
    lembed_extension: Optional[str],
    model: Optional[str],
) -> dict[str, Any]:
    """Load the local assets and report what this machine can actually do."""
    result: dict[str, Any] = {"available": False, "checks": {}, "reason": None}
    assets = {
        "vec-extension": vec_extension,
        "lembed-extension": lembed_extension,
        "model": model,
    }
    for label, value in assets.items():
        if not value:
            result["reason"] = f"{label}-missing"
            return result
        path = Path(value).expanduser()
        if not path.is_file() or path.is_symlink():
            result["reason"] = f"{label}-missing"
            return result
        result["checks"][label] = {"path": str(path), "bytes": path.stat().st_size}
    connection = sqlite3.connect(":memory:")
    try:
        try:
            connection.enable_load_extension(True)
        except AttributeError:
            result["reason"] = "extension-loading-unavailable"
            return result
        try:
            connection.load_extension(str(Path(vec_extension).expanduser()))
        except (sqlite3.Error, OSError):
            result["reason"] = "vec-extension-load-failed"
            return result
        result["checks"]["vec-version"] = _version(connection, "vec_version")
        try:
            connection.load_extension(str(Path(lembed_extension).expanduser()))
        except (sqlite3.Error, OSError):
            result["reason"] = "lembed-extension-load-failed"
            return result
        result["checks"]["lembed-version"] = _version(connection, "lembed_version")
        result["available"] = True
    finally:
        connection.close()
    return result


def reciprocal_rank_fusion(
    lexical: list[str], dense: list[str], *, k: int = 60
) -> list[dict[str, Any]]:
    """Fuse two ranked path lists without trusting either score scale."""
    if k < 1:
        raise ValueError("k must be positive")
    scores: dict[str, float] = {}
    ranks: dict[str, dict[str, int]] = {}
    for source, rows in (("lexical", lexical), ("dense", dense)):
        seen: set[str] = set()
        rank = 0
        for path in rows:
            if not isinstance(path, str) or not path or path in seen:
                continue
            seen.add(path)
            rank += 1
            scores[path] = scores.get(path, 0.0) + 1.0 / (k + rank)
            ranks.setdefault(path, {})[source] = rank
    return [
        {"path": path, "score": round(scores[path], 9), "ranks": ranks[path]}
        for path in sorted(scores, key=lambda value: (-scores[value], value))
    ]


def _paths(path: Path) -> list[str]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ValueError(f"{path}: expected a JSON list of paths")
    return value


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Local sqlite-vec/lembed probe and RRF.")
    commands = parser.add_subparsers(dest="command", required=True)
    probe_command = commands.add_parser("probe", help="load local vector assets")
    probe_command.add_argument("--vec-extension")
    probe_command.add_argument("--lembed-extension")
    probe_command.add_argument("--model")
    probe_command.add_argument("--json", action="store_true")
    fuse_command = commands.add_parser("fuse", help="RRF two ranked path lists")
    fuse_command.add_argument("--lexical", type=Path, required=True)
    fuse_command.add_argument("--dense", type=Path, required=True)
    fuse_command.add_argument("--k", type=int, default=60)
    fuse_command.add_argument("--json", action="store_true")
    arguments = parser.parse_args(argv)

    if arguments.command == "probe":
        result = probe(
            arguments.vec_extension,
            arguments.lembed_extension,
            arguments.model,
        )
        print(json.dumps(result, indent=1, sort_keys=True) if arguments.json else
              (f"available: {result['available']}"
               + (f" ({result['reason']})" if result["reason"] else "")))
        return 0 if result["available"] else 1

    try:
        fused = reciprocal_rank_fusion(
            _paths(arguments.lexical), _paths(arguments.dense), k=arguments.k
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"memory_sqlite_vec_pilot: {error}")
        return 2
    print(json.dumps(fused, indent=1) if arguments.json else
          "\n".join(f"{row['ranks']} {row['path']}" for row in fused))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
