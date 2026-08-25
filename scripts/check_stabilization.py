#!/usr/bin/env python3
"""Structural validator for the stabilization rule blocks.

`STABILIZATION.md` describes how a defect becomes an enforceable rule, and its
own `## Rule Template` says which fields a rule carries. Nothing checked that
the rules obeyed it: the contract rested entirely on the discipline of whoever
wrote the last one, which is exactly the kind of unverified convention this
repository gates everywhere else.

What is checked, and only what actually exists in the template:

* every `### Rule:` block carries `Trigger`, `Root cause`, `Rule`, `Example`,
  `Enforcement`;
* `Rule:` states an obligation - it has to contain MUST or MUST NOT, because a
  rule that merely describes something is not enforceable;
* `Enforcement:` is not empty, and names something other than the rule itself;
* `Added:` / `Retired:`, where present, are ISO dates, and `Retired:` implies
  the block sits under `## Retired rules` - a retired rule left among the live
  ones is still policy in every reader's eyes;
* `Superseded-by:` names another rule in the same file;
* `Evidence:`, where present, resolves to a Project Brain record on disk.

`Evidence` and `Added` are optional on purpose. Making them mandatory would
invalidate every rule written before they existed, and the repository has a
standing preference for a field that is checked when present over a migration
that turns working documents red.

Editions whose `STABILIZATION.md` carries no `### Rule:` block at all are
skipped and reported as skipped: Infrastructure-Creator deliberately writes the
cycle as prose rather than as rule blocks, and silently passing it would be
indistinguishable from validating it.

Exit status: 0 when every rule is well-formed, 1 on any finding, 2 on a usage
error. Standard library only; read-only.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import date
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
EDITIONS = ("Laravel", "Symfony", "PHP Core", "Infrastructure-Creator")

RULE_HEADING = re.compile(r"^### Rule:\s*(?P<name>.+?)\s*$", re.M)
SECTION_HEADING = re.compile(r"^## (?P<name>.+?)\s*$", re.M)
# `[ \t]*` and not `\s*`: `\s` matches a newline, so an empty field would
# swallow the line break and adopt the next line as its value - which is
# exactly the case worth catching, an Enforcement that names nothing.
FIELD = re.compile(r"^\*\*(?P<field>[A-Za-z][A-Za-z -]*):\*\*[ \t]*(?P<value>.*)$", re.M)

REQUIRED_FIELDS = ("Trigger", "Root cause", "Rule", "Example", "Enforcement")
DATE_FIELDS = ("Added", "Retired")
RETIRED_SECTION = "Retired rules"
# The template block itself is an example, not a rule.
TEMPLATE_NAME = "[Short Name]"
OBLIGATION = re.compile(r"\bMUST(?:\s+NOT)?\b")
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")

BRAIN_RECORD_DIRECTORIES = ("dynamic", "archive")


class Finding(dict):
    """One problem, as {path, rule, message}."""


def sections(text: str) -> list[tuple[str, int, int]]:
    """(name, start, end) for every `## ` section, in file order."""
    marks = [(match.group("name"), match.start()) for match in SECTION_HEADING.finditer(text)]
    spans = []
    for index, (name, start) in enumerate(marks):
        end = marks[index + 1][1] if index + 1 < len(marks) else len(text)
        spans.append((name, start, end))
    return spans


def section_of(spans: list[tuple[str, int, int]], position: int) -> str:
    for name, start, end in spans:
        if start <= position < end:
            return name
    return ""


def rule_blocks(text: str) -> list[tuple[str, int, str]]:
    """(name, offset, body) for every rule block, template excluded."""
    marks = list(RULE_HEADING.finditer(text))
    blocks = []
    for index, match in enumerate(marks):
        name = match.group("name")
        if name == TEMPLATE_NAME:
            continue
        end = marks[index + 1].start() if index + 1 < len(marks) else len(text)
        blocks.append((name, match.start(), text[match.end() : end]))
    return blocks


def brain_record_exists(edition_root: Path, identifier: str) -> bool:
    brain = edition_root / "project-brain"
    for directory in BRAIN_RECORD_DIRECTORIES:
        base = brain / directory
        if not base.is_dir():
            continue
        if any(base.rglob(f"{identifier}.md")):
            return True
    return False


def check_rule(
    edition_root: Path,
    relative: str,
    name: str,
    body: str,
    section: str,
    known_rules: set[str],
) -> list[Finding]:
    findings: list[Finding] = []

    def report(message: str) -> None:
        findings.append(Finding(path=relative, rule=name, message=message))

    fields = {
        match.group("field").strip(): match.group("value").strip()
        for match in FIELD.finditer(body)
    }
    for required in REQUIRED_FIELDS:
        if required not in fields:
            report(f"missing required field {required}")

    statement = fields.get("Rule", "")
    if statement and not OBLIGATION.search(statement):
        report("Rule must state an obligation (MUST or MUST NOT)")

    enforcement = fields.get("Enforcement", "")
    if "Enforcement" in fields and not enforcement:
        report("Enforcement must name where the rule is enforced")

    for field in DATE_FIELDS:
        value = fields.get(field)
        if value is None:
            continue
        try:
            date.fromisoformat(value)
        except ValueError:
            report(f"{field} must be an ISO date (YYYY-MM-DD), got {value!r}")

    retired = "Retired" in fields
    if retired and section != RETIRED_SECTION:
        report(
            f"carries Retired but sits under '{section or 'no section'}'; "
            f"a retired rule belongs under '## {RETIRED_SECTION}'"
        )
    if not retired and section == RETIRED_SECTION:
        report(f"sits under '## {RETIRED_SECTION}' without a Retired date")

    replacement = fields.get("Superseded-by")
    if replacement and replacement not in known_rules:
        report(f"Superseded-by names no rule in this file: {replacement!r}")

    evidence = fields.get("Evidence")
    if evidence:
        if not UUID.match(evidence):
            report(f"Evidence must be a record UUID, got {evidence!r}")
        elif not brain_record_exists(edition_root, evidence):
            report(f"Evidence names no Project Brain record on disk: {evidence}")

    return findings


def check_edition(edition: str, repo_root: Path) -> tuple[list[Finding], int, bool]:
    """Findings, the number of rules checked, and whether the file was skipped."""
    edition_root = repo_root / edition
    path = edition_root / ".claude" / "STABILIZATION.md"
    if not path.is_file():
        return (
            [Finding(path=f"{edition}/.claude/STABILIZATION.md", rule="", message="missing")],
            0,
            False,
        )
    relative = path.relative_to(repo_root).as_posix()
    text = path.read_text(encoding="utf-8")
    blocks = rule_blocks(text)
    if not blocks:
        # Prose-genre document. Reported as skipped rather than passed: a
        # validator that silently approves a file it never examined is the
        # defect it exists to prevent.
        return [], 0, True
    spans = sections(text)
    known = {name for name, _, _ in blocks}
    findings: list[Finding] = []
    for name, offset, body in blocks:
        findings.extend(
            check_rule(
                edition_root, relative, name, body, section_of(spans, offset), known
            )
        )
    return findings, len(blocks), False


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    arguments = parser.parse_args(argv)

    findings: list[Finding] = []
    checked: dict[str, int] = {}
    skipped: list[str] = []
    for edition in EDITIONS:
        if not (REPO_ROOT / edition).is_dir():
            continue
        edition_findings, count, was_skipped = check_edition(edition, REPO_ROOT)
        findings.extend(edition_findings)
        if was_skipped:
            skipped.append(edition)
        else:
            checked[edition] = count

    if arguments.json:
        print(
            json.dumps(
                {"checked": checked, "skipped": skipped, "findings": findings},
                ensure_ascii=False,
            )
        )
    elif findings:
        print(f"Stabilization rule findings ({len(findings)}):")
        for finding in findings:
            location = f"{finding['path']}"
            if finding["rule"]:
                location += f" [{finding['rule']}]"
            print(f"  {location}: {finding['message']}")
    else:
        summary = ", ".join(f"{name}: {count}" for name, count in checked.items())
        print(f"Stabilization rules well-formed ({summary}).")
        for edition in skipped:
            print(f"  skipped {edition}: no rule blocks (prose-genre document)")

    return 1 if findings else 0


if __name__ == "__main__":
    sys.exit(main())
