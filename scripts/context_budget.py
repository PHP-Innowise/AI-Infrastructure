#!/usr/bin/env python3
"""Measure the context budget of every edition.

An agent that opens an edition pays a fixed price before doing any work —
the edition's AGENTS.md, plus a one-line summary of everything the tool
lists as available: skills, commands and agents — and a second, much larger
price each time it actually invokes a skill and the body loads. This script
measures both, per edition:

  category            what is counted                          when paid
  ------------------  ---------------------------------------  ----------
  agents_md_bytes     UTF-8 bytes of <edition>/AGENTS.md       startup
  descriptor_bytes    `name` + `description` values of every   startup
                      canon SKILL.md - the text skill
                      selection actually reads, without the
                      YAML carrying it
  command_bytes       file stem + description value of every   startup
                      .claude/commands/*.md
  agent_bytes         file stem + description value of every   startup
                      .claude/agents/*-agent.md
  frontmatter_bytes   all YAML between the two `---`           startup
                      delimiters, keys included: a file fact,
                      not a rendered one
  body_bytes          everything after the closing `---`       on invocation
  skills              number of SKILL.md files parsed          -

Commands and agents were unmeasured until they were measured once by hand,
and together they were 62 % of Laravel's real startup surface — the agent
roster alone was three times the gated skill descriptors, because two thirds
of each agent `description` was `<example>` blocks. They are gated here so
that stays visible.

A skill carrying `disable-model-invocation: true` is excluded from
descriptor_bytes: Claude Code does not put such a skill's description in
context at all, so counting it would over-state what a session pays. It still
counts toward frontmatter_bytes and body_bytes, which are file facts.

Commands and agents normally have no `description` in frontmatter; Claude
Code then derives one from the first paragraph of the body, and so does this
script. That derivation was checked against the repository's own mirror
generator, which implements the same rule for the Cursor mirrors: 43 of 45
Laravel commands matched byte for byte, the two exceptions being the pinned
description_overrides in MIRROR_RULES.

Which tool these numbers describe
---------------------------------
command_bytes and agent_bytes are measured on the `.claude` tree, and that is
the richest of the three surfaces, so gating it bounds the others rather than
tracking each:

  Claude Code  pays all four startup categories.
  Cursor       same file counts in `.cursor/commands` and `.cursor/agents`;
               a few files are deliberately condensed for Cursor, so the
               bytes differ slightly from the gated figure either way.
  Codex        pays NEITHER. `.codex/` carries only config.toml, the
               governance documents, hooks and hooks.json - there is no
               commands or agents directory, and agent-forge forbids writing
               one. A Codex session's startup surface is AGENTS.md plus the
               skill descriptors, so for Codex these two categories
               over-state the cost by their whole value.

What is counted is the text a tool renders, not the file that carries it: the
value of a frontmatter field with the key, the colon, any surrounding quotes
and that scalar's backslash escapes removed, and continuation lines folded to
single spaces as YAML folds a wrapped scalar. So `descriptor_bytes` counts the
skill's name and description as the model reads them, and never the YAML
around them. frontmatter_bytes is the exception on purpose - it is a file
fact, keys included, and is gated as one.

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
    "command_bytes",
    "agent_bytes",
    "frontmatter_bytes",
    "body_bytes",
    "skills",
)
# The categories a session pays before any work happens.
STARTUP_CATEGORIES = ("agents_md_bytes", "descriptor_bytes", "command_bytes",
                      "agent_bytes")

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
# The three listing ratios below were re-measured on this repository once the
# categories began counting rendered values instead of whole YAML entries -
# the research figure of 4.99 for descriptors was taken on text that still
# carried `name:` and `description:`, so it no longer describes what is
# counted. Method: payload to one file, code2prompt cl100k, an empty file of
# the same name subtracted to remove the template wrapper.
#
#   descriptor  the four editions   41,387 B = 8,421 t   (4.78-5.05 range)
#   command     the four editions   19,336 B = 3,790 t   (5.06-5.15 range)
#   agent       the four editions   36,182 B = 7,036 t   (5.02-5.24 range)
#
# All three are prose-with-identifiers and tokenize alike, which is why they
# cluster and sit well away from the body ratio.
BYTES_PER_TOKEN = {
    "agents_md_bytes": 4.76,
    "descriptor_bytes": 4.91,
    "command_bytes": 5.10,
    "agent_bytes": 5.14,
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


def field_value(frontmatter: str, key: str) -> str | None:
    """Return what the tool renders for `key`: the value, no YAML syntax.

    The key, the colon, quotes around a quoted scalar and that scalar's
    backslash escapes are all YAML, not text the model is shown. Continuation
    lines are folded to single spaces, as YAML folds a wrapped plain scalar.
    """
    block = field_block(frontmatter, key)
    if block is None:
        return None
    value = block[len(key) + 1:]                       # drop `key:`
    value = " ".join(part.strip() for part in value.split("\n")).strip()
    if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
        quote, value = value[0], value[1:-1]
        if quote == '"':
            for esc, char in (("\\n", "\n"), ("\\t", "\t"),
                              ('\\"', '"'), ("\\\\", "\\")):
                value = value.replace(esc, char)
    return value


def descriptor_value(frontmatter: str, path: Path) -> str:
    """Return the `name` + `description` text skill selection reads.

    Everything else in the frontmatter (phase, flow-next, related) is
    orchestration metadata the model does not match against, so gating the
    full frontmatter over-states the surface that actually drives selection.
    The YAML around the two values is stripped for the same reason: the model
    is shown the values, not the syntax carrying them.
    """
    description = field_value(frontmatter, "description")
    if description is None:
        raise BudgetError(f"{path}: frontmatter has no description field")
    name = field_value(frontmatter, "name")
    if name is None:
        raise BudgetError(f"{path}: frontmatter has no name field")
    return f"{name}\n{description}"


def first_paragraph(body: str) -> str:
    """The description Claude Code derives when frontmatter declares none.

    Headings are skipped; the first run of non-blank lines is the paragraph.
    """
    para: list[str] = []
    for line in body.split("\n"):
        stripped = line.strip()
        if not stripped:
            if para:
                break
            continue
        if stripped.startswith("#"):
            if para:
                break
            continue
        para.append(stripped)
    return " ".join(para)


def listing_bytes(paths: list[Path]) -> tuple[int, int]:
    """Bytes of the `name + description` listing for commands or agents.

    Returns (bytes, count). The name is the file stem, which is what the tool
    lists; the description is the frontmatter field when present and the first
    body paragraph otherwise.

    Callers pass the `.claude` tree: it is the richest surface, so gating it
    bounds Cursor's and over-states Codex's, which has no commands or agents
    at all. See the module docstring.
    """
    total = 0
    for path in sorted(paths):
        text = path.read_text(encoding="utf-8")
        try:
            frontmatter, body = split_skill(text, path.relative_to(ROOT))
        except BudgetError:
            frontmatter, body = "", text
        description = field_value(frontmatter, "description")
        if description is None:
            description = first_paragraph(body)
        total += len(f"{path.stem}\n{description}".encode("utf-8"))
    return total, len(paths)


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
        # A user-invoked-only skill is never listed to the model, so its
        # descriptor is not part of what a session pays.
        if "disable-model-invocation: true" not in frontmatter:
            descriptor_bytes += len(descriptor.encode("utf-8"))
        skills += 1
    if skills == 0:
        raise BudgetError(f"{edition}: no */SKILL.md found under {skills_dir}")

    command_bytes, _ = listing_bytes(list((edition_dir / ".claude" / "commands").glob("*.md")))
    agent_bytes, _ = listing_bytes(list((edition_dir / ".claude" / "agents").glob("*-agent.md")))

    return {
        "agents_md_bytes": agents_md.stat().st_size,
        "descriptor_bytes": descriptor_bytes,
        "command_bytes": command_bytes,
        "agent_bytes": agent_bytes,
        "frontmatter_bytes": frontmatter_bytes,
        "body_bytes": body_bytes,
        "skills": skills,
    }


def tokens(nbytes: int, category: str) -> int:
    return round(nbytes / BYTES_PER_TOKEN[category])


def fmt_bytes(nbytes: int, category: str) -> str:
    return f"{nbytes} B (~{tokens(nbytes, category)} t)"


def startup_tokens(m: dict) -> int:
    """What loads before any work: AGENTS.md plus every listing the tool shows.

    Not frontmatter_bytes: the model is shown `name` + `description`, not the
    orchestration keys beside them, so the full frontmatter over-states this
    by roughly 1.5x. frontmatter_bytes stays gated as a file fact.
    """
    return sum(tokens(m[cat], cat) for cat in STARTUP_CATEGORIES)


STARTUP_LABELS = {
    "agents_md_bytes": "AGENTS.md",
    "descriptor_bytes": "skill descriptors",
    "command_bytes": "command listing",
    "agent_bytes": "agent listing",
}


def report(measurements: dict[str, dict]) -> None:
    headers = ("Edition", "Skills") + tuple(STARTUP_LABELS.values()) + (
        "Startup total", "Bodies (on invocation)")
    rows = [headers]
    total = {cat: 0 for cat in CATEGORIES}
    for edition, m in measurements.items():
        rows.append((edition, str(m["skills"]))
                    + tuple(fmt_bytes(m[cat], cat) for cat in STARTUP_CATEGORIES)
                    + (f"~{startup_tokens(m)} t",
                       fmt_bytes(m["body_bytes"], "body_bytes")))
        for cat in CATEGORIES:
            total[cat] += m[cat]
    rows.append(("ALL EDITIONS (monorepo checkout)", str(total["skills"]))
                + tuple(fmt_bytes(total[cat], cat) for cat in STARTUP_CATEGORIES)
                + (f"~{startup_tokens(total)} t",
                   fmt_bytes(total["body_bytes"], "body_bytes")))

    widths = [max(len(r[i]) for r in rows) for i in range(len(headers))]
    print("Context budget (bytes; token estimates calibrated per class, see "
          "BYTES_PER_TOKEN)")
    print()
    for n, row in enumerate(rows):
        print("  ".join(cell.ljust(widths[i]) for i, cell in enumerate(row)).rstrip())
        if n == 0:
            print("  ".join("-" * w for w in widths))
    print()
    print("Startup total = AGENTS.md plus every listing the tool shows the model:")
    print("skill descriptors, commands and agents. All of it is paid on every")
    print("session of that edition, whether or not any of it is used - which is")
    print("why a rarely-invoked skill or agent is expensive rather than cheap.")
    print("Bodies are paid per invocation instead, an order of magnitude larger,")
    print("which is why they are budgeted separately rather than ignored.")
    print()
    print("frontmatter_bytes is gated too but not shown here: it is a superset of")
    print("the descriptors including orchestration keys the model is never shown.")
    print()
    print("The command and agent listings are measured on the .claude tree, the")
    print("richest of the three: Cursor's differ slightly, and Codex has neither")
    print("directory at all, so a Codex session pays only the first two columns.")
    print()
    print("The last row is what a monorepo checkout exposes, where every")
    print("edition's listings are visible at once. A consuming project")
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


def headroom(measurements: dict[str, dict]) -> int:
    """Print what is left under each ceiling, largest pressure first.

    `--check` answers pass or fail and nothing else, so a rule author about to
    add a paragraph to a policy file could not see how much room was left. A
    procedural pillar that can only add rules needs that number in front of
    whoever is adding one.

    No warning threshold is printed on purpose. `token_budget.json` sets every
    ceiling at the observed value plus about five per cent, so headroom is
    ~4.8 % of the ceiling by construction and a percentage warning would fire
    on every category at once, which is the same as no warning at all.
    """
    try:
        ceilings = json.loads(BUDGET_FILE.read_text(encoding="utf-8"))["editions"]
    except (OSError, ValueError, KeyError) as exc:
        print(f"cannot read ceilings from {BUDGET_FILE}: {exc}", file=sys.stderr)
        return 1

    rows = []
    for edition, measured in measurements.items():
        for category, ceiling in ceilings.get(edition, {}).items():
            value = measured.get(category)
            if not isinstance(value, int) or not isinstance(ceiling, int):
                continue
            rows.append((ceiling - value, edition, category, value, ceiling))
    rows.sort()
    print(f"{'remaining':>10}  {'measured':>9}  {'ceiling':>9}  category")
    for remaining, edition, category, value, ceiling in rows:
        print(
            f"{remaining:>10}  {value:>9}  {ceiling:>9}  {edition}.{category}"
        )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true",
                        help="compare against scripts/token_budget.json ceilings; "
                             "exit 1 on any excess")
    parser.add_argument("--headroom", action="store_true",
                        help="print bytes remaining under each ceiling, "
                             "tightest first; always exits 0")
    args = parser.parse_args()

    try:
        measurements = {edition: measure_edition(edition) for edition in EDITIONS}
    except BudgetError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.check:
        return check(measurements)
    if args.headroom:
        return headroom(measurements)
    report(measurements)
    return 0


if __name__ == "__main__":
    sys.exit(main())
