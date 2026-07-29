#!/usr/bin/env python3
"""Contract tests for the combined Project Brain + Local Context Engine."""

from __future__ import annotations

import importlib.util
import contextlib
import io
import json
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


class ProjectBrainRuntimeTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="project-brain-test-")
        self.repository = Path(self.temporary.name)
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


if __name__ == "__main__":
    unittest.main()
