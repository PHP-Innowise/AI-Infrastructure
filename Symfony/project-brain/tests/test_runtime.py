#!/usr/bin/env python3
"""Contract tests for the combined Project Brain + Local Context Engine."""

from __future__ import annotations

import importlib.util
import contextlib
import io
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock


EDITION = Path(__file__).resolve().parents[2]
SCRIPTS = EDITION / "memory-bank" / "scripts"
CONTEXT_SCRIPT = SCRIPTS / "context.py"
sys.path.insert(0, str(SCRIPTS))
try:
    import brain_runtime as brain
    import context as context_cli
    import context_retrieval as retrieval
finally:
    sys.path.pop(0)


class RuntimeHarness(unittest.TestCase):
    """Temporary governed repository shared by the runtime test classes."""

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="project-brain-test-")
        self.repository = Path(self.temporary.name)
        subprocess.run(["git", "init", "--quiet", str(self.repository)], check=True, capture_output=True)
        self.repository.joinpath("memory-bank/chunks").mkdir(parents=True)
        self.repository.joinpath("memory-bank/README.md").write_text(
            "# Memory Bank\n", encoding="utf-8"
        )
        self.repository.joinpath("memory-bank/INDEX.md").write_text(
            "# Memory Index\n\n"
            "| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n"
            "| --- | --- | --- | --- | --- | --- | --- | --- |\n",
            encoding="utf-8",
        )
        self.repository.joinpath("memory-bank/.memory-counter").write_text(
            "1\n", encoding="utf-8"
        )
        self.repository.joinpath("specs").mkdir()
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nThe cobalt authority rule is canonical.\n",
            encoding="utf-8",
        )

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_cli(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(CONTEXT_SCRIPT),
                "--root",
                str(self.repository),
                *arguments,
            ],
            text=True,
            capture_output=True,
        )

    def run_main(self, *arguments: str) -> tuple[int, str, str]:
        stdout = io.StringIO()
        stderr = io.StringIO()
        argv = [
            str(CONTEXT_SCRIPT),
            "--root",
            str(self.repository),
            *arguments,
        ]
        with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(
            stdout
        ), contextlib.redirect_stderr(stderr):
            code = context_cli.main()
        return code, stdout.getvalue(), stderr.getvalue()

    def start(self, task_id: str = "TASK-1") -> dict:
        result = self.run_cli(
            "start",
            "--task-id",
            task_id,
            "--goal",
            "Apply the cobalt authority rule.",
            "--source",
            "specs/authority.md",
            "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)


class ProjectBrainRuntimeTest(RuntimeHarness):
    def test_governed_mode_is_default_and_sqlite_only_keeps_binding_pointer(self) -> None:
        task = self.start()
        self.assertTrue(brain.is_uuid4(task["task_uuid"]))
        self.assertEqual("project-brain", task["authority"])
        self.assertTrue(
            self.repository.joinpath(
                "project-brain/dynamic/tasks", f"{task['task_uuid']}.md"
            ).is_file()
        )
        self.assertTrue(
            self.repository.joinpath(
                "project-brain/control/handoffs", f"{task['task_uuid']}.md"
            ).is_file()
        )
        connection = sqlite3.connect(self.repository / "memory-bank/local/context.db")
        binding = connection.execute(
            "SELECT external_id, task_uuid, revision FROM task_bindings"
        ).fetchone()
        pointer = connection.execute(
            "SELECT goal, progress, next_steps, files, sources FROM working_tasks"
        ).fetchone()
        connection.close()
        self.assertEqual(("TASK-1", task["task_uuid"], 1), binding)
        self.assertEqual((task["task_uuid"], "", "[]", "[]", "[]"), pointer)

    def test_revision_cas_owner_authorization_and_transition_rules(self) -> None:
        task = brain.create_task(
            self.repository,
            "TASK-CAS",
            "Protect revision safety.",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        updated = brain.update_task(
            self.repository,
            task["id"],
            expected_revision=1,
            progress="Observed.",
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
        )
        self.assertEqual(2, updated["revision"])
        with self.assertRaisesRegex(brain.BrainError, "Stale task revision"):
            brain.update_task(
                self.repository,
                task["id"],
                expected_revision=1,
                progress="Stale.",
                next_steps=[],
                files=[],
                sources=[],
                actor="alice",
            )
        with self.assertRaisesRegex(brain.BrainError, "not authorized"):
            brain.update_task(
                self.repository,
                task["id"],
                expected_revision=2,
                progress="Unauthorized.",
                next_steps=[],
                files=[],
                sources=[],
                actor="bob",
            )
        brain.close_task(
            self.repository,
            task["id"],
            "Verified complete.",
            ["tests passed"],
            expected_revision=2,
            actor="alice",
        )
        with self.assertRaisesRegex(brain.BrainError, "Illegal task transition"):
            brain.cancel_task(self.repository, task["id"], actor="alice")

    def test_every_dynamic_type_enforces_its_own_lifecycle(self) -> None:
        scenarios = {
            "finding": ("open", "investigating"),
            "bug": ("reported", "triaged"),
            "incident": ("open", "contained"),
            "decision": ("proposed", "accepted"),
            "event": ("recorded", "superseded"),
        }
        for record_type, (initial, next_state) in scenarios.items():
            with self.subTest(record_type=record_type):
                record = brain.create_record(
                    self.repository,
                    record_type,
                    f"{record_type.upper()}-1",
                    f"{record_type} title",
                    [],
                    ["specs/authority.md"],
                    owner="alice",
                )
                self.assertEqual(initial, record["status"])
                updated = brain.update_record(
                    self.repository,
                    record["id"],
                    expected_revision=1,
                    progress=None,
                    next_steps=[],
                    files=[],
                    sources=[],
                    actor="alice",
                    transition_to=next_state,
                )
                self.assertEqual(next_state, updated["status"])
                with self.assertRaisesRegex(brain.BrainError, "Illegal"):
                    brain.update_record(
                        self.repository,
                        record["id"],
                        expected_revision=2,
                        progress=None,
                        next_steps=[],
                        files=[],
                        sources=[],
                        actor="alice",
                        transition_to="active",
                    )

    def test_event_content_is_immutable_and_non_task_cli_is_public(self) -> None:
        created = self.run_cli(
            "--owner",
            "alice",
            "brain-create",
            "event",
            "--external-id",
            "EVENT-CLI",
            "--title",
            "Immutable event",
            "--source",
            "specs/authority.md",
            "--json",
        )
        self.assertEqual(0, created.returncode, created.stderr)
        event = json.loads(created.stdout)
        changed = self.run_cli(
            "--owner",
            "alice",
            "brain-update",
            "--record-id",
            event["id"],
            "--revision",
            "1",
            "--progress",
            "Mutated",
            "--json",
        )
        self.assertNotEqual(0, changed.returncode)
        self.assertIn("Events are immutable", changed.stderr)

    def test_concurrent_compare_and_swap_allows_one_revision_writer(self) -> None:
        task = brain.create_task(
            self.repository, "TASK-RACE", "Serialize writes.", [], [], owner="alice"
        )
        results: list[object] = []

        def worker(progress: str) -> None:
            try:
                results.append(
                    brain.update_task(
                        self.repository,
                        task["id"],
                        expected_revision=1,
                        progress=progress,
                        next_steps=[],
                        files=[],
                        sources=[],
                        actor="alice",
                    )
                )
            except Exception as error:
                results.append(error)

        threads = [
            threading.Thread(target=worker, args=("first",)),
            threading.Thread(target=worker, args=("second",)),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=5)
        self.assertEqual(1, sum(isinstance(item, dict) for item in results))
        self.assertEqual(1, sum(isinstance(item, brain.BrainError) for item in results))

    def test_index_filters_private_record_before_fts_storage(self) -> None:
        task = brain.create_task(
            self.repository,
            "TASK-PRIVATE",
            "Never persist the ultramarine private phrase.",
            [],
            [],
            owner="alice",
            privacy="private",
        )
        indexed = self.run_cli("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        payload = json.loads(indexed.stdout)
        self.assertTrue(
            any(item["reason"] == "private" for item in payload["excluded"])
        )
        connection = sqlite3.connect(self.repository / "memory-bank/local/context.db")
        count = connection.execute(
            "SELECT COUNT(*) FROM documents WHERE content MATCH 'ultramarine'"
        ).fetchone()[0]
        connection.close()
        self.assertEqual(0, count)
        self.assertTrue(brain.is_uuid4(task["id"]))

    def test_retrieve_writes_bounded_budgeted_manifest_and_context_alias_matches(self) -> None:
        task = self.start("TASK-RETRIEVE")
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        result = self.run_cli(
            "retrieve",
            "cobalt authority",
            "--task-id",
            "TASK-RETRIEVE",
            "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        packet = json.loads(result.stdout)
        self.assertEqual(task["task_uuid"], packet["task_uuid"])
        self.assertLessEqual(packet["token_estimates"]["total"], 8000)
        self.assertTrue(
            all(len(item["snippet"]) <= 1200 for item in packet["selected"])
        )
        manifest = self.repository / packet["manifest"]
        self.assertTrue(manifest.is_file())
        manifest_data = json.loads(manifest.read_text(encoding="utf-8"))
        self.assertEqual(12000, manifest_data["token_estimates"]["hard"])
        alias = self.run_cli(
            "context", "cobalt", "--task-id", "TASK-RETRIEVE", "--json"
        )
        self.assertEqual(0, alias.returncode, alias.stderr)
        self.assertEqual("TASK-RETRIEVE", json.loads(alias.stdout)["task_id"])

    def test_skill_mirror_drift_fails_instead_of_hiding_parity_error(self) -> None:
        for edition, body in ((".agents", "canonical"), (".claude", "drifted")):
            path = self.repository / edition / "skills" / "review" / "SKILL.md"
            path.parent.mkdir(parents=True)
            path.write_text(f"# Review\n\n{body}\n", encoding="utf-8")
        indexed = self.run_cli("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        self.assertEqual(1, len(json.loads(indexed.stdout)["parity_drift"]))
        parity = self.run_cli("parity", "--json")
        self.assertNotEqual(0, parity.returncode)
        self.assertIn("mirror parity drift", parity.stderr)

    def test_compaction_moves_terminal_record_and_validates_archive(self) -> None:
        task = brain.create_task(
            self.repository, "TASK-DONE", "Complete then archive.", [], [], owner="alice"
        )
        brain.close_task(
            self.repository,
            task["id"],
            "Complete.",
            ["verified"],
            expected_revision=1,
            actor="alice",
        )
        result = brain.compact(self.repository)
        self.assertEqual({"moved": 1}, result)
        self.assertFalse(
            self.repository.joinpath(
                "project-brain/dynamic/tasks", f"{task['id']}.md"
            ).exists()
        )
        self.assertTrue(
            self.repository.joinpath(
                "project-brain/archive/task", f"{task['id']}.md"
            ).is_file()
        )
        self.assertEqual([], brain.validate_repository(self.repository))

    def test_compaction_rolls_back_all_moves_and_indexes_on_failure(self) -> None:
        first = brain.create_task(
            self.repository, "TASK-COMPACT-1", "Archive one.", [], [], owner="alice"
        )
        second = brain.create_task(
            self.repository, "TASK-COMPACT-2", "Archive two.", [], [], owner="alice"
        )
        for task in (first, second):
            brain.close_task(
                self.repository,
                task["id"],
                "Done.",
                ["verified"],
                expected_revision=1,
                actor="alice",
            )
        active_index = self.repository / "project-brain/indexes/active.json"
        before_index = active_index.read_bytes()
        with mock.patch.object(
            brain, "validate_repository", side_effect=[[], ["post failure"]]
        ):
            with self.assertRaisesRegex(brain.BrainError, "Archive validation"):
                brain.compact(self.repository)
        self.assertEqual(before_index, active_index.read_bytes())
        for task in (first, second):
            self.assertTrue(
                self.repository.joinpath(
                    "project-brain/dynamic/tasks", f"{task['id']}.md"
                ).is_file()
            )
            self.assertFalse(
                self.repository.joinpath(
                    "project-brain/archive/task", f"{task['id']}.md"
                ).exists()
            )

    def test_promotion_requires_independent_review_and_applies_to_memory(self) -> None:
        task = brain.create_task(
            self.repository,
            "TASK-PROMOTE",
            "Produce reusable knowledge.",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        proposal = brain.create_promotion(
            self.repository,
            [task["id"]],
            "Cobalt authority",
            "The cobalt authority rule is reusable.",
            proposer="alice",
        )
        with self.assertRaisesRegex(brain.BrainError, "independent"):
            brain.review_promotion(
                self.repository, proposal["id"], reviewer="alice", approve=True
            )
        brain.review_promotion(
            self.repository, proposal["id"], reviewer="human-reviewer", approve=True
        )
        applied = brain.apply_promotion(self.repository, proposal["id"])
        self.assertEqual("applied", applied["status"])
        self.assertEqual("MEM-0001", applied["destination_memory_id"])
        self.assertTrue(
            self.repository.joinpath(
                "memory-bank/chunks/MEM-0001-cobalt-authority.md"
            ).is_file()
        )
        self.assertEqual(
            "2", self.repository.joinpath("memory-bank/.memory-counter").read_text().strip()
        )

    def test_promotion_binds_generic_source_type_path_and_revision(self) -> None:
        finding = brain.create_record(
            self.repository,
            "finding",
            "FINDING-PROMOTE",
            "Reusable finding",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        proposal = brain.create_promotion(
            self.repository,
            [finding["id"]],
            "Reusable finding",
            "Reviewed consequence.",
            proposer="alice",
        )
        source = proposal["source_records"][0]
        self.assertEqual("finding", source["type"])
        self.assertEqual(
            f"project-brain/dynamic/findings/{finding['id']}.md", source["path"]
        )
        brain.review_promotion(
            self.repository, proposal["id"], "human", approve=True
        )
        applied = brain.apply_promotion(self.repository, proposal["id"])
        chunk = self.repository / "memory-bank/chunks/MEM-0001-reusable-finding.md"
        metadata, _ = brain.parse_markdown_record(chunk)
        self.assertEqual([source["path"]], metadata["sources"])
        self.assertEqual("applied", applied["status"])

    def test_promotion_rechecks_exact_generic_source_revision(self) -> None:
        bug = brain.create_record(
            self.repository,
            "bug",
            "BUG-PROMOTE",
            "Promotion source bug",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        proposal = brain.create_promotion(
            self.repository,
            [bug["id"]],
            "Bug lesson",
            "Reusable lesson.",
            proposer="alice",
        )
        brain.review_promotion(
            self.repository, proposal["id"], "human", approve=True
        )
        brain.update_record(
            self.repository,
            bug["id"],
            expected_revision=1,
            progress="Changed after review.",
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
        )
        with self.assertRaisesRegex(brain.BrainError, "revision changed"):
            brain.apply_promotion(self.repository, proposal["id"])

    def test_promotion_apply_restores_every_file_on_final_write_failure(self) -> None:
        finding = brain.create_record(
            self.repository,
            "finding",
            "FINDING-ROLLBACK",
            "Rollback finding",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        proposal = brain.create_promotion(
            self.repository,
            [finding["id"]],
            "Rollback promotion",
            "Must be atomic.",
            proposer="alice",
        )
        brain.review_promotion(
            self.repository, proposal["id"], "human", approve=True
        )
        promotion_path = self.repository.joinpath(
            "project-brain/control/promotions", f"{proposal['id']}.json"
        )
        before = {
            promotion_path: promotion_path.read_bytes(),
            self.repository / "memory-bank/INDEX.md": self.repository.joinpath(
                "memory-bank/INDEX.md"
            ).read_bytes(),
            self.repository / "memory-bank/.memory-counter": self.repository.joinpath(
                "memory-bank/.memory-counter"
            ).read_bytes(),
        }
        real_atomic_json = brain.atomic_json

        def fail_final(path: Path, value: object) -> None:
            if path == promotion_path and value.get("status") == "applied":
                raise OSError("final promotion write failed")
            real_atomic_json(path, value)

        with mock.patch.object(brain, "atomic_json", side_effect=fail_final):
            with self.assertRaisesRegex(OSError, "final promotion write failed"):
                brain.apply_promotion(self.repository, proposal["id"])
        for path, content in before.items():
            self.assertEqual(content, path.read_bytes())
        self.assertFalse(
            self.repository.joinpath(
                "memory-bank/chunks/MEM-0001-rollback-promotion.md"
            ).exists()
        )

    def test_promotion_rolls_back_each_memory_write_boundary(self) -> None:
        finding = brain.create_record(
            self.repository,
            "finding",
            "FINDING-WRITES",
            "Write boundary finding",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        proposal = brain.create_promotion(
            self.repository,
            [finding["id"]],
            "Write boundary",
            "Atomic memory writes.",
            proposer="alice",
        )
        brain.review_promotion(
            self.repository, proposal["id"], "human", approve=True
        )
        bank = self.repository / "memory-bank"
        promotion_path = self.repository.joinpath(
            "project-brain/control/promotions", f"{proposal['id']}.json"
        )
        destination = bank / "chunks/MEM-0001-write-boundary.md"
        for failed_path in (
            destination,
            bank / "INDEX.md",
            bank / ".memory-counter",
        ):
            with self.subTest(path=failed_path.name):
                before = {
                    promotion_path: promotion_path.read_bytes(),
                    bank / "INDEX.md": bank.joinpath("INDEX.md").read_bytes(),
                    bank / ".memory-counter": bank.joinpath(
                        ".memory-counter"
                    ).read_bytes(),
                }
                real_atomic_write = brain.atomic_write

                def fail_write(path: Path, content: str) -> None:
                    if path == failed_path:
                        raise OSError(f"failed {path.name}")
                    real_atomic_write(path, content)

                with mock.patch.object(
                    brain, "atomic_write", side_effect=fail_write
                ):
                    with self.assertRaisesRegex(OSError, "failed"):
                        brain.apply_promotion(self.repository, proposal["id"])
                for path, content in before.items():
                    self.assertEqual(content, path.read_bytes())
                self.assertFalse(destination.exists())

    def test_promotion_rolls_back_when_memory_validator_rejects_chunk(self) -> None:
        finding = brain.create_record(
            self.repository,
            "finding",
            "FINDING-VALIDATOR",
            "Validator finding",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        proposal = brain.create_promotion(
            self.repository,
            [finding["id"]],
            "Validator rollback",
            "Must validate.",
            proposer="alice",
        )
        brain.review_promotion(
            self.repository, proposal["id"], "human", approve=True
        )
        promotion_path = self.repository.joinpath(
            "project-brain/control/promotions", f"{proposal['id']}.json"
        )
        before = promotion_path.read_bytes()
        with mock.patch.object(
            brain, "validate_bank", return_value=["injected validation failure"]
        ):
            with self.assertRaisesRegex(brain.BrainError, "failed validation"):
                brain.apply_promotion(self.repository, proposal["id"])
        self.assertEqual(before, promotion_path.read_bytes())
        self.assertEqual("1", self.repository.joinpath(
            "memory-bank/.memory-counter"
        ).read_text().strip())
        self.assertEqual([], list(self.repository.joinpath(
            "memory-bank/chunks"
        ).glob("*.md")))

    def test_failed_start_update_and_complete_restore_both_stores(self) -> None:
        real_bind = context_cli.bind_governed_task

        def bind_then_fail(*arguments: object, **keywords: object) -> None:
            real_bind(*arguments, **keywords)
            raise sqlite3.OperationalError("binding boundary failed")

        with mock.patch.object(
            context_cli, "bind_governed_task", side_effect=bind_then_fail
        ):
            code, _, error = self.run_main(
                "start", "--task-id", "TASK-ROLLBACK", "--goal", "Rollback start"
            )
        self.assertEqual(1, code)
        self.assertIn("binding boundary failed", error)
        with self.assertRaises(brain.BrainError):
            brain.find_task(self.repository, "TASK-ROLLBACK")
        self.assertEqual(0, self.start("TASK-ROLLBACK")["revision"] - 1)

        before = brain.get_task(self.repository, "TASK-ROLLBACK")
        real_refresh = context_cli.refresh_governed_binding

        def refresh_then_fail(*arguments: object, **keywords: object) -> None:
            real_refresh(*arguments, **keywords)
            raise sqlite3.OperationalError("refresh boundary failed")

        with mock.patch.object(
            context_cli, "refresh_governed_binding", side_effect=refresh_then_fail
        ):
            code, _, error = self.run_main(
                "update",
                "--task-id",
                "TASK-ROLLBACK",
                "--progress",
                "Must roll back",
            )
        self.assertEqual(1, code)
        self.assertIn("refresh boundary failed", error)
        after_update = brain.get_task(self.repository, "TASK-ROLLBACK")
        self.assertEqual(before["revision"], after_update["revision"])
        self.assertEqual(before["progress"], after_update["progress"])

        real_close = context_cli.close_task

        def close_then_fail(*arguments: object, **keywords: object) -> dict:
            real_close(*arguments, **keywords)
            raise sqlite3.OperationalError("complete boundary failed")

        with mock.patch.object(
            context_cli, "close_task", side_effect=close_then_fail
        ):
            code, _, error = self.run_main(
                "complete",
                "--task-id",
                "TASK-ROLLBACK",
                "--outcome",
                "Must roll back",
            )
        self.assertEqual(1, code)
        self.assertIn("complete boundary failed", error)
        after_complete = brain.get_task(self.repository, "TASK-ROLLBACK")
        self.assertEqual(before["revision"], after_complete["revision"])
        status = json.loads(self.run_cli("status", "--json").stdout)
        self.assertEqual(1, status["working"])
        self.assertEqual(0, status["episodes"])

    def test_conflict_partner_is_fetched_without_lexical_match_and_forced_budget(self) -> None:
        task = brain.create_task(
            self.repository, "TASK-CONFLICT", "Retrieve conflict.", [], [], owner="alice"
        )
        first = brain.create_record(
            self.repository,
            "finding",
            "FINDING-ALPHA",
            "Alpha lexical position",
            [],
            [],
            owner="alice",
        )
        second = brain.create_record(
            self.repository,
            "finding",
            "FINDING-BETA",
            "Beta opposing position",
            [],
            [],
            owner="alice",
        )
        brain.update_record(
            self.repository,
            first["id"],
            expected_revision=1,
            progress=None,
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
            conflicts=[second["id"]],
        )
        brain.update_record(
            self.repository,
            second["id"],
            expected_revision=1,
            progress=None,
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
            conflicts=[first["id"]],
        )
        database = self.repository / "memory-bank/local/context.db"
        connection = context_cli.connect(database)
        try:
            context_cli.index_repository(connection, self.repository)
            with mock.patch.dict(retrieval.BUDGETS, {"dynamic": 1}):
                packet = retrieval.retrieve(
                    connection,
                    self.repository,
                    "alpha",
                    task["id"],
                    limit=3,
                )
        finally:
            connection.close()
        selected_ids = {
            item["record_id"] for item in packet["selected"] if item["record_id"]
        }
        self.assertTrue({first["id"], second["id"]}.issubset(selected_ids))
        manifest = json.loads(
            (self.repository / packet["manifest"]).read_text(encoding="utf-8")
        )
        self.assertIn("conflict pair", manifest["escalation_reason"])

    def test_stale_source_is_filtered_before_reindex_and_retrieval(self) -> None:
        task = brain.create_task(
            self.repository,
            "TASK-STALE",
            "Cobalt stale source",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        self.repository.joinpath("specs/authority.md").write_text(
            "# Changed\n\nCobalt content drifted.\n", encoding="utf-8"
        )
        database = self.repository / "memory-bank/local/context.db"
        connection = context_cli.connect(database)
        try:
            packet = retrieval.retrieve(
                connection, self.repository, "cobalt", task["id"], limit=3
            )
        finally:
            connection.close()
        self.assertFalse(
            any(item.get("record_id") == task["id"] for item in packet["selected"])
        )
        reindexed = json.loads(self.run_cli("index", "--json").stdout)
        self.assertTrue(
            any(
                item["reason"] == "stale" and task["id"] in item["path"]
                for item in reindexed["excluded"]
            )
        )

    def test_manifest_nested_schema_is_enforced_adversarially(self) -> None:
        schemas = self.repository / "project-brain/schemas"
        schemas.mkdir(parents=True)
        for name in (
            "dynamic-record.schema.json",
            "handoff.schema.json",
            "retrieval-manifest.schema.json",
            "promotion.schema.json",
        ):
            schemas.joinpath(name).write_bytes(
                EDITION.joinpath("project-brain/schemas", name).read_bytes()
            )
        self.start("TASK-MANIFEST")
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        packet = json.loads(
            self.run_cli(
                "retrieve", "cobalt", "--task-id", "TASK-MANIFEST", "--json"
            ).stdout
        )
        manifest_path = self.repository / packet["manifest"]
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["filters"]["unexpected"] = True
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        errors = brain.validate_repository(self.repository)
        self.assertTrue(any("unexpected keys" in error for error in errors))

    def test_validator_rejects_noncanonical_deterministic_index(self) -> None:
        self.start("TASK-INDEX")
        index = self.repository / "project-brain/indexes/active.json"
        index.write_text("[]\n", encoding="utf-8")
        errors = brain.validate_repository(self.repository)
        self.assertTrue(any("deterministic index" in error for error in errors))

    def test_contract_json_is_strict_and_telemetry_is_disabled_metadata_only(self) -> None:
        schemas = EDITION / "project-brain" / "schemas"
        for path in schemas.glob("*.schema.json"):
            schema = json.loads(path.read_text(encoding="utf-8"))
            self.assertFalse(schema["additionalProperties"], path)
        telemetry = json.loads(
            EDITION.joinpath("project-brain/config/telemetry.json").read_text(
                encoding="utf-8"
            )
        )
        self.assertFalse(telemetry["enabled"])
        self.assertTrue(telemetry["metadata_only"])
        self.assertIn("prompt", telemetry["prohibited_fields"])


class AutomaticWorkingMemoryTest(RuntimeHarness):
    """Cover the automated read and write paths the memory hooks depend on."""

    def open_database(self) -> sqlite3.Connection:
        return context_cli.connect(self.repository / "memory-bank/local/context.db")

    def test_incremental_index_reuses_unchanged_sources(self) -> None:
        connection = self.open_database()
        try:
            first = context_cli.index_repository(connection, self.repository)
            self.assertFalse(first["incremental"])
            self.assertEqual(0, first["reused"])

            warm = context_cli.index_repository(
                connection, self.repository, incremental=True
            )
            self.assertTrue(warm["incremental"])
            self.assertEqual(first["documents"], warm["documents"])
            self.assertEqual(warm["documents"], warm["reused"])
            self.assertEqual(0, warm["removed"])
        finally:
            connection.close()

    def test_incremental_index_indexes_added_and_drops_deleted_sources(self) -> None:
        connection = self.open_database()
        try:
            context_cli.index_repository(connection, self.repository, incremental=True)
            added = self.repository / "specs/cadmium.md"
            added.write_text("# Cadmium\n\nThe cadmium marker.\n", encoding="utf-8")

            grown = context_cli.index_repository(
                connection, self.repository, incremental=True
            )
            self.assertEqual(
                ["specs/cadmium.md"],
                [item["path"] for item in context_cli.search_documents(
                    connection, "cadmium", 3
                )],
            )
            self.assertEqual(grown["documents"] - 1, grown["reused"])

            added.unlink()
            shrunk = context_cli.index_repository(
                connection, self.repository, incremental=True
            )
            self.assertEqual(1, shrunk["removed"])
            self.assertEqual(
                [], context_cli.search_documents(connection, "cadmium", 3)
            )
        finally:
            connection.close()

    def test_incremental_index_reindexes_modified_sources(self) -> None:
        connection = self.open_database()
        try:
            context_cli.index_repository(connection, self.repository, incremental=True)
            spec = self.repository / "specs/authority.md"
            spec.write_text(
                "# Authority\n\nThe rhodium authority rule is canonical.\n",
                encoding="utf-8",
            )
            os.utime(spec, (0, 0))  # Force a stat change even on a coarse clock.

            context_cli.index_repository(connection, self.repository, incremental=True)
            self.assertEqual(
                ["specs/authority.md"],
                [item["path"] for item in context_cli.search_documents(
                    connection, "rhodium", 3
                )],
            )
        finally:
            connection.close()

    def test_refresh_reports_every_memory_layer(self) -> None:
        self.repository.joinpath("AGENTS.md").write_text(
            "# Agents\n\nThe cobalt policy applies.\n", encoding="utf-8"
        )
        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changelog\n\n- Applied the cobalt rule.\n", encoding="utf-8"
        )
        result = self.run_cli("refresh", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(result.stdout)

        for layer in ("procedural", "semantic", "episodic"):
            self.assertEqual("updated", report[layer], layer)
            self.assertGreater(report["counts"][layer], 0, layer)
        self.assertEqual([], report["warnings"])
        self.assertIsNone(report["capsule"])

    def test_refresh_assembles_a_capsule_without_indexing_twice(self) -> None:
        self.start("TASK-REFRESH-CAPSULE")
        result = self.run_cli(
            "refresh", "--query", "cobalt", "--task-id", "TASK-REFRESH-CAPSULE",
            "--ephemeral", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual("updated", report["procedural"])
        self.assertIsNotNone(report["capsule"])
        self.assertEqual(
            "TASK-REFRESH-CAPSULE", report["capsule"]["working"]["task_id"]
        )
        self.assertEqual("local", report["capsule"]["manifest_scope"])
        # One pass only: the governed manifest store stays empty.
        self.assertEqual(
            [],
            list(
                self.repository.glob(
                    "project-brain/control/retrieval-manifests/*.json"
                )
            ),
        )

    def test_refresh_still_updates_layers_when_the_capsule_is_unavailable(self) -> None:
        result = self.run_cli(
            "refresh", "--query", "cobalt", "--task-id", "TASK-ABSENT", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual("updated", report["semantic"])
        self.assertIsNone(report["capsule"])
        self.assertTrue(
            any("Capsule unavailable" in item for item in report["warnings"]),
            report["warnings"],
        )

    def test_refresh_reports_failed_layers_instead_of_silently_serving_stale(
        self,
    ) -> None:
        self.run_cli("refresh")
        self.repository.joinpath("specs/broken.md").write_bytes(b"# \xff\xfe invalid\n")

        result = self.run_cli("refresh", "--json")
        self.assertEqual(1, result.returncode)
        report = json.loads(result.stdout)
        for layer in ("procedural", "semantic", "episodic"):
            self.assertEqual("failed", report[layer], layer)
        self.assertTrue(
            any("Index refresh failed" in item for item in report["warnings"]),
            report["warnings"],
        )

    def test_refresh_requires_a_task_for_a_capsule(self) -> None:
        result = self.run_cli("refresh", "--query", "cobalt")
        self.assertEqual(1, result.returncode)
        self.assertIn("--query requires --task-id", result.stderr)

    def test_retrieval_stems_so_a_related_word_form_still_matches(self) -> None:
        self.start("TASK-STEM")
        self.repository.joinpath("specs/reviewing.md").write_text(
            "# Reviewing\n\nA reviewer checks cobalt handling before merge.\n",
            encoding="utf-8",
        )
        self.run_cli("refresh")

        # "review" must reach a document that only ever says "reviewer".
        result = self.run_cli("search", "review cobalt", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(
            "specs/reviewing.md",
            [item["path"] for item in json.loads(result.stdout)["documents"]],
        )

    def test_a_focused_document_survives_on_one_distinctive_term(self) -> None:
        # Counting terms equally hides the answer: a note containing only the
        # rare term that matters scores 1, while filler sharing two ordinary
        # words scores 2 and wins the slot.
        self.start("TASK-FOCUSED")
        for index in range(8):
            self.repository.joinpath(f"specs/filler-{index}.md").write_text(
                f"# Filler {index}\n\nHow this project records ordinary work.\n",
                encoding="utf-8",
            )
        self.repository.joinpath("specs/currency.md").write_text(
            "# Currency\n\nAmounts are stored in integer zorkmids.\n",
            encoding="utf-8",
        )
        self.run_cli("refresh")

        selected, _ = self.selected_paths(
            "how does this project represent zorkmids", "TASK-FOCUSED"
        )
        self.assertIn("specs/currency.md", selected)

    def test_capsule_drops_documents_sharing_only_a_common_word(self) -> None:
        self.start("TASK-NOISE")
        for index in range(6):
            self.repository.joinpath(f"specs/filler-{index}.md").write_text(
                f"# Filler {index}\n\nThis document is written for the record.\n",
                encoding="utf-8",
            )
        self.repository.joinpath("specs/cobalt-rule.md").write_text(
            "# Cobalt Rule\n\nThe cobalt rule is written for every request path.\n",
            encoding="utf-8",
        )
        self.run_cli("refresh")

        result = self.run_cli(
            "retrieve", "what is the cobalt rule written for", "--task-id",
            "TASK-NOISE", "--ephemeral", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        selected = [
            item["path"]
            for group in json.loads(result.stdout)["categories"].values()
            for item in group
        ]
        self.assertIn("specs/cobalt-rule.md", selected)
        # Filler shares only "is"/"for"/"written"; that is not relevance.
        self.assertEqual([], [path for path in selected if "filler-" in path])

    def test_overlapping_source_patterns_index_a_file_once(self) -> None:
        self.repository.joinpath("docs").mkdir()
        self.repository.joinpath("docs/overview.md").write_text(
            "# Overview\n\nCobalt everywhere.\n", encoding="utf-8"
        )
        overlapping = context_cli.SOURCE_PATTERNS + (
            ("semantic", "codebase", "docs/*.md"),
        )
        with mock.patch.object(context_cli, "SOURCE_PATTERNS", overlapping):
            documents, _, state = context_cli.discover_documents(self.repository)

        paths = [row[0] for row in documents]
        self.assertEqual(len(paths), len(set(paths)))
        self.assertEqual(1, paths.count("docs/overview.md"))
        # The earlier pattern owns the file, so it stays a spec.
        self.assertEqual(
            "spec",
            next(row[2] for row in documents if row[0] == "docs/overview.md"),
        )
        self.assertIn("docs/overview.md", state)

    def write_codebase_map(self, commit: str, scope: str = "src") -> None:
        self.repository.joinpath("codebase").mkdir(exist_ok=True)
        self.repository.joinpath("codebase/STRUCTURE.md").write_text(
            "---\n"
            "description: Where cobalt handling lives.\n"
            f"mapped_commit: {commit}\n"
            f"mapped_scope: {scope}\n"
            "---\n\n"
            "# Codebase Structure\n\n"
            "- `src/Cobalt.php` — the cobalt guard.\n",
            encoding="utf-8",
        )

    def head(self) -> str:
        return subprocess.run(
            ["git", "-C", str(self.repository), "rev-parse", "HEAD"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()

    def selected_paths(self, query: str, task_id: str) -> tuple[list[str], dict]:
        result = self.run_cli(
            "retrieve", query, "--task-id", task_id, "--ephemeral", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        packet = json.loads(result.stdout)
        return (
            [item["path"] for group in packet["categories"].values() for item in group],
            json.loads((self.repository / packet["manifest"]).read_text(encoding="utf-8")),
        )

    def test_a_current_codebase_map_is_retrievable(self) -> None:
        self.start("TASK-MAP")
        self.commit_main()
        self.write_codebase_map(self.head())
        self.run_cli("refresh")

        selected, _ = self.selected_paths("cobalt guard structure", "TASK-MAP")
        self.assertIn("codebase/STRUCTURE.md", selected)

    def test_a_codebase_map_left_behind_by_the_code_is_excluded(self) -> None:
        self.start("TASK-DRIFT")
        self.commit_main()
        self.write_codebase_map(self.head())
        self.git("add", "-A")
        self.git("commit", "-qm", "map")
        self.repository.joinpath("src").mkdir(exist_ok=True)
        for index in range(30):
            self.repository.joinpath("src/Cobalt.php").write_text(
                f"<?php // {index}\n", encoding="utf-8"
            )
            self.git("add", "-A")
            self.git("commit", "-qm", f"change {index}")
        self.run_cli("refresh")

        selected, manifest = self.selected_paths("cobalt guard structure", "TASK-DRIFT")
        self.assertNotIn("codebase/STRUCTURE.md", selected)
        # A map that no longer describes the code must say so, not vanish.
        self.assertIn(
            {"path": "codebase/STRUCTURE.md", "reason": "map-drift"},
            manifest["excluded"],
        )

    def test_a_codebase_map_without_a_commit_is_not_assumed_current(self) -> None:
        self.start("TASK-UNVERIFIED")
        self.commit_main()
        self.repository.joinpath("codebase").mkdir(exist_ok=True)
        self.repository.joinpath("codebase/STRUCTURE.md").write_text(
            "---\ndescription: Where cobalt handling lives.\n---\n\n"
            "# Codebase Structure\n\n- `src/Cobalt.php` — the cobalt guard.\n",
            encoding="utf-8",
        )
        self.run_cli("refresh")

        selected, manifest = self.selected_paths(
            "cobalt guard structure", "TASK-UNVERIFIED"
        )
        self.assertNotIn("codebase/STRUCTURE.md", selected)
        self.assertIn(
            {"path": "codebase/STRUCTURE.md", "reason": "map-unverifiable"},
            manifest["excluded"],
        )

    def test_refresh_reports_how_far_a_map_has_drifted(self) -> None:
        self.commit_main()
        self.write_codebase_map(self.head())
        result = self.run_cli("refresh", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(
            [{"path": "codebase/STRUCTURE.md", "commits_behind": 0}],
            json.loads(result.stdout)["codebase"],
        )

    def test_a_task_phase_reaches_the_handoff(self) -> None:
        self.start("TASK-PHASE")
        result = self.run_cli(
            "update", "--task-id", "TASK-PHASE", "--revision", "auto",
            "--phase", "execution", "--progress", "Arithmetic done.", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        task = json.loads(result.stdout)
        self.assertEqual("execution", task["phase"])

        handoff = next(
            (self.repository / "project-brain/control/handoffs").glob("*.md")
        ).read_text(encoding="utf-8")
        # Progress says what was touched; the phase says where work stopped.
        self.assertIn("## Phase\nexecution", handoff)

    def test_a_record_written_before_phases_stays_valid(self) -> None:
        task = self.start("TASK-LEGACY")
        path = (
            self.repository / "project-brain/dynamic/tasks" / f"{task['task_uuid']}.md"
        )
        self.assertNotIn("phase", json.loads(path.read_text(encoding="utf-8").split("---")[1]))

        self.assertEqual(0, self.run_cli("validate").returncode)
        self.assertIsNone(
            json.loads(
                self.run_cli("get", "--task-id", "TASK-LEGACY", "--json").stdout
            )["phase"]
        )

    def test_only_a_task_may_declare_a_phase(self) -> None:
        record = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", "FIND-PHASE",
                "--title", "Not a task", "--source", "specs/authority.md", "--json",
            ).stdout
        )
        result = self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--phase", "execution",
        )
        self.assertEqual(1, result.returncode)
        self.assertIn("only a task may declare a phase", result.stderr)

    def test_governed_retrieve_refreshes_a_stale_index(self) -> None:
        self.start("TASK-REFRESH")
        self.repository.joinpath("specs/iridium.md").write_text(
            "# Iridium\n\nThe iridium rule is canonical.\n", encoding="utf-8"
        )
        # No explicit index call: retrieval must not read a stale index.
        result = self.run_cli(
            "retrieve", "iridium", "--task-id", "TASK-REFRESH", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        packet = json.loads(result.stdout)
        self.assertEqual([], packet["warnings"])
        self.assertIn(
            "specs/iridium.md",
            [item["path"] for group in packet["categories"].values() for item in group],
        )

    def test_ephemeral_manifest_stays_out_of_shared_history(self) -> None:
        self.start("TASK-EPHEMERAL")
        governed = self.repository / "project-brain/control/retrieval-manifests"
        local = self.repository / "memory-bank/local/retrieval-manifests"

        result = self.run_cli(
            "retrieve", "cobalt", "--task-id", "TASK-EPHEMERAL", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual("governed", json.loads(result.stdout)["manifest_scope"])
        self.assertEqual(1, len(list(governed.glob("*.json"))))

        for _ in range(3):
            result = self.run_cli(
                "retrieve", "cobalt", "--task-id", "TASK-EPHEMERAL",
                "--ephemeral", "--json",
            )
            self.assertEqual(0, result.returncode, result.stderr)
            packet = json.loads(result.stdout)
            self.assertEqual("local", packet["manifest_scope"])
            self.assertTrue(packet["manifest"].startswith("memory-bank/local/"))
        self.assertEqual(1, len(list(governed.glob("*.json"))))
        self.assertEqual(3, len(list(local.glob("*.json"))))

    def test_revision_auto_performs_a_locked_compare_and_swap(self) -> None:
        task = self.start("TASK-AUTO")
        for expected in (2, 3):
            result = self.run_cli(
                "update", "--task-id", "TASK-AUTO", "--revision", "auto",
                "--progress", f"Step {expected}.", "--json",
            )
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(expected, json.loads(result.stdout)["revision"])

        stale = self.run_cli(
            "update", "--task-id", "TASK-AUTO", "--revision", str(task["revision"]),
            "--progress", "Stale write.",
        )
        self.assertEqual(1, stale.returncode)
        self.assertIn("Stale task revision", stale.stderr)

    def test_revision_argument_rejects_non_positive_and_non_auto_values(self) -> None:
        for value in ("0", "-1", "later"):
            result = self.run_cli(
                "update", "--task-id", "TASK-AUTO", "--revision", value,
                "--progress", "Nope.",
            )
            self.assertEqual(2, result.returncode)
            self.assertIn("positive integer", result.stderr)

    def test_turn_buffers_locally_until_the_flush_boundary(self) -> None:
        self.start("TASK-TURN")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        for turn in (1, 2):
            result = self.run_cli(
                "turn", "--task-id", "TASK-TURN", "--flush-after", "3", "--json"
            )
            self.assertEqual(0, result.returncode, result.stderr)
            payload = json.loads(result.stdout)
            self.assertFalse(payload["flushed"])
            self.assertEqual(turn, payload["pending"])

        # Buffering exists so continuity costs one revision per boundary, not
        # one per turn: the task must not have moved yet.
        self.assertEqual(1, json.loads(self.run_cli(
            "get", "--task-id", "TASK-TURN", "--json"
        ).stdout)["revision"])

        result = self.run_cli(
            "turn", "--task-id", "TASK-TURN", "--flush-after", "3", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["flushed"])
        self.assertEqual(0, payload["pending"])

        task = json.loads(
            self.run_cli("get", "--task-id", "TASK-TURN", "--json").stdout
        )
        self.assertEqual(2, task["revision"])
        self.assertIn("Auto-checkpoint: 3 turn(s)", task["progress"])
        self.assertIn("app.txt", task["files"])

    def test_turn_provisions_the_task_on_first_flush(self) -> None:
        # No start: the automated path must not depend on a manual command.
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        tasks = self.repository / "project-brain/dynamic/tasks"

        result = self.run_cli(
            "turn", "--task-id", "feature/report-caching", "--flush", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["provisioned"])
        self.assertTrue(payload["flushed"])
        self.assertEqual(1, len(list(tasks.glob("*.md"))))

        task = json.loads(
            self.run_cli(
                "get", "--task-id", "feature/report-caching", "--json"
            ).stdout
        )
        self.assertEqual(
            "Report caching (auto-provisioned from feature/report-caching)",
            task["goal"],
        )
        self.assertIn("app.txt", task["files"])
        self.assertTrue(
            self.repository.joinpath(
                "project-brain/control/handoffs", f"{task['task_uuid']}.md"
            ).is_file()
        )

    def test_turn_does_not_provision_before_the_flush_boundary(self) -> None:
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        tasks = self.repository / "project-brain/dynamic/tasks"

        result = self.run_cli(
            "turn", "--task-id", "feature/idle", "--flush-after", "3", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(json.loads(result.stdout)["provisioned"])
        # Visiting a branch must not mint a governed record; only work does.
        self.assertEqual([], list(tasks.glob("*.md")))

    def test_turn_reuses_a_task_it_already_provisioned(self) -> None:
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        first = self.run_cli("turn", "--task-id", "feature/reuse", "--flush", "--json")
        self.assertTrue(json.loads(first.stdout)["provisioned"])

        self.repository.joinpath("other.txt").write_text("more\n", encoding="utf-8")
        second = self.run_cli("turn", "--task-id", "feature/reuse", "--flush", "--json")
        self.assertEqual(0, second.returncode, second.stderr)
        payload = json.loads(second.stdout)
        self.assertFalse(payload["provisioned"])
        self.assertEqual(3, payload["revision"])
        self.assertEqual(
            1,
            len(list((self.repository / "project-brain/dynamic/tasks").glob("*.md"))),
        )

    def test_turn_does_not_overwrite_a_manually_started_task(self) -> None:
        self.start("TASK-MANUAL")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = self.run_cli("turn", "--task-id", "TASK-MANUAL", "--flush", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(json.loads(result.stdout)["provisioned"])
        self.assertEqual(
            "Apply the cobalt authority rule.",
            json.loads(
                self.run_cli("get", "--task-id", "TASK-MANUAL", "--json").stdout
            )["goal"],
        )

    def test_derived_goal_strips_branch_prefixes_and_states_provenance(self) -> None:
        for task_id, expected in (
            ("feature/add-caching", "Add caching"),
            ("bugfix/null_pointer", "Null pointer"),
            ("merge/context-brain", "Context brain"),
            # A ticket identifier is a name, not two hyphenated words.
            ("TASK-42", "TASK-42"),
            ("feature/BAUMAS-133-add-caching", "BAUMAS-133 add caching"),
        ):
            with self.subTest(task_id=task_id):
                goal = context_cli.derive_goal(task_id)
                self.assertTrue(goal.startswith(expected), goal)
                self.assertIn(f"auto-provisioned from {task_id}", goal)

    def enable_automation(self, **flags: bool) -> None:
        config = self.repository / "project-brain/config/runtime.json"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(
            json.dumps({"mode": "governed", **flags}), encoding="utf-8"
        )

    def enable_automatic_promotion(self) -> None:
        self.enable_automation(automatic_promotion=True)

    def git(self, *arguments: str) -> None:
        subprocess.run(
            [
                "git", "-C", str(self.repository),
                "-c", "user.email=t@t", "-c", "user.name=t",
                *arguments,
            ],
            check=True,
            capture_output=True,
        )

    def commit_main(self) -> None:
        """Give the repository a real default branch to merge into."""
        self.git("add", "-A")
        self.git("commit", "-qm", "init")
        self.git("branch", "-M", "main")

    def test_turn_completes_a_task_when_its_branch_is_merged(self) -> None:
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/reports")
        self.repository.joinpath("report.txt").write_text("work\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-qm", "work")
        self.repository.joinpath("pending.txt").write_text("x\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/reports", "--flush", "--json")

        # Still on the branch and unmerged: nothing may close.
        result = self.run_cli("turn", "--task-id", "feature/reports", "--flush", "--json")
        self.assertEqual([], json.loads(result.stdout)["closed"])

        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "feature/reports", "-m", "merge")
        self.repository.joinpath("after.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")

        closed = json.loads(result.stdout)["closed"]
        self.assertEqual(["feature/reports"], [item["task_id"] for item in closed])
        self.assertIsNotNone(closed[0]["episode_id"])

        record = json.loads(
            next(
                path
                for path in (self.repository / "project-brain/dynamic/tasks").glob("*.md")
                if "feature/reports" in path.read_text(encoding="utf-8")
            ).read_text(encoding="utf-8").split("---")[1]
        )
        self.assertEqual("completed", record["status"])
        # The outcome may claim only the merge, which is all that was checked.
        self.assertIn("merged into main", record["progress"])
        self.assertIn("is an ancestor of main", record["progress"])
        self.assertIn("Last recorded progress", record["progress"])

    def test_automatic_completion_ignores_a_branch_that_never_diverged(self) -> None:
        # A branch created a moment ago is already an ancestor of its target,
        # so ancestry alone would complete the task the instant it existed.
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/fresh")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = self.run_cli("turn", "--task-id", "feature/fresh", "--flush", "--json")
        self.assertEqual([], json.loads(result.stdout)["closed"])
        self.assertEqual(
            "active",
            json.loads(
                self.run_cli("get", "--task-id", "feature/fresh", "--json").stdout
            )["status"],
        )

    def test_automatic_completion_ignores_a_branch_kept_current_with_target(
        self,
    ) -> None:
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/long-lived")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-qm", "branch work")
        self.run_cli("turn", "--task-id", "feature/long-lived", "--flush", "--json")

        self.git("checkout", "-q", "main")
        self.repository.joinpath("main.txt").write_text("main\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-qm", "main work")
        self.git("checkout", "-q", "feature/long-lived")
        self.git("merge", "-q", "--no-ff", "main", "-m", "keep current")

        self.repository.joinpath("more.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli(
            "turn", "--task-id", "feature/long-lived", "--flush", "--json"
        )
        # Merging main in does not make the branch an ancestor of main.
        self.assertEqual([], json.loads(result.stdout)["closed"])

    def test_automatic_completion_never_closes_the_default_branch_task(self) -> None:
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        # A branch is its own ancestor, so this would close itself immediately.
        self.run_cli("turn", "--task-id", "main", "--flush", "--json")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")

        self.assertEqual([], json.loads(result.stdout)["closed"])
        self.assertEqual(
            "active",
            json.loads(self.run_cli("get", "--task-id", "main", "--json").stdout)[
                "status"
            ],
        )

    def test_automatic_completion_ignores_a_deleted_branch(self) -> None:
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/abandoned")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/abandoned", "--flush", "--json")
        self.git("checkout", "-q", "main")
        self.git("branch", "-qD", "feature/abandoned")

        self.repository.joinpath("other.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")

        # Deleted cannot be told apart from abandoned, so it must not complete.
        self.assertEqual([], json.loads(result.stdout)["closed"])

    def test_automatic_completion_stays_off_unless_configured(self) -> None:
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/quiet")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/quiet", "--flush", "--json")
        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "feature/quiet", "-m", "merge")

        self.repository.joinpath("other.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")
        self.assertEqual([], json.loads(result.stdout)["closed"])

    def accepted_decision(self, external_id: str = "DEC-1") -> str:
        record = json.loads(
            self.run_cli(
                "brain-create", "decision", "--external-id", external_id,
                "--title", "Cobalt rule is canonical",
                "--source", "specs/authority.md", "--authority", "verified",
                "--json",
            ).stdout
        )
        self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--progress", "Cobalt applies to every request path, not only reads.",
            "--transition", "accepted", "--reason", "Accepted", "--json",
        )
        return str(record["id"])

    def test_turn_promotes_resolved_verified_knowledge_without_review(self) -> None:
        self.enable_automatic_promotion()
        record_id = self.accepted_decision()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        promoted = json.loads(result.stdout)["promoted"]
        self.assertEqual(1, len(promoted))
        self.assertEqual(record_id, promoted[0]["record_id"])

        chunk = next(
            (self.repository / "memory-bank/chunks").glob(
                f"{promoted[0]['memory_id']}-*.md"
            )
        )
        # The bank itself must show which knowledge no human approved.
        self.assertIn("auto-promoted", chunk.read_text(encoding="utf-8"))

    def test_promotion_opens_a_validity_period(self) -> None:
        self.enable_automatic_promotion()
        self.accepted_decision()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")

        chunk = next((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        metadata = json.loads(chunk.read_text(encoding="utf-8").split("---")[1])
        self.assertEqual(metadata["created"], metadata["valid_from"])
        # Open-ended: a successor closes the period, nothing deletes the chunk.
        self.assertIsNone(metadata["valid_to"])

    def test_knowledge_past_its_validity_may_not_call_itself_active(self) -> None:
        self.enable_automatic_promotion()
        self.accepted_decision()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")

        chunk = next((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        _, frontmatter, body = chunk.read_text(encoding="utf-8").split("---\n", 2)
        metadata = json.loads(frontmatter)

        index = self.repository / "memory-bank/INDEX.md"

        def rewrite(**changes: object) -> None:
            chunk.write_text(
                "---\n" + json.dumps({**metadata, **changes}, indent=2, sort_keys=True)
                + "\n---\n" + body,
                encoding="utf-8",
            )
            # The index carries the status too, and the bank refuses to let the
            # two disagree.
            status = str(changes.get("status", metadata["status"]))
            index.write_text(
                index.read_text(encoding="utf-8").replace(
                    f"| {metadata['status']} |", f"| {status} |"
                ),
                encoding="utf-8",
            )

        # A period that opened and closed before today, as a superseded fact's
        # would: knowledge may predate the chunk that records it.
        rewrite(valid_from="2019-01-01", valid_to="2020-01-01")
        errors = brain.validate_bank(self.repository / "memory-bank")
        self.assertTrue(
            any("past its valid_to" in error for error in errors), errors
        )

        # `superseded` demands a successor, so knowledge that simply ceased is
        # archived. The period says when it held; the status says whether
        # anything took its place.
        rewrite(valid_from="2019-01-01", valid_to="2020-01-01", status="archived")
        self.assertEqual([], brain.validate_bank(self.repository / "memory-bank"))
        # Closed knowledge stays on disk but leaves retrieval.
        self.assertFalse(context_cli.active_memory(chunk, self.repository))

    def test_a_chunk_written_before_validity_periods_stays_valid(self) -> None:
        bank = self.repository / "memory-bank"
        (bank / "chunks/MEM-0001-legacy.md").write_text(
            "---\n"
            + json.dumps(
                {
                    "id": "MEM-0001", "title": "Legacy", "type": "decision",
                    "status": "active", "scope": ["application"], "tags": ["legacy"],
                    "created": "2026-01-01", "last_verified": "2026-01-01",
                    "review_after": "2099-01-01", "sources": ["specs/authority.md"],
                    "supersedes": [], "superseded_by": None,
                }
            )
            + "\n---\n\n# Legacy\n\nStill true.\n",
            encoding="utf-8",
        )
        (bank / "INDEX.md").write_text(
            (bank / "INDEX.md").read_text(encoding="utf-8")
            + "| MEM-0001 | Legacy | decision | application | legacy | active |"
            " 2026-01-01 | chunks/MEM-0001-legacy.md |\n",
            encoding="utf-8",
        )
        (bank / ".memory-counter").write_text("2\n", encoding="utf-8")

        self.assertEqual([], brain.validate_bank(bank))

    def test_automatic_promotion_never_claims_a_reviewer(self) -> None:
        self.enable_automatic_promotion()
        self.accepted_decision()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")

        promotion = json.loads(
            next(
                (self.repository / "project-brain/control/promotions").glob("*.json")
            ).read_text(encoding="utf-8")
        )
        self.assertEqual("automatic", promotion["review_mode"])
        self.assertIsNone(promotion["reviewer"])
        self.assertEqual("applied", promotion["status"])

    def test_automatic_promotion_is_not_repeated_for_the_same_record(self) -> None:
        self.enable_automatic_promotion()
        self.accepted_decision()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")

        self.repository.joinpath("more.txt").write_text("more\n", encoding="utf-8")
        second = self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")
        self.assertEqual([], json.loads(second.stdout)["promoted"])
        self.assertEqual(
            1, len(list((self.repository / "memory-bank/chunks").glob("MEM-*.md")))
        )

    def test_automatic_promotion_excludes_tasks_and_unverified_records(self) -> None:
        self.enable_automatic_promotion()
        # An auto-provisioned task is checkpoint bookkeeping, not knowledge.
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")

        observed = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", "FIND-1",
                "--title", "Observed only", "--source", "specs/authority.md",
                "--authority", "observed", "--json",
            ).stdout
        )
        self.run_cli(
            "brain-update", "--record-id", observed["id"], "--revision", "auto",
            "--transition", "resolved", "--reason", "Resolved", "--json",
        )
        self.repository.joinpath("more.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")

        self.assertEqual([], json.loads(result.stdout)["promoted"])
        self.assertEqual(
            [], list((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        )

    def test_automatic_promotion_stays_off_unless_configured(self) -> None:
        self.accepted_decision()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")
        self.assertEqual([], json.loads(result.stdout)["promoted"])

    def test_automatic_promotion_cannot_be_dressed_up_as_human_review(self) -> None:
        self.enable_automatic_promotion()
        self.accepted_decision()
        promotion = json.loads(
            self.run_cli(
                "promote-propose", "--source-id", "DEC-1", "--title", "Manual",
                "--content", "Manual content", "--json",
            ).stdout
        )
        path = (
            self.repository / "project-brain/control/promotions"
            / f"{promotion['id']}.json"
        )
        record = json.loads(path.read_text(encoding="utf-8"))
        record["review_mode"] = "automatic"
        path.write_text(json.dumps(record), encoding="utf-8")

        result = self.run_cli(
            "promote-review", "--promotion-id", promotion["id"],
            "--reviewer", "someone-else",
        )
        self.assertEqual(1, result.returncode)
        self.assertIn("not human-reviewable", result.stderr)

    def resolved_finding(self, index: int) -> str:
        record = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", f"FIND-{index}",
                "--title", f"Finding {index}", "--source", "specs/authority.md",
                "--authority", "verified", "--json",
            ).stdout
        )
        self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--progress", f"Finding {index} resolved by tightening the cobalt guard.",
            "--transition", "resolved", "--reason", "Resolved", "--json",
        )
        return str(record["id"])

    def test_compaction_waits_for_the_threshold_before_moving_files(self) -> None:
        self.enable_automation(automatic_compaction=True, compaction_threshold=3)
        for index in range(2):
            self.resolved_finding(index)
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json")
        payload = json.loads(result.stdout)
        self.assertEqual(0, payload["archived"])
        self.assertEqual(2, payload["archivable_pending"])
        self.assertEqual(
            [], list((self.repository / "project-brain/archive").glob("**/*.md"))
        )

        self.resolved_finding(2)
        self.repository.joinpath("more.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json")
        payload = json.loads(result.stdout)
        self.assertEqual(3, payload["archived"])
        self.assertEqual(0, payload["archivable_pending"])
        self.assertEqual(
            3,
            len(
                list(
                    (self.repository / "project-brain/archive/finding").glob("*.md")
                )
            ),
        )

    def test_compaction_never_archives_knowledge_before_it_is_promoted(self) -> None:
        # A resolved finding is both promotable and archivable. Promotion is
        # capped per run, so compaction can reach a record first; an archived
        # record must stay promotable or the knowledge is lost for good.
        self.enable_automation(
            automatic_promotion=True, automatic_compaction=True,
            compaction_threshold=1,
        )
        for index in range(7):
            self.resolved_finding(index)
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        first = json.loads(
            self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json").stdout
        )
        self.assertEqual(5, len(first["promoted"]))
        self.assertGreater(first["archived"], 0)

        self.repository.joinpath("more.txt").write_text("more\n", encoding="utf-8")
        second = json.loads(
            self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json").stdout
        )
        self.assertEqual(2, len(second["promoted"]))
        self.assertEqual(
            7, len(list((self.repository / "memory-bank/chunks").glob("MEM-*.md")))
        )

    def test_compaction_repoints_promoted_chunks_at_the_archive(self) -> None:
        # A chunk cites its source by path. Archiving the record without
        # repointing the citation leaves the bank permanently invalid, which
        # blocks every later promotion.
        self.enable_automation(automatic_promotion=True)
        self.resolved_finding(0)
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json")

        chunk = next((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        self.assertIn("project-brain/dynamic/findings/", chunk.read_text(encoding="utf-8"))

        self.assertEqual(0, self.run_cli("compact").returncode)

        self.assertIn(
            "project-brain/archive/finding/", chunk.read_text(encoding="utf-8")
        )
        self.assertNotIn(
            "project-brain/dynamic/findings/", chunk.read_text(encoding="utf-8")
        )
        bank = subprocess.run(
            [
                sys.executable,
                str(CONTEXT_SCRIPT.parent / "validate.py"),
                str(self.repository / "memory-bank"),
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(0, bank.returncode, bank.stdout + bank.stderr)

    def test_compaction_stays_off_unless_configured(self) -> None:
        for index in range(6):
            self.resolved_finding(index)
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json")
        self.assertEqual(0, json.loads(result.stdout)["archived"])
        self.assertEqual(
            [], list((self.repository / "project-brain/archive").glob("**/*.md"))
        )

    def test_compaction_archives_a_completed_task_with_its_handoff(self) -> None:
        self.enable_automation(
            automatic_completion=True, automatic_compaction=True,
            compaction_threshold=1,
        )
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/archived")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/archived", "--flush", "--json")
        self.git("add", "-A")
        self.git("commit", "-qm", "work")
        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "feature/archived", "-m", "merge")

        self.repository.joinpath("after.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")

        payload = json.loads(result.stdout)
        self.assertEqual(["feature/archived"], [i["task_id"] for i in payload["closed"]])
        self.assertGreater(payload["archived"], 0)
        self.assertEqual(
            1, len(list((self.repository / "project-brain/archive/task").glob("*.md")))
        )
        self.assertEqual(
            1,
            len(
                list(
                    (self.repository / "project-brain/archive/handoffs").glob("*.md")
                )
            ),
        )

    def test_turn_excludes_sensitive_paths_and_runtime_churn(self) -> None:
        self.start("TASK-SAFE")
        self.repository.joinpath(".env.local").write_text("TOKEN=1\n", encoding="utf-8")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = self.run_cli(
            "turn", "--task-id", "TASK-SAFE", "--flush", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual([".env.local"], payload["excluded"])

        files = json.loads(
            self.run_cli("get", "--task-id", "TASK-SAFE", "--json").stdout
        )["files"]
        self.assertIn("app.txt", files)
        self.assertNotIn(".env.local", files)
        # The runtime's own record, handoff, and index writes are not user work.
        self.assertEqual(
            [], [item for item in files if item.startswith("project-brain/")]
        )

    def test_turn_reports_paths_beyond_the_per_flush_limit(self) -> None:
        self.start("TASK-LIMIT")
        for index in range(6):
            self.repository.joinpath(f"file-{index}.txt").write_text(
                "work\n", encoding="utf-8"
            )

        result = self.run_cli(
            "turn", "--task-id", "TASK-LIMIT", "--flush", "--max-files", "2", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["flushed"])
        self.assertGreater(payload["files_omitted"], 0)

        task = json.loads(
            self.run_cli("get", "--task-id", "TASK-LIMIT", "--json").stdout
        )
        self.assertEqual(2, len(task["files"]))
        self.assertIn("beyond the per-flush limit", task["progress"])


if __name__ == "__main__":
    unittest.main()
