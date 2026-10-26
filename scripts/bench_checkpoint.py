"""The checkpoint skill as a literal model follows it, for the benches' stand-in.

`claude_code_bench.py --cli checkpoint` and `codex_bench.py --cli checkpoint` have
their scripted model make a checkpoint before it reads the file: the
explicit memory commands the `checkpoint` skill names, which an agent runs
itself rather than a hook. In a worktree without the installation those
are the commands that failed: `python3 memory-bank/scripts/context.py`
names no file there, and the skill stopped on a detached HEAD.

The steps are the skill's Governed Workflow, read from the copy the session
was given (the project root's): the task (`CONTEXT_TASK_ID`, otherwise
`git symbolic-ref --quiet --short HEAD`; on a detached HEAD the task
`context.py status --json` reports, where the installed skill says so, and
otherwise the stop it orders), then `turn --flush` and `update`. The memory
CLI is the one the session was told of - a line saying what to run in place
of `python3 memory-bank/scripts/context.py` - else the policy's own.
"""
import json
import re
import shlex
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

POLICY_CLI = "python3 memory-bank/scripts/context.py"
# What a session in a worktree is told to run instead (local-context.sh).
HINT = re.compile(r"Memory CLI: [^`\n]*`([^`\n]+)` in place of `python3 memory-bank/scripts/context\.py`")
PROGRESS = "Checkpoint: refunds keep the invoice date."
NEXT_STEP = "Check credit notes against the invoice date."


def takes_status(skill: str) -> bool:
    """Whether the installed skill's step 1 names a detached HEAD's task by
    `status` instead of stopping there."""
    start = skill.find("## Governed Workflow")
    step = skill[start:skill.find("2. ", start)] if start >= 0 else ""
    words = " ".join(step.split())
    return "On a detached HEAD" in words and "task_identity.task_id" in words


def task_of(step: str, output: str) -> Optional[str]:
    if step == "branch":
        name = output.strip()
        return name if name and "\n" not in name and " " not in name else None
    try:
        identity = json.loads(output)["automatic_memory"]["task_identity"]
    except (ValueError, KeyError, TypeError):
        return None
    return identity.get("task_id") if isinstance(identity, dict) else None


class Checkpoint:
    """One checkpoint, step by step: what to run next given what ran."""

    def __init__(self, skill: str) -> None:
        self.skill = skill
        self.steps: List[Dict[str, Any]] = []
        self.cli = ""
        self.task: Optional[str] = None
        self.stopped = ""

    def next(self, heard: str, done: List[Tuple[str, str, bool]]) -> Optional[Tuple[str, str]]:
        """(step, command) to run, or None once the checkpoint is over;
        `done` is each earlier step's (name, output, failed)."""
        if not self.cli:
            found = HINT.search(heard)
            self.cli = found.group(1) if found else POLICY_CLI
        if not done:
            return "branch", "git symbolic-ref --quiet --short HEAD"
        name, output, failed = done[-1]
        if name in ("branch", "status"):
            self.task = None if failed else task_of(name, output)
            if self.task:
                return "flush", f"{self.cli} turn --task-id {shlex.quote(self.task)} --flush --json"
            if name == "branch" and takes_status(self.skill):
                return "status", f"{self.cli} status --json"
            self.stopped = ("the skill stops on a detached HEAD" if name == "branch"
                            else "status named no task")
            return None
        if name == "flush" and not failed:
            return "update", (f"{self.cli} update --task-id {shlex.quote(self.task or '')} --revision auto "
                              f"--progress {shlex.quote(PROGRESS)} --next-step {shlex.quote(NEXT_STEP)} --json")
        if failed:
            self.stopped = f"{name} failed"
        return None

    def record(self, name: str, output: str, failed: bool) -> None:
        queued = False
        try:
            queued = json.loads(output).get("queued") is True
        except (ValueError, AttributeError):
            pass
        self.steps.append({"step": name, "ok": not failed, "queued": queued, "answer": " ".join(output.split())[:160]})

    def report(self, project: Path, task: str) -> Dict[str, Any]:
        """What the checkpoint did, and whether it landed: the root's Project
        Brain holds the task `task` with the checkpoint's next step (the save
        after the answer keeps a plan it names none for)."""
        tasks = [json.loads(path.read_text(encoding="utf-8").split("---\n")[1])
                 for path in (project / "project-brain/dynamic/tasks").glob("*.md")]
        landed = any(record.get("external_id") == task and NEXT_STEP in (record.get("next_steps") or [])
                     for record in tasks)
        made = [step["step"] for step in self.steps]
        return {"cli": self.cli, "status_on_detached_head": takes_status(self.skill), "task": self.task,
                "steps": self.steps, "stopped": self.stopped or None, "landed": landed,
                "ok": landed and self.task == task and made[-2:] == ["flush", "update"]
                and all(step["ok"] for step in self.steps[-2:])}
