#!/usr/bin/env python3
"""Check that the plan gate still catches what it exists for, on this plan.

A plan that passes every gate proves one half of the bargain: that the rules are
satisfiable. It says nothing about the other half - whether the rules would have
noticed had the plan been worse. A gate can decay in ways nothing reports: a
rule made conditional on a field that stopped being emitted, a pattern that
stopped matching after a rename, an exemption that widened. Every plan still
passes, and the passing is what a reviewer reads.

So before publication the plan is deliberately damaged, one way at a time, and
the gate must object to each damage by name. Nothing here is a new rule; this is
the existing rules held against a plan that should fail them.

The damages are derived from the plan itself rather than hardcoded, so the check
travels to any target. A damage the plan has no shape for is reported as not
applicable - visibly, because a silently skipped check reads exactly like a
passing one, which is the failure this whole script exists to prevent.

Invariants shared with the other gates here: standard library only, no target
execution, no network, byte-stable JSON output, fail-closed. The mutated plans
are written to a scratch copy of the task directory and never replace the real
one.
"""

from __future__ import annotations

import argparse
import copy
import importlib.util
import json
import shutil
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

QUALITY_GATE = Path(__file__).resolve().parent / "validate_skill_quality.py"


def _load_gate():
    spec = importlib.util.spec_from_file_location(
        "validate_skill_quality_for_mutations", QUALITY_GATE
    )
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load the quality gate at {QUALITY_GATE}")
    module = importlib.util.module_from_spec(spec)
    # Registered before execution: the gate defines dataclasses, and a dataclass
    # in an unregistered module cannot resolve its own annotations.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


@dataclass(frozen=True)
class Diagnostic:
    severity: str
    code: str
    message: str


def _diag(
    diagnostics: list[Diagnostic], code: str, message: str, severity: str = "error"
) -> None:
    diagnostics.append(Diagnostic(severity, code, message))


def _skills(plan: dict) -> list[dict]:
    return [item for item in plan.get("skills") or [] if isinstance(item, dict)]


def _project_skills(plan: dict, gate) -> list[dict]:
    return [
        skill
        for skill in _skills(plan)
        if str(skill.get("kind", "")).lower() != gate.RUNTIME_FIXED_KIND
    ]


