#!/usr/bin/env python3
"""Validate the record of the adversarial review a plan must survive.

A deterministic gate can prove that a contract is well formed, that its evidence
resolves, and that its obligations are wired to steps. It cannot ask whether the
skill would actually be picked for a request nobody wrote down yet, whether it
survives having its nouns removed, or whether the command it prescribes really
does what the plan says. Those need a reader.

What a gate *can* do is refuse to accept the reader's word for having looked.
This validator holds the review record against the plan: every selected skill
reviewed, every dimension answered, every prompt written without naming its own
answer, and every prescribed command actually run rather than judged from the
page - which is the one thing that separated the judge who found a broken
verification from the judge who did not.

Invariants shared with the other gates here: standard library only, no
execution, no network, byte-stable JSON output, fail-closed.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

# The dimensions a review must answer for every selected skill. Each is a
# question the deterministic gate provably cannot ask.
REQUEST_DIMENSIONS = (
    ("unseen_positive_request", "a request this skill should win that nobody has written before"),
    ("sibling_request", "a request its nearest sibling should win instead"),
    ("ambiguous_request", "a request that could go either way"),
    ("cross_domain_request", "a request spanning this skill and another subject"),
)
JUDGED_DIMENSIONS = (
    ("catalog_role_coverage", "whether the procedure carries the candidate's obligations"),
    ("refusal_branch", "whether the skill refuses what it must refuse"),
    ("verification_realism", "whether the prescribed checks do what the plan claims"),
    ("identity_erasure", "whether the contract stays distinguishable with its nouns removed"),
)
VERDICTS = {"pass", "fail"}
REQUEST_FIELDS = {"verdict", "prompt", "note"}
JUDGED_FIELDS = {"verdict", "note"}
REALISM_FIELDS = {"verdict", "note", "executed"}
RUNTIME_FIXED_KIND = "runtime-fixed"


@dataclass(frozen=True)
class Diagnostic:
    severity: str
    code: str
    message: str


def _diag(
    diagnostics: list[Diagnostic], code: str, message: str, severity: str = "error"
) -> None:
    diagnostics.append(Diagnostic(severity, code, message))


def _is_nonempty_string(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def _load(path: Path, label: str, diagnostics: list[Diagnostic]) -> dict | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        _diag(diagnostics, "REVIEW_UNREADABLE", f"{label} is not readable JSON: {error}")
        return None
    if not isinstance(value, dict):
        _diag(diagnostics, "REVIEW_UNREADABLE", f"{label} root must be an object")
        return None
    return value


def _names_its_answer(prompt: str, skill: str) -> bool:
    """Whether a fixture prompt gives away the skill it expects to win."""
    spoken = skill.replace("-", " ").replace("_", " ").lower()
    said = prompt.replace("-", " ").replace("_", " ").lower()
    if spoken in said:
        return True
    words = {word for word in spoken.split() if len(word) >= 3}
    return bool(words) and words <= {
        token
        for token in "".join(
            character if character.isalnum() else " " for character in said
        ).split()
        if len(token) >= 3
    }


def _skill_commands(skill: dict) -> set[str]:
    return {
        str(check["command"]).strip()
        for check in skill.get("verification") or []
        if isinstance(check, dict) and _is_nonempty_string(check.get("command"))
    }


def validate(plan_path: Path, review_path: Path) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []
    plan = _load(plan_path, plan_path.name, diagnostics)
    review = _load(review_path, review_path.name, diagnostics)
    if plan is None or review is None:
        return diagnostics
    if set(review) != {"plan", "reviewer", "skills", "blockers"}:
        _diag(
            diagnostics,
            "REVIEW_INVALID",
            "review must define exactly plan, reviewer, skills, and blockers",
        )
        return diagnostics
    if review.get("reviewer") != "independent":
        _diag(
            diagnostics,
            "REVIEW_NOT_INDEPENDENT",
            "the review must be carried out by a reader that did not author the "
            "plan; a contract's author is the worst judge of whether it is "
            "distinguishable from a template",
        )
    blockers = review.get("blockers")
    if not isinstance(blockers, list) or any(
        not _is_nonempty_string(item) for item in blockers
    ):
        _diag(diagnostics, "REVIEW_INVALID", "blockers must be an array of statements")
        blockers = []
    for blocker in blockers:
        _diag(
            diagnostics,
            "REVIEW_BLOCKER_OPEN",
            f"the review left a blocker open: {blocker}",
        )

    planned = {
        str(skill["name"]).strip(): skill
        for skill in plan.get("skills") or []
        if isinstance(skill, dict) and _is_nonempty_string(skill.get("name"))
    }
    reviewed: set[str] = set()
    entries = review.get("skills")
    if not isinstance(entries, list):
        _diag(diagnostics, "REVIEW_INVALID", "skills must be an array")
        entries = []
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict) or not _is_nonempty_string(entry.get("name")):
            _diag(
                diagnostics,
                "REVIEW_INVALID",
                f"skills[{index}] must name the skill it reviews",
            )
            continue
        name = str(entry["name"]).strip()
        if name not in planned:
            _diag(
                diagnostics,
                "REVIEW_SKILL_UNKNOWN",
                f"the review covers {name}, which the plan does not select",
            )
            continue
        if name in reviewed:
            _diag(
                diagnostics,
                "REVIEW_SKILL_DUPLICATE",
                f"the review covers {name} more than once",
            )
            continue
        reviewed.add(name)
        expected = {"name"} | {key for key, _ in REQUEST_DIMENSIONS} | {
            key for key, _ in JUDGED_DIMENSIONS
        }
        if set(entry) != expected:
            _diag(
                diagnostics,
                "REVIEW_DIMENSION_MISSING",
                f"{name} was not answered on every dimension: "
                f"{', '.join(sorted(expected - set(entry))) or 'unexpected members'}",
            )
            continue
        for key, question in REQUEST_DIMENSIONS + JUDGED_DIMENSIONS:
            answer = entry[key]
            fields = REQUEST_FIELDS if key in dict(REQUEST_DIMENSIONS) else JUDGED_FIELDS
            if key == "verification_realism":
                fields = REALISM_FIELDS
            if (
                not isinstance(answer, dict)
                or set(answer) != fields
                or answer.get("verdict") not in VERDICTS
                or not _is_nonempty_string(answer.get("note"))
                or ("prompt" in fields and not _is_nonempty_string(answer.get("prompt")))
            ):
                _diag(
                    diagnostics,
                    "REVIEW_ANSWER_INVALID",
                    f"{name}.{key} must record a verdict and a note on {question}",
                )
                continue
            if answer["verdict"] == "fail":
                _diag(
                    diagnostics,
                    "REVIEW_VERDICT_FAILED",
                    f"{name} failed review on {key}: {answer['note']}",
                )
            if "prompt" in fields and _names_its_answer(str(answer["prompt"]), name):
                _diag(
                    diagnostics,
                    "REVIEW_PROMPT_TAUTOLOGICAL",
                    f"{name}.{key} names the skill it expects to win, so it "
                    "tests nothing the plan did not already assert",
                )
        realism = entry.get("verification_realism")
        if isinstance(realism, dict) and set(realism) == REALISM_FIELDS:
            executed = realism.get("executed")
            if not isinstance(executed, list) or any(
                not _is_nonempty_string(item) for item in executed
            ):
                _diag(
                    diagnostics,
                    "REVIEW_ANSWER_INVALID",
                    f"{name}.verification_realism must list the commands it ran",
                )
                continue
            prescribed = _skill_commands(planned[name])
            ran = {str(item).strip() for item in executed}
            unrun = sorted(prescribed - ran)
            if unrun:
                _diag(
                    diagnostics,
                    "REVIEW_VERIFICATION_NOT_EXECUTED",
                    f"{name} prescribes commands the review judged from the page "
                    f"rather than ran: {', '.join(unrun[:4])}",
                )
            invented = sorted(ran - prescribed)
            if invented:
                _diag(
                    diagnostics,
                    "REVIEW_VERIFICATION_UNPLANNED",
                    f"{name} was reviewed against commands the plan does not "
                    f"prescribe: {', '.join(invented[:4])}",
                )
    for name in sorted(set(planned) - reviewed):
        _diag(
            diagnostics,
            "REVIEW_SKILL_MISSING",
            f"{name} was selected but never reviewed",
        )
    return diagnostics


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", required=True)
    parser.add_argument("--review", required=True)
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    diagnostics = validate(Path(args.plan), Path(args.review))
    errors = [item for item in diagnostics if item.severity == "error"]
    if args.as_json:
        print(
            json.dumps(
                {
                    "valid": not errors,
                    "diagnostics": [
                        {
                            "severity": item.severity,
                            "code": item.code,
                            "message": item.message,
                        }
                        for item in diagnostics
                    ],
                },
                indent=2,
                sort_keys=True,
            )
        )
    else:
        for item in diagnostics:
            stream = sys.stderr if item.severity == "error" else sys.stdout
            print(f"{item.severity.upper()}: {item.code}: {item.message}", file=stream)
        if not diagnostics:
            print("plan review OK")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
