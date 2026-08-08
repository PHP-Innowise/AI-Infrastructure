#!/usr/bin/env python3
"""Measure the context budget of every edition.

An agent that opens an edition pays a fixed price before doing any work —
the edition's AGENTS.md plus every skill descriptor — and a second, much
larger price each time it actually invokes a skill and the body loads. This
script measures both, per edition:

  category            what is counted                          when paid
  ------------------  ---------------------------------------  ----------
  agents_md_bytes     UTF-8 bytes of <edition>/AGENTS.md       startup
  descriptor_bytes    `name` + `description` of every canon    startup
                      SKILL.md (the text skill selection
                      actually reads; continuation lines
                      included)
  frontmatter_bytes   all YAML between the two `---`           startup
                      delimiters (a superset of the above)
  body_bytes          everything after the closing `---`       on invocation
  skills              number of SKILL.md files parsed          -

Token estimates use per-class bytes-per-token ratios measured with
cl100k on this repository's own files (docs/TOKEN-ECONOMY-RESEARCH.md),
not the flat bytes / 4 this script used to apply — that heuristic runs
19-25 % high on exactly these files, which is the difference between a
gate that reflects spend and one that does not. The ratios are estimates
of a real tokenizer, not a tokenizer: a tokenizer would mean a
third-party dependency, and this script runs in a stdlib-only CI job.

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
CATEGORIES = (
    "agents_md_bytes",
    "descriptor_bytes",
    "frontmatter_bytes",
    "body_bytes",
    "skills",
)

# Bytes per cl100k token, measured on this repository's own files
# (docs/TOKEN-ECONOMY-RESEARCH.md, all figures marked [M] there). Each class
# tokenizes differently, and one flat divisor cannot represent them: prose
# packs more bytes per token than YAML, and skill bodies - which carry code
# blocks, command lines and tables - pack the least.
#
#   agents_md    Laravel AGENTS.md      12,887 B = 2,706 t
#   frontmatter  Laravel, 44 skills     18,446 B = 4,015 t
#   descriptor   Laravel, 44 skills     11,412 B = 2,288 t
#   body         Laravel, 44 skills    ~345,335 B = 81,719 t
#
# The first three are direct measurements of the same quantity this script
# counts. The body ratio pairs the research's token count with the byte
# count of the same file set, so it carries that pairing's drift; it is
# still an order of magnitude closer than bytes / 4.
BYTES_PER_TOKEN = {
    "agents_md_bytes": 4.76,
    "descriptor_bytes": 4.99,
    "frontmatter_bytes": 4.59,
    "body_bytes": 4.23,
}


class BudgetError(Exception):
    pass


def split_skill(text: str, path: Path) -> tuple[str, str]:
    """Return (frontmatter, body): the YAML between the two '---' delimiters
    (exclusive) and everything after the closing delimiter."""
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        raise BudgetError(f"{path}: no frontmatter ('---' expected on line 1)")
    for idx in range(1, len(lines)):
        if lines[idx].strip() == "---":
            return "\n".join(lines[1:idx]), "\n".join(lines[idx + 1:])
    raise BudgetError(f"{path}: unterminated frontmatter (no closing '---')")


def field_block(frontmatter: str, key: str) -> str | None:
    """Return the whole `key:` entry, continuation lines included."""
    lines = frontmatter.split("\n")
    for idx, line in enumerate(lines):
        if not line.startswith(f"{key}:"):
            continue
        parts = [line]
        for cont in lines[idx + 1:]:
            if cont.strip() and not cont[0].isspace():
                break  # next top-level key
            parts.append(cont)
        return "\n".join(parts)
    return None


def descriptor_value(frontmatter: str, path: Path) -> str:
    """Return the `name` + `description` entries: the text skill selection reads.

    Everything else in the frontmatter (phase, flow-next, related) is
    orchestration metadata the model does not match against, so gating the
    full frontmatter over-states the surface that actually drives selection.
    """
    description = field_block(frontmatter, "description")
    if description is None:
        raise BudgetError(f"{path}: frontmatter has no description field")
    name = field_block(frontmatter, "name")
    if name is None:
        raise BudgetError(f"{path}: frontmatter has no name field")
    return f"{name}\n{description}"


def measure_edition(edition: str) -> dict:
    edition_dir = ROOT / edition
    agents_md = edition_dir / "AGENTS.md"
    if not agents_md.is_file():
        raise BudgetError(f"{edition}: missing AGENTS.md")
    skills_dir = edition_dir / ".agents" / "skills"
    if not skills_dir.is_dir():
        raise BudgetError(f"{edition}: missing canon {skills_dir.relative_to(ROOT)}")

    frontmatter_bytes = 0
    descriptor_bytes = 0
    body_bytes = 0
    skills = 0
    for skill_md in sorted(skills_dir.glob("*/SKILL.md")):
        text = skill_md.read_text(encoding="utf-8")
        frontmatter, body = split_skill(text, skill_md.relative_to(ROOT))
        frontmatter_bytes += len(frontmatter.encode("utf-8"))
        body_bytes += len(body.encode("utf-8"))
        descriptor = descriptor_value(frontmatter, skill_md.relative_to(ROOT))
        descriptor_bytes += len(descriptor.encode("utf-8"))
        skills += 1
    if skills == 0:
        raise BudgetError(f"{edition}: no */SKILL.md found under {skills_dir}")

    return {
        "agents_md_bytes": agents_md.stat().st_size,
        "descriptor_bytes": descriptor_bytes,
        "frontmatter_bytes": frontmatter_bytes,
        "body_bytes": body_bytes,
        "skills": skills,
    }


def tokens(nbytes: int, category: str) -> int:
    return round(nbytes / BYTES_PER_TOKEN[category])


def fmt_bytes(nbytes: int, category: str) -> str:
    return f"{nbytes} B (~{tokens(nbytes, category)} t)"


def startup_tokens(m: dict) -> int:
    """Token estimate for what loads before any work: AGENTS.md + frontmatter."""
    return (tokens(m["agents_md_bytes"], "agents_md_bytes")
            + tokens(m["frontmatter_bytes"], "frontmatter_bytes"))


def report(measurements: dict[str, dict]) -> None:
    headers = ("Edition", "Skills", "AGENTS.md", "Frontmatter",
               "  of which descriptor", "Startup total", "Bodies (on invocation)")
    rows = [headers]
    total = {cat: 0 for cat in CATEGORIES}
    for edition, m in measurements.items():
        rows.append((
            edition, str(m["skills"]),
            fmt_bytes(m["agents_md_bytes"], "agents_md_bytes"),
            fmt_bytes(m["frontmatter_bytes"], "frontmatter_bytes"),
            fmt_bytes(m["descriptor_bytes"], "descriptor_bytes"),
            f"~{startup_tokens(m)} t",
            fmt_bytes(m["body_bytes"], "body_bytes"),
        ))
        for cat in CATEGORIES:
            total[cat] += m[cat]
    rows.append((
        "ALL EDITIONS (monorepo checkout)", str(total["skills"]),
        fmt_bytes(total["agents_md_bytes"], "agents_md_bytes"),
        fmt_bytes(total["frontmatter_bytes"], "frontmatter_bytes"),
        fmt_bytes(total["descriptor_bytes"], "descriptor_bytes"),
        f"~{startup_tokens(total)} t",
        fmt_bytes(total["body_bytes"], "body_bytes"),
    ))

    widths = [max(len(r[i]) for r in rows) for i in range(len(headers))]
    print("Context budget (bytes; token estimates calibrated per class, see "
          "BYTES_PER_TOKEN)")
    print()
    for n, row in enumerate(rows):
        print("  ".join(cell.ljust(widths[i]) for i, cell in enumerate(row)).rstrip())
        if n == 0:
            print("  ".join("-" * w for w in widths))
    print()
    print("Startup total = AGENTS.md + skill frontmatter, paid on every session")
    print("of that edition. The descriptor column is the part of the frontmatter")
    print("skill selection actually matches against. Bodies are paid per skill")
    print("invocation, not at startup - they are an order of magnitude larger,")
    print("which is why they are budgeted separately rather than ignored.")
    print()
    print("The last row is what a monorepo checkout exposes, where every")
    print("edition's descriptors are visible at once. A consuming project")
    print("installs ONE edition and pays only that edition's row.")


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
        print(f"\n{failures} budget violation(s). Either trim the context back")
        print("under the ceiling, or — for a justified permanent increase —")
        print("raise the ceiling in scripts/token_budget.json in the same change.")
        return 1
    print("\nAll context budgets within ceilings.")
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
