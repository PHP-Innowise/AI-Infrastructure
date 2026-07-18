#!/usr/bin/env python3
"""Evaluate whether a repository skill triggers for positive and negative queries."""

from __future__ import annotations

import argparse
import json
import sys
from concurrent.futures import ProcessPoolExecutor, as_completed
from pathlib import Path

from scripts.platform_cli import evaluate_trigger, find_project_root, platform_config
from scripts.utils import parse_skill_md


def run_single_query(
    query: str,
    skill_name: str,
    skill_description: str,
    timeout: int,
    project_root: str,
    model: str | None = None,
) -> bool:
    """Run one isolated native CLI probe and report whether the skill body loaded."""
    return evaluate_trigger(
        query=query,
        skill_name=skill_name,
        description=skill_description,
        timeout=timeout,
        project_root=Path(project_root),
        model=model,
    )


def run_eval(
    eval_set: list[dict],
    skill_name: str,
    description: str,
    num_workers: int,
    timeout: int,
    project_root: Path,
    runs_per_query: int = 1,
    trigger_threshold: float = 0.5,
    model: str | None = None,
) -> dict:
    """Run the complete evaluation set and return aggregate trigger results."""
    if not 0 < trigger_threshold <= 1:
        raise ValueError("trigger_threshold must be greater than 0 and at most 1")
    if runs_per_query < 1:
        raise ValueError("runs_per_query must be at least 1")
    if num_workers < 1:
        raise ValueError("num_workers must be at least 1")

    query_triggers: dict[str, list[bool]] = {}
    query_items: dict[str, dict] = {}

    with ProcessPoolExecutor(max_workers=num_workers) as executor:
        future_to_item = {}
        for item in eval_set:
            query = item.get("query")
            should_trigger = item.get("should_trigger")
            if not isinstance(query, str) or not query.strip():
                raise ValueError("Every eval item requires a non-empty string query")
            if not isinstance(should_trigger, bool):
                raise ValueError("Every eval item requires a boolean should_trigger")
            if query in query_items:
                raise ValueError(f"Duplicate eval query: {query}")
            query_items[query] = item
            query_triggers[query] = []
            for _ in range(runs_per_query):
                future = executor.submit(
                    run_single_query,
                    query,
                    skill_name,
                    description,
                    timeout,
                    str(project_root),
                    model,
                )
                future_to_item[future] = item

        for future in as_completed(future_to_item):
            item = future_to_item[future]
            try:
                query_triggers[item["query"]].append(future.result())
            except Exception as error:
                raise RuntimeError(f"Trigger probe failed for {item['query']!r}") from error

    results = []
    for query, triggers in query_triggers.items():
        item = query_items[query]
        trigger_rate = sum(triggers) / len(triggers)
        expected = item["should_trigger"]
        passed = trigger_rate >= trigger_threshold if expected else trigger_rate < trigger_threshold
        results.append(
            {
                "query": query,
                "should_trigger": expected,
                "trigger_rate": trigger_rate,
                "triggers": sum(triggers),
                "runs": len(triggers),
                "pass": passed,
            }
        )

    passed_count = sum(1 for result in results if result["pass"])
    return {
        "platform": platform_config().key,
        "skill_name": skill_name,
        "description": description,
        "results": results,
        "summary": {
            "total": len(results),
            "passed": passed_count,
            "failed": len(results) - passed_count,
        },
    }


def main() -> None:
    config = platform_config()
    parser = argparse.ArgumentParser(
        description=f"Run {config.display_name} trigger evaluation for a skill description"
    )
    parser.add_argument("--eval-set", required=True, help="Path to eval set JSON")
    parser.add_argument("--skill-path", required=True, help="Path to the skill directory")
    parser.add_argument("--description", default=None, help="Override description to test")
    parser.add_argument("--num-workers", type=int, default=4, help="Parallel native CLI workers")
    parser.add_argument("--timeout", type=int, default=60, help="Timeout per query in seconds")
    parser.add_argument("--runs-per-query", type=int, default=3, help="Runs per query")
    parser.add_argument("--trigger-threshold", type=float, default=0.5, help="Trigger-rate threshold")
    parser.add_argument("--model", default=None, help=f"Optional {config.display_name} model")
    parser.add_argument("--verbose", action="store_true", help="Print progress to stderr")
    args = parser.parse_args()

    skill_path = Path(args.skill_path).resolve()
    if not (skill_path / "SKILL.md").is_file():
        parser.error(f"No SKILL.md found at {skill_path}")

    try:
        eval_set = json.loads(Path(args.eval_set).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        parser.error(f"Cannot read eval set: {error}")
    if not isinstance(eval_set, list) or not eval_set:
        parser.error("Eval set must be a non-empty JSON array")

    name, original_description, _ = parse_skill_md(skill_path)
    description = args.description or original_description
    project_root = find_project_root()
    output = run_eval(
        eval_set=eval_set,
        skill_name=name,
        description=description,
        num_workers=args.num_workers,
        timeout=args.timeout,
        project_root=project_root,
        runs_per_query=args.runs_per_query,
        trigger_threshold=args.trigger_threshold,
        model=args.model,
    )

    if args.verbose:
        summary = output["summary"]
        print(f"Results: {summary['passed']}/{summary['total']} passed", file=sys.stderr)
        for result in output["results"]:
            status = "PASS" if result["pass"] else "FAIL"
            print(
                f"  [{status}] {result['triggers']}/{result['runs']} "
                f"expected={result['should_trigger']}: {result['query'][:70]}",
                file=sys.stderr,
            )

    print(json.dumps(output, indent=2))


if __name__ == "__main__":
    main()
