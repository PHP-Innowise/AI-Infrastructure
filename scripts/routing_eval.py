#!/usr/bin/env python3
"""Measure which skill the roster actually selects for a request.

The accelerator's value is realised through choosing among roughly forty-five
skills, and that choice is made by the prose in `description:`. There was no
eval data of any kind: 38 of 41 Laravel agents had their `<example>` blocks
moved out of `description:` on a purely economic argument, with no number
about whether the selection got worse.

Three decisions this script embodies, because the roadmap left them open:

* **The unit is the skill, not the agent.** Agent name equals skill name for
  every entry in the roster, so a name-based detector cannot distinguish
  "invoked the coder skill" from "spawned the coder agent". The skill layer is
  also the only one all three tool integrations share - Codex has no agent
  layer at all - so a skill-level baseline is the only one that describes all
  of them.
* **The capsule stays on.** The prompt hook injects a procedural pointer on
  every turn, so a baseline measured with it disabled would describe a
  configuration nobody runs.
* **Three labels, not two.** `hit`, `miss` (nothing triggered) and `wrong`
  (something else triggered) are separated because they are repaired in
  opposite directions: a miss means the description is too narrow, a wrong
  means it overlaps a neighbour. Collapsing them hides which fix applies.

Each case runs N times because selection is not deterministic; the baseline
records the mean, the standard deviation and the per-case breakdown, keyed by
the edition's `policy_digest` so a stale baseline is mechanically visible.

This is NOT part of CI: it invokes a model, costs money and takes minutes.
Run it deliberately, the way `docs/CI.md` describes for the harness.
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
EDITIONS = ("Laravel", "Symfony", "PHP Core")
DEFAULT_RUNS = 3
DEFAULT_TIMEOUT = 120


class RoutingEvalError(Exception):
    """A usage or environment failure."""


def load_cases(edition: str) -> dict:
    path = REPO_ROOT / edition / ".agents" / "evals" / "routing.json"
    if not path.is_file():
        raise RoutingEvalError(f"no routing eval data at {path}")
    return json.loads(path.read_text(encoding="utf-8"))


def policy_digest(edition: str) -> str:
    """Tie a baseline to the surface that produced it."""
    lock = REPO_ROOT / edition / ".accelerator-policy-lock.json"
    if not lock.is_file():
        return "unknown"
    try:
        return str(json.loads(lock.read_text(encoding="utf-8"))["policy_digest"])
    except (ValueError, KeyError):
        return "unknown"


def selected_skill(
    edition: str, request: str, *, model: str | None, timeout: int
) -> str | None:
    """The skill the model reached for, or None if it reached for none.

    Detection follows the vendored `run_eval.py`: watch the streamed events for
    a tool-use block naming Skill or Read, and take the first skill slug that
    appears in its accumulated arguments. `CLAUDECODE` is removed so this can
    nest inside a Claude Code session.
    """
    command = [
        "claude",
        "-p",
        request,
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
    ]
    if model:
        command.extend(["--model", model])
    environment = {k: v for k, v in os.environ.items() if k != "CLAUDECODE"}

    try:
        process = subprocess.Popen(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            cwd=str(REPO_ROOT / edition),
            env=environment,
            text=True,
        )
    except FileNotFoundError as error:
        raise RoutingEvalError(
            "the `claude` CLI is required; this script invokes a model"
        ) from error

    skills = {
        path.name
        for path in (REPO_ROOT / edition / ".agents" / "skills").iterdir()
        if path.is_dir()
    }
    deadline = time.time() + timeout
    pending = ""
    try:
        for line in process.stdout or []:
            if time.time() > deadline:
                break
            line = line.strip()
            if not line:
                continue
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if event.get("type") != "stream_event":
                continue
            inner = event.get("event", {})
            kind = inner.get("type", "")
            if kind == "content_block_start":
                block = inner.get("content_block", {})
                if block.get("type") == "tool_use" and block.get("name") in (
                    "Skill",
                    "Read",
                ):
                    pending = ""
            elif kind == "content_block_delta":
                delta = inner.get("delta", {})
                if delta.get("type") == "input_json_delta":
                    pending += delta.get("partial_json", "")
                    for slug in skills:
                        if slug in pending:
                            return slug
    finally:
        process.kill()
        if process.stdout:
            process.stdout.close()
    return None


def label(expected: str | None, observed: str | None) -> str:
    """hit / miss / wrong — separated because they are repaired oppositely."""
    if expected is None:
        return "hit" if observed is None else "wrong"
    if observed is None:
        return "miss"
    return "hit" if observed == expected else "wrong"


def evaluate(
    edition: str, *, runs: int, model: str | None, timeout: int, dry_run: bool
) -> dict:
    fixture = load_cases(edition)
    per_case = []
    for case in fixture["cases"]:
        observations = []
        for _ in range(runs):
            if dry_run:
                # Offline shape check: exercises the scoring and the report
                # without invoking a model. It proves the plumbing, never the
                # routing.
                observations.append(case["expect"])
            else:
                observations.append(
                    selected_skill(
                        edition, case["request"], model=model, timeout=timeout
                    )
                )
        labels = [label(case["expect"], observed) for observed in observations]
        per_case.append(
            {
                "request": case["request"],
                "expect": case["expect"],
                "observed": observations,
                "labels": labels,
                "pass_rate": round(labels.count("hit") / runs, 4),
            }
        )
    rates = [case["pass_rate"] for case in per_case]
    return {
        "edition": edition,
        "policy_digest": policy_digest(edition),
        "runs": runs,
        "dry_run": dry_run,
        "cases": len(per_case),
        "pass_rate": round(statistics.fmean(rates), 4) if rates else 0.0,
        "mean": round(statistics.fmean(rates), 4) if rates else 0.0,
        "stddev": round(statistics.pstdev(rates), 4) if len(rates) > 1 else 0.0,
        "labels": {
            name: sum(case["labels"].count(name) for case in per_case)
            for name in ("hit", "miss", "wrong")
        },
        "per_case": per_case,
    }


def baseline_path(edition: str) -> Path:
    slug = {"Laravel": "laravel", "Symfony": "symfony", "PHP Core": "php-core"}[edition]
    return REPO_ROOT / "install" / "policy-lock" / f"{slug}-routing-baseline.json"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--edition", action="append", choices=EDITIONS)
    parser.add_argument("--runs", type=int, default=DEFAULT_RUNS)
    parser.add_argument("--model", default=None)
    parser.add_argument("--timeout", type=int, default=DEFAULT_TIMEOUT)
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="score the fixture offline without invoking a model; proves the "
        "plumbing, never the routing",
    )
    parser.add_argument(
        "--write-baseline",
        action="store_true",
        help="record the result under install/policy-lock/",
    )
    parser.add_argument("--json", action="store_true")
    arguments = parser.parse_args(argv)

    if arguments.runs < 1:
        print("routing_eval: --runs must be positive", file=sys.stderr)
        return 2
    if arguments.write_baseline and arguments.dry_run:
        print(
            "routing_eval: refusing to record a baseline from a dry run",
            file=sys.stderr,
        )
        return 2

    results = {}
    for edition in arguments.edition or EDITIONS:
        try:
            result = evaluate(
                edition,
                runs=arguments.runs,
                model=arguments.model,
                timeout=arguments.timeout,
                dry_run=arguments.dry_run,
            )
        except RoutingEvalError as error:
            print(f"routing_eval: {edition}: {error}", file=sys.stderr)
            return 2
        results[edition] = result
        if arguments.write_baseline:
            path = baseline_path(edition)
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                json.dumps(result, indent=2, ensure_ascii=False) + "\n",
                encoding="utf-8",
            )
        if not arguments.json:
            counts = result["labels"]
            print(
                f"{edition}\tpass {result['pass_rate']}\tstddev {result['stddev']}\t"
                f"hit {counts['hit']} miss {counts['miss']} wrong {counts['wrong']}\t"
                f"policy {result['policy_digest'][:8]}"
                + ("\t(dry run)" if result["dry_run"] else "")
            )
            for case in result["per_case"]:
                if case["pass_rate"] < 1.0:
                    print(
                        f"  {case['pass_rate']:>5}  expect "
                        f"{case['expect'] or 'nothing'!s:<28} got "
                        f"{[o or 'nothing' for o in case['observed']]}"
                    )

    if arguments.json:
        print(json.dumps(results, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
