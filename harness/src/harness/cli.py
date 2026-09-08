"""Command-line entry point.

    harness run --project PATH [--scope TEXT] [--worker dry-run|claude-cli]
    harness resume --project PATH --thread ID --approve|--reject
"""

from __future__ import annotations

import argparse
import datetime as _datetime
import json
import sqlite3
import sys
from pathlib import Path

from langgraph.checkpoint.sqlite import SqliteSaver

from .blackboard import Blackboard
from .config import DEFAULT_LENSES, HarnessConfig
from .graphs.fleet_review import build_graph, resume_command
from .workers import resolve_worker


def _config(arguments: argparse.Namespace) -> HarnessConfig:
    return HarnessConfig(
        project=Path(arguments.project).resolve(),
        worker=arguments.worker,
        lenses=tuple(arguments.lenses.split(",")) if arguments.lenses else DEFAULT_LENSES,
        model=arguments.model,
        budget_usd=arguments.budget_usd,
        worker_timeout_seconds=arguments.worker_timeout,
        state_dir=Path(arguments.state_dir),
        reports_dir=Path(arguments.reports_dir),
    )


def _graph(config: HarnessConfig, saver: SqliteSaver):
    worker = resolve_worker(config.worker)
    return build_graph(config, worker, Blackboard(config.project), saver)


def _emit(state: object) -> None:
    if isinstance(state, dict) and state.get("__interrupt__"):
        payload = state["__interrupt__"][0].value
        print("PAUSED at the approval gate:")
        print(json.dumps(payload, ensure_ascii=False, indent=2))
        print("Resume with: harness resume --thread <ID> --approve|--reject")
        return
    if isinstance(state, dict) and state.get("report"):
        print(state["report"])
    else:
        print("Run finished; report rejected or empty.")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="harness", description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    shared = argparse.ArgumentParser(add_help=False)
    shared.add_argument("--project", required=True, help="target project root")
    shared.add_argument("--worker", default="claude-cli")
    shared.add_argument("--lenses", help="comma-separated roster agent names")
    shared.add_argument("--model")
    shared.add_argument("--budget-usd", type=lambda value: None if value.lower() == 'none' else float(value), default=None,
                        help="Optional USD budget for this graph attempt (Claude/dry-run only); default: none")
    shared.add_argument("--worker-timeout", type=int, default=1800)
    shared.add_argument("--state-dir", default="state")
    shared.add_argument("--reports-dir", default="reports")

    run = commands.add_parser("run", parents=[shared], help="start a fleet review")
    run.add_argument("--scope", default="the current branch's changes against the default branch")
    run.add_argument("--task-id", help="project-brain task id (default: harness/<thread>)")
    run.add_argument("--thread", help="thread id (default: fleet-review-<UTC timestamp>)")

    resume = commands.add_parser("resume", parents=[shared], help="resume a paused run")
    resume.add_argument("--thread", required=True)
    verdict = resume.add_mutually_exclusive_group(required=True)
    verdict.add_argument("--approve", action="store_true")
    verdict.add_argument("--reject", action="store_true")

    arguments = parser.parse_args(argv)
    config = _config(arguments)
    config.state_dir.mkdir(parents=True, exist_ok=True)
    saver = SqliteSaver(
        sqlite3.connect(str(config.checkpoint_path), check_same_thread=False)
    )
    graph = _graph(config, saver)

    if arguments.command == "run":
        thread = arguments.thread or "fleet-review-" + _datetime.datetime.now(
            _datetime.timezone.utc
        ).strftime("%Y%m%d-%H%M%S")
        task_id = arguments.task_id or f"harness/{thread}"
        state = graph.invoke(
            {
                "scope": arguments.scope,
                "task_id": task_id,
                "thread_id": thread,
                "lenses": list(config.lenses),
            },
            config={"configurable": {"thread_id": thread}},
        )
        print(f"Thread: {thread}")
        _emit(state)
        return 0

    state = graph.invoke(
        resume_command(bool(arguments.approve)),
        config={"configurable": {"thread_id": arguments.thread}},
    )
    _emit(state)
    return 0


if __name__ == "__main__":
    sys.exit(main())
