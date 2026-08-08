#!/usr/bin/env python3
"""Attribute real token spend from local Claude Code transcripts.

The accelerator budgets context statically (scripts/context_budget.py measures
what an edition costs at startup). This script answers the other half: what a
session actually *spent*, and on what. It reads the transcripts Claude Code
already writes under ~/.claude/projects and needs no telemetry exporter, no
network, and no configuration.

Two things it exists to make visible:

  Raw token counts are not costs. Cached input is billed at a fraction of base
  input and output at a multiple of it, so a component that is 97% of the
  tokens can be a minority of the bill - and the reverse. Every table below is
  reported in BTE (base-token equivalent): tokens x the weight of the tier they
  were billed in.

  A token's cost scales with how long it stays resident. Cache reads dominate
  because the whole context is re-read on every turn, so anything placed in
  context early is paid again on each subsequent turn of the session.

Usage:
    python3 scripts/cost_attribution.py
    python3 scripts/cost_attribution.py --project AI-Infrastructure
    python3 scripts/cost_attribution.py --by skill --top 20
    python3 scripts/cost_attribution.py --root /path/to/projects --json

Developer-local tooling: it reads a directory outside the repository and is
never invoked by a hook, a skill, or CI.

Python 3 stdlib only.
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

# Billing tiers relative to one uncached input token. Cache writes are charged
# above base and reads far below it, so weighting is what makes components
# comparable at all. Adjust here if the published ratios change.
WEIGHTS = {
    "input": 1.0,
    "cache_write_5m": 1.25,
    "cache_write_1h": 2.0,
    "cache_read": 0.1,
    "output": 5.0,
}

# Record fields Claude Code stamps on the same assistant record that carries
# message.usage, so cost can be grouped by them without any extra plumbing.
DIMENSIONS = {
    "skill": "attributionSkill",
    "mcp": "attributionMcpServer",
    "tool": "attributionMcpTool",
    "plugin": "attributionPlugin",
    "agent": "attributionAgent",
}


def tier_tokens(usage: dict) -> dict[str, int]:
    """Split one call's usage into billing tiers.

    cache_creation_input_tokens is the total; the per-TTL breakdown under
    cache_creation splits it. When that breakdown is absent the whole write is
    attributed to the 5m tier, which is the cheaper of the two - this
    under-states rather than inflates.
    """
    created = usage.get("cache_creation") or {}
    write_1h = created.get("ephemeral_1h_input_tokens") or 0
    write_5m = created.get("ephemeral_5m_input_tokens") or 0
    total_write = usage.get("cache_creation_input_tokens") or 0
    if not (write_1h or write_5m):
        write_5m = total_write
    return {
        "input": usage.get("input_tokens") or 0,
        "cache_write_5m": write_5m,
        "cache_write_1h": write_1h,
        "cache_read": usage.get("cache_read_input_tokens") or 0,
        "output": usage.get("output_tokens") or 0,
    }


def bte(tiers: dict[str, int]) -> float:
    return sum(tiers[name] * WEIGHTS[name] for name in WEIGHTS)


def iter_calls(root: Path, project_filter: str | None):
    """Yield one record per billed API call.

    Claude Code writes one transcript record per content block and repeats
    message.usage verbatim on each, so summing records multiplies a call by its
    block count. Collapsing on message.id is what makes the totals real.
    """
    for path in sorted(root.rglob("*.jsonl")):
        project = path.relative_to(root).parts[0]
        if project_filter and project_filter not in project:
            continue
        seen: dict[str, dict] = {}
        try:
            handle = path.open(encoding="utf-8", errors="replace")
        except OSError:
            continue
        with handle:
            for line in handle:
                try:
                    record = json.loads(line)
                except ValueError:
                    continue
                message = record.get("message")
                if not isinstance(message, dict):
                    continue
                if not isinstance(message.get("usage"), dict):
                    continue
                key = message.get("id")
                if key is None:
                    yield project, record, message["usage"]
                else:
                    seen[key] = record
        for record in seen.values():
            yield project, record, record["message"]["usage"]


def report(title: str, rows: dict[str, float], calls: Counter, top: int) -> None:
    total = sum(rows.values())
    if not total:
        print(f"\n{title}: no data")
        return
    print(f"\n{title}")
    print(f"{'':<44}{'BTE':>18}{'share':>9}{'calls':>10}")
    for name, cost in sorted(rows.items(), key=lambda kv: -kv[1])[:top]:
        label = name if len(name) <= 43 else name[:40] + "..."
        print(f"{label:<44}{cost:>18,.0f}{100 * cost / total:>8.1f}%{calls[name]:>10,}")
    print(f"{'TOTAL':<44}{total:>18,.0f}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--root",
        type=Path,
        default=Path.home() / ".claude" / "projects",
        help="transcript root (default: ~/.claude/projects)",
    )
    parser.add_argument("--project", help="only projects whose directory contains this")
    parser.add_argument(
        "--by",
        choices=sorted(DIMENSIONS),
        action="append",
        help="attribution dimension (repeatable; default: all)",
    )
    parser.add_argument("--top", type=int, default=15, help="rows per table")
    parser.add_argument("--json", action="store_true", help="emit totals as JSON")
    args = parser.parse_args()

    if not args.root.is_dir():
        print(f"cost-attribution: no transcript directory at {args.root}", file=sys.stderr)
        return 1

    tiers = Counter()
    strata = defaultdict(float)
    stratum_calls = Counter()
    by_dimension = {name: defaultdict(float) for name in DIMENSIONS}
    dimension_calls = {name: Counter() for name in DIMENSIONS}
    projects = defaultdict(float)
    project_calls = Counter()
    calls = 0

    for project, record, usage in iter_calls(args.root, args.project):
        split = tier_tokens(usage)
        cost = bte(split)
        calls += 1
        tiers.update(split)
        stratum = "subagent" if record.get("isSidechain") else "main"
        strata[stratum] += cost
        stratum_calls[stratum] += 1
        projects[project] += cost
        project_calls[project] += 1
        for name, field in DIMENSIONS.items():
            value = record.get(field)
            if value:
                by_dimension[name][str(value)] += cost
                dimension_calls[name][str(value)] += 1

    total = bte(tiers)
    if not calls:
        print("cost-attribution: no billed calls found", file=sys.stderr)
        return 1

    if args.json:
        print(json.dumps({
            "calls": calls,
            "weights": WEIGHTS,
            "tiers": {k: tiers[k] for k in WEIGHTS},
            "total_bte": total,
            "strata": dict(strata),
            "attribution": {k: dict(v) for k, v in by_dimension.items()},
        }, indent=2))
        return 0

    print(f"{calls:,} billed calls under {args.root}")
    print("BTE = base-token equivalent; weights " + ", ".join(
        f"{k} x{v}" for k, v in WEIGHTS.items()
    ))
    print(f"\n{'tier':<44}{'raw tokens':>18}{'BTE':>18}{'share':>9}")
    for name in WEIGHTS:
        cost = tiers[name] * WEIGHTS[name]
        print(f"{name:<44}{tiers[name]:>18,}{cost:>18,.0f}{100 * cost / total:>8.1f}%")
    print(f"{'TOTAL':<44}{sum(tiers[k] for k in WEIGHTS):>18,}{total:>18,.0f}")

    report("BY STRATUM", dict(strata), stratum_calls, args.top)
    report("BY PROJECT", dict(projects), project_calls, args.top)
    for name in args.by or sorted(DIMENSIONS):
        report(
            f"BY {name.upper()} ({DIMENSIONS[name]})",
            dict(by_dimension[name]),
            dimension_calls[name],
            args.top,
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
