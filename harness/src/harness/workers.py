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
import subprocess
import time
from dataclasses import dataclass, field
from typing import Callable, Protocol

from .config import HarnessConfig


@dataclass
class WorkerResult:
    text: str
    ok: bool
    cost_usd: float = 0.0
    session_id: str | None = None
    duration_seconds: float = 0.0
    error: str | None = None


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
    completed, duration, error = _run(command, config)
    if completed is None:
        return WorkerResult(text="", ok=False, duration_seconds=duration, error=error)
    if completed.returncode != 0:
        return WorkerResult(
            text=completed.stdout,
            ok=False,
            duration_seconds=duration,
            error=(completed.stderr or f"exit {completed.returncode}").strip()[:500],
        )
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError:
        # A zero-exit run whose stdout is not JSON still carries the answer.
        return WorkerResult(
            text=completed.stdout, ok=True, duration_seconds=duration
        )
    return WorkerResult(
        text=str(payload.get("result") or ""),
        ok=True,
        cost_usd=float(payload.get("total_cost_usd") or 0.0),
        session_id=payload.get("session_id"),
        duration_seconds=duration,
    )


def codex_cli_worker(prompt: str, config: HarnessConfig) -> WorkerResult:
    """One headless Codex run; read-only sandbox by default, JSONL events."""
    command = ["codex", "exec", "--json", prompt]
    completed, duration, error = _run(command, config)
    if completed is None:
        return WorkerResult(text="", ok=False, duration_seconds=duration, error=error)
    if completed.returncode != 0:
        return WorkerResult(
            text=completed.stdout,
            ok=False,
            duration_seconds=duration,
            error=(completed.stderr or f"exit {completed.returncode}").strip()[:500],
        )
    # The last agent message in the JSONL event stream is the answer.
    text = ""
    for line in completed.stdout.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        item = event.get("item") or {}
        if isinstance(item, dict) and item.get("type") == "agent_message":
            text = str(item.get("text") or text)
    return WorkerResult(
        text=text or completed.stdout, ok=True, duration_seconds=duration
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
