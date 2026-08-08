"""Bridge to the project's own blackboard: project-brain via context.py.

The harness keeps NO parallel state store. Task lifecycle, the agent message
channel, capsule validation and the dispatch journal all live in the target
project's governed runtime, so interactive sessions and unattended runs read
and write the same audit trail. LangGraph's checkpointer holds only graph
position.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path


class BlackboardError(Exception):
    pass


class Blackboard:
    def __init__(self, project: Path, timeout_seconds: int = 30) -> None:
        self.project = project
        self.timeout_seconds = timeout_seconds
        self.cli = project / "memory-bank" / "scripts" / "context.py"

    @property
    def available(self) -> bool:
        return self.cli.is_file()

    def _run(self, *arguments: str, check: bool = True) -> subprocess.CompletedProcess[str]:
        completed = subprocess.run(
            [sys.executable, str(self.cli), "--root", str(self.project), *arguments],
            text=True,
            capture_output=True,
            timeout=self.timeout_seconds,
        )
        if check and completed.returncode != 0:
            raise BlackboardError(
                (completed.stderr or completed.stdout).strip()[:500]
            )
        return completed

    def ensure_task(self, task_id: str, goal: str) -> None:
        if not self.available:
            return
        started = self._run(
            "start", "--task-id", task_id, "--goal", goal, check=False
        )
        if started.returncode != 0 and "already" not in (
            started.stderr + started.stdout
        ).lower():
            # An existing task is fine; anything else is a real failure.
            probe = self._run("get", "--task-id", task_id, check=False)
            if probe.returncode != 0:
                raise BlackboardError(
                    (started.stderr or started.stdout).strip()[:500]
                )

    def validate_capsule(self, capsule: str) -> list[str]:
        """Return capsule problems (empty list = valid)."""
        if not self.available:
            return []
        with tempfile.NamedTemporaryFile(
            "w", suffix=".md", delete=False, encoding="utf-8"
        ) as handle:
            handle.write(capsule)
            path = handle.name
        try:
            completed = self._run(
                "capsule", "--validate", "--file", path, "--json", check=False
            )
        finally:
            Path(path).unlink(missing_ok=True)
        if completed.returncode == 0:
            return []
        import json as _json

        try:
            return list(_json.loads(completed.stdout).get("problems", []))
        except Exception:  # noqa: BLE001 - degraded CLI output is still a failure
            return [(completed.stderr or completed.stdout).strip()[:200] or "invalid capsule"]

    def dispatch_spawn(self, task_id: str, agent: str, capsule: str) -> None:
        if not self.available:
            return
        with tempfile.NamedTemporaryFile(
            "w", suffix=".md", delete=False, encoding="utf-8"
        ) as handle:
            handle.write(capsule)
            path = handle.name
        try:
            self._run(
                "msg-dispatch", "--task-id", task_id, "--agent", agent,
                "--event", "spawn", "--capsule-file", path,
            )
        finally:
            Path(path).unlink(missing_ok=True)

    def dispatch_complete(self, task_id: str, agent: str, note: str) -> None:
        if not self.available:
            return
        self._run(
            "msg-dispatch", "--task-id", task_id, "--agent", agent,
            "--event", "complete", "--note", note[:160],
        )

    def record_progress(self, task_id: str, progress: str) -> None:
        if not self.available:
            return
        self._run(
            "update", "--task-id", task_id, "--actor", "harness",
            "--progress", progress[:500],
        )


class NullBlackboard(Blackboard):
    """No-op blackboard for projects without the context runtime."""

    def __init__(self) -> None:  # noqa: D107 - trivial
        self.project = Path(".")
        self.timeout_seconds = 0
        self.cli = Path("/nonexistent")
