"""Worker adapters: one graph node's unit of agent work, run headlessly.

A worker takes a prompt, runs one headless coding-agent session in the
project directory, and returns the final text plus cost metadata. The
subprocess pattern for Claude Code is the officially documented integration
path (``claude -p --output-format json``); ``--bare`` is deliberately NOT
passed, so the worker session loads the project's ``.claude/`` world —
permissions deny rules, the subagent gate, skills, hooks — exactly like an
interactive session would. Cursor has no adapter on purpose: its headless
mode is unreliable (documented hangs); route Cursor work through interactive
flows instead.
"""

from __future__ import annotations

import json
import math
import subprocess
import time
from dataclasses import dataclass, field
from typing import Callable, Protocol

from .config import HarnessConfig


@dataclass
class WorkerResult:
    text: str
    ok: bool
    cost_usd: float | None = None
    session_id: str | None = None
    duration_seconds: float = 0.0
    error: str | None = None
    limit_reached: str | None = None


class Worker(Protocol):
    def __call__(self, prompt: str, config: HarnessConfig) -> WorkerResult: ...


def _run(command: list[str], config: HarnessConfig) -> tuple[subprocess.CompletedProcess[str] | None, float, str | None]:
    started = time.monotonic()
    try:
        completed = subprocess.run(
            command,
            cwd=str(config.project),
            text=True,
            capture_output=True,
            timeout=config.worker_timeout_seconds,
        )
        return completed, time.monotonic() - started, None
    except subprocess.TimeoutExpired:
        return None, time.monotonic() - started, (
            f"worker timed out after {config.worker_timeout_seconds}s"
        )
    except OSError as error:
        return None, time.monotonic() - started, f"worker failed to start: {error}"


def claude_cli_worker(prompt: str, config: HarnessConfig) -> WorkerResult:
    """One headless Claude Code session; JSON payload carries the cost."""
    command = [
        "claude",
        "-p",
        prompt,
        "--output-format",
        "json",
        "--max-turns",
        str(config.max_turns),
        "--permission-mode",
        config.permission_mode,
    ]
    if config.model:
        command += ["--model", config.model]
    if config.thinking_effort:
        command += ["--effort", config.thinking_effort]
    if config.budget_usd is not None:
        command += ["--max-budget-usd", str(config.budget_usd)]
    completed, duration, error = _run(command, config)
    if completed is None:
        return WorkerResult(text="", ok=False, duration_seconds=duration, error=error)
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError:
        return WorkerResult(
            text="", ok=False, duration_seconds=duration,
            error="Claude did not return a valid terminal result.",
        )
    if not isinstance(payload, dict):
        return WorkerResult(text="", ok=False, duration_seconds=duration,
                            error="Claude did not return a valid terminal result.")
    cost = payload.get("total_cost_usd")
    if isinstance(cost, bool) or not isinstance(cost, (int, float)) or not math.isfinite(cost) or cost < 0:
        cost = None
    ok = (completed.returncode == 0 and payload.get("subtype") == "success"
          and payload.get("is_error") is False)
    over_budget = config.budget_usd is not None and cost is not None and cost > config.budget_usd
    ok = ok and not over_budget
    return WorkerResult(
        text=payload.get("result") if isinstance(payload.get("result"), str) else "",
        ok=ok,
        cost_usd=cost,
        session_id=payload.get("session_id"),
        duration_seconds=duration,
        limit_reached='USD' if over_budget or payload.get('subtype') == 'error_max_budget_usd' else None,
        error=(f"Claude exceeded its budget: ${cost:.4f} reported for ${config.budget_usd:.4f} allocated."
               if over_budget else "Claude stopped at its native USD budget; the review is incomplete."
               if payload.get('subtype') == 'error_max_budget_usd' else
               None if ok else "Claude reported an unsuccessful result."),
    )


def codex_cli_worker(prompt: str, config: HarnessConfig) -> WorkerResult:
    """One headless Codex run; read-only sandbox by default, JSONL events."""
    if config.budget_usd is not None:
        raise ValueError("Codex does not support a native USD limit; use budget_usd=None.")
    command = ["codex", "--ask-for-approval", "never", "--sandbox", "read-only"]
    if config.thinking_effort:
        command += ["-c", "model_reasoning_effort=" + json.dumps(config.thinking_effort)]
    command += ["exec", "--json"]
    if config.model:
        command += ["--model", config.model]
    command.append(prompt)
    completed, duration, error = _run(command, config)
    if completed is None:
        return WorkerResult(text="", ok=False, duration_seconds=duration, error=error)
    if completed.returncode != 0:
        return WorkerResult(
            text=completed.stdout,
            ok=False,
            duration_seconds=duration,
            error="Codex exited unsuccessfully.",
        )
    # The last agent message in the JSONL event stream is the answer.
    text = ""
    terminal, failed = False, False
    session_id = None
    for line in completed.stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(event, dict):
            continue
        if event.get("type") == "thread.started" and isinstance(event.get("thread_id"), str):
            session_id = event["thread_id"]
        if event.get("type") == "turn.completed":
            terminal = True
        if event.get("type") in ("turn.failed", "error"):
            failed = True
        item = event.get("item") or {}
        if isinstance(item, dict) and item.get("type") == "agent_message":
            text = str(item.get("text") or text)
    return WorkerResult(
        text=text, ok=terminal and not failed, session_id=session_id, duration_seconds=duration,
        error=None if terminal and not failed else "Codex did not return a successful terminal result.",
    )


@dataclass
class DryRunWorker:
    """Deterministic offline worker for tests and pipeline rehearsal.

    Returns a canned findings payload per call. ``script`` maps a substring
    of the prompt (for example a lens name) to the canned response text.
    """

    script: dict[str, str] = field(default_factory=dict)
    cost_per_call_usd: float = 0.01
    calls: list[str] = field(default_factory=list)

    def __call__(self, prompt: str, config: HarnessConfig) -> WorkerResult:
        self.calls.append(prompt)
        if config.budget_usd is not None and self.cost_per_call_usd > config.budget_usd:
            return WorkerResult(text="", ok=False, cost_usd=0.0,
                                error="Dry-run reviewer exceeds its allocated budget.")
        text = next(
            (body for marker, body in self.script.items() if marker in prompt),
            '{"findings": []}',
        )
        return WorkerResult(
            text=text, ok=True, cost_usd=self.cost_per_call_usd, session_id=None
        )


def resolve_worker(name: str) -> Callable[[str, HarnessConfig], WorkerResult]:
    registry: dict[str, Callable[[str, HarnessConfig], WorkerResult]] = {
        "claude-cli": claude_cli_worker,
        "codex-cli": codex_cli_worker,
        "dry-run": DryRunWorker(),
    }
    try:
        return registry[name]
    except KeyError:
        raise ValueError(
            f"Unknown worker {name!r}; expected one of {sorted(registry)}"
        ) from None
