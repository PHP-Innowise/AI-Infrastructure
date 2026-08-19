#!/usr/bin/env python3
"""Automatic-memory readiness regression coverage."""

from __future__ import annotations

import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Optional
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
CONTEXT = ROOT / ".agents/skills/memory-seed/assets/scripts/context.py"
VALIDATOR_SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(VALIDATOR_SCRIPTS))
VALIDATOR_SPEC = importlib.util.spec_from_file_location(
    "readiness_validate_generated", VALIDATOR_SCRIPTS / "validate_generated.py"
)
validator = importlib.util.module_from_spec(VALIDATOR_SPEC)
VALIDATOR_SPEC.loader.exec_module(validator)
sys.path.insert(0, str(CONTEXT.parent))
CONTEXT_SPEC = importlib.util.spec_from_file_location(
    "readiness_context_runtime", CONTEXT
)
context_runtime = importlib.util.module_from_spec(CONTEXT_SPEC)
CONTEXT_SPEC.loader.exec_module(context_runtime)
TIMEOUT = 30


class MemoryReadinessTest(unittest.TestCase):
    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="memory-readiness-")
        self.addCleanup(self._tmp.cleanup)
        self.base = Path(self._tmp.name)

    def status(
        self, repository: Path, task_id: Optional[str] = None
    ) -> dict[str, object]:
        environment = dict(os.environ)
        if task_id is None:
            environment.pop("CONTEXT_TASK_ID", None)
        else:
            environment["CONTEXT_TASK_ID"] = task_id
        result = subprocess.run(
            [
                sys.executable,
                str(CONTEXT),
                "--root",
                str(repository),
                "status",
                "--json",
            ],
            capture_output=True,
            text=True,
            env=environment,
            timeout=TIMEOUT,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)["automatic_memory"]

    def git(self, repository: Path, *arguments: str) -> None:
        result = subprocess.run(
            ["git", "-C", str(repository), *arguments],
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def initialized_repository(self) -> Path:
        repository = self.base / "repository"
        repository.mkdir()
        self.git(repository, "-c", "init.defaultBranch=feature/readiness", "init", "-q")
        return repository

    def commit(self, repository: Path) -> None:
        tracked = repository / "tracked.txt"
        tracked.write_text("ready\n", encoding="utf-8")
        self.git(repository, "add", "tracked.txt")
        self.git(
            repository,
            "-c",
            "user.name=Readiness Test",
            "-c",
            "user.email=readiness@example.invalid",
            "commit",
            "-q",
            "-m",
            "readiness fixture",
        )

    def test_no_identity_outside_git_is_degraded(self) -> None:
        repository = self.base / "plain"
        repository.mkdir()

        readiness = self.status(repository)

        self.assertEqual(readiness["status"], "degraded")
        self.assertEqual(readiness["task_identity"]["status"], "degraded")
        self.assertEqual(readiness["git_metadata"]["state"], "not-a-worktree")
        self.assertIn("CONTEXT_TASK_ID", readiness["remediation"])

    def test_explicit_identity_outside_git_is_retrieval_only(self) -> None:
        repository = self.base / "plain"
        repository.mkdir()

        readiness = self.status(repository, "TASK-204")

        self.assertEqual(readiness["status"], "retrieval-only")
        self.assertEqual(readiness["task_identity"]["source"], "environment")
        self.assertEqual(readiness["git_metadata"]["status"], "degraded")
        self.assertIn("checkpointing", readiness["reason"])

    def test_detached_head_without_explicit_identity_is_degraded(self) -> None:
        repository = self.initialized_repository()
        self.commit(repository)
        self.git(repository, "checkout", "--detach", "-q", "HEAD")

        readiness = self.status(repository)

        self.assertEqual(readiness["status"], "degraded")
        self.assertEqual(readiness["task_identity"]["status"], "degraded")
        self.assertEqual(readiness["git_metadata"]["status"], "active")
        self.assertEqual(readiness["git_metadata"]["state"], "detached-head")

    def test_detached_head_with_explicit_identity_is_active(self) -> None:
        repository = self.initialized_repository()
        self.commit(repository)
        self.git(repository, "checkout", "--detach", "-q", "HEAD")

        readiness = self.status(repository, "TASK-205")

        self.assertEqual(readiness["status"], "active")
        self.assertEqual(readiness["task_identity"]["source"], "environment")
        self.assertEqual(readiness["git_metadata"]["state"], "detached-head")

    def test_healthy_branch_is_active(self) -> None:
        repository = self.initialized_repository()
        self.commit(repository)

        readiness = self.status(repository)

        self.assertEqual(readiness["status"], "active")
        self.assertEqual(readiness["task_identity"]["source"], "git-branch")
        self.assertEqual(readiness["task_identity"]["task_id"], "feature/readiness")
        self.assertEqual(readiness["git_metadata"]["state"], "worktree")
        self.assertIsNone(readiness["remediation"])

    def test_unborn_branch_reports_unborn_head_with_branch_identity(self) -> None:
        repository = self.initialized_repository()

        readiness = self.status(repository)

        self.assertEqual(readiness["git_metadata"]["state"], "unborn-head")
        self.assertEqual(readiness["git_metadata"]["status"], "active")
        self.assertIn("unborn", readiness["git_metadata"]["reason"])
        self.assertNotIn("detached", readiness["git_metadata"]["reason"])
        self.assertEqual(readiness["task_identity"]["source"], "git-branch")
        self.assertEqual(readiness["task_identity"]["task_id"], "feature/readiness")
        self.assertEqual(readiness["status"], "active")

    def readiness_with_probe_failure(self, repository, marker: str) -> dict:
        real_run = subprocess.run

        def flaky_run(command, *arguments, **keywords):
            if marker in command:
                raise subprocess.TimeoutExpired(cmd=command, timeout=2)
            return real_run(command, *arguments, **keywords)

        environment = dict(os.environ)
        environment.pop("CONTEXT_TASK_ID", None)
        with mock.patch.dict(os.environ, environment, clear=True):
            with mock.patch.object(
                context_runtime.subprocess, "run", side_effect=flaky_run
            ):
                return context_runtime.automatic_memory_readiness(repository)

    def test_branch_probe_timeout_is_probe_failure_not_detached(self) -> None:
        repository = self.initialized_repository()
        self.commit(repository)

        readiness = self.readiness_with_probe_failure(repository, "symbolic-ref")

        self.assertEqual(readiness["git_metadata"]["state"], "git-probe-failed")
        self.assertEqual(readiness["git_metadata"]["status"], "degraded")
        self.assertIn("branch probe", readiness["git_metadata"]["reason"])
        self.assertNotIn("detached", readiness["git_metadata"]["reason"])
        self.assertIsNone(readiness["git_metadata"]["branch"])
        self.assertEqual(readiness["status"], "degraded")
        errors: list[str] = []
        validator.validate_memory_readiness({"automatic_memory": readiness}, errors)
        self.assertEqual(errors, [])

    def test_head_probe_timeout_on_branch_keeps_branch_identity(self) -> None:
        repository = self.initialized_repository()
        self.commit(repository)

        readiness = self.readiness_with_probe_failure(repository, "--verify")

        self.assertEqual(readiness["git_metadata"]["state"], "worktree")
        self.assertEqual(readiness["git_metadata"]["status"], "active")
        self.assertEqual(readiness["git_metadata"]["branch"], "feature/readiness")
        self.assertEqual(readiness["task_identity"]["source"], "git-branch")
        self.assertEqual(readiness["task_identity"]["task_id"], "feature/readiness")
        self.assertEqual(readiness["status"], "active")

    def test_head_probe_timeout_when_detached_is_probe_failure_not_unborn(
        self,
    ) -> None:
        repository = self.initialized_repository()
        self.commit(repository)
        self.git(repository, "checkout", "--detach", "-q", "HEAD")

        readiness = self.readiness_with_probe_failure(repository, "--verify")

        self.assertEqual(readiness["git_metadata"]["state"], "git-probe-failed")
        self.assertEqual(readiness["git_metadata"]["status"], "degraded")
        self.assertIsNone(readiness["git_metadata"]["branch"])
        self.assertIn("HEAD-classification", readiness["git_metadata"]["reason"])
        self.assertNotIn("branch probe", readiness["git_metadata"]["reason"])
        self.assertNotIn("unborn", readiness["git_metadata"]["reason"])
        self.assertEqual(readiness["status"], "degraded")
        errors: list[str] = []
        validator.validate_memory_readiness({"automatic_memory": readiness}, errors)
        self.assertEqual(errors, [])

    def test_bootstrap_readiness_contract_accepts_runtime_report(self) -> None:
        repository = self.base / "plain"
        repository.mkdir()
        payload = {"automatic_memory": self.status(repository, "TASK-206")}
        errors: list[str] = []

        validator.validate_memory_readiness(payload, errors)

        self.assertEqual(errors, [])

    def test_bootstrap_readiness_contract_rejects_missing_git_report(self) -> None:
        errors: list[str] = []

        validator.validate_memory_readiness(
            {
                "automatic_memory": {
                    "status": "active",
                    "reason": "incomplete fixture",
                    "remediation": None,
                    "task_identity": {
                        "status": "active",
                        "reason": "available",
                        "remediation": None,
                    },
                }
            },
            errors,
        )

        self.assertIn(
            "context.py status automatic_memory.git_metadata missing", errors
        )


class StagedBankSourceResolutionTest(unittest.TestCase):
    """A seeded bank cites the target's files while living apart from them."""

    BANK_VALIDATOR = ROOT / ".agents/skills/memory-seed/assets/scripts/validate.py"

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="staged-bank-")
        self.addCleanup(self._tmp.cleanup)
        base = Path(self._tmp.name)

        # The project the bundle describes: it owns the cited file.
        self.evidence_target = base / "target"
        (self.evidence_target / "src").mkdir(parents=True)
        (self.evidence_target / "src/Publisher.php").write_text(
            "<?php\nclass Publisher {}\n", encoding="utf-8"
        )

        # The staged bundle: a bank beside nothing else.
        self.staging = base / "staging"
        bank = self.staging / "memory-bank"
        (bank / "chunks").mkdir(parents=True)
        (bank / "scripts").mkdir(parents=True)
        (bank / "scripts/validate.py").write_text(
            self.BANK_VALIDATOR.read_text(encoding="utf-8"), encoding="utf-8"
        )
        frontmatter = {
            "id": "MEM-0001",
            "title": "Publishing writes only under an explicit flag",
            "type": "domain",
            "status": "active",
            "scope": ["publication"],
            "tags": ["publishing"],
            "created": "2026-08-19",
            "last_verified": "2026-08-19",
            "review_after": "2027-02-19",
            "sources": ["src/Publisher.php"],
            "supersedes": [],
            "superseded_by": None,
            "valid_from": "2026-08-19",
            "valid_to": None,
        }
        (bank / "chunks/MEM-0001-publishing.md").write_text(
            "---\n"
            + json.dumps(frontmatter, indent=2)
            + "\n---\n\n# Publishing writes only under an explicit flag\n\n"
            "## Durable Context\n\nThe publisher flushes only when asked to.\n\n"
            "## Consequences\n\nA default flush changes what the cron job does.\n\n"
            "## Verification\n\nsrc/Publisher.php declares the guard.\n",
            encoding="utf-8",
        )
        (bank / "INDEX.md").write_text(
            "# Memory Index\n\n"
            "| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n"
            "| --- | --- | --- | --- | --- | --- | --- | --- |\n"
            "| MEM-0001 | Publishing writes only under an explicit flag | domain | "
            "publication | publishing | active | 2026-08-19 | "
            "chunks/MEM-0001-publishing.md |\n",
            encoding="utf-8",
        )
        (bank / ".memory-counter").write_text("2\n", encoding="utf-8")
        (bank / "README.md").write_text("# Memory Bank\n", encoding="utf-8")
        self.bank = bank

    def run_validator(self, *arguments: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(self.bank / "scripts/validate.py"), str(self.bank)]
            + list(arguments),
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )

    def test_staged_bank_without_source_root_cannot_see_the_cited_file(self) -> None:
        result = self.run_validator()
        self.assertEqual(result.returncode, 1)
        self.assertIn(
            "source path does not exist: src/Publisher.php",
            result.stdout + result.stderr,
        )

    def test_source_root_points_the_bank_at_the_project_it_describes(self) -> None:
        result = self.run_validator("--source-root", str(self.evidence_target))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_published_bank_still_resolves_against_its_own_parent(self) -> None:
        # Default behaviour is unchanged: beside the project, no flag needed.
        (self.staging / "src").mkdir()
        (self.staging / "src/Publisher.php").write_text("<?php\n", encoding="utf-8")
        result = self.run_validator()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_generation_gate_passes_the_evidence_target_through(self) -> None:
        files = {"memory-bank/scripts/validate.py": {}}

        staged_errors: list = []
        validator.validate_memory_bank(self.staging, files, staged_errors)
        self.assertTrue(
            staged_errors, "a staged bank citing the target must fail unaided"
        )

        threaded_errors: list = []
        validator.validate_memory_bank(
            self.staging, files, threaded_errors, self.evidence_target
        )
        self.assertEqual(threaded_errors, [])


if __name__ == "__main__":
    unittest.main()
