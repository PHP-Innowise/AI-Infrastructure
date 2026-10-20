#!/usr/bin/env python3
"""Deterministic fact consolidation: explicit versions, then timestamps.

No model call, no vector or graph store. The 2026 conflict-resolution study
found the assembly step, not storage, was the bottleneck: extracting the
candidates and taking the maximum explicit version (a serial, a revision, a
timestamp) in Python beat letting the model judge freshness. This tool is the
maintainer-side implementation of that rule, and the local
``tests/fixtures/fact-consolidation.json`` fixture is its known-answer set.

It is deliberately not wired into the prompt-time capsule. Two Project Brain
records that declare each other in ``conflicts`` are *shown together* by
contract, with ``match: conflict``; that is a disagreement to inspect, not a
version to resolve. A restatement carries an explicit version or timestamp and
belongs here, at the write/consolidation boundary.

Input is a JSON list of facts, or an object with a ``facts`` list:

    [{"id": "v1", "fact": "deploy status", "serial": 1,
      "updated_at": "2026-06-01T00:00:00Z", "claim": "blue"}, ...]

Output groups by ``fact`` and reports the winner, the superseded IDs and any
genuine tie. Only explicit fields decide: a missing version is ``None`` and
sorts below any present one; equal version and timestamp is a tie, not a path
or an ID arbitration.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Optional


def _version(fact: dict[str, Any]) -> Optional[int]:
    for key in ("serial", "version", "revision"):
        value = fact.get(key)
        if type(value) is int:
            return value
    return None


def _timestamp(fact: dict[str, Any]) -> str:
    value = fact.get("updated_at", fact.get("at", ""))
    return value if isinstance(value, str) else ""


def _rank(fact: dict[str, Any]) -> tuple[bool, int, str]:
    version = _version(fact)
    return (version is not None, version if version is not None else -1, _timestamp(fact))


def resolve(facts: list[dict[str, Any]]) -> dict[str, Any]:
    """Group by ``fact`` and choose the maximum explicit version/timestamp."""
    groups: dict[str, list[dict[str, Any]]] = {}
    seen: set[str] = set()
    for index, fact in enumerate(facts):
        if not isinstance(fact, dict):
            raise ValueError(f"fact {index} must be an object")
        identifier = fact.get("id")
        if not isinstance(identifier, str) or not identifier:
            raise ValueError(f"fact {index} has no string id")
        if identifier in seen:
            raise ValueError(f"duplicate fact id: {identifier}")
        seen.add(identifier)
        key = fact.get("fact")
        if not isinstance(key, str) or not key.strip():
            raise ValueError(f"fact {identifier} has no fact key")
        groups.setdefault(key.strip(), []).append(fact)
    result = []
    for key in sorted(groups):
        candidates = groups[key]
        best = max(_rank(fact) for fact in candidates)
        winners = [fact for fact in candidates if _rank(fact) == best]
        winner = winners[0] if len(winners) == 1 else None
        result.append(
            {
                "fact": key,
                "winner": winner.get("id") if winner is not None else None,
                "ambiguous": len(winners) > 1,
                "superseded": sorted(
                    fact["id"] for fact in candidates if fact not in winners
                ),
                "candidates": sorted(fact["id"] for fact in candidates),
            }
        )
    return {"groups": result}


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Deterministic fact consolidation.")
    parser.add_argument("--input", type=Path, required=True, help="JSON facts")
    parser.add_argument("--json", action="store_true")
    arguments = parser.parse_args(argv)
    try:
        data = json.loads(arguments.input.read_text(encoding="utf-8"))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"memory_fact_consolidation: {error}")
        return 2
    facts = data.get("facts") if isinstance(data, dict) else data
    if not isinstance(facts, list):
        print("memory_fact_consolidation: input must be a list or {facts: [...]}")
        return 2
    try:
        result = resolve(facts)
    except ValueError as error:
        print(f"memory_fact_consolidation: {error}")
        return 2
    if arguments.json:
        print(json.dumps(result, indent=1, sort_keys=True))
    else:
        for group in result["groups"]:
            if group["ambiguous"]:
                print(f"{group['fact']}: ambiguous ({', '.join(group['candidates'])})")
            else:
                print(
                    f"{group['fact']}: {group['winner']} "
                    f"(supersedes {', '.join(group['superseded']) or 'nothing'})"
                )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
