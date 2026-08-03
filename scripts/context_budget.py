#!/usr/bin/env python3
"""Measure the startup context budget of every edition.

An agent that opens an edition pays a fixed context price before doing any
work: the edition's AGENTS.md plus the frontmatter of every skill in the
canon (`.agents/skills/*/SKILL.md` — all skill descriptors are loaded at
startup, and the `description` field inside them is the trigger text for
skill selection). This script makes that price measurable per edition:

  category            what is counted
  ------------------  ---------------------------------------------------
  agents_md_bytes     UTF-8 bytes of <edition>/AGENTS.md
  frontmatter_bytes   sum of the YAML between the two `---` delimiters of
                      every SKILL.md in the canon (delimiter lines excluded)
  description_bytes   of that, the bytes of the `description` value alone
                      (key name excluded; continuation lines included)
  skills              number of SKILL.md files parsed

Token estimates are bytes / 4 — a deliberate rough heuristic, good enough
to compare editions and catch regressions, not a tokenizer.

Usage:
    python3 scripts/context_budget.py            # human-readable report
    python3 scripts/context_budget.py --check    # compare against ceilings

--check reads scripts/token_budget.json ({"editions": {name: {category:
ceiling}}}), compares every measured value against its ceiling and exits 1
on any excess (or on a budget file that is missing, unreadable, or out of
sync with the editions on disk). Ceilings are the observed values + ~5%
headroom, so the gate catches regressions without flagging normal noise;
after a deliberate slimming, tighten them to the new observed values + ~5%.

Python 3 stdlib only.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUDGET_FILE = ROOT / "scripts" / "token_budget.json"
EDITIONS = ("Laravel", "Symfony", "PHP Core", "Infrastructure-Creator")
CATEGORIES = ("agents_md_bytes", "frontmatter_bytes", "description_bytes", "skills")


class BudgetError(Exception):
    pass


def parse_frontmatter(text: str, path: Path) -> str:
    """Return the YAML between the two '---' delimiters (exclusive)."""
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        raise BudgetError(f"{path}: no frontmatter ('---' expected on line 1)")
    for idx in range(1, len(lines)):
        if lines[idx].strip() == "---":
            return "\n".join(lines[1:idx])
    raise BudgetError(f"{path}: unterminated frontmatter (no closing '---')")


def description_value(frontmatter: str, path: Path) -> str:
    """Return the value of the description field, continuation lines included."""
    lines = frontmatter.split("\n")
    for idx, line in enumerate(lines):
        if not line.startswith("description:"):
            continue
        parts = [line[len("description:"):].strip()]
        for cont in lines[idx + 1:]:
            if cont.strip() and not cont[0].isspace():
                break  # next top-level key
            parts.append(cont.strip())
        return "\n".join(p for p in parts if p)
    raise BudgetError(f"{path}: frontmatter has no description field")


def measure_edition(edition: str) -> dict:
    edition_dir = ROOT / edition
    agents_md = edition_dir / "AGENTS.md"
    if not agents_md.is_file():
        raise BudgetError(f"{edition}: missing AGENTS.md")
    skills_dir = edition_dir / ".agents" / "skills"
    if not skills_dir.is_dir():
        raise BudgetError(f"{edition}: missing canon {skills_dir.relative_to(ROOT)}")

    frontmatter_bytes = 0
    description_bytes = 0
    skills = 0
    for skill_md in sorted(skills_dir.glob("*/SKILL.md")):
        text = skill_md.read_text(encoding="utf-8")
        frontmatter = parse_frontmatter(text, skill_md.relative_to(ROOT))
        frontmatter_bytes += len(frontmatter.encode("utf-8"))
        desc = description_value(frontmatter, skill_md.relative_to(ROOT))
        description_bytes += len(desc.encode("utf-8"))
        skills += 1
    if skills == 0:
        raise BudgetError(f"{edition}: no */SKILL.md found under {skills_dir}")

    return {
        "agents_md_bytes": agents_md.stat().st_size,
        "frontmatter_bytes": frontmatter_bytes,
        "description_bytes": description_bytes,
        "skills": skills,
    }


def tokens(nbytes: int) -> int:
    return round(nbytes / 4)


def fmt_bytes(nbytes: int) -> str:
    return f"{nbytes} B (~{tokens(nbytes)} t)"


def report(measurements: dict[str, dict]) -> None:
    headers = ("Edition", "Skills", "AGENTS.md", "Frontmatter",
               "  of which description", "Startup total")
    rows = [headers]
    total = {cat: 0 for cat in CATEGORIES}
    for edition, m in measurements.items():
        startup = m["agents_md_bytes"] + m["frontmatter_bytes"]
        rows.append((edition, str(m["skills"]), fmt_bytes(m["agents_md_bytes"]),
                     fmt_bytes(m["frontmatter_bytes"]),
                     fmt_bytes(m["description_bytes"]), fmt_bytes(startup)))
        for cat in CATEGORIES:
            total[cat] += m[cat]
    startup = total["agents_md_bytes"] + total["frontmatter_bytes"]
    rows.append(("TOTAL (monorepo root)", str(total["skills"]),
                 fmt_bytes(total["agents_md_bytes"]),
                 fmt_bytes(total["frontmatter_bytes"]),
                 fmt_bytes(total["description_bytes"]), fmt_bytes(startup)))

    widths = [max(len(r[i]) for r in rows) for i in range(len(headers))]
    print("Startup context budget (bytes; ~tokens = bytes / 4)")
    print()
    for n, row in enumerate(rows):
        print("  ".join(cell.ljust(widths[i]) for i, cell in enumerate(row)).rstrip())
        if n == 0:
            print("  ".join("-" * w for w in widths))
    print()
    print("Startup total = AGENTS.md + skill frontmatter (descriptions are part")
    print("of the frontmatter). The monorepo-root row is what an agent sees when")
    print("all editions' skill descriptors are visible at once.")


def check(measurements: dict[str, dict]) -> int:
    try:
        budget = json.loads(BUDGET_FILE.read_text(encoding="utf-8"))
        ceilings = budget["editions"]
    except (OSError, ValueError, KeyError) as exc:
        print(f"cannot read ceilings from {BUDGET_FILE}: {exc}", file=sys.stderr)
        return 1

    failures = 0
    stale = set(ceilings) - set(measurements)
    if stale:
        print(f"FAIL  token_budget.json lists unknown edition(s): {sorted(stale)}")
        failures += 1

    for edition, m in measurements.items():
        if edition not in ceilings:
            print(f"FAIL  {edition}: no ceilings in token_budget.json")
            failures += 1
            continue
        for cat in CATEGORIES:
            if cat not in ceilings[edition]:
                print(f"FAIL  {edition}.{cat}: no ceiling in token_budget.json")
                failures += 1
                continue
            ceiling = ceilings[edition][cat]
            if m[cat] > ceiling:
                print(f"FAIL  {edition}.{cat}: {m[cat]} > ceiling {ceiling}")
                failures += 1
            else:
                print(f"ok    {edition}.{cat}: {m[cat]} <= {ceiling}")

    if failures:
        print(f"\n{failures} budget violation(s). Either trim the startup context")
        print("back under the ceiling, or — for a justified permanent increase —")
        print("raise the ceiling in scripts/token_budget.json in the same change.")
        return 1
    print("\nAll startup context budgets within ceilings.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true",
                        help="compare against scripts/token_budget.json ceilings; "
                             "exit 1 on any excess")
    args = parser.parse_args()

    try:
        measurements = {edition: measure_edition(edition) for edition in EDITIONS}
    except BudgetError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.check:
        return check(measurements)
    report(measurements)
    return 0


if __name__ == "__main__":
    sys.exit(main())