def _registry_roles(registry_path: Path) -> dict:
    try:
        registry = json.loads(registry_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    return {
        item["id"]: item.get("roles") or []
        for item in registry.get("candidates") or []
        if isinstance(item, dict) and isinstance(item.get("id"), str)
    }


# ── the damages ─────────────────────────────────────────────────────────────
# Each returns the name of the skill it damaged, or None when the plan has no
# shape for it. Every one of them is a defect a real run produced at least once.


def _drop_catalog_obligation(plan, gate, roles) -> str | None:
    for skill in _project_skills(plan, gate):
        candidate = (skill.get("selection_gate") or {}).get("candidate_id")
        declared = roles.get(candidate) or []
        entries = skill.get("required_procedure_roles") or []
        if declared and len(entries) > 1:
            skill["required_procedure_roles"] = entries[:-1]
            return skill["name"]
    return None


def _collapse_obligations(plan, gate, roles) -> str | None:
    for skill in _project_skills(plan, gate):
        entries = skill.get("required_procedure_roles") or []
        steps = skill.get("procedure_steps") or []
        if len(entries) >= 3 and len(steps) >= 2:
            for entry in entries:
                if isinstance(entry, dict):
                    entry["procedure_step_ids"] = [steps[0]["id"]]
            return skill["name"]
    return None


def _drop_recorded_baseline(plan, gate, roles) -> str | None:
    for skill in _project_skills(plan, gate):
        for check in skill.get("verification") or []:
            if isinstance(check, dict) and check.get("baseline"):
                check["baseline"] = None
                return skill["name"]
    return None


def _promise_absolute_success(plan, gate, roles) -> str | None:
    for skill in _project_skills(plan, gate):
        for check in skill.get("verification") or []:
            baseline = check.get("baseline") if isinstance(check, dict) else None
            if isinstance(baseline, dict) and baseline.get("outcome") == "failing":
                check["expected_result"] = "The command exits zero and reports no errors"
                return skill["name"]
    return None


def _promise_zero_for_a_runtime_command(plan, gate, roles) -> str | None:
    declared = gate._runtime_command_outcomes()
    for skill in _skills(plan):
        if str(skill.get("kind", "")).lower() != gate.RUNTIME_FIXED_KIND:
            continue
        for check in skill.get("verification") or []:
            if not isinstance(check, dict):
                continue
            key = gate._normalize_command_text(str(check.get("command") or "")).strip()
            outcomes = declared.get(key) or []
            if any(entry["exit"] != 0 for entry in outcomes):
                check["expected_result"] = "The command exits zero and reports no error"
                return skill["name"]
    return None


def _name_the_skill_in_its_own_fixture(plan, gate, roles) -> str | None:
    for skill in _project_skills(plan, gate):
        cases = skill.get("routing_cases") or []
        if cases and isinstance(cases[0], dict):
            spoken = skill["name"].replace("-", " ")
            cases[0]["prompt"] = f"use {spoken} to handle this {spoken} request"
            return skill["name"]
    return None


def _drop_claim_references(plan, gate, roles) -> str | None:
    for skill in _project_skills(plan, gate):
        if skill.get("claim_ids"):
            skill["claim_ids"] = []
            return skill["name"]
    return None


def _drop_evidence_dispositions(plan, gate, roles) -> str | None:
    for skill in _project_skills(plan, gate):
        if skill.get("evidence_dispositions"):
            skill["evidence_dispositions"] = []
            return skill["name"]
    return None


MUTATIONS: tuple[tuple[str, str, Callable], ...] = (
    (
        "a catalog obligation is dropped from a skill whose candidate declares them",
        "CATALOG_ROLE_UNCOVERED",
        _drop_catalog_obligation,
    ),
    (
        "every obligation of one skill is discharged by the same single step",
        "PROCEDURE_ROLE_COLLAPSED",
        _collapse_obligations,
    ),
    (
        "an executable check loses the baseline recorded for it",
        "VERIFICATION_BASELINE_MISSING",
        _drop_recorded_baseline,
    ),
    (
        "an expectation promises success against a failing recorded baseline",
        "VERIFICATION_BASELINE_CONTRADICTED",
        _promise_absolute_success,
    ),
    (
        "a runtime check promises zero where the contract declares a nonzero exit",
        "RUNTIME_EXPECTATION_CONTRADICTED",
        _promise_zero_for_a_runtime_command,
    ),
    (
        "a routing fixture names the skill it expects to win",
        "ROUTING_CASE_TAUTOLOGICAL",
        _name_the_skill_in_its_own_fixture,
    ),
    (
        "a skill stops naming the reconciled claims it rests on",
        "SKILL_CLAIM_IDS_INVALID",
        _drop_claim_references,
    ),
    (
        "a skill stops ruling out the evidence inside its own paths",
        "SKILL_EVIDENCE_UNDISPOSED",
        _drop_evidence_dispositions,
    ),
)


def validate(plan_path: Path, target: Path, registry_path: Path) -> list[Diagnostic]:
    diagnostics: list[Diagnostic] = []
    gate = _load_gate()
    try:
        original = json.loads(plan_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        _diag(
            diagnostics,
            "MUTATION_PLAN_UNREADABLE",
            f"plan is not readable JSON: {error}",
        )
        return diagnostics
    roles = _registry_roles(registry_path)

    with tempfile.TemporaryDirectory() as scratch:
        # The plan cites a sibling profile, so the whole task directory travels
        # with it; the real one is never written to.
        workspace = Path(scratch) / plan_path.parent.name
        shutil.copytree(plan_path.parent, workspace)
        probe = workspace / plan_path.name

        def errors_for(plan: dict) -> list[str]:
            probe.write_text(json.dumps(plan, indent=2), encoding="utf-8")
            return sorted(
                {
                    item.code
                    for item in gate.validate_plan(probe, target, registry_path)
                    if item.severity == "error"
                }
            )

        control = errors_for(original)
        if control:
            _diag(
                diagnostics,
                "MUTATION_CONTROL_DIRTY",
                "the unmutated plan does not validate clean, so nothing this "
                f"check reports means anything: {', '.join(control[:4])}",
            )
            return diagnostics

        for description, expected, mutate in MUTATIONS:
            damaged = copy.deepcopy(original)
            touched = mutate(damaged, gate, roles)
            if touched is None:
                _diag(
                    diagnostics,
                    "MUTATION_NOT_APPLICABLE",
                    f"this plan has no shape for the damage where {description}, "
                    "so the rule that guards it is unexercised here",
                    "warning",
                )
                continue
            codes = errors_for(damaged)
            if not codes:
                _diag(
                    diagnostics,
                    "MUTATION_UNCAUGHT",
                    f"{touched}: the gate accepted a plan where {description}",
                )
            elif expected not in codes:
                _diag(
                    diagnostics,
                    "MUTATION_CODE_MISSING",
                    f"{touched}: a plan where {description} was rejected, but not "
                    f"by {expected} - the rule that guards it did not fire "
                    f"({', '.join(codes[:3])})",
                )
    return diagnostics


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--registry", required=True)
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args(argv)
    diagnostics = validate(
        Path(args.plan), Path(args.target).resolve(), Path(args.registry)
    )
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
        if not errors:
            exercised = len(MUTATIONS) - sum(
                1 for item in diagnostics if item.code == "MUTATION_NOT_APPLICABLE"
            )
            print(f"plan mutations OK ({exercised} of {len(MUTATIONS)} exercised)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
