"""Run configuration for the harness.

Everything here is deliberately plain and serializable-adjacent: the graph
state must survive checkpointing, so runtime objects (workers, blackboard)
are bound to nodes by closure at build time, and only data lives in state.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path

DEFAULT_LENSES = ("code-reviewer", "security-reviewer", "performance-optimization")


@dataclass(frozen=True)
class HarnessConfig:
    """One pipeline run's settings.

    ``project`` is the consuming project (or accelerator edition) the workers
    operate on: its ``.claude/`` world — settings.json deny rules, the
    subagent gate, skills, memory-bank runtime — applies inside every worker
    because headless sessions load it exactly like interactive ones.
    """

    project: Path
    worker: str = "claude-cli"
    lenses: tuple[str, ...] = DEFAULT_LENSES
    model: str | None = None
    budget_usd: float = 5.0
    worker_timeout_seconds: int = 1800
    max_turns: int = 40
    permission_mode: str = "dontAsk"
    state_dir: Path = field(
        default_factory=lambda: Path(
            os.environ.get("HARNESS_STATE_DIR", "state")
        )
    )
    reports_dir: Path = field(
        default_factory=lambda: Path(
            os.environ.get("HARNESS_REPORTS_DIR", "reports")
        )
    )

    def __post_init__(self) -> None:
        if not self.project.is_dir():
            raise ValueError(f"Project directory not found: {self.project}")
        if self.budget_usd <= 0:
            raise ValueError("budget_usd must be positive")

    @property
    def checkpoint_path(self) -> Path:
        return self.state_dir / "checkpoints.sqlite"
