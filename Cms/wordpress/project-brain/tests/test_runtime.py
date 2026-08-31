#!/usr/bin/env python3
"""Contract tests for the combined Project Brain + Local Context Engine."""

from __future__ import annotations

import importlib.util
import contextlib
import io
import json
import os
import sqlite3
import shutil
import subprocess
import sys
import tempfile
import threading
import unittest
from datetime import datetime, timedelta, timezone
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
    import telemetry as telemetry_runtime
finally:
    sys.path.pop(0)


class DirectQueryPrivacyBoundaryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="query-privacy-test-")
        self.repository = Path(self.temporary.name)
        self.database = self.repository / "sentinel.db"
        self.database.write_bytes(b"unchanged-index-sentinel")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_direct_queries_reject_before_database_or_manifest_mutation(self) -> None:
        unsafe_queries = (
            "person@example.test",
            "Customer id: 10492",
            "Call +370 600 12345",
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ123456",
            "User: copied request",
            "Assistant: copied response",
        )
        for mode in ("governed", "lightweight"):
            for command in ("refresh", "retrieve", "context"):
                for ephemeral in (False, True):
                    for query in unsafe_queries:
                        with self.subTest(
                            mode=mode,
                            command=command,
                            ephemeral=ephemeral,
                            query=query.split(":", 1)[0],
                        ):
                            arguments = [
                                sys.executable,
                                str(CONTEXT_SCRIPT),
                                "--root",
                                str(self.repository),
                                "--db",
                                str(self.database),
                                "--mode",
                                mode,
                                command,
                            ]
                            if command == "refresh":
                                arguments.extend(
                                    ["--query", query, "--task-id", "TASK-PRIVACY"]
                                )
                            else:
                                arguments.extend(
                                    [query, "--task-id", "TASK-PRIVACY"]
                                )
                            if ephemeral:
                                arguments.append("--ephemeral")
                            result = subprocess.run(
                                arguments, text=True, capture_output=True
                            )
                            self.assertNotEqual(0, result.returncode)
                            self.assertEqual("", result.stdout)
                            self.assertNotIn(query, result.stderr)
                            self.assertEqual(
                                b"unchanged-index-sentinel",
                                self.database.read_bytes(),
                            )
                            self.assertEqual(
                                [],
                                list(self.repository.glob("**/*manifest*.json")),
                            )


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

    @staticmethod
    def expected_memory_id(source_uuid: str) -> str:
        """Conflict-free chunk ID: promotion date + source-record UUID hex."""
        today = datetime.now(timezone.utc).date()
        return f"MEM-{today.strftime('%Y%m%d')}-{source_uuid.replace('-', '')[:8]}"

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

    def resolve_verified_finding(
        self, record: dict, progress: str = "Verified reusable consequence."
    ) -> dict:
        verified = brain.update_record(
            self.repository,
            record["id"],
            expected_revision=record["revision"],
            progress=None,
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
            authority="verified",
            reason="Verified against the cited source",
        )
        return brain.update_record(
            self.repository,
            record["id"],
            expected_revision=verified["revision"],
            progress=progress,
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
            transition_to="resolved",
            reason="Resolved after verification",
        )


class ProjectBrainRuntimeTest(RuntimeHarness):
    def test_governed_capsule_enforces_layer_and_character_contract_deterministically(
        self,
    ) -> None:
        self.start("TASK-CAPSULE-LIMITS")
        for index in range(8):
            self.repository.joinpath(f"specs/cobalt-{index}.md").write_text(
                f"# Cobalt {index}\n\n"
                + "cobalt capsule deterministic ranking "
                + ("bounded context " * 600),
                encoding="utf-8",
            )
        for filename in ("AGENTS.md", "CLAUDE.md"):
            self.repository.joinpath(filename).write_text(
                "# Cobalt policy\n\ncobalt capsule deterministic ranking\n",
                encoding="utf-8",
            )
        first = self.run_cli(
            "retrieve", "cobalt capsule deterministic ranking",
            "--task-id", "TASK-CAPSULE-LIMITS", "--limit", "20",
            "--ephemeral", "--json",
        )
        second = self.run_cli(
            "retrieve", "cobalt capsule deterministic ranking",
            "--task-id", "TASK-CAPSULE-LIMITS", "--limit", "20",
            "--ephemeral", "--json",
        )
        self.assertEqual(0, first.returncode, first.stderr)
        self.assertEqual(0, second.returncode, second.stderr)
        left, right = json.loads(first.stdout), json.loads(second.stdout)
        self.assertLessEqual(len(left["procedural"]), 2)
        self.assertLessEqual(len(left["semantic"]), 3)
        self.assertLessEqual(len(left["episodic"]), 1)
        self.assertLessEqual(len(first.stdout.strip()), 8000)
        for layer in ("procedural", "semantic", "episodic"):
            self.assertEqual(left[layer], right[layer])
        manifest = json.loads(
            (self.repository / left["manifest"]).read_text(encoding="utf-8")
        )
        self.assertTrue(
            any(item["reason"] == "layer-limit" for item in manifest["excluded"])
        )

    def test_capsule_says_when_memory_matched_nothing(self) -> None:
        # Until this line existed, a capsule that found nothing and a capsule
        # that was never consulted rendered identically, so any policy telling
        # the model to "refuse when there is no data" was asking it to observe
        # something the harness never reported.
        self.start("TASK-NO-MATCH")
        self.repository.joinpath("README.md").write_text(
            "# Overview\n\nzorkmid ledger overview.\n", encoding="utf-8"
        )

        rendered = self.run_cli(
            "retrieve", "unladen swallow airspeed velocity",
            "--task-id", "TASK-NO-MATCH", "--ephemeral",
        )
        self.assertEqual(0, rendered.returncode, rendered.stderr)
        # The Cursor hooks accept a capsule only if it opens with "working:".
        self.assertTrue(rendered.stdout.startswith("working:"), rendered.stdout)
        self.assertIn("no-match: episodic, procedural, semantic", rendered.stdout)

        payload = self.run_cli(
            "retrieve", "unladen swallow airspeed velocity",
            "--task-id", "TASK-NO-MATCH", "--ephemeral", "--json",
        )
        capsule = json.loads(payload.stdout)
        self.assertEqual(
            ["procedural", "semantic", "episodic"], capsule["no_match"], capsule
        )
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        # Nothing selected AND nothing excluded is what distinguishes "memory
        # found nothing" from "memory found something a filter withheld".
        self.assertEqual([], manifest["selected"], manifest)
        self.assertEqual([], manifest["excluded"], manifest)

    def test_a_layer_emptied_by_a_filter_is_not_reported_as_no_match(self) -> None:
        # The distinction the previous test rests on, from the other side: a
        # layer whose candidate was withheld downstream must not claim memory
        # had nothing, or the two states collapse again.
        self.start("TASK-FILTERED")
        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changelog\n\n## Unreleased\n\ncobalt authority rollout.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )

        # The query reaches the task's own handoff, which the lifecycle filter
        # withholds: without a document a filter actually removes, "a layer a
        # filter emptied" has no instance and the test proves nothing.
        payload = self.run_cli(
            "retrieve", "cobalt authority rule", "--task-id", "TASK-FILTERED",
            "--ephemeral", "--json",
        )
        self.assertEqual(0, payload.returncode, payload.stderr)
        capsule = json.loads(payload.stdout)
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        # Both layers found candidates, so neither may claim memory had
        # nothing — only `procedural`, which genuinely matched nothing, is
        # entitled to say so.
        self.assertEqual(["procedural"], capsule["no_match"], capsule)
        self.assertTrue(capsule["semantic"], capsule)
        self.assertTrue(capsule["episodic"], capsule)
        # CHANGELOG.md is the episodic layer's own document and is now ranked,
        # filtered and recorded through the governed path rather than fetched
        # around it, so it appears in the manifest it is delivered under.
        self.assertIn(
            "CHANGELOG.md", [item["path"] for item in capsule["episodic"]], capsule
        )
        self.assertIn(
            "CHANGELOG.md", [item["path"] for item in manifest["selected"]], manifest
        )
        # The other half — a document a filter removed arriving in `excluded`
        # with its reason rather than silently shrinking a layer — is covered
        # by test_no_document_occupies_two_layers_of_one_capsule, whose
        # fixture produces a lifecycle exclusion.

    def test_capsule_marks_an_item_admitted_on_a_single_rare_term(self) -> None:
        # A distinctive match needs both query terms to exist in the corpus
        # (an absent term is dropped as uninformative before coverage is
        # counted) and the corpus to be large enough for one document to count
        # as rare: `rare` is `hits <= 0.1 * documents`, so ten fillers are the
        # minimum that lets a single-document term qualify.
        self.start("TASK-WEAK")
        self.repository.joinpath("README.md").write_text(
            "# Overview\n\nzorkmid ledger overview.\n", encoding="utf-8"
        )
        self.repository.joinpath("specs/plover.md").write_text(
            "# Plover\n\nplover migration notes.\n", encoding="utf-8"
        )
        for index in range(10):
            self.repository.joinpath(f"specs/filler-{index}.md").write_text(
                f"# Filler {index}\n\nunrelated boilerplate paragraph {index}.\n",
                encoding="utf-8",
            )

        rendered = self.run_cli(
            "retrieve", "zorkmid plover", "--task-id", "TASK-WEAK", "--ephemeral",
        )
        self.assertEqual(0, rendered.returncode, rendered.stderr)
        self.assertIn("weak-match: README.md", rendered.stdout)

        payload = self.run_cli(
            "retrieve", "zorkmid plover", "--task-id", "TASK-WEAK",
            "--ephemeral", "--json",
        )
        capsule = json.loads(payload.stdout)
        strengths = {
            item["path"]: item.get("match")
            for layer in ("procedural", "semantic", "episodic")
            for item in capsule[layer]
            if "path" in item
        }
        self.assertEqual("distinctive", strengths.get("README.md"), capsule)
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        self.assertEqual(
            "distinctive",
            {item["path"]: item["match"] for item in manifest["selected"]}["README.md"],
            manifest,
        )

    def test_a_covering_match_costs_the_capsule_nothing_to_declare(self) -> None:
        # The capsule is zero-sum against an 8,000-character ceiling and each
        # selected item is serialized up to three times, so the common verdict
        # is carried by its absence: the manifest records it in full, the
        # capsule spends characters only on the answer that needs a caveat.
        self.start("TASK-COVERED")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )

        payload = self.run_cli(
            "retrieve", "cobalt authority", "--task-id", "TASK-COVERED",
            "--ephemeral", "--json",
        )
        capsule = json.loads(payload.stdout)
        semantic = {item["path"]: item for item in capsule["semantic"]}
        self.assertIn("specs/authority.md", semantic, capsule)
        self.assertNotIn("match", semantic["specs/authority.md"], capsule)
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        self.assertEqual(
            "covered",
            {item["path"]: item["match"] for item in manifest["selected"]}[
                "specs/authority.md"
            ],
            manifest,
        )
        rendered = self.run_cli(
            "retrieve", "cobalt authority", "--task-id", "TASK-COVERED", "--ephemeral",
        )
        self.assertNotIn("weak-match", rendered.stdout)

    def latest_manifest(self, capsule: dict) -> dict:
        return json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )

    def test_manifest_records_the_signals_a_retrieval_gate_needs(self) -> None:
        # Everything a gate decision needs is already computed during a
        # retrieval and was thrown away at the end of it. A manifest that
        # records only the selection cannot answer, later, whether a bad
        # capsule came from a bad query or a bad ranking.
        self.start("TASK-SIGNALS")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        payload = self.run_cli(
            "retrieve", "cobalt authority", "--task-id", "TASK-SIGNALS",
            "--ephemeral", "--json",
        )
        self.assertEqual(0, payload.returncode, payload.stderr)
        manifest = self.latest_manifest(json.loads(payload.stdout))

        self.assertEqual(3, manifest["schema_version"], manifest)
        self.assertEqual("explicit", manifest["query_source"], manifest)
        phases = manifest["phase_seconds"]
        self.assertEqual({"stat", "index", "retrieval"}, set(phases), phases)
        self.assertIsInstance(phases["retrieval"], (int, float))
        self.assertGreaterEqual(phases["retrieval"], 0)
        entry = next(
            item for item in manifest["selected"]
            if item["path"] == "specs/authority.md"
        )
        self.assertIsInstance(entry["score"], (int, float))
        self.assertGreaterEqual(entry["score"], 0)
        self.assertIsInstance(entry["rank"], int)
        self.assertGreaterEqual(entry["rank"], 1)

    def test_manifest_records_safe_host_and_entry_point_provenance(self) -> None:
        self.start("TASK-HOST-PROVENANCE")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        for host in ("cli", "claude", "codex", "cursor"):
            with self.subTest(host=host):
                result = self.run_cli(
                    "retrieve", "cobalt authority",
                    "--task-id", "TASK-HOST-PROVENANCE",
                    "--host", host, "--ephemeral", "--json",
                )
                self.assertEqual(0, result.returncode, result.stderr)
                manifest = self.latest_manifest(json.loads(result.stdout))
                self.assertEqual(host, manifest["host"], manifest)
                self.assertEqual("retrieve", manifest["entry_point"], manifest)

        context = self.run_cli(
            "context", "cobalt authority",
            "--task-id", "TASK-HOST-PROVENANCE",
            "--host", "cli", "--ephemeral", "--json",
        )
        self.assertEqual(0, context.returncode, context.stderr)
        self.assertEqual(
            "context", self.latest_manifest(json.loads(context.stdout))["entry_point"]
        )
        hook = self.run_cli(
            "hook-context", "--task-id", "TASK-HOST-PROVENANCE",
            "--host", "cursor", "--json",
        )
        self.assertEqual(0, hook.returncode, hook.stderr)
        self.assertEqual(
            "hook-context", self.latest_manifest(json.loads(hook.stdout))["entry_point"]
        )
        refresh = self.run_cli(
            "refresh", "--query", "cobalt authority",
            "--task-id", "TASK-HOST-PROVENANCE", "--host", "claude",
            "--ephemeral", "--json",
        )
        self.assertEqual(0, refresh.returncode, refresh.stderr)
        self.assertEqual(
            "refresh",
            self.latest_manifest(json.loads(refresh.stdout)["capsule"])["entry_point"],
        )

    def test_retrieval_manifest_template_satisfies_the_v3_schema(self) -> None:
        template = json.loads(
            EDITION.joinpath(
                "project-brain/templates/retrieval-manifest.json"
            ).read_text(encoding="utf-8")
        )
        template.update(
            {
                "id": "00000000-0000-4000-8000-000000000010",
                "created_at": "2026-08-31T00:00:00+00:00",
                "task_id": "00000000-0000-4000-8000-000000000011",
            }
        )
        self.assertEqual(3, template["schema_version"], template)
        self.assertEqual(0, template["local_episode_count"], template)
        self.assertEqual(0, template["token_estimates"]["local_episodes"], template)
        brain.validate_schema_file(
            EDITION, "retrieval-manifest.schema.json", template
        )

    def test_manifest_records_where_the_query_came_from(self) -> None:
        # A gate that cannot tell a real request from a branch name would be
        # deciding on the strength of a slug.
        self.start("TASK-PROVENANCE")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        prompt = self.run_cli(
            "refresh", "--query", "how is cobalt authority specified",
            "--task-id", "TASK-PROVENANCE", "--ephemeral", "--json",
        )
        self.assertEqual(0, prompt.returncode, prompt.stderr)
        capsule = json.loads(prompt.stdout)["capsule"]
        self.assertIsNotNone(capsule, prompt.stdout)
        self.assertEqual("prompt", self.latest_manifest(capsule)["query_source"])

        # The Cursor-facing path supplies a task identifier, and the query is
        # built from the task behind it - `task`, not `task-id`.
        hook = self.run_cli(
            "hook-context", "--task-id", "TASK-PROVENANCE", "--json",
        )
        self.assertEqual(0, hook.returncode, hook.stderr)
        self.assertEqual(
            "task", self.latest_manifest(json.loads(hook.stdout))["query_source"]
        )

    def test_a_conflict_partner_reports_no_lexical_rank(self) -> None:
        # A conflict partner is pulled in by record id, never ranked against
        # the query. Reporting a rank for it would invent a relevance it never
        # had; the manifest says so instead.
        self.start("TASK-CONFLICT-RANK")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        payload = self.run_cli(
            "retrieve", "cobalt authority", "--task-id", "TASK-CONFLICT-RANK",
            "--ephemeral", "--json",
        )
        manifest = self.latest_manifest(json.loads(payload.stdout))
        for item in manifest["selected"]:
            self.assertIn("score", item, item)
            self.assertTrue(
                item["rank"] is None or item["rank"] >= 1, item
            )

    def test_local_manifest_retention_is_configurable_and_floored(self) -> None:
        self.start("TASK-RETENTION")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        config_path = self.repository / "project-brain/config/runtime.json"

        def retrieve_five(retention: object) -> str:
            config_path.parent.mkdir(parents=True, exist_ok=True)
            config = (
                json.loads(config_path.read_text(encoding="utf-8"))
                if config_path.is_file()
                else {"mode": "governed"}
            )
            config["local_manifest_retention"] = retention
            config_path.write_text(json.dumps(config, indent=2), encoding="utf-8")
            last = ""
            for index in range(5):
                result = self.run_cli(
                    "retrieve", f"cobalt authority {index}",
                    "--task-id", "TASK-RETENTION", "--ephemeral", "--json",
                )
                self.assertEqual(0, result.returncode, result.stderr)
                last = json.loads(result.stdout)["manifest"]
            return last

        directory = self.repository / "memory-bank/local/retrieval-manifests"
        last = retrieve_five(3)
        self.assertLessEqual(len(list(directory.glob("*.json"))), 3)
        # The window must never eat the manifest the caller was just handed.
        self.assertTrue((self.repository / last).is_file(), last)

        for bogus in (0, -1, "many"):
            with self.subTest(retention=bogus):
                last = retrieve_five(bogus)
                self.assertTrue((self.repository / last).is_file(), last)

    def test_a_version_one_manifest_still_validates(self) -> None:
        # The migration guarantee for installed projects: upgrading the engine
        # must not turn every manifest a consuming project already wrote into
        # a validation error, which is what a single frozen key set would do.
        self.start("TASK-MIGRATION")
        manifests = self.repository / "project-brain/control/retrieval-manifests"
        manifests.mkdir(parents=True, exist_ok=True)
        legacy_id = "00000000-0000-4000-8000-000000000001"
        legacy = {
            "schema_version": 1,
            "id": legacy_id,
            "created_at": "2026-01-01T00:00:00+00:00",
            "query": "cobalt authority",
            "task_id": "00000000-0000-4000-8000-000000000002",
            "task_revision": 1,
            "filters": {
                "privacy": ["public"], "owners": ["*"],
                "authority": ["verified"], "freshness": True, "active_only": True,
            },
            "selected": [],
            "excluded": [],
            "token_estimates": {
                "policy": 0, "handoff": 0, "durable": 0, "dynamic": 0,
                "evidence": 0, "total": 0, "target": 8000, "hard": 12000,
            },
            "provider": "sqlite-fts5",
            "escalation_reason": None,
        }
        (manifests / f"{legacy_id}.json").write_text(
            json.dumps(legacy, indent=2), encoding="utf-8"
        )
        self.assertEqual([], brain.validate_repository(self.repository))

        # Version 2 remains valid after version 3 adds host provenance.
        version_2_id = "00000000-0000-4000-8000-000000000003"
        version_2 = {
            **legacy,
            "schema_version": 2,
            "id": version_2_id,
            "query_source": "explicit",
            "phase_seconds": {"stat": None, "index": None, "retrieval": 0.0},
            "gate": {
                "decision": "retrieve",
                "mode": "shadow",
                "reason": "new-selection",
                "signals": {
                    "informative_terms": 1,
                    "distinctive_matches": 0,
                    "top_score": None,
                    "no_match": [],
                    "query_unchanged_from_previous_turn": False,
                    "selection_identical_to_previous_turn": False,
                },
            },
        }
        (manifests / f"{version_2_id}.json").write_text(
            json.dumps(version_2, indent=2), encoding="utf-8"
        )
        self.assertEqual([], brain.validate_repository(self.repository))

        version_3_id = "00000000-0000-4000-8000-000000000004"
        version_3 = {
            **version_2,
            "schema_version": 3,
            "id": version_3_id,
            "host": "cli",
            "entry_point": "retrieve",
            "local_episode_count": 0,
            "token_estimates": {
                **version_2["token_estimates"],
                "local_episodes": 0,
            },
        }
        (manifests / f"{version_3_id}.json").write_text(
            json.dumps(version_3, indent=2), encoding="utf-8"
        )
        self.assertEqual([], brain.validate_repository(self.repository))

        # A manifest that claims version 3 must carry host provenance.
        incomplete_id = "00000000-0000-4000-8000-000000000005"
        (manifests / f"{incomplete_id}.json").write_text(
            json.dumps({**version_2, "schema_version": 3, "id": incomplete_id}, indent=2),
            encoding="utf-8",
        )
        errors = brain.validate_repository(self.repository)
        self.assertTrue(
            any("strict schema" in error for error in errors), errors
        )

        # The top-level keys alone are not enough: version 3's token total must
        # say how much locally replayed context it includes. Versions 1 and 2
        # above keep their historical token shape.
        incomplete_token_id = "00000000-0000-4000-8000-000000000006"
        (manifests / f"{incomplete_token_id}.json").write_text(
            json.dumps(
                {
                    **version_3,
                    "id": incomplete_token_id,
                    "token_estimates": version_2["token_estimates"],
                },
                indent=2,
            ),
            encoding="utf-8",
        )
        errors = brain.validate_repository(self.repository)
        self.assertTrue(
            any(
                incomplete_token_id in error and "token estimates" in error
                for error in errors
            ),
            errors,
        )

    def test_hook_query_comes_from_the_task_not_the_branch_name(self) -> None:
        # Cursor has no prompt-submit event, so its one automatic memory entry
        # hands over a branch name. Retrieving on the slug asks memory about
        # the word "main"; the task behind it is what the turn is actually
        # about.
        self.run_cli(
            "start", "--task-id", "main",
            "--goal", "Write a Doctrine migration for the invoice table.",
            "--source", "specs/authority.md", "--json",
        )
        self.repository.joinpath("specs/doctrine-migration.md").write_text(
            "# Doctrine migration\n\n"
            "Writing a Doctrine migration for an invoice table.\n",
            encoding="utf-8",
        )
        # The goal is echoed into the task record and its handoff, so in a
        # five-document corpus its own words already look corpus-common and
        # `informative_tokens` discards them. Fillers restore the ratio a real
        # repository has; without them the fixture measures its own size.
        for index in range(10):
            self.repository.joinpath(f"specs/filler-{index}.md").write_text(
                f"# Filler {index}\n\nUnrelated boilerplate paragraph {index}.\n",
                encoding="utf-8",
            )
        self.repository.joinpath(".agents/skills/main-landmark").mkdir(
            parents=True, exist_ok=True
        )
        self.repository.joinpath(
            ".agents/skills/main-landmark/SKILL.md"
        ).write_text(
            "# Main landmark\n\nEvery page needs one main landmark region.\n",
            encoding="utf-8",
        )

        hook = self.run_cli("hook-context", "--task-id", "main", "--json")
        self.assertEqual(0, hook.returncode, hook.stderr)
        capsule = json.loads(hook.stdout)
        self.assertEqual("task", capsule["query_source"], capsule)
        paths = [
            item["path"]
            for layer in ("procedural", "semantic", "episodic")
            for item in capsule[layer]
            if "path" in item
        ]
        self.assertNotIn(".agents/skills/main-landmark/SKILL.md", paths, capsule)
        self.assertIn("specs/doctrine-migration.md", paths, capsule)

        rendered = self.run_cli("hook-context", "--task-id", "main")
        self.assertTrue(rendered.stdout.startswith("working:"), rendered.stdout)
        self.assertIn("query: from task goal", rendered.stdout)

    def test_hook_query_falls_back_to_the_identifier_and_says_so(self) -> None:
        # An auto-provisioned goal is the branch slug re-cased and nothing
        # more, so treating it as task content would put the branch noise back
        # under another name. The capsule reports that it retrieved on a
        # branch name and nothing else, which is a different claim from having
        # retrieved on the task.
        branch = "chore/accelerator-hardening"
        self.run_cli(
            "start", "--task-id", branch,
            "--goal", context_cli.derive_goal(branch), "--json",
        )
        payload = json.loads(
            self.run_cli("hook-context", "--task-id", branch, "--json").stdout
        )
        self.assertEqual("task-id", payload["query_source"], payload)
        # The identifier tokenized and nothing else: no goal words rode along,
        # because the only goal available was the slug spelled differently.
        self.assertEqual("chore accelerator hardening", payload["query"], payload)
        rendered = self.run_cli("hook-context", "--task-id", branch)
        self.assertTrue(rendered.stdout.startswith("working:"), rendered.stdout)
        self.assertIn("query: from branch name only", rendered.stdout)

    def gate_fixture(self, task: str) -> None:
        self.start(task)
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )

    def retrieve_gated(self, task: str, query: str, *extra: str) -> tuple[dict, dict]:
        result = self.run_cli(
            "retrieve", query, "--task-id", task, "--ephemeral", "--json", *extra
        )
        self.assertEqual(0, result.returncode, result.stderr)
        capsule = json.loads(result.stdout)
        return capsule, self.latest_manifest(capsule)

    def test_shadow_gate_records_a_verdict_on_every_manifest(self) -> None:
        # Document-level restraint cannot express "this turn needed nothing",
        # and there was nowhere to write such a decision even if it existed.
        self.gate_fixture("TASK-GATE-SHADOW")
        _, manifest = self.retrieve_gated("TASK-GATE-SHADOW", "cobalt authority")

        gate = manifest["gate"]
        self.assertEqual("shadow", gate["mode"], gate)
        self.assertIn(gate["decision"], ("retrieve", "skip"))
        self.assertTrue(gate["reason"], gate)
        self.assertEqual(
            {
                "informative_terms", "distinctive_matches", "top_score",
                "no_match", "query_unchanged_from_previous_turn",
                "selection_identical_to_previous_turn",
            },
            set(gate["signals"]),
            gate,
        )
        self.assertGreater(gate["signals"]["informative_terms"], 0, gate)

    def test_shadow_is_the_default_and_never_withholds(self) -> None:
        # The whole point of shadow: the decision is computed and recorded,
        # and the turn is served anyway. Enforcement is H3-05's call, taken on
        # the report this mode produces — the same standard that kept
        # embeddings out.
        self.gate_fixture("TASK-GATE-DEFAULT")
        first, _ = self.retrieve_gated("TASK-GATE-DEFAULT", "cobalt authority")
        second, manifest = self.retrieve_gated(
            "TASK-GATE-DEFAULT", "cobalt authority"
        )

        self.assertEqual("shadow", manifest["gate"]["mode"], manifest)
        self.assertEqual("skip", manifest["gate"]["decision"], manifest)
        self.assertEqual("repeat-retrieval", manifest["gate"]["reason"], manifest)
        # Verdict recorded, capsule delivered unchanged.
        self.assertEqual(
            [item["path"] for item in first["semantic"]],
            [item["path"] for item in second["semantic"]],
            second,
        )
        self.assertTrue(second["semantic"], second)
        self.assertTrue(manifest["selected"], manifest)

    def test_a_repeat_is_recognized_by_query_and_selection_not_by_bytes(self) -> None:
        # The returned packet can never repeat byte-for-byte: every call mints
        # a fresh manifest UUID, and the rendered form carries a checkpoint
        # sentence and a last-turn summary that both move on their own. The
        # invariant that does hold is the distilled query and the selected set.
        self.gate_fixture("TASK-GATE-REPEAT")
        first, first_manifest = self.retrieve_gated(
            "TASK-GATE-REPEAT", "cobalt authority"
        )
        second, second_manifest = self.retrieve_gated(
            "TASK-GATE-REPEAT", "cobalt authority"
        )
        self.assertNotEqual(first["manifest"], second["manifest"])

        signals = second_manifest["gate"]["signals"]
        self.assertTrue(signals["query_unchanged_from_previous_turn"], signals)
        self.assertTrue(signals["selection_identical_to_previous_turn"], signals)
        self.assertEqual("skip", second_manifest["gate"]["decision"], second_manifest)
        self.assertEqual(
            "repeat-retrieval", second_manifest["gate"]["reason"], second_manifest
        )

        _, changed = self.retrieve_gated(
            "TASK-GATE-REPEAT", "doctrine migration invoice"
        )
        self.assertFalse(
            changed["gate"]["signals"]["query_unchanged_from_previous_turn"],
            changed["gate"],
        )

    def test_repeat_gate_retrieves_when_selected_content_changes(self) -> None:
        self.gate_fixture("TASK-GATE-CONTENT")
        self.retrieve_gated("TASK-GATE-CONTENT", "cobalt authority")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification changed in place.\n",
            encoding="utf-8",
        )

        capsule, manifest = self.retrieve_gated(
            "TASK-GATE-CONTENT", "cobalt authority", "--gate", "enforce"
        )

        self.assertEqual("retrieve", manifest["gate"]["decision"], manifest)
        self.assertFalse(
            manifest["gate"]["signals"]["selection_identical_to_previous_turn"],
            manifest,
        )
        self.assertTrue(capsule["semantic"], capsule)

    def test_gate_baseline_isolated_by_host_and_entry_point(self) -> None:
        self.gate_fixture("TASK-GATE-BASELINE-SCOPE")
        self.retrieve_gated("TASK-GATE-BASELINE-SCOPE", "cobalt authority")

        other_host, host_manifest = self.retrieve_gated(
            "TASK-GATE-BASELINE-SCOPE", "cobalt authority",
            "--host", "claude", "--gate", "enforce",
        )
        self.assertEqual("retrieve", host_manifest["gate"]["decision"], host_manifest)
        self.assertTrue(other_host["semantic"], other_host)

        context = self.run_cli(
            "context", "cobalt authority",
            "--task-id", "TASK-GATE-BASELINE-SCOPE",
            "--ephemeral", "--host", "cli", "--gate", "enforce", "--json",
        )
        self.assertEqual(0, context.returncode, context.stderr)
        context_capsule = json.loads(context.stdout)
        context_manifest = self.latest_manifest(context_capsule)
        self.assertEqual("retrieve", context_manifest["gate"]["decision"], context_manifest)

        repeated = self.run_cli(
            "context", "cobalt authority",
            "--task-id", "TASK-GATE-BASELINE-SCOPE",
            "--ephemeral", "--host", "cli", "--gate", "enforce", "--json",
        )
        self.assertEqual(0, repeated.returncode, repeated.stderr)
        self.assertEqual(
            "skip",
            self.latest_manifest(json.loads(repeated.stdout))["gate"]["decision"],
        )

    def test_local_episode_participates_in_gate_before_the_decision(self) -> None:
        self.start("TASK-GATE-EPISODE")
        recorded = self.run_cli(
            "record",
            "--summary", "Vermilion semaphore rollout",
            "--outcome", "The rollout completed cleanly under the scarlet protocol.",
            "--json",
        )
        self.assertEqual(0, recorded.returncode, recorded.stderr)

        capsule, manifest = self.retrieve_gated(
            "TASK-GATE-EPISODE", "vermilion semaphore", "--gate", "enforce"
        )

        self.assertEqual("retrieve", manifest["gate"]["decision"], manifest)
        self.assertNotIn("episodic", manifest["gate"]["signals"]["no_match"])
        self.assertEqual(1, len(capsule["episodic"]), capsule)
        self.assertEqual("Vermilion semaphore rollout", capsule["episodic"][0]["summary"])
        self.assertEqual(1, manifest["local_episode_count"], manifest)
        local_tokens = manifest["token_estimates"]["local_episodes"]
        self.assertGreater(local_tokens, 0, manifest)
        self.assertEqual(
            sum(
                manifest["token_estimates"][name]
                for name in (
                    "policy", "handoff", "durable", "dynamic", "evidence",
                    "local_episodes",
                )
            ),
            manifest["token_estimates"]["total"],
        )
        self.assertNotIn("scarlet protocol", json.dumps(manifest))

    def test_prompt_refresh_keeps_terms_found_only_in_local_episodes(self) -> None:
        self.start("TASK-PROMPT-EPISODE")
        self.repository.joinpath("specs/developing.md").write_text(
            "# Developing\n\nDeveloping guidance for this repository.\n",
            encoding="utf-8",
        )
        recorded = self.run_cli(
            "record",
            "--summary", "Amber viaduct rollout",
            "--outcome", "The local episode completed cleanly.",
            "--json",
        )
        self.assertEqual(0, recorded.returncode, recorded.stderr)

        refreshed = self.run_cli(
            "refresh", "--query", "developing amber viaduct",
            "--task-id", "TASK-PROMPT-EPISODE", "--ephemeral",
            "--host", "codex", "--gate", "enforce", "--json",
        )

        self.assertEqual(0, refreshed.returncode, refreshed.stderr)
        capsule = json.loads(refreshed.stdout)["capsule"]
        self.assertIsNotNone(capsule, refreshed.stdout)
        manifest = self.latest_manifest(capsule)
        self.assertEqual("retrieve", manifest["gate"]["decision"], manifest)
        self.assertEqual(1, manifest["local_episode_count"], manifest)
        self.assertEqual("Amber viaduct rollout", capsule["episodic"][0]["summary"])

    def test_local_episode_cannot_bypass_the_target_budget(self) -> None:
        self.start("TASK-GATE-EPISODE-BUDGET")
        recorded = self.run_cli(
            "record",
            "--summary", "Obsidian walrus replay",
            "--outcome", "x" * 33_000,
            "--json",
        )
        self.assertEqual(0, recorded.returncode, recorded.stderr)

        capsule, manifest = self.retrieve_gated(
            "TASK-GATE-EPISODE-BUDGET", "obsidian walrus", "--gate", "enforce"
        )

        self.assertEqual([], capsule["episodic"], capsule)
        self.assertEqual("skip", manifest["gate"]["decision"], manifest)
        self.assertEqual("empty-after-filter", manifest["gate"]["reason"], manifest)
        self.assertEqual(0, manifest["local_episode_count"], manifest)
        self.assertEqual(0, manifest["token_estimates"]["local_episodes"], manifest)
        self.assertIn(
            {"path": "local-episode", "reason": "budget"},
            manifest["excluded"],
        )

    def test_cursor_retrieves_when_the_task_revision_changes(self) -> None:
        task = self.start("TASK-GATE-CURSOR-REVISION")
        for index in range(10):
            self.repository.joinpath(f"specs/revision-filler-{index}.md").write_text(
                f"# Filler {index}\n\nUnrelated boilerplate paragraph {index}.\n",
                encoding="utf-8",
            )
        first = self.run_cli(
            "hook-context", "--task-id", "TASK-GATE-CURSOR-REVISION",
            "--host", "cursor", "--gate", "enforce", "--json",
        )
        self.assertEqual(0, first.returncode, first.stderr)
        first_manifest = self.latest_manifest(json.loads(first.stdout))
        self.assertEqual("retrieve", first_manifest["gate"]["decision"], first_manifest)

        updated = self.run_cli(
            "update", "--task-id", "TASK-GATE-CURSOR-REVISION",
            "--revision", str(task["revision"]), "--phase", "implementation", "--json",
        )
        self.assertEqual(0, updated.returncode, updated.stderr)
        second = self.run_cli(
            "hook-context", "--task-id", "TASK-GATE-CURSOR-REVISION",
            "--host", "cursor", "--gate", "enforce", "--json",
        )
        self.assertEqual(0, second.returncode, second.stderr)
        manifest = self.latest_manifest(json.loads(second.stdout))
        self.assertEqual("retrieve", manifest["gate"]["decision"], manifest)
        self.assertGreater(manifest["task_revision"], first_manifest["task_revision"])

    def test_gate_distinguishes_no_relevant_match_from_filtered_results(self) -> None:
        task = self.start("TASK-GATE-EMPTY-REASONS")
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        connection = context_cli.connect(
            context_cli.default_database(self.repository)
        )
        try:
            no_match = retrieval.retrieve(
                connection, self.repository, "vermilion semaphore", task["task_uuid"],
                limit=3,
            )
            self.assertEqual("no-relevant-match", no_match["gate"]["reason"], no_match)

            self.repository.joinpath("specs/authority.md").write_text(
                "# Changed\n\nCobalt authority source changed after indexing.\n",
                encoding="utf-8",
            )
            filtered = retrieval.retrieve(
                connection, self.repository, "cobalt authority", task["task_uuid"],
                limit=3,
            )
            self.assertEqual("empty-after-filter", filtered["gate"]["reason"], filtered)
        finally:
            connection.close()

    def test_enforce_withholds_the_selection_and_says_it_did(self) -> None:
        self.gate_fixture("TASK-GATE-ENFORCE")
        self.retrieve_gated("TASK-GATE-ENFORCE", "cobalt authority")
        capsule, manifest = self.retrieve_gated(
            "TASK-GATE-ENFORCE", "cobalt authority", "--gate", "enforce"
        )

        self.assertEqual("skip", manifest["gate"]["decision"], manifest)
        self.assertEqual([], capsule["semantic"], capsule)
        self.assertEqual([], capsule["procedural"], capsule)
        # The decision is still on the record: a withheld turn has to be
        # countable, or the skip rate H3 needs cannot be computed.
        self.assertEqual([], manifest["selected"], manifest)
        self.assertEqual("enforce", manifest["gate"]["mode"], manifest)

        rendered = self.run_cli(
            "retrieve", "cobalt authority", "--task-id", "TASK-GATE-ENFORCE",
            "--ephemeral", "--gate", "enforce",
        )
        self.assertTrue(rendered.stdout.startswith("working:"), rendered.stdout)
        self.assertIn("gate: skipped — repeat-retrieval", rendered.stdout)

    def test_enforce_serves_a_turn_whose_question_changed(self) -> None:
        self.gate_fixture("TASK-GATE-CHANGED")
        self.retrieve_gated("TASK-GATE-CHANGED", "cobalt authority")
        capsule, manifest = self.retrieve_gated(
            "TASK-GATE-CHANGED", "authority specification detail",
            "--gate", "enforce",
        )
        self.assertEqual("retrieve", manifest["gate"]["decision"], manifest)
        self.assertTrue(capsule["semantic"], capsule)

    def test_a_skip_does_not_become_the_baseline_for_the_next_turn(self) -> None:
        # If a skip overwrote the remembered retrieval, the turn after it
        # would be comparing against nothing and would always look new.
        self.gate_fixture("TASK-GATE-BASELINE")
        self.retrieve_gated("TASK-GATE-BASELINE", "cobalt authority")
        self.retrieve_gated(
            "TASK-GATE-BASELINE", "cobalt authority", "--gate", "enforce"
        )
        _, manifest = self.retrieve_gated(
            "TASK-GATE-BASELINE", "cobalt authority", "--gate", "enforce"
        )
        self.assertEqual("repeat-retrieval", manifest["gate"]["reason"], manifest)

    def test_gate_off_never_decides(self) -> None:
        self.gate_fixture("TASK-GATE-OFF")
        self.retrieve_gated("TASK-GATE-OFF", "cobalt authority", "--gate", "off")
        capsule, manifest = self.retrieve_gated(
            "TASK-GATE-OFF", "cobalt authority", "--gate", "off"
        )
        self.assertEqual("retrieve", manifest["gate"]["decision"], manifest)
        self.assertEqual("gate-off", manifest["gate"]["reason"], manifest)
        self.assertTrue(capsule["semantic"], capsule)

    def test_a_gated_hook_capsule_leaves_the_cursor_rule_alone(self) -> None:
        # The enforce hazard. Both Cursor deliveries accept any stdout that
        # opens with "working:" at exit 0 and move it over the rule file, so a
        # withheld capsule rendered normally would replace Cursor's only
        # memory channel with an empty rule on every skipped turn. The skip
        # exits with a status those hooks leave alone, and prints nothing.
        self.gate_fixture("TASK-GATE-HOOK")
        self.assertEqual(
            0, self.run_cli("hook-context", "--task-id", "TASK-GATE-HOOK").returncode
        )
        skipped = self.run_cli(
            "hook-context", "--task-id", "TASK-GATE-HOOK", "--gate", "enforce"
        )
        self.assertEqual(4, skipped.returncode, skipped.stdout + skipped.stderr)
        self.assertEqual("", skipped.stdout)
        # Not 0 (would be rendered over the rule) and not 3 (would delete it).
        self.assertNotIn(skipped.returncode, (0, 3))

    def test_an_unknown_gate_mode_is_rejected(self) -> None:
        self.gate_fixture("TASK-GATE-BAD")
        config_path = self.repository / "project-brain/config/runtime.json"
        config_path.parent.mkdir(parents=True, exist_ok=True)
        config = (
            json.loads(config_path.read_text(encoding="utf-8"))
            if config_path.is_file()
            else {"mode": "governed"}
        )
        config["retrieval_gate"] = "sometimes"
        config_path.write_text(json.dumps(config, indent=2), encoding="utf-8")
        # hook-context never sees the flag, so a bad configured value has to be
        # rejected where every caller passes, not only by argparse choices.
        result = self.run_cli("hook-context", "--task-id", "TASK-GATE-BAD")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("off, shadow or enforce", result.stderr)

    def test_no_document_occupies_two_layers_of_one_capsule(self) -> None:
        # The semantic layer is grouped by retrieval category and the episodic
        # layer by the layer column, and the two taxonomies disagree:
        # `category_for` has no `changelog` branch, so CHANGELOG.md is
        # category 'evidence' and layer 'episodic' at the same time. It used to
        # take one of the three semantic slots and the only episodic slot
        # together, while the degradation ladder dropped real content to fit
        # the budget.
        self.start("TASK-LAYER-DEDUPE")
        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changelog\n\n## Unreleased\n\ncobalt authority rollout shipped.\n",
            encoding="utf-8",
        )
        for index in range(4):
            self.repository.joinpath(f"specs/cobalt-{index}.md").write_text(
                f"# Cobalt {index}\n\ncobalt authority rollout detail {index}.\n",
                encoding="utf-8",
            )

        governed = self.run_cli(
            "retrieve", "cobalt authority rollout",
            "--task-id", "TASK-LAYER-DEDUPE", "--limit", "20",
            "--ephemeral", "--json",
        )
        self.assertEqual(0, governed.returncode, governed.stderr)
        capsule = json.loads(governed.stdout)

        placements: dict[str, list[str]] = {}
        for layer in ("procedural", "semantic", "episodic"):
            for item in capsule[layer]:
                if "path" in item:
                    placements.setdefault(item["path"], []).append(layer)
        collisions = {
            path: layers for path, layers in placements.items() if len(layers) > 1
        }
        self.assertEqual({}, collisions, capsule)
        self.assertIn(
            "CHANGELOG.md", [item["path"] for item in capsule["episodic"]], capsule
        )

        # The freed slot goes to the next ranked candidate rather than being
        # lost, and the manifest keeps describing what was actually delivered.
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        selected = {item["path"] for item in manifest["selected"]}
        delivered = {
            item["path"]
            for layer in ("procedural", "semantic", "episodic")
            for item in capsule[layer]
            if "path" in item
        }
        # The manifest describes every layer the capsule delivers, episodic
        # included. Before the episodic layer was ranked here it was fetched
        # around the manifest entirely, so the audit record was silent about a
        # document the model was shown.
        self.assertEqual(selected, delivered, manifest)
        self.assertIn("CHANGELOG.md", selected, manifest)

    def test_lightweight_capsule_also_keeps_layers_disjoint(self) -> None:
        # The lightweight path queries each layer separately and has never had
        # the collision; asserting it here keeps the two paths answerable to
        # one contract rather than to whichever one was last looked at.
        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changelog\n\n## Unreleased\n\ncobalt authority rollout shipped.\n",
            encoding="utf-8",
        )
        result = self.run_cli(
            "--mode", "lightweight",
            "retrieve", "cobalt authority rollout",
            "--task-id", "TASK-LIGHTWEIGHT-LAYERS", "--limit", "20", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        capsule = json.loads(result.stdout)
        placements: dict[str, list[str]] = {}
        for layer in ("procedural", "semantic", "episodic"):
            for item in capsule.get(layer, []):
                if "path" in item:
                    placements.setdefault(item["path"], []).append(layer)
        self.assertEqual(
            {},
            {path: layers for path, layers in placements.items() if len(layers) > 1},
            capsule,
        )

    def test_frozen_retrieval_corpus_meets_quality_and_privacy_gates(self) -> None:
        fixture = json.loads(
            (
                Path(__file__).parent / "fixtures/retrieval-golden.json"
            ).read_text(encoding="utf-8")
        )
        for document in fixture["documents"]:
            path = self.repository / document["path"]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                f"# Golden fixture\n\n{document['text']}\n",
                encoding="utf-8",
            )
        self.repository.joinpath("specs/private-golden.md").write_text(
            "# Private fixture\n\nCobalt pagination cursor private customer evidence.\n",
            encoding="utf-8",
        )
        private = brain.create_record(
            self.repository,
            "finding",
            "PRIVATE-GOLDEN",
            "Cobalt pagination cursor private customer evidence",
            [],
            ["specs/private-golden.md"],
            owner="local",
            privacy="private",
            authority="verified",
        )
        stale = brain.create_record(
            self.repository,
            "finding",
            "STALE-GOLDEN",
            "Amber webhook signature stale evidence",
            [],
            ["specs/authority.md"],
            owner="local",
            privacy="team",
            authority="verified",
        )
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nChanged after both source fingerprints.\n",
            encoding="utf-8",
        )

        connection = context_cli.connect(
            context_cli.default_database(self.repository)
        )
        try:
            context_cli.index_repository(connection, self.repository)
            config = brain.load_config(self.repository)
            precisions: list[float] = []
            recalls: list[float] = []
            for case in fixture["cases"]:
                first, _ = retrieval._runtime_filter(
                    self.repository,
                    retrieval._candidates(connection, case["query"])[0],
                    config,
                )
                second, _ = retrieval._runtime_filter(
                    self.repository,
                    retrieval._candidates(connection, case["query"])[0],
                    config,
                )
                first_paths = [item["path"] for item in first[:5]]
                self.assertEqual(
                    first_paths, [item["path"] for item in second[:5]]
                )
                self.assertNotIn(
                    f"project-brain/dynamic/findings/{private['id']}.md",
                    first_paths,
                )
                self.assertNotIn(
                    f"project-brain/dynamic/findings/{stale['id']}.md",
                    first_paths,
                )
                expected = set(case["expected"])
                relevant = len(expected.intersection(first_paths))
                precisions.append(relevant / len(first_paths))
                recalls.append(relevant / len(expected))
            self.assertGreaterEqual(sum(precisions) / len(precisions), 0.80)
            self.assertGreaterEqual(sum(recalls) / len(recalls), 0.90)
        finally:
            connection.close()

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

    def test_concurrent_completion_cas_creates_exactly_one_episode(self) -> None:
        task = self.start("TASK-COMPLETE-RACE")
        results: list[subprocess.CompletedProcess[str]] = []

        def complete(outcome: str) -> None:
            results.append(
                self.run_cli(
                    "complete",
                    "--task-id", "TASK-COMPLETE-RACE",
                    "--revision", str(task["revision"]),
                    "--outcome", outcome,
                    "--verification", "Focused checks passed",
                    "--json",
                )
            )

        threads = [
            threading.Thread(target=complete, args=("First completion.",)),
            threading.Thread(target=complete, args=("Second completion.",)),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=5)
        self.assertEqual([0, 1], sorted(result.returncode for result in results))
        failed = next(result for result in results if result.returncode)
        self.assertTrue(
            "Stale task revision" in failed.stderr
            or "Working task not found" in failed.stderr
        )
        connection = sqlite3.connect(
            self.repository / "memory-bank/local/context.db"
        )
        try:
            self.assertEqual(
                1, connection.execute("SELECT COUNT(*) FROM episodes").fetchone()[0]
            )
        finally:
            connection.close()

    def test_governed_completion_requires_an_explicit_numeric_revision(self) -> None:
        self.start("TASK-EXPLICIT-COMPLETE")
        for revision_arguments in ((), ("--revision", "auto")):
            with self.subTest(revision_arguments=revision_arguments):
                result = self.run_cli(
                    "complete",
                    "--task-id", "TASK-EXPLICIT-COMPLETE",
                    *revision_arguments,
                    "--outcome", "Must not close.",
                )
                self.assertEqual(1, result.returncode)
                self.assertIn("current numeric --revision", result.stderr)
        self.assertEqual(
            "active",
            json.loads(
                self.run_cli(
                    "get", "--task-id", "TASK-EXPLICIT-COMPLETE", "--json"
                ).stdout
            )["status"],
        )

    def test_flush_and_manual_update_do_not_lose_a_successful_update(self) -> None:
        task = self.start("TASK-FLUSH-RACE")
        self.repository.joinpath("race.txt").write_text("work\n", encoding="utf-8")
        results: dict[str, subprocess.CompletedProcess[str]] = {}

        threads = [
            threading.Thread(
                target=lambda: results.setdefault(
                    "update",
                    self.run_cli(
                        "update", "--task-id", "TASK-FLUSH-RACE",
                        "--revision", str(task["revision"]),
                        "--progress", "Manual progress survives.", "--json",
                    ),
                )
            ),
            threading.Thread(
                target=lambda: results.setdefault(
                    "flush",
                    self.run_cli(
                        "turn", "--task-id", "TASK-FLUSH-RACE",
                        "--flush", "--json",
                    ),
                )
            ),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=5)
        self.assertEqual(0, results["flush"].returncode, results["flush"].stderr)
        self.assertIn(results["update"].returncode, (0, 1))
        task_after = json.loads(
            self.run_cli(
                "get", "--task-id", "TASK-FLUSH-RACE", "--json"
            ).stdout
        )
        if results["update"].returncode == 0:
            self.assertEqual("Manual progress survives.", task_after["progress"])
        else:
            self.assertIn("Stale", results["update"].stderr)
        self.assertIn("Auto-checkpoint:", task_after["auto_checkpoint"])

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

    def write_skill(self, edition: str, name: str, body: str) -> None:
        path = self.repository / edition / "skills" / name / "SKILL.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"# {name}\n\n{body}\n", encoding="utf-8")

    def test_skill_mirror_drift_fails_instead_of_hiding_parity_error(self) -> None:
        self.write_skill(".agents", "review", "canonical")
        self.write_skill(".claude", "review", "drifted")
        indexed = self.run_cli("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        self.assertEqual(1, len(json.loads(indexed.stdout)["parity_drift"]))

        parity = self.run_cli("parity")
        self.assertNotEqual(0, parity.returncode)
        self.assertIn("mirror parity drift", parity.stderr)

    def test_parity_json_reports_every_drifted_path_on_the_failure_path(self) -> None:
        """--json used to be unreachable when drift existed, and only the first
        drifted path was ever named - so a repair took one run per file."""
        for name in ("alpha", "beta", "gamma"):
            self.write_skill(".agents", name, "canonical")
            self.write_skill(".claude", name, "drifted")

        parity = self.run_cli("parity", "--json")

        self.assertEqual(1, parity.returncode)
        self.assertTrue(parity.stdout.strip(), "--json produced no output on failure")
        result = json.loads(parity.stdout)
        self.assertFalse(result["valid"])
        self.assertEqual(
            ["alpha/SKILL.md", "beta/SKILL.md", "gamma/SKILL.md"],
            sorted(item["logical_path"] for item in result["drift"]),
        )

        text = self.run_cli("parity")
        for name in ("alpha", "beta", "gamma"):
            self.assertIn(f"{name}/SKILL.md", text.stderr)

    def test_parity_catches_a_file_that_only_one_mirror_carries(self) -> None:
        """A copy absent from canonical is drift; skipping it let a stray file
        sit in one mirror indefinitely without parity ever mentioning it."""
        self.write_skill(".agents", "shared", "canonical")
        self.write_skill(".claude", "shared", "canonical")
        self.write_skill(".cursor", "shared", "canonical")
        self.write_skill(".claude", "stray", "only in claude")

        parity = self.run_cli("parity", "--json")

        self.assertEqual(1, parity.returncode)
        drift = {item["logical_path"]: item for item in json.loads(parity.stdout)["drift"]}
        self.assertIn("stray/SKILL.md", drift)
        self.assertEqual("absent from canonical", drift["stray/SKILL.md"]["reason"])
        self.assertNotIn("shared/SKILL.md", drift)

    def test_parity_catches_a_canonical_file_missing_from_a_mirror(self) -> None:
        self.write_skill(".agents", "shared", "canonical")
        self.write_skill(".claude", "shared", "canonical")
        self.write_skill(".cursor", "other", "unrelated")

        parity = self.run_cli("parity", "--json")

        self.assertEqual(1, parity.returncode)
        drift = {item["logical_path"]: item for item in json.loads(parity.stdout)["drift"]}
        self.assertIn("shared/SKILL.md", drift)
        self.assertIn(".cursor", drift["shared/SKILL.md"]["missing"])

    def test_parity_passes_and_stays_quiet_when_mirrors_match(self) -> None:
        for edition in (".agents", ".claude", ".cursor"):
            self.write_skill(edition, "review", "identical")

        parity = self.run_cli("parity", "--json")

        self.assertEqual(0, parity.returncode, parity.stderr)
        result = json.loads(parity.stdout)
        self.assertTrue(result["valid"])
        self.assertEqual([], result["drift"])

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
        finding = brain.create_record(
            self.repository,
            "finding",
            "FIND-PROMOTE",
            "Cobalt authority is reusable.",
            [],
            ["specs/authority.md"],
            owner="alice",
            authority="verified",
        )
        finding = brain.update_record(
            self.repository,
            finding["id"],
            expected_revision=finding["revision"],
            progress="The cobalt authority rule applies across later requests.",
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
            transition_to="resolved",
            reason="Resolved after verification",
        )
        proposal = brain.create_promotion(
            self.repository,
            [finding["id"]],
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
        self.assertEqual("approved", applied["outcome"])
        memory_id = self.expected_memory_id(finding["id"])
        self.assertEqual(memory_id, applied["destination_memory_id"])
        self.assertTrue(
            self.repository.joinpath(
                f"memory-bank/chunks/{memory_id}-cobalt-authority.md"
            ).is_file()
        )
        self.assertIn(
            f"| {memory_id} |",
            self.repository.joinpath("memory-bank/INDEX.md").read_text(
                encoding="utf-8"
            ),
        )
        # The retired legacy counter is neither read nor advanced.
        self.assertEqual(
            "1", self.repository.joinpath("memory-bank/.memory-counter").read_text().strip()
        )

    def test_reviewed_apply_rejects_every_ineligible_source(self) -> None:
        def source(name: str) -> str:
            relative = f"specs/{name}.md"
            self.repository.joinpath(relative).write_text(
                f"# {name}\n\nCanonical evidence for {name}.\n",
                encoding="utf-8",
            )
            return relative

        def resolved_finding(
            external_id: str,
            source_path: str,
            *,
            authority: str = "verified",
            privacy: str = "team",
            progress: Optional[str] = None,
        ) -> dict:
            title = f"{external_id} reusable consequence"
            record = brain.create_record(
                self.repository,
                "finding",
                external_id,
                title,
                [],
                [source_path],
                owner="alice",
                authority=authority,
                privacy=privacy,
            )
            return brain.update_record(
                self.repository,
                record["id"],
                expected_revision=record["revision"],
                progress=(
                    f"{external_id} remains reusable across later work."
                    if progress is None
                    else progress
                ),
                next_steps=[],
                files=[],
                sources=[],
                actor="alice",
                transition_to="resolved",
                reason="Resolved",
            )

        task_source = source("human-task")
        task = brain.create_task(
            self.repository,
            "TASK-HUMAN-SOURCE",
            "Task progress is not durable knowledge.",
            [],
            [task_source],
            owner="alice",
        )
        open_source = source("human-open")
        open_finding = brain.create_record(
            self.repository,
            "finding",
            "FIND-HUMAN-OPEN",
            "Open finding",
            [],
            [open_source],
            owner="alice",
            authority="verified",
        )
        observed_source = source("human-observed")
        observed = resolved_finding(
            "FIND-HUMAN-OBSERVED", observed_source, authority="observed"
        )
        private_source = source("human-private")
        private = resolved_finding(
            "FIND-HUMAN-PRIVATE", private_source, privacy="private"
        )
        stale_source = source("human-stale")
        stale = resolved_finding("FIND-HUMAN-STALE", stale_source)
        empty_source = source("human-empty")
        empty = resolved_finding("FIND-HUMAN-EMPTY", empty_source, progress="")
        repeated_source = source("human-repeated")
        repeated = resolved_finding(
            "FIND-HUMAN-REPEATED",
            repeated_source,
            progress="FIND-HUMAN-REPEATED reusable consequence.",
        )

        scenarios = (
            ("task", task, "task records are not promotable", None),
            ("open", open_finding, "finding status open is not promotable", None),
            ("observed", observed, "authority is observed, not verified", None),
            ("private", private, "privacy private is not allowed", None),
            ("stale", stale, "cited source changed", stale_source),
            ("empty", empty, "no content beyond its own title", None),
            ("repeated", repeated, "no content beyond its own title", None),
        )
        for name, record, message, changed_source in scenarios:
            with self.subTest(name=name):
                proposal = brain.create_promotion(
                    self.repository,
                    [record["id"]],
                    f"Reviewed {name} source",
                    f"Reviewed consequence for {name} source.",
                    proposer="alice",
                )
                brain.review_promotion(
                    self.repository,
                    proposal["id"],
                    reviewer="human-reviewer",
                    approve=True,
                )
                if changed_source is not None:
                    self.repository.joinpath(changed_source).write_text(
                        "# Changed\n\nThe cited evidence changed after review.\n",
                        encoding="utf-8",
                    )
                with self.assertRaisesRegex(brain.BrainError, message):
                    brain.apply_promotion(self.repository, proposal["id"])

        self.assertEqual([], list(self.repository.joinpath("memory-bank/chunks").glob("*.md")))

    def test_apply_rejects_outcome_that_misstates_review_provenance(self) -> None:
        for review_mode, wrong_outcome in (
            ("human", "approved-without-review"),
            ("automatic", "approved"),
        ):
            with self.subTest(review_mode=review_mode):
                record = brain.create_record(
                    self.repository,
                    "finding",
                    f"FIND-OUTCOME-{review_mode.upper()}",
                    f"{review_mode} provenance",
                    [],
                    ["specs/authority.md"],
                    owner="alice",
                )
                record = self.resolve_verified_finding(
                    record, f"{review_mode} provenance remains explicit."
                )
                proposal = brain.create_promotion(
                    self.repository,
                    [record["id"]],
                    f"{review_mode} provenance",
                    "Reviewed consequence.",
                    proposer="alice",
                    review_mode=review_mode,
                )
                if review_mode == "human":
                    brain.review_promotion(
                        self.repository,
                        proposal["id"],
                        reviewer="human-reviewer",
                        approve=True,
                    )
                else:
                    brain.auto_review_promotion(self.repository, proposal["id"])
                path = self.repository.joinpath(
                    "project-brain/control/promotions", f"{proposal['id']}.json"
                )
                tampered = json.loads(path.read_text(encoding="utf-8"))
                tampered["outcome"] = wrong_outcome
                path.write_text(json.dumps(tampered), encoding="utf-8")

                with self.assertRaisesRegex(brain.BrainError, "outcome"):
                    brain.apply_promotion(self.repository, proposal["id"])

        self.assertEqual([], list(self.repository.joinpath("memory-bank/chunks").glob("*.md")))

    def test_legacy_applied_promotion_outcome_stays_readable(self) -> None:
        record = brain.create_record(
            self.repository,
            "finding",
            "FIND-LEGACY-OUTCOME",
            "Legacy promotion outcome",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        proposal = brain.create_promotion(
            self.repository,
            [record["id"]],
            "Legacy promotion outcome",
            "Historical content.",
            proposer="alice",
        )
        proposal.update(
            status="applied",
            reviewer="human-reviewer",
            reviewed_at=proposal["created_at"],
            outcome="promoted",
            destination_memory_id="MEM-legacy",
            destination_revision=1,
        )

        brain.validate_promotion_record(self.repository, proposal)

    def test_promotions_from_different_records_allocate_distinct_ids(self) -> None:
        applied = []
        records = []
        for suffix, title in (("A", "First lesson"), ("B", "Second lesson")):
            record = brain.create_record(
                self.repository,
                "finding",
                f"FINDING-{suffix}",
                f"Reusable finding {suffix}",
                [],
                ["specs/authority.md"],
                owner="alice",
            )
            record = self.resolve_verified_finding(
                record, f"Reusable consequence {suffix}."
            )
            records.append(record)
            proposal = brain.create_promotion(
                self.repository,
                [record["id"]],
                title,
                "Reusable consequence.",
                proposer="alice",
            )
            brain.review_promotion(
                self.repository, proposal["id"], "human", approve=True
            )
            applied.append(brain.apply_promotion(self.repository, proposal["id"]))
        memory_ids = [item["destination_memory_id"] for item in applied]
        # No shared counter: each ID derives from its own source-record UUID.
        self.assertEqual(2, len(set(memory_ids)))
        for record, memory_id in zip(records, memory_ids):
            self.assertEqual(self.expected_memory_id(record["id"]), memory_id)
        index_text = self.repository.joinpath("memory-bank/INDEX.md").read_text(
            encoding="utf-8"
        )
        for memory_id in memory_ids:
            self.assertIn(f"| {memory_id} |", index_text)
        self.assertEqual([], brain.validate_bank(self.repository / "memory-bank"))

    def test_reindex_bank_is_idempotent_and_restores_a_deleted_index(self) -> None:
        record = brain.create_record(
            self.repository,
            "finding",
            "FINDING-REINDEX",
            "Reindex finding",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        record = self.resolve_verified_finding(record)
        proposal = brain.create_promotion(
            self.repository,
            [record["id"]],
            "Reindex lesson",
            "Reusable consequence.",
            proposer="alice",
        )
        brain.review_promotion(
            self.repository, proposal["id"], "human", approve=True
        )
        memory_id = brain.apply_promotion(self.repository, proposal["id"])[
            "destination_memory_id"
        ]
        index_path = self.repository / "memory-bank/INDEX.md"

        rerun = json.loads(self.run_cli("reindex-bank", "--json").stdout)
        self.assertFalse(rerun["changed"])
        after_promotion = index_path.read_text(encoding="utf-8")

        index_path.unlink()
        restored = json.loads(self.run_cli("reindex-bank", "--json").stdout)
        self.assertTrue(restored["changed"])
        self.assertEqual(1, restored["chunks"])
        restored_text = index_path.read_text(encoding="utf-8")
        self.assertIn(f"| {memory_id} |", restored_text)
        # The restored table matches what the promotion had rendered; only
        # the preamble may differ (it falls back to the canonical wording).
        self.assertEqual(
            after_promotion.partition("| ID | Title |")[2],
            restored_text.partition("| ID | Title |")[2],
        )
        self.assertEqual([], brain.validate_bank(self.repository / "memory-bank"))

        # Regenerating the restored index changes nothing further.
        second = json.loads(self.run_cli("reindex-bank", "--json").stdout)
        self.assertFalse(second["changed"])
        self.assertEqual(restored_text, index_path.read_text(encoding="utf-8"))

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
        finding = self.resolve_verified_finding(finding)
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
        chunk = self.repository / (
            "memory-bank/chunks/"
            f"{self.expected_memory_id(finding['id'])}-reusable-finding.md"
        )
        metadata, _ = brain.parse_markdown_record(chunk)
        # The promoted record, and what that record itself cited. Carrying the
        # second is what keeps the citation chain unbroken: without it the
        # chunk names the record, the record names the document, and nothing
        # traverses two hops.
        self.assertEqual(
            [source["path"], "specs/authority.md"], metadata["sources"]
        )
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
        finding = self.resolve_verified_finding(finding)
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
                "memory-bank/chunks/"
                f"{self.expected_memory_id(finding['id'])}-rollback-promotion.md"
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
        finding = self.resolve_verified_finding(finding)
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
        destination = bank / (
            f"chunks/{self.expected_memory_id(finding['id'])}-write-boundary.md"
        )
        # The legacy .memory-counter is no longer written, so the write
        # boundaries are the chunk itself and the regenerated index; the
        # counter stays byte-identical throughout (asserted below).
        for failed_path in (
            destination,
            bank / "INDEX.md",
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
        finding = self.resolve_verified_finding(finding)
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
                "--revision",
                str(before["revision"]),
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


class AuthorityLifecycleTest(RuntimeHarness):
    """The observed -> verified authority edge that unlocks auto-promotion."""

    def observed_finding(self, external_id: str = "FIND-AUTH") -> dict:
        return brain.create_record(
            self.repository,
            "finding",
            external_id,
            "Cobalt guard closes the request gap",
            [],
            ["specs/authority.md"],
            owner="alice",
        )

    def enable_automatic_promotion(self) -> None:
        config = self.repository / "project-brain/config/runtime.json"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(
            json.dumps({"mode": "governed", "automatic_promotion": True}),
            encoding="utf-8",
        )

    def test_verified_then_terminal_record_reaches_auto_promotion(self) -> None:
        self.enable_automatic_promotion()
        record = self.observed_finding()
        self.assertEqual("observed", record["authority"])

        verified = brain.update_record(
            self.repository,
            record["id"],
            expected_revision=1,
            progress="Cobalt guard confirmed against the authority spec.",
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
            authority="verified",
            reason="Verified: authority spec re-checked",
        )
        self.assertEqual("verified", verified["authority"])
        ledger = verified["transitions"][-1]
        self.assertEqual("observed", ledger["from"])
        self.assertEqual("verified", ledger["to"])
        self.assertEqual("alice", ledger["actor"])
        self.assertEqual("Verified: authority spec re-checked", ledger["reason"])
        # The widened ledger must survive a disk round-trip and revalidation.
        self.assertEqual("verified", brain.get_record(self.repository, record["id"])["authority"])

        brain.update_record(
            self.repository,
            record["id"],
            expected_revision=2,
            progress=None,
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
            transition_to="resolved",
            reason="Resolved after verification",
        )
        candidates, blocked = brain.promotable_records(
            self.repository, brain.load_config(self.repository)
        )
        self.assertEqual([record["id"]], [item["record"]["id"] for item in candidates])
        self.assertEqual([], blocked)

        promoted = brain.auto_promote(self.repository, owner="alice")
        self.assertEqual(
            [record["id"]], [item["record_id"] for item in promoted["promoted"]]
        )
        self.assertEqual([], promoted["failed"])
        chunk = next((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        self.assertIn("auto-promoted", chunk.read_text(encoding="utf-8"))

    def test_terminal_record_without_verification_stays_blocked(self) -> None:
        record = self.observed_finding("FIND-UNVERIFIED")
        brain.update_record(
            self.repository,
            record["id"],
            expected_revision=1,
            progress="Resolved without any verification pass.",
            next_steps=[],
            files=[],
            sources=[],
            actor="alice",
            transition_to="resolved",
            reason="Resolved",
        )
        candidates, blocked = brain.promotable_records(
            self.repository, brain.load_config(self.repository)
        )
        self.assertEqual([], candidates)
        self.assertEqual(
            ["authority is observed, not verified"],
            [item["reason"] for item in blocked],
        )

    def test_reverse_and_arbitrary_authority_transitions_are_rejected(self) -> None:
        scenarios = (
            ("verified", "observed"),   # nothing downgrades
            ("verified", "inferred"),   # nothing downgrades
            ("observed", "inferred"),   # nothing downgrades
            ("inferred", "verified"),   # inference never skips verification
            ("observed", "observed"),   # a no-op claims a promotion that never ran
        )
        for index, (initial, requested) in enumerate(scenarios):
            with self.subTest(initial=initial, requested=requested):
                record = brain.create_record(
                    self.repository,
                    "finding",
                    f"FIND-EDGE-{index}",
                    f"Edge case {index}",
                    [],
                    ["specs/authority.md"],
                    owner="alice",
                    authority=initial,
                )
                with self.assertRaisesRegex(
                    brain.BrainError,
                    f"Illegal authority transition: {initial} -> {requested}",
                ):
                    brain.update_record(
                        self.repository,
                        record["id"],
                        expected_revision=1,
                        progress=None,
                        next_steps=[],
                        files=[],
                        sources=[],
                        actor="alice",
                        authority=requested,
                        reason="Attempted",
                    )
                unchanged = brain.get_record(self.repository, record["id"])
                self.assertEqual(initial, unchanged["authority"])
                self.assertEqual(1, unchanged["revision"])

    def test_authority_promotion_runs_under_the_same_compare_and_swap(self) -> None:
        record = self.observed_finding("FIND-CAS")
        with self.assertRaisesRegex(brain.BrainError, "Stale finding revision"):
            brain.update_record(
                self.repository,
                record["id"],
                expected_revision=7,
                progress=None,
                next_steps=[],
                files=[],
                sources=[],
                actor="alice",
                authority="verified",
                reason="Stale writer",
            )
        self.assertEqual(
            "observed", brain.get_record(self.repository, record["id"])["authority"]
        )

    def test_event_authority_is_immutable(self) -> None:
        record = brain.create_record(
            self.repository,
            "event",
            "EVENT-AUTH",
            "Deploy happened",
            [],
            ["specs/authority.md"],
            owner="alice",
        )
        with self.assertRaisesRegex(brain.BrainError, "Events are immutable"):
            brain.update_record(
                self.repository,
                record["id"],
                expected_revision=1,
                progress=None,
                next_steps=[],
                files=[],
                sources=[],
                actor="alice",
                authority="verified",
                reason="Attempted",
            )

    def test_tampered_authority_ledger_is_rejected_by_the_validator(self) -> None:
        record = self.observed_finding("FIND-TAMPER")
        path = self.repository / "project-brain/dynamic/findings" / f"{record['id']}.md"
        metadata, body = brain.parse_markdown_record(path)
        metadata["authority"] = "observed"
        metadata["transitions"].append(
            {
                "from": "verified", "to": "observed", "at": brain.utc_now(),
                "actor": "mallory", "reason": "Quietly downgraded",
            }
        )
        path.write_text(brain.render_markdown_record(metadata, body), encoding="utf-8")
        with self.assertRaisesRegex(
            brain.BrainError, "illegal authority transition: verified -> observed"
        ):
            brain.get_record(self.repository, record["id"])

    def test_cli_brain_update_promotes_authority_and_reports_refusals(self) -> None:
        record = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", "FIND-CLI",
                "--title", "CLI finding", "--source", "specs/authority.md",
                "--json",
            ).stdout
        )
        promoted = self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--authority", "verified",
            "--reason", "Verified: covered by the CLI contract test", "--json",
        )
        self.assertEqual(0, promoted.returncode, promoted.stderr)
        self.assertEqual("verified", json.loads(promoted.stdout)["authority"])

        refused = self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--authority", "observed", "--reason", "Downgrade", "--json",
        )
        self.assertNotEqual(0, refused.returncode)
        self.assertIn(
            "Illegal authority transition: verified -> observed", refused.stderr
        )


class ConsolidationPipelineTest(RuntimeHarness):
    """The pipeline had no producers, so its emptiness was unreportable."""

    def counters(self) -> dict:
        result = self.run_cli("status", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)["consolidation"]

    def test_status_reports_an_empty_pipeline_as_zero_not_as_silence(self) -> None:
        # A Project Brain that exists and has produced nothing. Before this
        # the same output meant that and "consolidation ran normally".
        self.start("TASK-EMPTY-PIPELINE")
        counters = self.counters()
        self.assertEqual(0, counters["promotable"], counters)
        self.assertEqual(0, counters["applied"], counters)
        self.assertEqual(0, counters["chunks"], counters)
        rendered = self.run_cli("status")
        self.assertIn("Consolidation: 0 promotable candidate(s)", rendered.stdout)

    def test_a_review_finding_reaches_the_pipeline_when_it_is_a_record(self) -> None:
        # The wiring the whole item is about: a finding written into a task's
        # progress is bookkeeping on a `task` record, and `task` can never be
        # promoted. As its own `finding` record it becomes a candidate.
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        created = self.run_cli(
            "brain-create", "finding", "--external-id", "TASK-1-F1",
            "--title", "Cobalt guard closes the request gap",
            "--source", "specs/authority.md", "--json",
        )
        self.assertEqual(0, created.returncode, created.stderr)
        record_id = json.loads(created.stdout)["id"]
        self.assertEqual(0, self.counters()["promotable"])

        verified = self.run_cli(
            "brain-update", "--record-id", record_id, "--revision", "auto",
            "--authority", "verified", "--reason", "Verified: guard covers it",
            "--json",
        )
        self.assertEqual(0, verified.returncode, verified.stderr)
        resolved = self.run_cli(
            "brain-update", "--record-id", record_id, "--revision", "auto",
            "--progress", "Guard the request gap before dispatch.",
            "--transition", "resolved", "--reason", "Resolved", "--json",
        )
        self.assertEqual(0, resolved.returncode, resolved.stderr)

        self.assertEqual(1, self.counters()["promotable"], self.counters())

    def test_a_finding_without_content_is_reported_as_blocked_not_missing(self) -> None:
        # Resolving without `--progress` produces a record that looks done and
        # can never be promoted. Counting it as blocked is what makes that
        # visible instead of leaving the pipeline mysteriously empty.
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        created = self.run_cli(
            "brain-create", "finding", "--external-id", "TASK-1-F2",
            "--title", "Cobalt guard closes the request gap",
            "--source", "specs/authority.md", "--json",
        )
        record_id = json.loads(created.stdout)["id"]
        self.run_cli(
            "brain-update", "--record-id", record_id, "--revision", "auto",
            "--authority", "verified", "--reason", "Verified", "--json",
        )
        self.run_cli(
            "brain-update", "--record-id", record_id, "--revision", "auto",
            "--transition", "resolved", "--reason", "Resolved", "--json",
        )
        counters = self.counters()
        self.assertEqual(0, counters["promotable"], counters)
        self.assertEqual(1, counters["blocked"], counters)

    def test_counters_report_null_rather_than_zero_without_a_brain(self) -> None:
        # "No Project Brain" and "an empty Project Brain" are different facts;
        # reporting 0 for both is the conflation this whole horizon is about.
        result = self.run_cli("--mode", "lightweight", "status", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        counters = json.loads(result.stdout)["consolidation"]
        self.assertIsNone(counters["promotable"], counters)
        self.assertIsNone(counters["chunks"], counters)
        rendered = self.run_cli("--mode", "lightweight", "status")
        self.assertIn("unavailable promotable candidate(s)", rendered.stdout)


class NearDuplicatePromotionTest(RuntimeHarness):
    """Two different records saying one thing must not become two chunks."""

    def resolved_finding(self, external_id: str, title: str, progress: str) -> dict:
        record = brain.create_record(
            self.repository, "finding", external_id, title, [],
            ["specs/authority.md"], owner="alice",
        )
        brain.update_record(
            self.repository, record["id"], expected_revision=record["revision"],
            progress=None, next_steps=[], files=[], sources=[], actor="alice",
            authority="verified", reason="Verified",
        )
        brain.update_record(
            self.repository, record["id"], expected_revision=record["revision"] + 1,
            progress=progress, next_steps=[], files=[], sources=[], actor="alice",
            transition_to="resolved", reason="Resolved",
        )
        return record

    def promote(self) -> dict:
        return brain.auto_promote(self.repository, owner="alice", limit=5)

    def setUp(self) -> None:
        super().setUp()
        config = self.repository / "project-brain/config/runtime.json"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(
            json.dumps({"mode": "governed", "automatic_promotion": True}),
            encoding="utf-8",
        )

    def test_a_second_record_saying_the_same_thing_is_blocked_by_name(self) -> None:
        consequence = (
            "Guard the request gap before dispatch so the cobalt authority "
            "rule cannot be bypassed by a queued job."
        )
        self.resolved_finding("TASK-F1", "Cobalt guard closes the gap", consequence)
        first = self.promote()
        self.assertEqual(1, len(first["promoted"]), first)
        memory_id = first["promoted"][0]["memory_id"]

        self.resolved_finding("TASK-F2", "Cobalt guard closes the gap", consequence)
        second = self.promote()
        self.assertEqual([], second["promoted"], second)
        self.assertEqual(1, len(second["blocked"]), second)
        # The block names the chunk, so it reads as an instruction rather than
        # as a refusal.
        self.assertIn(memory_id, second["blocked"][0]["reason"], second)
        self.assertIn("merge or supersede", second["blocked"][0]["reason"])

    def test_duplicates_inside_one_flush_are_caught_too(self) -> None:
        # The batch case is the realistic one: a single review produces several
        # findings and one flush promotes up to five of them. A guard evaluated
        # once before the loop would see none of the chunks it is creating.
        consequence = (
            "Guard the request gap before dispatch so the cobalt authority "
            "rule cannot be bypassed by a queued job."
        )
        for index in range(3):
            self.resolved_finding(
                f"TASK-B{index}", "Cobalt guard closes the gap", consequence
            )
        result = self.promote()
        self.assertEqual(1, len(result["promoted"]), result)
        self.assertEqual(2, len(result["blocked"]), result)
        for entry in result["blocked"]:
            self.assertIn("near-duplicate", entry["reason"])

    def test_a_genuinely_different_consequence_still_promotes(self) -> None:
        # The other half of the contract: the guard must not become a cap on
        # how much durable memory a project may hold.
        self.resolved_finding(
            "TASK-D1", "Cobalt guard closes the gap",
            "Guard the request gap before dispatch so the authority rule holds.",
        )
        self.promote()
        self.resolved_finding(
            "TASK-D2", "Doctrine migration needs a backfill window",
            "Run the invoice backfill in batches of a thousand with a resumable "
            "cursor, because a single transaction locks the ledger table.",
        )
        result = self.promote()
        self.assertEqual(1, len(result["promoted"]), result)
        self.assertEqual([], result["blocked"], result)

    def test_the_threshold_comes_from_configuration(self) -> None:
        consequence = (
            "Guard the request gap before dispatch so the cobalt authority "
            "rule cannot be bypassed by a queued job."
        )
        self.resolved_finding("TASK-T1", "Cobalt guard closes the gap", consequence)
        self.promote()
        config = self.repository / "project-brain/config/runtime.json"
        config.write_text(
            json.dumps(
                {
                    "mode": "governed",
                    "automatic_promotion": True,
                    "bank_duplicate_ratio": 1.0,
                }
            ),
            encoding="utf-8",
        )
        self.resolved_finding("TASK-T2", "Cobalt guard closes the gap", consequence)
        result = self.promote()
        # A threshold of 1.0 demands identity, so a near duplicate gets through.
        self.assertEqual(1, len(result["promoted"]), result)

    def test_similarity_is_reproducible_without_a_local_index(self) -> None:
        # The decision must not depend on `memory-bank/local/`, which is
        # git-ignored and disposable: two machines on one commit have to
        # promote the same set.
        left = "Guard the request gap before dispatch so the rule holds."
        self.assertEqual(
            brain.text_similarity(left, left), 1.0
        )
        self.assertEqual(0.0, brain.text_similarity(left, ""))
        self.assertLess(
            brain.text_similarity(left, "Batch the invoice backfill by cursor."),
            0.6,
        )


class EpisodicPillarTest(RuntimeHarness):
    """The episodic layer gets a git-tracked source of its own."""

    OUTCOME = "Zirconium gateway retry window widened to five minutes."
    CHECK = "phpunit --filter Gateway"

    def complete_task(self, task_id: str) -> dict:
        task = self.start(task_id)
        result = self.run_cli(
            "complete", "--task-id", task_id,
            "--revision", str(task["revision"]),
            "--outcome", self.OUTCOME, "--verification", self.CHECK, "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def document_layers(self, prefix: str) -> dict:
        connection = sqlite3.connect(
            self.repository / "memory-bank/local/context.db"
        )
        try:
            return dict(
                connection.execute(
                    "SELECT path, layer FROM documents WHERE path LIKE ?",
                    (f"{prefix}%",),
                ).fetchall()
            )
        finally:
            connection.close()

    def test_completing_a_task_writes_one_git_tracked_event(self) -> None:
        # Completed work lived only in the disposable local database, so the
        # one layer meant to hold "what happened here" could not survive a
        # fresh clone.
        payload = self.complete_task("TASK-EPISODE")
        self.assertIsNotNone(payload["event_id"], payload)

        events = sorted(
            (self.repository / "project-brain/dynamic/events").glob("*.md")
        )
        self.assertEqual(1, len(events), events)
        record, _ = brain.parse_markdown_record(events[0])
        self.assertEqual("event", record["type"])
        self.assertIn("Zirconium gateway retry window", record["goal"])
        self.assertIn(self.CHECK, record["goal"])

    def test_the_completion_event_lands_in_the_episodic_layer(self) -> None:
        self.complete_task("TASK-EPISODE-LAYER")
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        layers = self.document_layers("project-brain/dynamic/events/")
        self.assertEqual(1, len(layers), layers)
        self.assertEqual(["episodic"], list(layers.values()), layers)

    def test_the_event_fills_the_episodic_slot_ahead_of_the_changelog(self) -> None:
        # The readiness criterion: on a query about what happened, the slot
        # carries the record of what happened, not the repository changelog.
        self.repository.joinpath("CHANGELOG.md").write_text(
            "# Changelog\n\n## Unreleased\n\nUnrelated tooling change.\n",
            encoding="utf-8",
        )
        self.complete_task("TASK-EPISODE-SLOT")
        self.start("TASK-EPISODE-READER")
        capsule = json.loads(
            self.run_cli(
                "retrieve", "zirconium gateway retry window",
                "--task-id", "TASK-EPISODE-READER", "--ephemeral", "--json",
            ).stdout
        )
        episodic = [item["path"] for item in capsule["episodic"] if "path" in item]
        self.assertEqual(1, len(episodic), capsule)
        self.assertTrue(
            episodic[0].startswith("project-brain/dynamic/events/"), capsule
        )
        # It came through the governed path, so the manifest describes it.
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        self.assertIn(
            episodic[0], [item["path"] for item in manifest["selected"]], manifest
        )

    def test_an_incident_stays_semantic(self) -> None:
        # Only `event` is episodic. An open incident is active, urgent,
        # promotable content: demoting it to the single episodic slot would
        # take it out of the runtime filters, the budget and the manifest.
        self.start("TASK-INCIDENT")
        brain.create_record(
            self.repository, "incident", "INC-1",
            "Zirconium gateway outage", [], ["specs/authority.md"],
            owner="alice",
        )
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        layers = self.document_layers("project-brain/dynamic/incidents/")
        self.assertEqual(["semantic"], list(layers.values()), layers)

    def test_a_failed_event_write_never_reopens_a_completed_task(self) -> None:
        # The episode is a record of something that already happened. If it
        # cannot be written, the thing still happened: a completion must not
        # be undone because its bookkeeping failed.
        task = self.start("TASK-EPISODE-FAILS")
        with mock.patch.object(
            context_cli, "create_record", side_effect=brain.BrainError("no")
        ):
            code, stdout, _ = self.run_main(
                "complete", "--task-id", "TASK-EPISODE-FAILS",
                "--revision", str(task["revision"]),
                "--outcome", self.OUTCOME, "--verification", self.CHECK, "--json",
            )
        self.assertEqual(0, code, stdout)
        payload = json.loads(stdout)
        self.assertEqual("completed", payload["status"])
        self.assertIsNone(payload["event_id"], payload)
        self.assertEqual(
            [], list((self.repository / "project-brain/dynamic/events").glob("*.md"))
        )


class RetrievalReportTest(RuntimeHarness):
    """Manifests were written and never read; this is the reader."""

    def retrieve(self, task: str, query: str, *extra: str) -> dict:
        result = self.run_cli(
            "retrieve", query, "--task-id", task, "--ephemeral", "--json", *extra
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def report(self, *extra: str) -> dict:
        result = self.run_cli("retrieval-report", "--json", *extra)
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def setUp(self) -> None:
        super().setUp()
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        self.start("TASK-REPORT")

    def test_a_report_over_no_manifests_says_so(self) -> None:
        rendered = self.run_cli("retrieval-report")
        self.assertEqual(0, rendered.returncode, rendered.stderr)
        self.assertIn("Lightweight mode writes none", rendered.stdout)

    def test_the_report_aggregates_the_signals_the_gate_records(self) -> None:
        self.retrieve("TASK-REPORT", "cobalt authority")
        self.retrieve("TASK-REPORT", "cobalt authority")
        report = self.report()

        self.assertEqual(2, report["turns"], report)
        self.assertEqual({"explicit": 2}, report["query_source"], report)
        self.assertEqual(2, report["gate"]["decided"], report)
        # The second turn repeats the first, so the gate records a skip even
        # though shadow mode delivered it anyway.
        self.assertEqual(2, report["gate"]["shadow_decided"], report)
        self.assertEqual(1, report["gate"]["would_skip"], report)
        self.assertEqual(0.5, report["gate"]["would_skip_rate"], report)
        self.assertEqual(0, report["gate"]["withheld"], report)
        self.assertIn("repeat-retrieval", report["gate"]["skip_reasons"], report)
        self.assertGreater(report["phase_seconds"]["retrieval"]["samples"], 0)
        self.assertEqual(2, report["token_estimates"]["total"]["samples"], report)
        self.assertGreater(report["no_match"].get("episodic", 0), 0, report)
        self.assertEqual(
            [
                {
                    "decision": "retrieve", "query_source": "explicit",
                    "host": "cli", "entry_point": "retrieve", "mode": "shadow",
                    "turns": 1,
                },
                {
                    "decision": "skip", "query_source": "explicit",
                    "host": "cli", "entry_point": "retrieve", "mode": "shadow",
                    "turns": 1,
                },
            ],
            report["gate"]["slices"],
        )

    def test_report_separates_shadow_verdicts_from_enforced_withholding(self) -> None:
        self.retrieve("TASK-REPORT", "cobalt authority", "--gate", "off")
        self.retrieve("TASK-REPORT", "cobalt authority", "--gate", "shadow")
        self.retrieve("TASK-REPORT", "cobalt authority", "--gate", "enforce")

        report = self.report()

        self.assertEqual(2, report["gate"]["decided"], report)
        self.assertEqual(1, report["gate"]["shadow_decided"], report)
        self.assertEqual(1, report["gate"]["would_skip"], report)
        self.assertEqual(1.0, report["gate"]["would_skip_rate"], report)
        self.assertEqual(1, report["gate"]["enforce_decided"], report)
        self.assertEqual(1, report["gate"]["withheld"], report)
        self.assertEqual(1.0, report["gate"]["withheld_rate"], report)
        self.assertEqual(
            {("shadow", "skip"), ("enforce", "skip")},
            {
                (item["mode"], item["decision"])
                for item in report["gate"]["slices"]
            },
            report,
        )

    def test_episode_only_turn_is_not_reported_as_empty(self) -> None:
        recorded = self.run_cli(
            "record",
            "--summary", "Vermilion semaphore replay",
            "--outcome", "The rollout completed cleanly.",
            "--json",
        )
        self.assertEqual(0, recorded.returncode, recorded.stderr)
        self.retrieve("TASK-REPORT", "vermilion semaphore")

        report = self.report()

        self.assertEqual(0, report["empty_selection"], report)
        self.assertEqual(1, report["local_episode_count"], report)
        self.assertEqual(1, report["token_estimates"]["local_episodes"]["samples"], report)
        self.assertGreater(report["token_estimates"]["local_episodes"]["p50"], 0)

    def test_both_top_scores_are_reported_because_they_diverge(self) -> None:
        # `top_candidate` is the best candidate before any policy filter;
        # `top_delivered` is the best the capsule carried. They differ exactly
        # when the best match was withheld, which is the case a report exists
        # to surface, so the report never picks one for the reader.
        self.retrieve("TASK-REPORT", "cobalt authority")
        report = self.report()
        self.assertIn("top_candidate_score", report)
        self.assertIn("top_delivered_score", report)
        self.assertGreater(report["top_candidate_score"]["samples"], 0)

    def test_a_version_one_manifest_is_counted_not_crashed_on(self) -> None:
        # 95% of a real corpus predates the fields this report reads. Silently
        # averaging over the subset that has them would misreport the rest.
        self.retrieve("TASK-REPORT", "cobalt authority")
        directory = self.repository / "memory-bank/local/retrieval-manifests"
        legacy = {
            "schema_version": 1,
            "id": "00000000-0000-4000-8000-00000000000a",
            "created_at": "2026-01-01T00:00:00+00:00",
            "query": "legacy",
            "task_id": "00000000-0000-4000-8000-00000000000b",
            "task_revision": 1,
            "filters": {
                "privacy": ["public"], "owners": ["*"],
                "authority": ["verified"], "freshness": True, "active_only": True,
            },
            "selected": [],
            "excluded": [],
            "token_estimates": {
                "policy": 0, "handoff": 0, "durable": 0, "dynamic": 0,
                "evidence": 0, "total": 0, "target": 8000, "hard": 12000,
            },
            "provider": "sqlite-fts5",
            "escalation_reason": None,
        }
        (directory / f"{legacy['id']}.json").write_text(
            json.dumps(legacy, indent=2), encoding="utf-8"
        )

        report = self.report()
        self.assertEqual(2, report["turns"], report)
        self.assertEqual({"1": 1, "3": 1}, report["schema_versions"], report)
        # One of the two could answer the gate question; the report says so
        # rather than dividing by two.
        self.assertEqual(1, report["gate"]["decided"], report)
        self.assertEqual(1, report["empty_selection"], report)

    def test_the_report_never_carries_the_query_text(self) -> None:
        # The manifest keeps the distilled query under privacy rules; an
        # aggregate must not become a second copy of it.
        self.retrieve("TASK-REPORT", "cobalt authority")
        rendered = self.run_cli("retrieval-report")
        payload = self.run_cli("retrieval-report", "--json")
        for output in (rendered.stdout, payload.stdout):
            self.assertNotIn("cobalt authority", output)


class RefreshHealthTest(RuntimeHarness):
    """The hook's honesty used to live exactly one turn."""

    def health(self, *extra: str) -> dict:
        result = self.run_cli("health", "--json", *extra)
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def test_health_over_no_records_says_so(self) -> None:
        rendered = self.run_cli("health")
        self.assertEqual(0, rendered.returncode, rendered.stderr)
        self.assertIn("No refresh health records", rendered.stdout)

    def test_each_refresh_appends_one_record(self) -> None:
        self.assertEqual(0, self.run_cli("refresh").returncode)
        self.assertEqual(0, self.run_cli("refresh").returncode)
        health = self.health()
        self.assertEqual(2, health["turns"], health)
        self.assertGreater(health["phase_seconds"]["index"]["samples"], 0)
        self.assertEqual(0.0, health["timeout_rate"], health)

    def test_a_zeroed_omitted_counter_is_not_a_lossy_capsule(self) -> None:
        # A lightweight capsule always carries the keys with zeros, so testing
        # the dict for emptiness would report every turn as lossy.
        path = self.repository / "memory-bank/local/refresh-health.ndjson"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            json.dumps({"at": "x", "omitted": {"procedural": 0, "semantic": 0}})
            + "\n"
            + json.dumps({"at": "y", "omitted": {"procedural": 1, "semantic": 0}})
            + "\n",
            encoding="utf-8",
        )
        health = self.health()
        self.assertEqual(0.5, health["lossy_capsule_rate"], health)

    def test_the_window_bounds_what_is_aggregated(self) -> None:
        path = self.repository / "memory-bank/local/refresh-health.ndjson"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            "".join(
                json.dumps({"at": str(index), "hook_status": 0}) + "\n"
                for index in range(10)
            ),
            encoding="utf-8",
        )
        self.assertEqual(10, self.health()["turns"])
        self.assertEqual(3, self.health("--window", "3")["turns"])

    def test_a_health_record_carries_no_query_text(self) -> None:
        self.start("TASK-HEALTH")
        self.run_cli(
            "refresh", "--query", "cobalt authority rollout",
            "--task-id", "TASK-HEALTH", "--ephemeral",
        )
        path = self.repository / "memory-bank/local/refresh-health.ndjson"
        self.assertNotIn("cobalt", path.read_text(encoding="utf-8"))


class MemoryProbeTest(RuntimeHarness):
    """Category contracts for retrieval, with negatives declared as data.

    The frozen fixture beside this one expresses only `{query, expected[]}`, so
    a negative had to be hardcoded in a test body and a restraint case could not
    be written at all. Here every case names its own seed, its forbidden paths
    and whether it expects a match, and the category decides which contract
    applies.

    On RESTRAINT the predicate is deliberately narrower than the roadmap
    proposed. That item asked for "the capsule selected no seed document, or
    marked its selection weak", which would declare a documented and
    deliberate behaviour a defect: `docs/CONTEXT-AND-MEMORY.md` records that a
    query sharing one term with the corpus surfaces the least-bad lexical
    match rather than nothing, on the stated ground that a hidden answer costs
    more than a spurious one. So restraint is measured where the claim is
    unambiguous - a subject the corpus does not contain must produce
    `no-match` - and the single-rare-term case asserts the weaker, checkable
    thing instead: the selection is *marked* `distinctive`, which is what
    H1-04 built the label for.
    """

    PROBE_DIR = "specs/probe"

    def fixture(self) -> dict:
        return json.loads(
            (Path(__file__).parent / "fixtures/memory-probes.json").read_text(
                encoding="utf-8"
            )
        )

    def reset_corpus(self, filler: int) -> None:
        # Both the seeded documents and the probe chunks are cleared: without
        # this a later case retrieves an earlier case's corpus and the gate
        # stops measuring the contract it names.
        probe = self.repository / self.PROBE_DIR
        if probe.is_dir():
            for path in probe.glob("*.md"):
                path.unlink()
        probe.mkdir(parents=True, exist_ok=True)
        chunks = self.repository / "memory-bank/chunks"
        if chunks.is_dir():
            for path in chunks.glob("MEM-2026010*-probe.md"):
                path.unlink()
            self.run_cli("reindex-bank")
        # A term counts as distinctive at a document frequency of ten per cent
        # or less, so a three-document corpus cannot produce one and the
        # weak-match branch would be unreachable.
        for index in range(filler):
            (probe / f"filler-{index}.md").write_text(
                f"# Filler {index}\n\nUnrelated boilerplate paragraph {index}.\n",
                encoding="utf-8",
            )

    def seed_documents(self, seed: list) -> None:
        for document in seed:
            path = self.repository / document["path"]
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(
                f"# Seed\n\n{document['text']}\n", encoding="utf-8"
            )

    def seed_update_pair(self, case: dict) -> tuple[str, str]:
        """Write a chunk, retire it, and write its replacement.

        UPDATE is the category the old fixture shape could not express at all:
        it needs a fact that stopped being true and the fact that replaced it,
        and the retired one has to leave retrieval without leaving the disk.
        """
        today = datetime.now(timezone.utc).date()
        dates = {"yesterday": today - timedelta(days=1), "today": today}
        chunks = self.repository / "memory-bank/chunks"
        chunks.mkdir(parents=True, exist_ok=True)

        def write(memory_id: str, spec: dict) -> Path:
            metadata = {
                "id": memory_id,
                "title": spec["title"],
                "type": "convention",
                "status": "active",
                "scope": ["application"],
                "tags": ["probe"],
                "created": today.isoformat(),
                "last_verified": today.isoformat(),
                "review_after": (today + timedelta(days=365)).isoformat(),
                "sources": ["specs/authority.md"],
                "supersedes": [],
                "superseded_by": None,
            }
            path = chunks / f"{memory_id}-probe.md"
            path.write_text(
                f"---\n{json.dumps(metadata, indent=2)}\n---\n\n"
                f"# {spec['title']}\n\n{spec['body']}\n",
                encoding="utf-8",
            )
            return path

        retired = write("MEM-20260101-aaaaaaaa", case["retire_chunk"])
        replacement = write("MEM-20260102-bbbbbbbb", case["replacement_chunk"])
        self.assertEqual(0, self.run_cli("reindex-bank").returncode)
        result = self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa",
            "--valid-to", dates[case["retire_chunk"]["valid_to"]].isoformat(),
            "--superseded-by", "MEM-20260102-bbbbbbbb",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return (
            retired.relative_to(self.repository).as_posix(),
            replacement.relative_to(self.repository).as_posix(),
        )

    def test_memory_probes_meet_category_contracts(self) -> None:
        fixture = self.fixture()
        self.start("TASK-PROBES")
        for case in fixture["cases"]:
            with self.subTest(case=case["id"], category=case["category"]):
                self.reset_corpus(fixture["filler"])
                self.seed_documents(case.get("seed") or [])
                expected = list(case["expected_paths"])
                forbidden = list(case["forbidden_paths"])
                if "retire_chunk" in case:
                    retired, replacement = self.seed_update_pair(case)
                    expected = [
                        replacement if path == "__replacement__" else path
                        for path in expected
                    ]
                    forbidden = [
                        retired if path == "__retired__" else path
                        for path in forbidden
                    ]
                self.assertEqual(0, self.run_cli("index", "--json").returncode)

                result = self.run_cli(
                    "retrieve", case["query"], "--task-id", "TASK-PROBES",
                    "--ephemeral", "--json",
                )
                self.assertEqual(0, result.returncode, result.stderr)
                capsule = json.loads(result.stdout)
                delivered = {
                    item["path"]
                    for layer in ("procedural", "semantic", "episodic")
                    for item in capsule[layer]
                    if "path" in item
                }

                # Universal, every category: a forbidden path is never
                # delivered. This is the assertion the old fixture had to
                # hardcode in the test body.
                for path in forbidden:
                    self.assertNotIn(path, delivered, capsule)

                if case["expect_no_match"]:
                    # The subject is absent from the corpus, so every layer
                    # this call answers for must say so.
                    self.assertEqual(
                        ["procedural", "semantic", "episodic"],
                        capsule["no_match"],
                        capsule,
                    )
                    self.assertEqual(set(), delivered, capsule)
                    continue

                weak = case.get("expect_weak_match")
                if weak:
                    strengths = {
                        item["path"]: item.get("match")
                        for layer in ("procedural", "semantic", "episodic")
                        for item in capsule[layer]
                        if "path" in item
                    }
                    for path in weak:
                        self.assertEqual(
                            "distinctive", strengths.get(path), capsule
                        )
                    continue

                # recall / update / reasoning: every expected path is
                # delivered. The capsule carries at most three semantic slots,
                # so a case may not expect more than three.
                self.assertLessEqual(len(expected), 3, case["id"])
                for path in expected:
                    self.assertIn(path, delivered, capsule)


class SkillRoutingTest(RuntimeHarness):
    """Does the request reach the skill that answers it?

    The procedural layer is the largest slice of the index and had no eval
    case at all, while the capsule gives it two slots - so a skill ranked
    third never reaches the model no matter how right it is. This gate copies
    the edition's real skill tree, asks it real questions, and asserts that an
    acceptable skill lands in the top two at least as often as it does today.

    `acceptable` is a set, not one slug. Several of these requests have two
    defensibly correct answers, and asserting a single one would measure the
    fixture author's taste rather than routing quality.

    `min_top2` is the measured floor, not a target. Routing is poor today -
    Symfony scores 10 of 16 - and the number exists so that it cannot quietly
    get worse while someone edits a `description:`. Raise it only together
    with a change that improves it.
    """

    def test_skill_routing_meets_its_measured_floor(self) -> None:
        # The golden set names skills from this edition's own roster and its
        # floor is a measured number, so unlike the memory probes it cannot be
        # shared between editions or invented for one. An edition that ships
        # no set is reported as uncovered rather than silently passing: the
        # gap is real work its author owes, not an engine defect.
        golden = Path(__file__).parent / "fixtures/skill-routing-golden.json"
        if not golden.is_file():
            self.skipTest(
                "this edition ships no fixtures/skill-routing-golden.json; "
                "author its cases and record the measured min_top2 floor"
            )
        fixture = json.loads(
            golden.read_text(
                encoding="utf-8"
            )
        )
        source = EDITION / ".agents" / "skills"
        if not source.is_dir():
            self.skipTest("edition ships no canonical skill tree")
        shutil.copytree(source, self.repository / ".agents" / "skills")
        self.assertEqual(0, self.run_cli("index", "--json").returncode)

        connection = context_cli.connect(
            self.repository / "memory-bank/local/context.db"
        )
        hits = 0
        misses = []
        try:
            for case in fixture["cases"]:
                rows = context_cli.search_documents(
                    connection, case["request"], 2, "procedural", relevant_only=True
                )
                slugs = {
                    row["path"].split("/skills/", 1)[1].split("/")[0]
                    for row in rows
                    if "/skills/" in row["path"]
                }
                if slugs & set(case["acceptable"]):
                    hits += 1
                else:
                    misses.append((case["request"], sorted(slugs)))
        finally:
            connection.close()

        self.assertGreaterEqual(
            hits,
            fixture["min_top2"],
            "skill routing regressed below its recorded floor "
            f"({hits}/{len(fixture['cases'])} < {fixture['min_top2']}); "
            f"misses: {misses}",
        )


class BankFixture(RuntimeHarness):
    """A governed repository with one durable chunk citing one real source."""

    def write_chunk(
        self, memory_id: str, slug: str, body: str, **overrides: object
    ) -> Path:
        today = datetime.now(timezone.utc).date()
        metadata = {
            "id": memory_id,
            "title": slug.replace("-", " ").title(),
            "type": "convention",
            "status": "active",
            "scope": ["application"],
            "tags": ["context"],
            "created": today.isoformat(),
            "last_verified": today.isoformat(),
            "review_after": (today + timedelta(days=365)).isoformat(),
            "sources": ["specs/authority.md"],
            "supersedes": [],
            "superseded_by": None,
        }
        metadata.update(overrides)
        path = self.repository / "memory-bank/chunks" / f"{memory_id}-{slug}.md"
        path.write_text(
            f"---\n{json.dumps(metadata, indent=2)}\n---\n\n# {metadata['title']}\n\n{body}\n",
            encoding="utf-8",
        )
        return path

    def setUp(self) -> None:
        super().setUp()
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\ncobalt authority specification detail.\n",
            encoding="utf-8",
        )
        self.chunk = self.write_chunk(
            "MEM-20260101-aaaaaaaa", "old-rule", "The cerulean rollout is current."
        )
        self.assertEqual(0, self.run_cli("reindex-bank").returncode)

    def bank_errors(self) -> list:
        return brain.validate_bank(self.repository / "memory-bank")


class BankRetireTest(BankFixture):
    """Closing a chunk's period as one transaction rather than by hand."""

    def test_retire_without_a_successor_archives_and_dates_the_chunk(self) -> None:
        # `superseded` demands a successor, so knowledge that simply ceased is
        # archived. The date says when it stopped being true; the status says
        # that nothing took its place.
        yesterday = (datetime.now(timezone.utc).date() - timedelta(days=1)).isoformat()
        result = self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa",
            "--valid-to", yesterday, "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual("archived", payload["status"])
        self.assertEqual(yesterday, payload["valid_to"])

        metadata = json.loads(
            self.chunk.read_text(encoding="utf-8")[4:].split("\n---\n", 1)[0]
        )
        self.assertEqual("archived", metadata["status"])
        self.assertEqual(yesterday, metadata["valid_to"])
        self.assertIsNone(metadata["superseded_by"])
        self.assertEqual([], self.bank_errors())

    def test_retire_leaves_the_body_and_the_untouched_fields_alone(self) -> None:
        before = self.chunk.read_text(encoding="utf-8")
        before_metadata = json.loads(before[4:].split("\n---\n", 1)[0])
        before_body = before.split("\n---\n", 1)[1]

        self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa",
            "--valid-to", (datetime.now(timezone.utc).date()).isoformat(),
        )
        after = self.chunk.read_text(encoding="utf-8")
        after_metadata = json.loads(after[4:].split("\n---\n", 1)[0])
        self.assertEqual(before_body, after.split("\n---\n", 1)[1])
        # Key order survives, so the file stays recognisable as the one a
        # human edited rather than being re-serialized wholesale.
        self.assertEqual(
            [key for key in before_metadata if key not in ("valid_to",)],
            [key for key in after_metadata if key not in ("valid_to",)],
        )
        for key in ("created", "last_verified", "review_after", "sources", "title"):
            self.assertEqual(before_metadata[key], after_metadata[key], key)

    def test_retire_with_a_successor_writes_both_sides_of_the_link(self) -> None:
        successor = self.write_chunk(
            "MEM-20260102-bbbbbbbb", "new-rule", "The cerulean rollout changed."
        )
        self.assertEqual(0, self.run_cli("reindex-bank").returncode)
        today = datetime.now(timezone.utc).date().isoformat()

        result = self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa", "--valid-to", today,
            "--superseded-by", "MEM-20260102-bbbbbbbb", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)

        retired = json.loads(
            self.chunk.read_text(encoding="utf-8")[4:].split("\n---\n", 1)[0]
        )
        replacement = json.loads(
            successor.read_text(encoding="utf-8")[4:].split("\n---\n", 1)[0]
        )
        self.assertEqual("superseded", retired["status"])
        self.assertEqual("MEM-20260102-bbbbbbbb", retired["superseded_by"])
        self.assertIn("MEM-20260101-aaaaaaaa", replacement["supersedes"])
        # Both sides written, so the bank is never valid-on-one-side-only.
        self.assertEqual([], self.bank_errors())

    def test_an_interrupted_retire_leaves_the_bank_exactly_as_it_was(self) -> None:
        # The defect this replaces: retiring by hand means editing two chunks
        # and the index, and between any two of those edits the bank is
        # invalid. Here the second write fails and nothing survives it.
        successor = self.write_chunk(
            "MEM-20260102-bbbbbbbb", "new-rule", "The cerulean rollout changed."
        )
        self.assertEqual(0, self.run_cli("reindex-bank").returncode)
        before_chunk = self.chunk.read_text(encoding="utf-8")
        before_successor = successor.read_text(encoding="utf-8")
        before_index = (self.repository / "memory-bank/INDEX.md").read_text(
            encoding="utf-8"
        )

        real_write = brain.atomic_write
        calls = {"n": 0}

        def failing_write(path: Path, content: str) -> None:
            calls["n"] += 1
            if calls["n"] == 2:
                raise OSError("interrupted between the two sides of the link")
            real_write(path, content)

        with mock.patch.object(brain, "atomic_write", failing_write):
            with self.assertRaises(OSError):
                brain.retire_chunk(
                    self.repository,
                    "MEM-20260101-aaaaaaaa",
                    valid_to=datetime.now(timezone.utc).date().isoformat(),
                    superseded_by="MEM-20260102-bbbbbbbb",
                )

        self.assertEqual(before_chunk, self.chunk.read_text(encoding="utf-8"))
        self.assertEqual(before_successor, successor.read_text(encoding="utf-8"))
        self.assertEqual(
            before_index,
            (self.repository / "memory-bank/INDEX.md").read_text(encoding="utf-8"),
        )
        self.assertEqual([], self.bank_errors())

    def test_a_missing_successor_is_refused_before_anything_is_written(self) -> None:
        before = self.chunk.read_text(encoding="utf-8")
        result = self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa",
            "--valid-to", datetime.now(timezone.utc).date().isoformat(),
            "--superseded-by", "MEM-20260103-cccccccc",
        )
        self.assertNotEqual(0, result.returncode)
        self.assertIn("MEM-20260103-cccccccc", result.stderr)
        self.assertEqual(before, self.chunk.read_text(encoding="utf-8"))
        self.assertEqual([], self.bank_errors())

    def test_a_retired_chunk_keeps_its_index_row_and_leaves_retrieval(self) -> None:
        # INDEX.md must keep the row — a chunk on disk without one is a
        # validation error — so "gone from the index" means gone from
        # retrieval, which is a different index and a separate step.
        yesterday = (datetime.now(timezone.utc).date() - timedelta(days=1)).isoformat()
        self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa", "--valid-to", yesterday
        )
        index_text = (self.repository / "memory-bank/INDEX.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("MEM-20260101-aaaaaaaa", index_text)
        self.assertIn("archived", index_text)
        self.assertEqual([], self.bank_errors())

        indexed = self.run_cli("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        excluded = {
            item["path"]: item["reason"]
            for item in json.loads(indexed.stdout)["excluded"]
        }
        self.assertEqual(
            "archived",
            excluded.get("memory-bank/chunks/MEM-20260101-aaaaaaaa-old-rule.md"),
            excluded,
        )
        self.assertTrue(self.chunk.is_file())

    def test_a_bad_date_is_refused_without_touching_the_bank(self) -> None:
        before = self.chunk.read_text(encoding="utf-8")
        result = self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa", "--valid-to", "yesterday"
        )
        self.assertNotEqual(0, result.returncode)
        self.assertIn("ISO date", result.stderr)
        self.assertEqual(before, self.chunk.read_text(encoding="utf-8"))


class DocumentLinkTest(BankFixture):
    """The reverse index, written as the measurement that justified it.

    Two chunks are seeded whose only connection is that both cite the same
    specification, and neither repeats that specification's vocabulary. Asked
    for in the source's own words, retrieval returns the source and neither
    chunk — 0 of 2. That is the measured failure H4-02 recorded, and these
    tests assert both halves of it: that it is real (so the fixture cannot rot
    into one that passes for the wrong reason) and that the link route closes
    it without anyone authoring an edge.
    """

    SOURCE = "specs/rounding.md"
    # Distinctive vocabulary, present in the source and in neither chunk.
    SOURCE_WORDS = "half-to-even banker tie breaking"
    # Vocabulary the two chunks share with each other and not with the source.
    SHARED_WORDS = "stored invoice total"

    def setUp(self) -> None:
        super().setUp()
        self.repository.joinpath(self.SOURCE).write_text(
            "# Rounding\n\nAmounts use half-to-even tie breaking, sometimes "
            "called banker rounding, which suppresses the upward bias of "
            "half-up tie breaking.\n",
            encoding="utf-8",
        )
        self.first = self.write_chunk(
            "MEM-20260101-bbbbbbbb",
            "minor-units",
            "Every invoice total is persisted as an integer count of minor "
            "units, so floating point never touches a stored invoice total.",
            sources=[self.SOURCE],
        )
        self.second = self.write_chunk(
            "MEM-20260101-cccccccc",
            "refund-reconciliation",
            "A refund is validated against the stored invoice total, so a "
            "refund can never exceed the stored invoice total.",
            sources=[f"{self.SOURCE}#L1-L3"],
        )
        self.assertEqual(0, self.run_cli("index").returncode)

    def chunk_paths(self) -> set:
        return {
            path.relative_to(self.repository).as_posix()
            for path in (self.first, self.second)
        }

    def search_paths(self, query: str) -> set:
        result = self.run_cli("search", query, "--limit", "40", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        return {item["path"] for item in json.loads(result.stdout)["documents"]}

    def link_paths(self, path: str, *extra: str) -> set:
        result = self.run_cli("links", "--path", path, *extra, "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        return {item["path"] for item in json.loads(result.stdout)["documents"]}

    def test_the_sources_words_do_not_reach_the_chunks(self) -> None:
        # The failure this index exists to repair. If this ever starts passing
        # the fixture has lost its point and the rest of the class proves
        # nothing.
        found = self.search_paths(self.SOURCE_WORDS)
        self.assertIn(self.SOURCE, found)
        self.assertEqual(set(), self.chunk_paths() & found)

    def test_the_chunks_are_retrievable_by_their_own_words(self) -> None:
        # Control: 0 of 2 above is a missing edge, not a missing index entry.
        self.assertEqual(
            self.chunk_paths(), self.chunk_paths() & self.search_paths(self.SHARED_WORDS)
        )

    def test_the_source_path_reaches_both_chunks(self) -> None:
        self.assertEqual(self.chunk_paths(), self.link_paths(self.SOURCE))

    def test_a_line_range_links_to_the_document(self) -> None:
        # The second chunk cites `#L1-L3`; `fingerprint()` anchors the same
        # way, so a link is to the document rather than to a range inside it.
        self.assertIn(
            self.second.relative_to(self.repository).as_posix(),
            self.link_paths(f"{self.SOURCE}#L9-L12"),
        )

    def test_a_prefix_query_matches_a_directory(self) -> None:
        # A superset assertion: the inherited fixture chunk cites
        # specs/authority.md, and it belongs under this prefix too.
        self.assertTrue(self.chunk_paths() <= self.link_paths("specs", "--prefix"))

    def test_a_prefix_query_does_not_match_a_sibling_by_string(self) -> None:
        # "spec" must not reach "specs/..." — a bare LIKE would.
        self.assertEqual(set(), self.link_paths("spec", "--prefix"))

    def test_an_uncited_path_links_to_nothing(self) -> None:
        self.assertEqual(set(), self.link_paths("specs/authority.md") & self.chunk_paths())

    def test_links_withholds_a_document_that_no_longer_matches_the_index(self) -> None:
        # The runtime filter has to run, not just the join. Removing the chunks
        # without reindexing leaves their rows in place, which is exactly the
        # `stale` case.
        self.first.unlink()
        self.second.unlink()
        result = self.run_cli("links", "--path", self.SOURCE, "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual([], report["documents"])
        self.assertEqual(
            self.chunk_paths(), {item["path"] for item in report["excluded"]}
        )

    def test_links_withholds_a_record_the_config_no_longer_allows(self) -> None:
        # The hazard `search_documents` carries a comment about: the index is a
        # cache, so a `team` record indexed before the policy narrowed is still
        # sitting in it. A links command modelled on `search` — which does not
        # join document_metadata — would republish it.
        self.assertEqual(
            0,
            self.run_cli(
                "start", "--task-id", "TASK-LINK", "--goal",
                "Apply the rounding policy.", "--source", self.SOURCE,
            ).returncode,
        )
        self.assertEqual(0, self.run_cli("index").returncode)
        record = {
            item["path"]
            for item in json.loads(
                self.run_cli("links", "--path", self.SOURCE, "--json").stdout
            )["documents"]
        } - self.chunk_paths()
        self.assertTrue(record, "the governed record should link before the policy narrows")

        config = self.repository / "project-brain/config/runtime.json"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(
            json.dumps({"allowed_privacy": ["public"]}), encoding="utf-8"
        )
        report = json.loads(
            self.run_cli("links", "--path", self.SOURCE, "--json").stdout
        )
        self.assertEqual(set(), {item["path"] for item in report["documents"]} & record)
        # A subset assertion: the task's handoff carries the same sources and
        # is withheld for the same reason.
        self.assertTrue(
            record
            <= {
                item["path"]
                for item in report["excluded"]
                if item["reason"] == "privacy"
            }
        )

    def test_a_record_source_is_reachable_by_path_in_its_own_body(self) -> None:
        # The cheapest half of the same repair: the index reads bodies, so a
        # record created with `--source` was previously unreachable by the one
        # string a reader is most likely to search for.
        self.assertEqual(
            0,
            self.run_cli(
                "start", "--task-id", "TASK-BODY", "--goal",
                "Apply the rounding policy.", "--source", self.SOURCE,
            ).returncode,
        )
        self.assertEqual(0, self.run_cli("index").returncode)
        self.assertTrue(
            any(
                path.startswith("project-brain/")
                for path in self.search_paths(self.SOURCE)
            ),
            "a record citing the path should be findable by that path",
        )

    def test_a_dropped_chunk_drops_its_links(self) -> None:
        self.second.unlink()
        self.assertEqual(0, self.run_cli("index").returncode)
        self.assertEqual(
            {self.first.relative_to(self.repository).as_posix()},
            self.link_paths(self.SOURCE),
        )

    def test_an_index_built_before_the_table_is_rebuilt_not_left_partial(self) -> None:
        # `discover_documents` returns only *changed* documents, so a table
        # created beside a warm index would be populated for whatever happens
        # to change next and would silently claim to be complete.
        database = self.repository / "memory-bank/local/context.db"
        connection = sqlite3.connect(database)
        with connection:
            connection.execute("DROP TABLE document_links")
        connection.close()
        self.assertEqual(0, self.run_cli("index").returncode)
        self.assertEqual(self.chunk_paths(), self.link_paths(self.SOURCE))


class PathLinkedRetrievalTest(DocumentLinkTest):
    """`retrieve --path`: the link index used during retrieval, not beside it.

    Inherits the 0-of-2 fixture, so these tests speak about the same two
    chunks the link tests do — and the same query that cannot reach them.
    """

    def setUp(self) -> None:
        super().setUp()
        self.assertEqual(0, self.run_cli("start", "--task-id", "TASK-PATH",
                                         "--goal", "Apply the rounding policy.").returncode)
        self.assertEqual(0, self.run_cli("index").returncode)

    def capsule(self, *extra: str) -> dict:
        result = self.run_cli(
            "retrieve", self.SOURCE_WORDS, "--task-id", "TASK-PATH", "--json", *extra
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def manifest(self) -> dict:
        manifests = sorted(
            (self.repository / "project-brain/control/retrieval-manifests").glob("*.json"),
            key=lambda path: path.stat().st_mtime_ns,
        )
        self.assertTrue(manifests, "a governed retrieval writes a manifest")
        return json.loads(manifests[-1].read_text(encoding="utf-8"))

    def delivered(self, capsule: dict) -> set:
        return {
            item["path"]
            for layer in ("procedural", "semantic", "episodic")
            for item in capsule.get(layer) or []
        }

    def test_without_the_flag_the_chunks_stay_unreachable(self) -> None:
        # The baseline the flag is measured against.
        self.assertEqual(set(), self.chunk_paths() & self.delivered(self.capsule()))

    def test_the_flag_delivers_what_the_query_could_not_reach(self) -> None:
        self.assertEqual(
            self.chunk_paths(),
            self.chunk_paths() & self.delivered(self.capsule("--path", self.SOURCE)),
        )

    def test_a_path_linked_item_is_marked_as_such_in_the_manifest(self) -> None:
        self.capsule("--path", self.SOURCE)
        selected = {
            item["path"]: item for item in self.manifest()["selected"]
        }
        for path in self.chunk_paths():
            self.assertEqual("path-link", selected[path].get("selection"), path)
            # `match` is a lexical reading and nothing lexical selected these.
            self.assertNotIn("match", selected[path])
            self.assertIsNone(selected[path].get("rank"))

    def test_the_manifest_still_validates(self) -> None:
        self.capsule("--path", self.SOURCE)
        # Raises on a schema violation; `selection` is `additionalProperties:
        # false` territory, so an unregistered key would fail here.
        brain.validate_schema_file(
            self.repository, "retrieval-manifest", self.manifest()
        )

    def test_a_path_link_does_not_make_no_match_lie(self) -> None:
        # `no_match` is a claim about the QUERY. Computing it after injection
        # would silently flip it, and both the capsule line and the gate read
        # it as "your words found nothing in this layer".
        without = self.capsule()
        with_path = self.capsule("--path", self.SOURCE)
        self.assertEqual(
            without["gate"]["signals"]["no_match"],
            with_path["gate"]["signals"]["no_match"],
        )
        self.assertEqual(without["no_match"], with_path["no_match"])

    def test_a_path_linked_snippet_is_prose_not_frontmatter(self) -> None:
        # A chunk opens with a JSON metadata block long enough to fill the
        # whole snippet allowance; `substr(content, 1, N)` would ship that.
        capsule = self.capsule("--path", self.SOURCE)
        for item in capsule["semantic"]:
            if item["path"] in self.chunk_paths():
                self.assertFalse(
                    item["snippet"].lstrip().startswith(("---", "{")), item["snippet"][:80]
                )
                self.assertNotIn('"review_after"', item["snippet"])

    def test_an_uncited_path_changes_nothing(self) -> None:
        self.assertEqual(
            self.delivered(self.capsule()),
            self.delivered(self.capsule("--path", "app/Nothing.php")),
        )

    def test_filtered_path_links_are_not_reported_as_no_relevant_match(self) -> None:
        self.first.unlink()
        self.second.unlink()
        connection = context_cli.connect(context_cli.default_database(self.repository))
        try:
            binding = context_cli.governed_binding(connection, "TASK-PATH")
            capsule = retrieval.retrieve(
                connection,
                self.repository,
                "vermilion semaphore",
                binding["task_uuid"],
                limit=3,
                paths=[self.SOURCE],
                gate_mode="enforce",
            )
        finally:
            connection.close()

        self.assertEqual("skip", capsule["gate"]["decision"], capsule)
        self.assertEqual("empty-after-filter", capsule["gate"]["reason"], capsule)
        self.assertTrue(
            self.chunk_paths()
            <= {
                item["path"]
                for item in json.loads(
                    (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
                )["excluded"]
            },
            capsule,
        )


class ChunkSourceDigestTest(BankFixture):
    """A durable chunk that can notice its own citation moved on."""

    def digests_of(self, path: Path) -> list:
        metadata = json.loads(
            path.read_text(encoding="utf-8")[4:].split("\n---\n", 1)[0]
        )
        return metadata.get("source_digests") or []

    def test_reverify_records_a_digest_for_every_local_citation(self) -> None:
        result = self.run_cli(
            "bank-reverify", "--id", "MEM-20260101-aaaaaaaa", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        digests = self.digests_of(self.chunk)
        self.assertEqual(["specs/authority.md"], [d["path"] for d in digests])
        self.assertRegex(digests[0]["sha256"], r"^[0-9a-f]{64}$")
        self.assertEqual([], self.bank_errors())

    def test_a_url_source_is_cited_but_never_digested(self) -> None:
        # The runtime has no network, so the only honest thing it can say
        # about a remote citation is nothing.
        self.write_chunk(
            "MEM-20260101-aaaaaaaa", "old-rule", "The cerulean rollout is current.",
            sources=["specs/authority.md", "https://example.test/spec"],
        )
        self.assertEqual(
            0, self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa").returncode
        )
        self.assertEqual(
            ["specs/authority.md"], [d["path"] for d in self.digests_of(self.chunk)]
        )
        self.assertEqual([], self.bank_errors())

    def test_a_chunk_whose_source_changed_leaves_retrieval_with_that_reason(
        self,
    ) -> None:
        self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa")
        indexed = self.run_cli("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        self.start("TASK-DIGEST")
        found = self.run_cli(
            "retrieve", "cerulean rollout", "--task-id", "TASK-DIGEST",
            "--ephemeral", "--json",
        )
        chunk_path = "memory-bank/chunks/MEM-20260101-aaaaaaaa-old-rule.md"
        self.assertIn(
            chunk_path,
            [item["path"] for item in json.loads(found.stdout)["selected"]],
            found.stdout,
        )

        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nThe cobalt authority rule was replaced.\n",
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        after = self.run_cli(
            "retrieve", "cerulean rollout", "--task-id", "TASK-DIGEST",
            "--ephemeral", "--json",
        )
        capsule = json.loads(after.stdout)
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        self.assertNotIn(
            chunk_path, [item["path"] for item in manifest["selected"]], manifest
        )
        # Not `stale`: a Brain record is refreshed by a revisioned mutation, a
        # chunk by re-reading the source. The reason names which remedy applies.
        self.assertEqual(
            "source-changed",
            {item["path"]: item["reason"] for item in manifest["excluded"]}.get(
                chunk_path
            ),
            manifest["excluded"],
        )

    def test_re_verifying_returns_the_chunk_to_retrieval(self) -> None:
        # Without this the digests would be a one-way ratchet: the first time a
        # cited file changed, the chunk would leave retrieval and stay out.
        self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nThe cobalt authority rule was replaced.\n",
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        self.start("TASK-REVERIFY")
        chunk_path = "memory-bank/chunks/MEM-20260101-aaaaaaaa-old-rule.md"

        audit = json.loads(self.run_cli("bank-audit", "--json").stdout)
        self.assertEqual(
            [chunk_path], [item["chunk"] for item in audit["source_changed"]], audit
        )

        self.assertEqual(
            0, self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa").returncode
        )
        self.assertEqual(
            [], json.loads(self.run_cli("bank-audit", "--json").stdout)["source_changed"]
        )
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        capsule = json.loads(
            self.run_cli(
                "retrieve", "cerulean rollout", "--task-id", "TASK-REVERIFY",
                "--ephemeral", "--json",
            ).stdout
        )
        self.assertIn(
            chunk_path, [item["path"] for item in capsule["selected"]], capsule
        )

    def test_a_retired_chunk_is_not_audited_for_freshness(self) -> None:
        # A chunk that already left retrieval has nothing to attest to, and
        # reporting its dead citations would bury the live ones.
        self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa")
        self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa",
            "--valid-to", datetime.now(timezone.utc).date().isoformat(),
        )
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nThe cobalt authority rule was replaced.\n",
            encoding="utf-8",
        )
        audit = json.loads(self.run_cli("bank-audit", "--json").stdout)
        self.assertEqual([], audit["source_changed"], audit)
        # And it cannot be re-attested either: there is nothing left to attest
        # to, and re-verifying would quietly resurrect it.
        refused = self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa")
        self.assertNotEqual(0, refused.returncode)
        self.assertIn("Only an active chunk", refused.stderr)

    def test_audit_reports_a_chunk_overdue_for_review(self) -> None:
        yesterday = (datetime.now(timezone.utc).date() - timedelta(days=1)).isoformat()
        self.write_chunk(
            "MEM-20260101-aaaaaaaa", "old-rule", "The cerulean rollout is current.",
            review_after=yesterday,
        )
        audit = json.loads(self.run_cli("bank-audit", "--json").stdout)
        self.assertEqual(
            [yesterday], [item["review_after"] for item in audit["overdue_review"]]
        )

    def test_a_chunk_without_digests_stays_retrievable(self) -> None:
        # Optional means optional: every chunk written before this existed
        # must keep working exactly as it did.
        self.assertEqual([], self.digests_of(self.chunk))
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        self.start("TASK-UNDIGESTED")
        capsule = json.loads(
            self.run_cli(
                "retrieve", "cerulean rollout", "--task-id", "TASK-UNDIGESTED",
                "--ephemeral", "--json",
            ).stdout
        )
        self.assertIn(
            "memory-bank/chunks/MEM-20260101-aaaaaaaa-old-rule.md",
            [item["path"] for item in capsule["selected"]],
            capsule,
        )


class AutomaticWorkingMemoryTest(RuntimeHarness):
    """Cover the automated read and write paths the memory hooks depend on."""

    def test_changed_paths_are_edition_relative_and_collapse_untracked_trees(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory(prefix="nested-edition-test-") as temporary:
            git_root = Path(temporary)
            edition = git_root / "Symfony"
            (edition / "Task").mkdir(parents=True)
            (git_root / "sibling").mkdir()
            (edition / "tracked.txt").write_text("base\n", encoding="utf-8")
            (edition / "Task/README.md").write_text("# Task\n", encoding="utf-8")
            subprocess.run(
                ["git", "init", "--quiet", str(git_root)], check=True,
                capture_output=True,
            )
            subprocess.run(
                ["git", "-C", str(git_root), "add", "Symfony/tracked.txt",
                 "Symfony/Task/README.md"],
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["git", "-C", str(git_root), "-c", "user.name=Test",
                 "-c", "user.email=test@example.test", "commit", "-qm", "base"],
                check=True,
                capture_output=True,
            )

            (edition / "tracked.txt").write_text("changed\n", encoding="utf-8")
            (edition / "new.txt").write_text("new\n", encoding="utf-8")
            (edition / ".env.local").write_text("TOKEN=private\n", encoding="utf-8")
            for index in range(300):
                generated = edition / "Task/app/vendor/cache" / str(index) / "item.php"
                generated.parent.mkdir(parents=True, exist_ok=True)
                generated.write_text("generated\n", encoding="utf-8")
            (git_root / "sibling/outside.txt").write_text(
                "outside\n", encoding="utf-8"
            )

            paths, excluded = context_cli.changed_paths(edition)

        self.assertEqual([".env.local"], excluded)
        self.assertEqual(
            ["Task/app/", "new.txt", "tracked.txt"], sorted(paths)
        )
        self.assertLessEqual(len(paths) + len(excluded), 4)
        self.assertNotIn("sibling/outside.txt", paths)

    def test_hook_context_warms_four_turns_then_switches_to_governed(self) -> None:
        self.repository.joinpath("safe.php").write_text(
            "<?php // work\n", encoding="utf-8"
        )
        self.repository.joinpath(".env.local").write_text(
            "SECRET=never-render\n", encoding="utf-8"
        )
        for turn in range(1, 5):
            buffered = self.run_cli(
                "turn", "--task-id", "feature/warming",
                "--flush-after", "5", "--json",
            )
            self.assertEqual(0, buffered.returncode, buffered.stderr)
            self.assertFalse(json.loads(buffered.stdout)["flushed"])
            warming = self.run_cli(
                "hook-context", "--task-id", "feature/warming", "--json"
            )
            self.assertEqual(0, warming.returncode, warming.stderr)
            capsule = json.loads(warming.stdout)
            self.assertEqual("warming", capsule["kind"])
            self.assertEqual(turn, capsule["pending_turns"])
            self.assertEqual(1, capsule["excluded_files"])
            self.assertNotIn("safe.php", warming.stdout)
            self.assertNotIn(".env.local", warming.stdout)
            self.assertNotIn("never-render", warming.stdout)

        fifth = self.run_cli(
            "turn", "--task-id", "feature/warming",
            "--flush-after", "5", "--json",
        )
        self.assertEqual(0, fifth.returncode, fifth.stderr)
        self.assertTrue(json.loads(fifth.stdout)["flushed"])
        governed = self.run_cli(
            "hook-context", "--task-id", "feature/warming", "--json"
        )
        self.assertEqual(0, governed.returncode, governed.stderr)
        capsule = json.loads(governed.stdout)
        self.assertNotEqual("warming", capsule.get("kind"))
        self.assertTrue(brain.is_uuid4(capsule["task_uuid"]))

    def test_hook_context_returns_valid_empty_for_clean_unbound_branch(self) -> None:
        self.repository.joinpath(".gitignore").write_text(
            "memory-bank/local/\nproject-brain/local/\n",
            encoding="utf-8",
        )
        self.git("add", "-A")
        self.git("commit", "-qm", "clean baseline")
        result = self.run_cli(
            "hook-context", "--task-id", "feature/clean", "--json"
        )
        self.assertEqual(3, result.returncode, result.stderr)
        self.assertEqual("", result.stdout)

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

    def test_a_fresh_record_outranks_a_stale_one_at_equal_lexical_fit(self) -> None:
        # BM25 alone cannot tell these records apart: identical title, goal,
        # and body. Recency decay over updated_at must break the tie in favor
        # of the record that was touched last.
        self.start("TASK-RANK-FRESH")
        with mock.patch.object(
            brain, "utc_now", return_value="2026-01-01T00:00:00+00:00"
        ):
            stale = brain.create_record(
                self.repository, "finding", "RANK-OLD1",
                "Amaranth ranking subject", [], [],
                owner="alice", authority="verified",
            )
        fresh = brain.create_record(
            self.repository, "finding", "RANK-NEW1",
            "Amaranth ranking subject", [], [],
            owner="alice", authority="verified",
        )
        self.run_cli("refresh")

        selected, _ = self.selected_paths(
            "amaranth ranking subject", "TASK-RANK-FRESH"
        )
        fresh_path = f"project-brain/dynamic/findings/{fresh['id']}.md"
        stale_path = f"project-brain/dynamic/findings/{stale['id']}.md"
        self.assertIn(fresh_path, selected)
        self.assertIn(stale_path, selected)
        self.assertLess(selected.index(fresh_path), selected.index(stale_path))

    def test_a_verified_record_outranks_an_observed_one_at_equal_lexical_fit(
        self,
    ) -> None:
        self.start("TASK-RANK-AUTH")
        observed = brain.create_record(
            self.repository, "finding", "RANK-OBS1",
            "Byzantium ranking subject", [], [],
            owner="alice", authority="observed",
        )
        verified = brain.create_record(
            self.repository, "finding", "RANK-VER1",
            "Byzantium ranking subject", [], [],
            owner="alice", authority="verified",
        )
        self.run_cli("refresh")

        selected, _ = self.selected_paths(
            "byzantium ranking subject", "TASK-RANK-AUTH"
        )
        verified_path = f"project-brain/dynamic/findings/{verified['id']}.md"
        observed_path = f"project-brain/dynamic/findings/{observed['id']}.md"
        self.assertIn(verified_path, selected)
        self.assertIn(observed_path, selected)
        self.assertLess(
            selected.index(verified_path), selected.index(observed_path)
        )

    def test_refresh_distills_a_raw_prompt_so_the_tail_subject_survives(
        self,
    ) -> None:
        # The hook passes the prompt as-is; the CLI must not decide relevance
        # by position the way the old first-24-words extraction did.
        self.start("TASK-DISTILL")
        self.repository.joinpath("specs/currency.md").write_text(
            "# Currency\n\nAmounts are stored in integer zorkmids.\n",
            encoding="utf-8",
        )
        preamble = " ".join(f"zzfiller{index}" for index in range(30))
        result = self.run_cli(
            "refresh", "--query",
            f"{preamble} how does this project represent zorkmids",
            "--task-id", "TASK-DISTILL", "--ephemeral", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(result.stdout)
        self.assertIsNotNone(report["capsule"], report["warnings"])
        query = report["capsule"]["query"]
        self.assertIn("zorkmids", query)
        self.assertNotIn("zzfiller0", query)
        self.assertIn(
            "specs/currency.md",
            [item["path"] for item in report["capsule"]["selected"]],
        )
        for phase in ("stat", "index", "retrieval"):
            self.assertIn(phase, report["phases"])

    def test_overlapping_source_patterns_index_a_file_once(self) -> None:
        self.repository.joinpath("docs").mkdir()
        self.repository.joinpath("docs/overview.md").write_text(
            "# Overview\n\nCobalt everywhere.\n", encoding="utf-8"
        )
        overlapping = context_cli.SOURCE_PATTERNS + (
            ("semantic", "codebase", "docs/*.md"),
        )
        with mock.patch.object(context_cli, "SOURCE_PATTERNS", overlapping):
            documents, _, state, _ = context_cli.discover_documents(self.repository)

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

    def test_a_codebase_map_split_across_modules_is_indexed(self) -> None:
        # `codebase/*.md` matched one level, so a map filed into per-module
        # directories — the shape a map big enough to be worth writing takes —
        # was indexed at the top and nowhere else.
        self.start("TASK-MODULES")
        self.commit_main()
        commit = self.head()
        self.write_codebase_map(commit)
        module = self.repository / "codebase/modules"
        module.mkdir(parents=True, exist_ok=True)
        module.joinpath("billing.md").write_text(
            "---\n"
            "description: Where invoice rounding lives.\n"
            f"mapped_commit: {commit}\n"
            "mapped_scope: src\n"
            "---\n\n"
            "# Billing\n\n- `src/Cobalt.php` — the cobalt rounding guard.\n",
            encoding="utf-8",
        )
        self.run_cli("refresh")

        selected, _ = self.selected_paths("cobalt rounding guard", "TASK-MODULES")
        self.assertIn("codebase/modules/billing.md", selected)

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

    def test_canonical_and_alias_phases_round_trip_canonically(self) -> None:
        cases = (
            ("understanding", "understanding"),
            ("planning", "planning"),
            ("implementation", "implementation"),
            ("verification", "verification"),
            ("finalization", "finalization"),
            ("implementing", "implementation"),
            ("execution", "implementation"),
            ("review", "verification"),
        )
        for index, (supplied, canonical) in enumerate(cases):
            task_id = f"TASK-PHASE-{index}"
            self.start(task_id)
            result = self.run_cli(
                "update", "--task-id", task_id, "--revision", "auto",
                "--phase", supplied, "--progress", "Arithmetic done.", "--json",
            )
            self.assertEqual(0, result.returncode, result.stderr)
            task = json.loads(result.stdout)
            self.assertEqual(canonical, task["phase"])
            record_path = (
                self.repository
                / "project-brain/dynamic/tasks"
                / f"{task['task_uuid']}.md"
            )
            self.assertEqual(
                canonical,
                json.loads(record_path.read_text(encoding="utf-8").split("---")[1])[
                    "phase"
                ],
            )
            handoff = (
                self.repository
                / "project-brain/control/handoffs"
                / f"{task['task_uuid']}.md"
            ).read_text(encoding="utf-8")
            self.assertIn(f"## Phase\n{canonical}", handoff)
            if supplied != canonical:
                self.assertNotIn(f"## Phase\n{supplied}", handoff)

            packet = self.run_cli(
                "retrieve", "cobalt authority", "--task-id", task_id,
                "--ephemeral", "--json",
            )
            self.assertEqual(0, packet.returncode, packet.stderr)
            self.assertEqual(canonical, json.loads(packet.stdout)["working"]["phase"])

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

    def test_legacy_execution_phase_is_readable_and_migrates_on_update(self) -> None:
        task = self.start("TASK-LEGACY-EXECUTION")
        phased = self.run_cli(
            "update", "--task-id", "TASK-LEGACY-EXECUTION", "--revision", "auto",
            "--phase", "implementation", "--progress", "Started.", "--json",
        )
        self.assertEqual(0, phased.returncode, phased.stderr)
        task_uuid = json.loads(phased.stdout)["task_uuid"]
        paths = (
            self.repository / "project-brain/dynamic/tasks" / f"{task_uuid}.md",
            brain.handoff_path(self.repository, task_uuid),
        )
        for path in paths:
            metadata, body = brain.parse_markdown_record(path)
            metadata["phase"] = "execution"
            path.write_text(
                brain.render_markdown_record(metadata, body),
                encoding="utf-8",
            )

        self.assertEqual(0, self.run_cli("validate").returncode)
        loaded = json.loads(
            self.run_cli(
                "get", "--task-id", "TASK-LEGACY-EXECUTION", "--json"
            ).stdout
        )
        self.assertEqual("implementation", loaded["phase"])

        updated = self.run_cli(
            "update", "--task-id", "TASK-LEGACY-EXECUTION", "--revision", "auto",
            "--progress", "Continued safely.", "--json",
        )
        self.assertEqual(0, updated.returncode, updated.stderr)
        for path in paths:
            raw = json.loads(path.read_text(encoding="utf-8").split("---")[1])
            self.assertEqual("implementation", raw["phase"])

    def test_only_a_task_may_declare_a_phase(self) -> None:
        record = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", "FIND-PHASE",
                "--title", "Not a task", "--source", "specs/authority.md", "--json",
            ).stdout
        )
        result = self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--phase", "implementation",
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
        # The consolidated summary is telemetry, so it lands in its own field
        # and leaves the operator's progress untouched.
        self.assertIn("Auto-checkpoint: 3 turn(s)", task["auto_checkpoint"])
        self.assertEqual("", task["progress"])
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

    def test_turn_reports_merge_candidate_without_completing_task(self) -> None:
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
        self.assertEqual([], json.loads(result.stdout)["completion_candidates"])

        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "feature/reports", "-m", "merge")
        self.repository.joinpath("after.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")

        candidates = json.loads(result.stdout)["completion_candidates"]
        self.assertEqual(["feature/reports"], [item["task_id"] for item in candidates])

        record = json.loads(
            next(
                path
                for path in (self.repository / "project-brain/dynamic/tasks").glob("*.md")
                if "feature/reports" in path.read_text(encoding="utf-8")
            ).read_text(encoding="utf-8").split("---")[1]
        )
        self.assertEqual("active", record["status"])
        self.assertEqual(record["revision"], candidates[0]["revision"])
        self.assertNotIn("merged into main", record["progress"])

    def test_completion_candidate_ignores_a_branch_that_never_diverged(self) -> None:
        # A branch created a moment ago is already an ancestor of its target,
        # so ancestry alone would complete the task the instant it existed.
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/fresh")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = self.run_cli("turn", "--task-id", "feature/fresh", "--flush", "--json")
        self.assertEqual([], json.loads(result.stdout)["completion_candidates"])
        self.assertEqual(
            "active",
            json.loads(
                self.run_cli("get", "--task-id", "feature/fresh", "--json").stdout
            )["status"],
        )

    def test_completion_candidate_ignores_a_branch_kept_current_with_target(
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
        self.assertEqual([], json.loads(result.stdout)["completion_candidates"])

    def test_completion_candidate_ignores_the_default_branch_task(self) -> None:
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        # A branch is its own ancestor, so this would close itself immediately.
        self.run_cli("turn", "--task-id", "main", "--flush", "--json")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")

        self.assertEqual([], json.loads(result.stdout)["completion_candidates"])
        self.assertEqual(
            "active",
            json.loads(self.run_cli("get", "--task-id", "main", "--json").stdout)[
                "status"
            ],
        )

    def test_completion_candidate_ignores_a_deleted_branch(self) -> None:
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
        self.assertEqual([], json.loads(result.stdout)["completion_candidates"])

    def test_completion_candidate_is_advisory_even_when_automation_is_off(self) -> None:
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/quiet")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/quiet", "--flush", "--json")
        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "feature/quiet", "-m", "merge")

        self.repository.joinpath("other.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")
        self.assertEqual([], json.loads(result.stdout)["completion_candidates"])

    def test_maintenance_waits_for_the_flush_boundary(self) -> None:
        # The Stop hook drives `turn` under a hard timeout on every turn, so
        # the expensive scans may run only where the flush already writes; an
        # ordinary turn stays at one status probe plus a buffer insert.
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        promotion = {
            "enabled": False, "promoted": [], "failed": [],
            "blocked": [], "skipped": 0,
        }
        compaction = {"enabled": False, "moved": 0, "pending": 0, "error": None}
        with mock.patch.object(
            context_cli, "merge_completion_candidates", return_value=[]
        ) as candidates, mock.patch.object(
            context_cli, "auto_promote", return_value=promotion
        ) as promote, mock.patch.object(
            context_cli, "auto_compact", return_value=compaction
        ) as compact:
            code, stdout, stderr = self.run_main(
                "turn", "--task-id", "feature/gated", "--flush-after", "3",
                "--json",
            )
            self.assertEqual(0, code, stderr)
            self.assertFalse(json.loads(stdout)["flushed"])
            candidates.assert_not_called()
            promote.assert_not_called()
            compact.assert_not_called()

            code, stdout, stderr = self.run_main(
                "turn", "--task-id", "feature/gated", "--flush", "--json"
            )
            self.assertEqual(0, code, stderr)
            self.assertTrue(json.loads(stdout)["flushed"])
            candidates.assert_called_once()
            promote.assert_called_once()
            compact.assert_called_once()

    def test_a_buffered_turn_defers_completion_candidate_scan_until_flush(self) -> None:
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/deferred")
        self.repository.joinpath("work.txt").write_text("work\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-qm", "work")
        self.repository.joinpath("pending.txt").write_text("x\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/deferred", "--flush", "--json")
        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "feature/deferred", "-m", "merge")

        self.repository.joinpath("after.txt").write_text("more\n", encoding="utf-8")
        buffered = self.run_cli(
            "turn", "--task-id", "main", "--flush-after", "5", "--json"
        )
        payload = json.loads(buffered.stdout)
        self.assertFalse(payload["flushed"])
        # The merge is already observable, but a buffered turn does not pay
        # for the scan; the task stays open until the boundary.
        self.assertEqual([], payload["completion_candidates"])
        self.assertEqual(
            "active",
            json.loads(
                self.run_cli("get", "--task-id", "feature/deferred", "--json").stdout
            )["status"],
        )

        flushed = self.run_cli("turn", "--task-id", "main", "--flush", "--json")
        payload = json.loads(flushed.stdout)
        self.assertTrue(payload["flushed"])
        self.assertEqual(
            ["feature/deferred"], [item["task_id"] for item in payload["completion_candidates"]]
        )

    def test_batched_merge_scan_reports_only_the_merged_branch(self) -> None:
        # One reference listing now serves every task; the per-branch verdicts
        # must not change: merged is reported, unmerged stays unreported.
        self.enable_automation(automatic_completion=True)
        self.commit_main()
        for branch in ("feature/one", "feature/two"):
            self.git("checkout", "-q", "-b", branch, "main")
            name = branch.split("/", 1)[1]
            self.repository.joinpath(f"{name}.txt").write_text(
                "work\n", encoding="utf-8"
            )
            # Stage only the work file: `add -A` would also track the runtime
            # database a later turn keeps writing, blocking the checkout back.
            self.git("add", f"{name}.txt")
            self.git("commit", "-qm", branch)
            self.repository.joinpath(f"pending-{name}.txt").write_text(
                "x\n", encoding="utf-8"
            )
            self.run_cli("turn", "--task-id", branch, "--flush", "--json")
        self.git("checkout", "-q", "main")
        self.git("merge", "-q", "--no-ff", "feature/one", "-m", "merge one")

        self.repository.joinpath("after.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")
        candidates = json.loads(result.stdout)["completion_candidates"]
        self.assertEqual(["feature/one"], [item["task_id"] for item in candidates])
        self.assertEqual(
            "active",
            json.loads(
                self.run_cli("get", "--task-id", "feature/two", "--json").stdout
            )["status"],
        )

    def test_default_branch_cache_survives_calls_and_resets_on_rename(self) -> None:
        self.commit_main()
        connection = context_cli.connect(
            context_cli.default_database(self.repository)
        )
        try:
            self.assertEqual(
                "main",
                context_cli.cached_default_branch(connection, self.repository),
            )
            self.assertEqual(
                "main",
                retrieval.load_index_state(connection)[
                    context_cli.DEFAULT_BRANCH_STATE_KEY
                ],
            )
            # A cached target is reused without paying for detection again.
            with mock.patch.object(
                context_cli, "default_branch",
                side_effect=AssertionError("re-detected a cached branch"),
            ):
                for _ in range(2):
                    self.assertEqual(
                        "main",
                        context_cli.cached_default_branch(
                            connection, self.repository
                        ),
                    )
            # A renamed branch fails the probe and is re-detected and stored.
            self.git("branch", "-M", "master")
            self.assertEqual(
                "master",
                context_cli.cached_default_branch(connection, self.repository),
            )
            self.assertEqual(
                "master",
                retrieval.load_index_state(connection)[
                    context_cli.DEFAULT_BRANCH_STATE_KEY
                ],
            )
            # A name detection cannot find drops the stale entry outright.
            self.git("branch", "-M", "trunk")
            self.assertIsNone(
                context_cli.cached_default_branch(connection, self.repository)
            )
            self.assertNotIn(
                context_cli.DEFAULT_BRANCH_STATE_KEY,
                retrieval.load_index_state(connection),
            )
        finally:
            connection.close()

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

    def test_verified_finding_completes_the_real_memory_journey(self) -> None:
        self.enable_automatic_promotion()
        self.start("TASK-MEMORY-E2E")
        created = self.run_cli(
            "brain-create", "finding",
            "--external-id", "TASK-MEMORY-E2E-F1",
            "--title", "Quartz falcon retry boundary",
            "--source", "specs/authority.md",
            "--json",
        )
        self.assertEqual(0, created.returncode, created.stderr)
        finding = json.loads(created.stdout)

        verified = self.run_cli(
            "brain-update", "--record-id", finding["id"],
            "--revision", "auto", "--authority", "verified",
            "--reason", "Verified: authority source re-read", "--json",
        )
        self.assertEqual(0, verified.returncode, verified.stderr)
        authority_transition = json.loads(verified.stdout)["transitions"][-1]
        self.assertEqual(
            {
                "from": "observed",
                "to": "verified",
                "actor": "local",
                "reason": "Verified: authority source re-read",
            },
            {
                key: authority_transition[key]
                for key in ("from", "to", "actor", "reason")
            },
        )

        resolved = self.run_cli(
            "brain-update", "--record-id", finding["id"],
            "--revision", "auto",
            "--progress",
            "Quartz falcon retries stop after the third guarded dispatch.",
            "--transition", "resolved",
            "--reason", "Resolved after verification", "--json",
        )
        self.assertEqual(0, resolved.returncode, resolved.stderr)
        self.assertEqual("resolved", json.loads(resolved.stdout)["status"])

        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        flushed = self.run_cli(
            "turn", "--task-id", "TASK-MEMORY-E2E", "--flush", "--json"
        )
        self.assertEqual(0, flushed.returncode, flushed.stderr)
        promoted = json.loads(flushed.stdout)["promoted"]
        self.assertEqual(1, len(promoted), promoted)
        self.assertEqual(finding["id"], promoted[0]["record_id"])
        memory_id = promoted[0]["memory_id"]
        chunk = next(
            (self.repository / "memory-bank/chunks").glob(f"{memory_id}-*.md")
        )
        metadata, _ = brain.parse_markdown_record(chunk)
        finding_path = (
            f"project-brain/dynamic/findings/{finding['id']}.md"
        )
        self.assertEqual("domain", metadata["type"])
        self.assertEqual("active", metadata["status"])
        self.assertIn("auto-promoted", metadata["tags"])
        self.assertEqual(
            [finding_path, "specs/authority.md"], metadata["sources"]
        )
        self.assertEqual(
            [finding_path, "specs/authority.md"],
            [item["path"] for item in metadata["source_digests"]],
        )

        current = json.loads(
            self.run_cli(
                "get", "--task-id", "TASK-MEMORY-E2E", "--json"
            ).stdout
        )
        completed = self.run_cli(
            "complete", "--task-id", "TASK-MEMORY-E2E",
            "--revision", str(current["revision"]),
            "--outcome", "Memory journey verified.",
            "--verification", "temporary end-to-end regression passed",
            "--source", "specs/authority.md", "--json",
        )
        self.assertEqual(0, completed.returncode, completed.stderr)
        completion = json.loads(completed.stdout)
        self.assertEqual("completed", completion["status"])
        self.assertIsNotNone(completion["episode_id"])

        status_result = self.run_cli("status", "--json")
        self.assertEqual(0, status_result.returncode, status_result.stderr)
        status = json.loads(status_result.stdout)
        self.assertEqual(1, status["episodes"], status)
        self.assertEqual(0, status["working"], status)
        self.assertEqual(1, status["consolidation"]["applied"], status)
        self.assertEqual(1, status["consolidation"]["chunks"], status)

        promotion = json.loads(
            next(
                (self.repository / "project-brain/control/promotions").glob("*.json")
            ).read_text(encoding="utf-8")
        )
        self.assertEqual("applied", promotion["status"])
        self.assertEqual("automatic", promotion["review_mode"])
        self.assertIsNone(promotion["reviewer"])
        self.assertEqual("approved-without-review", promotion["outcome"])
        self.assertEqual(memory_id, promotion["destination_memory_id"])

        audit = self.run_cli("bank-audit", "--json")
        self.assertEqual(0, audit.returncode, audit.stderr)
        self.assertEqual(
            {
                "chunks": 1,
                "source_changed": [],
                "overdue_review": [],
                "undigested": [],
                "duplicates": [],
            },
            json.loads(audit.stdout),
        )

        self.start("TASK-MEMORY-READER")
        retrieved = self.run_cli(
            "retrieve", "quartz falcon third guarded dispatch",
            "--task-id", "TASK-MEMORY-READER",
            "--gate", "off", "--ephemeral", "--json",
        )
        self.assertEqual(0, retrieved.returncode, retrieved.stderr)
        selected = [item["path"] for item in json.loads(retrieved.stdout)["selected"]]
        self.assertIn(chunk.relative_to(self.repository).as_posix(), selected)

    def test_a_promoted_chunk_declares_what_it_was_promoted_from(self) -> None:
        # Every promotion wrote `type: decision` whatever it promoted, so a
        # resolved bug and a closed incident both entered the bank claiming to
        # be decisions. INDEX.md, bank-audit and every reader keyed on type
        # were reading a value nothing had chosen.
        self.enable_automatic_promotion()
        # Distinct subjects, not distinct identifiers: the near-duplicate guard
        # refuses a promotion whose text merely repeats an existing chunk.
        for external_id, record_type, ladder, expected, subject, consequence in (
            ("BUG-1", "bug", ("triaged", "fixing", "verifying", "resolved"), "constraint",
             "Pagination offsets skipped the final row",
             "Every listing endpoint must use keyset pagination."),
            ("INC-1", "incident", ("contained", "resolved", "closed"), "operations",
             "Queue workers stalled behind a poisoned message",
             "Dead-letter routing is mandatory for every consumer."),
            ("FND-1", "finding", ("resolved",), "domain",
             "Invoice currency was inferred from the browser locale",
             "Currency belongs to the customer account, never the request."),
            ("DEC-9", "decision", ("accepted",), "decision",
             "Read models are rebuilt nightly rather than on write",
             "Nightly rebuild trades staleness for predictable write latency."),
        ):
            with self.subTest(record_type=record_type):
                record = json.loads(
                    self.run_cli(
                        "brain-create", record_type, "--external-id", external_id,
                        "--title", subject,
                        "--source", "specs/authority.md",
                        "--authority", "verified", "--json",
                    ).stdout
                )
                # Each type reaches its promotable state by its own ladder;
                # a bug cannot jump from reported straight to resolved.
                for step in ladder:
                    result = self.run_cli(
                        "brain-update", "--record-id", record["id"],
                        "--revision", "auto", "--progress", consequence,
                        "--transition", step, "--reason", "Done", "--json",
                    )
                    self.assertEqual(0, result.returncode, result.stderr)
                self.repository.joinpath("app.txt").write_text(
                    f"work {external_id}\n", encoding="utf-8"
                )
                self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")
                chunk = next(
                    path
                    for path in (self.repository / "memory-bank/chunks").glob("MEM-*.md")
                    if subject in path.read_text(encoding="utf-8")
                )
                metadata = json.loads(chunk.read_text(encoding="utf-8").split("---")[1])
                self.assertEqual(expected, metadata["type"])

    def test_a_promoted_chunk_inherits_what_its_record_cited(self) -> None:
        """The citation chain must survive promotion.

        Found on a real installation, not in a fixture: two findings about one
        design document promoted into two chunks that shared no source at all,
        because `apply_promotion` recorded only the records. `links` and
        `retrieve --path` are one hop by design, so the document reached the
        records and never the knowledge derived from them.
        """
        self.enable_automatic_promotion()
        self.repository.joinpath("specs/rounding.md").write_text(
            "# Rounding\n\nHalf-to-even tie breaking.\n", encoding="utf-8"
        )
        for external_id, title, consequence in (
            ("DEC-A", "Totals are stored in minor units",
             "Every total is persisted as an integer count of minor units."),
            ("DEC-B", "Refunds reconcile against the stored total",
             "A refund is validated against the total already persisted."),
        ):
            record = json.loads(
                self.run_cli(
                    "brain-create", "decision", "--external-id", external_id,
                    "--title", title, "--source", "specs/rounding.md",
                    "--authority", "verified", "--json",
                ).stdout
            )
            self.run_cli(
                "brain-update", "--record-id", record["id"], "--revision", "auto",
                "--progress", consequence, "--transition", "accepted",
                "--reason", "Accepted", "--json",
            )
            self.repository.joinpath("app.txt").write_text(
                consequence, encoding="utf-8"
            )
            self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")

        chunks = {
            path.relative_to(self.repository).as_posix()
            for path in (self.repository / "memory-bank/chunks").glob("MEM-*.md")
        }
        self.assertEqual(2, len(chunks), chunks)
        self.assertEqual(0, self.run_cli("index").returncode)
        linked = {
            item["path"]
            for item in json.loads(
                self.run_cli("links", "--path", "specs/rounding.md", "--json").stdout
            )["documents"]
        }
        self.assertEqual(chunks, chunks & linked)

    def test_a_promoted_chunk_never_inherits_a_records_files(self) -> None:
        # `files` is Git churn in the Git-toplevel frame. Merging it would put
        # code paths under `chunk_source_digests`, where the next edit evicts
        # the chunk as `source-changed`.
        self.enable_automatic_promotion()
        record = json.loads(
            self.run_cli(
                "brain-create", "decision", "--external-id", "DEC-FILES",
                "--title", "Read models rebuild nightly",
                "--source", "specs/authority.md", "--authority", "verified",
                "--json",
            ).stdout
        )
        self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--progress", "Nightly rebuild trades staleness for write latency.",
            "--file", "src/ReadModel.php", "--transition", "accepted",
            "--reason", "Accepted", "--json",
        )
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/auto", "--flush", "--json")
        chunk = next((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        metadata = json.loads(chunk.read_text(encoding="utf-8").split("---")[1])
        self.assertNotIn("src/ReadModel.php", metadata["sources"])

    def test_a_citation_deleted_after_review_blocks_apply(self) -> None:
        self.repository.joinpath("specs/doomed.md").write_text(
            "# Doomed\n\nAbout to be deleted.\n", encoding="utf-8"
        )
        record = brain.create_record(
            self.repository, "decision", "DEC-GONE", "Queues drain before deploy",
            [], ["specs/doomed.md"], owner="alice", authority="verified",
        )
        brain.update_record(
            self.repository, record["id"], expected_revision=record["revision"],
            progress="A deploy waits for the queue to drain first.",
            next_steps=[], files=[], sources=[], actor="alice",
            transition_to="accepted", reason="Accepted",
        )
        proposal = brain.create_promotion(
            self.repository, [record["id"]], "Queues drain before deploy",
            "Reviewed consequence.", proposer="alice",
        )
        brain.review_promotion(
            self.repository, proposal["id"], "human", approve=True
        )
        self.repository.joinpath("specs/doomed.md").unlink()

        with self.assertRaisesRegex(brain.BrainError, "cited source changed"):
            brain.apply_promotion(self.repository, proposal["id"])
        self.assertEqual(
            [], list((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        )

    def test_a_source_tree_lost_entirely_to_gitignore_is_reported(self) -> None:
        """The silent case, found on a real installation.

        A project whose own .gitignore carried a bare `docs` entry indexed 96
        accelerator skills, one README and none of its own design documents,
        and nothing said so. Individual ignored files stay silent on purpose —
        Symfony alone contributes hundreds — but a glob that matched files on
        disk and lost every one of them means a documented source of truth is
        invisible.
        """
        docs = self.repository / "docs"
        docs.mkdir(exist_ok=True)
        docs.joinpath("architecture.md").write_text(
            "# Architecture\n\nThe cobalt boundary.\n", encoding="utf-8"
        )
        result = self.run_cli("index", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertNotIn(
            "docs/**/*.md",
            {item["path"] for item in json.loads(result.stdout)["excluded"]},
            "a visible tree must not be reported",
        )

        self.repository.joinpath(".gitignore").write_text("docs\n", encoding="utf-8")
        result = self.run_cli("index", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        excluded = {
            item["path"]: item["reason"]
            for item in json.loads(result.stdout)["excluded"]
        }
        self.assertEqual("pattern-all-git-ignored", excluded.get("docs/**/*.md"))
        # And only the tree that actually went dark.
        self.assertNotIn("specs/**/*.md", excluded)

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
            automatic_completion=False, automatic_compaction=True,
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

        task = json.loads(
            self.run_cli(
                "get", "--task-id", "feature/archived", "--json"
            ).stdout
        )
        completed = self.run_cli(
            "complete",
            "--task-id", "feature/archived",
            "--revision", str(task["revision"]),
            "--outcome", "Verified and explicitly completed after merge.",
            "--verification", "Focused tests passed",
            "--json",
        )
        self.assertEqual(0, completed.returncode, completed.stderr)
        self.repository.joinpath("after.txt").write_text("more\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "main", "--flush", "--json")

        payload = json.loads(result.stdout)
        self.assertEqual([], payload["completion_candidates"])
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
        self.assertIn("beyond the per-flush limit", task["auto_checkpoint"])


class LastTurnReportTest(RuntimeHarness):
    """The silenced Stop-hook turn reports through the next request's capsule.

    The Stop hook discards `turn` stdout, so a blocked promotion, an excluded
    path, or a failed compaction printed there reached nobody. The turn now
    also writes `memory-bank/local/last-turn-report.json`, and the next
    refresh folds a fresh report into the Task Capsule as a 'Last turn'
    section. The file is advisory: stale, missing, or malformed content must
    never change what refresh returns beyond omitting the section.
    """

    def enable_automatic_promotion(self) -> None:
        config = self.repository / "project-brain/config/runtime.json"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(
            json.dumps({"mode": "governed", "automatic_promotion": True}),
            encoding="utf-8",
        )

    def resolve_observed_finding(self, external_id: str = "FIND-HELD") -> None:
        """A terminal `observed` record: promotable type, blocked by authority."""
        record = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", external_id,
                "--title", "Observed but never verified",
                "--source", "specs/authority.md",
                "--authority", "observed", "--json",
            ).stdout
        )
        self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--transition", "resolved", "--reason", "Resolved", "--json",
        )

    def report_path(self) -> Path:
        return self.repository / "memory-bank/local/last-turn-report.json"

    def flush_turn(self, task_id: str = "feature/held") -> dict:
        result = self.run_cli("turn", "--task-id", task_id, "--flush", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def refresh_capsule(
        self, task_id: str = "feature/held"
    ) -> tuple[dict, str]:
        """One refresh in both shapes: the JSON capsule and the hook's text."""
        text = self.run_cli(
            "refresh", "--query", "cobalt authority", "--task-id", task_id,
            "--ephemeral",
        )
        self.assertEqual(0, text.returncode, text.stderr)
        result = self.run_cli(
            "refresh", "--query", "cobalt authority", "--task-id", task_id,
            "--ephemeral", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)["capsule"], text.stdout

    def test_blocked_promotion_reaches_the_next_refresh_capsule(self) -> None:
        self.enable_automatic_promotion()
        self.resolve_observed_finding()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.flush_turn()

        report = json.loads(self.report_path().read_text(encoding="utf-8"))
        self.assertEqual(
            ["authority is observed, not verified"],
            [item["reason"] for item in report["promotion_blocked"]],
        )

        capsule, text = self.refresh_capsule()
        # The blocked reason is the point of the section: it must be quoted.
        self.assertIn(
            "promotion blocked: authority is observed, not verified",
            capsule["last_turn"],
        )
        self.assertIn("Last turn: promotion blocked:", text)

    def test_excluded_paths_and_flush_reach_the_section(self) -> None:
        self.repository.joinpath(".env.local").write_text(
            "TOKEN=1\n", encoding="utf-8"
        )
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.flush_turn("feature/paths")

        capsule, text = self.refresh_capsule("feature/paths")
        self.assertIn("excluded paths: .env.local", capsule["last_turn"])
        self.assertIn("buffer flushed", capsule["last_turn"])
        self.assertIn("Last turn: ", text)

    def test_a_quiet_buffering_turn_replaces_the_report_and_says_nothing(
        self,
    ) -> None:
        self.enable_automatic_promotion()
        self.resolve_observed_finding()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.flush_turn()

        # The next turn only buffers; its report supersedes the flush report,
        # so a reason already surfaced once is not repeated forever.
        result = self.run_cli(
            "turn", "--task-id", "feature/held", "--flush-after", "9", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(json.loads(result.stdout)["flushed"])

        capsule, text = self.refresh_capsule()
        self.assertIsNone(capsule["last_turn"])
        self.assertNotIn("Last turn:", text)

    def test_a_stale_report_is_ignored(self) -> None:
        self.enable_automatic_promotion()
        self.resolve_observed_finding()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.flush_turn()

        report = json.loads(self.report_path().read_text(encoding="utf-8"))
        report["timestamp"] = "2026-01-01T00:00:00+00:00"
        self.report_path().write_text(json.dumps(report), encoding="utf-8")

        capsule, text = self.refresh_capsule()
        self.assertIsNone(capsule["last_turn"])
        self.assertNotIn("Last turn:", text)

    def test_a_missing_or_malformed_report_never_breaks_refresh(self) -> None:
        self.start("TASK-NO-REPORT")
        self.assertFalse(self.report_path().is_file())
        capsule, text = self.refresh_capsule("TASK-NO-REPORT")
        self.assertIsNone(capsule["last_turn"])
        self.assertNotIn("Last turn:", text)

        self.report_path().parent.mkdir(parents=True, exist_ok=True)
        self.report_path().write_text("{not json", encoding="utf-8")
        capsule, _ = self.refresh_capsule("TASK-NO-REPORT")
        self.assertIsNone(capsule["last_turn"])

        # A report whose timestamp cannot be trusted is treated as absent.
        self.report_path().write_text(
            json.dumps({"timestamp": "soon", "flushed": True}), encoding="utf-8"
        )
        capsule, _ = self.refresh_capsule("TASK-NO-REPORT")
        self.assertIsNone(capsule["last_turn"])

    def test_an_unwritable_report_degrades_silently(self) -> None:
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        with mock.patch.object(
            context_cli, "atomic_json", side_effect=PermissionError("denied")
        ):
            code, stdout, stderr = self.run_main(
                "turn", "--task-id", "feature/degrade", "--flush", "--json"
            )
        # The checkpoint landed; telemetry that could not be written must not
        # turn it into a failed turn.
        self.assertEqual(0, code, stderr)
        self.assertTrue(json.loads(stdout)["flushed"])
        self.assertFalse(self.report_path().is_file())


class MultiMachineContinuityTest(RuntimeHarness):
    """Continuity when the ignored binding database does not travel with Git.

    The governed record is authoritative in Git, but the compatibility
    commands resolve it through a machine-local SQLite binding. A second
    machine or fresh clone therefore holds the task without the binding, and
    recreating it is rejected as a duplicate. The flush must reconnect to the
    existing record instead of failing invisibly every turn, and an operator
    must be able to repair the pointer explicitly with `rebind`.
    """

    def report_path(self) -> Path:
        return self.repository / "memory-bank/local/last-turn-report.json"

    def simulate_second_machine(self) -> None:
        # Git-tracked Brain files stay; the ignored local database does not.
        (self.repository / "memory-bank/local/context.db").unlink()

    def test_flush_on_a_second_machine_lands_in_the_existing_task(self) -> None:
        task = self.start("feature/second-machine")
        self.simulate_second_machine()
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = self.run_cli(
            "turn", "--task-id", "feature/second-machine", "--flush", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["flushed"])
        # Reconnected, not re-minted: no new record, no provisioning claim.
        self.assertFalse(payload["provisioned"])
        self.assertEqual(task["task_uuid"], payload["rebound"])
        self.assertEqual(
            1,
            len(list((self.repository / "project-brain/dynamic/tasks").glob("*.md"))),
        )

        record = json.loads(
            self.run_cli(
                "get", "--task-id", "feature/second-machine", "--json"
            ).stdout
        )
        self.assertEqual(task["task_uuid"], record["task_uuid"])
        self.assertIn("app.txt", record["files"])

        # The silenced Stop hook cannot show the reconnection, so the report
        # carries it and the next refresh folds it into the capsule.
        report = json.loads(self.report_path().read_text(encoding="utf-8"))
        self.assertEqual(task["task_uuid"], report["rebound"])
        text = self.run_cli(
            "refresh", "--query", "cobalt authority",
            "--task-id", "feature/second-machine", "--ephemeral",
        )
        self.assertEqual(0, text.returncode, text.stderr)
        self.assertIn("binding restored to the existing task", text.stdout)

    def test_an_ordinary_flush_reports_no_rebinding(self) -> None:
        self.start("feature/plain")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        result = self.run_cli("turn", "--task-id", "feature/plain", "--flush", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIsNone(json.loads(result.stdout)["rebound"])
        self.assertIsNone(
            json.loads(self.report_path().read_text(encoding="utf-8"))["rebound"]
        )

    def test_rebind_restores_the_binding_for_an_explicit_record(self) -> None:
        task = self.start("TASK-REBIND")
        self.simulate_second_machine()
        # Without the binding the compatibility read cannot resolve the task.
        self.assertNotEqual(
            0, self.run_cli("get", "--task-id", "TASK-REBIND").returncode
        )

        result = self.run_cli(
            "rebind", "--task-id", "TASK-REBIND",
            "--record", task["task_uuid"], "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertFalse(payload["already_bound"])
        self.assertEqual(task["task_uuid"], payload["task_uuid"])

        restored = json.loads(
            self.run_cli("get", "--task-id", "TASK-REBIND", "--json").stdout
        )
        self.assertEqual(task["task_uuid"], restored["task_uuid"])
        # A pointer repair consumes no revision: the record never moved.
        self.assertEqual(task["revision"], restored["revision"])

    def test_rebind_resolves_the_record_by_task_id_alone(self) -> None:
        task = self.start("TASK-BY-ID")
        self.simulate_second_machine()
        result = self.run_cli("rebind", "--task-id", "TASK-BY-ID", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(task["task_uuid"], json.loads(result.stdout)["task_uuid"])

    def test_rebind_reports_an_intact_binding_instead_of_recreating_it(
        self,
    ) -> None:
        task = self.start("TASK-IDEMPOTENT")
        result = self.run_cli("rebind", "--task-id", "TASK-IDEMPOTENT", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["already_bound"])
        self.assertEqual(task["task_uuid"], payload["task_uuid"])

    def test_rebind_refuses_a_missing_record(self) -> None:
        result = self.run_cli("rebind", "--task-id", "feature/ghost")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("not found", result.stderr)

    def test_rebind_refuses_a_non_task_record(self) -> None:
        record = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", "FIND-BIND",
                "--title", "Not a task", "--source", "specs/authority.md",
                "--json",
            ).stdout
        )
        result = self.run_cli(
            "rebind", "--task-id", "FIND-BIND", "--record", record["id"]
        )
        self.assertNotEqual(0, result.returncode)
        self.assertIn("requires a task record", result.stderr)

    def test_rebind_refuses_to_repoint_one_task_at_another(self) -> None:
        task = self.start("TASK-A")
        self.simulate_second_machine()
        result = self.run_cli(
            "rebind", "--task-id", "TASK-B", "--record", task["task_uuid"]
        )
        self.assertNotEqual(0, result.returncode)
        self.assertIn("belongs to task id", result.stderr)

    def test_rebind_requires_governed_mode(self) -> None:
        result = self.run_cli("--mode", "lightweight", "rebind", "--task-id", "T-1")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("requires governed mode", result.stderr)

    def test_a_terminal_task_is_refused_and_the_failed_turn_is_reported(
        self,
    ) -> None:
        task = self.start("feature/finished")
        complete = self.run_cli(
            "complete", "--task-id", "feature/finished", "--revision", "1",
            "--outcome", "Done.", "--verification", "tests", "--json",
        )
        self.assertEqual(0, complete.returncode, complete.stderr)

        # Rebinding to a completed record would let flushes mutate history.
        result = self.run_cli("rebind", "--task-id", "feature/finished")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("completed", result.stderr)

        # The automated flush hits the same wall — and because the Stop hook
        # discards its output, the failure must reach the report instead.
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        turn = self.run_cli(
            "turn", "--task-id", "feature/finished", "--flush", "--json"
        )
        self.assertNotEqual(0, turn.returncode)
        report = json.loads(self.report_path().read_text(encoding="utf-8"))
        self.assertIn(task["task_uuid"], report["error"])
        self.assertIn("completed", report["error"])
        self.assertFalse(report["flushed"])
        # The raw branch argv stays out of the failure report by design.
        self.assertNotIn("task_id", report)

        other = self.start("TASK-WITNESS")
        self.assertTrue(other["task_uuid"])
        text = self.run_cli(
            "refresh", "--query", "cobalt authority",
            "--task-id", "TASK-WITNESS", "--ephemeral",
        )
        self.assertEqual(0, text.returncode, text.stderr)
        self.assertIn("Last turn: turn failed:", text.stdout)


class AutoCheckpointFieldTest(RuntimeHarness):
    """The automatic flush reports in its own field, never over the operator's.

    ``progress`` is the operator's narrative — the checkpoint skill writes it
    and the handoff quotes it. The turn flush used to overwrite it wholesale,
    so ten quiet turns erased the one account of the work a successor could
    not reconstruct. The flush now lands in ``auto_checkpoint`` and every
    rendering shows the manual narrative first with the checkpoint as a
    labelled supplement.
    """

    def handoff_state(self) -> str:
        path = next(
            (self.repository / "project-brain/control/handoffs").glob("*.md")
        )
        metadata, _ = brain.parse_markdown_record(path)
        return metadata["current_state"]

    def test_manual_progress_survives_ten_automatic_flushes(self) -> None:
        self.start("TASK-CKPT")
        result = self.run_cli(
            "update", "--task-id", "TASK-CKPT", "--revision", "auto",
            "--progress", "Manual narrative from the checkpoint skill.", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)

        for turn in range(10):
            self.repository.joinpath("app.txt").write_text(
                f"work {turn}\n", encoding="utf-8"
            )
            flushed = self.run_cli(
                "turn", "--task-id", "TASK-CKPT", "--flush", "--json"
            )
            self.assertEqual(0, flushed.returncode, flushed.stderr)
            self.assertTrue(json.loads(flushed.stdout)["flushed"])

        task = json.loads(
            self.run_cli("get", "--task-id", "TASK-CKPT", "--json").stdout
        )
        self.assertEqual(
            "Manual narrative from the checkpoint skill.", task["progress"]
        )
        self.assertIn("Auto-checkpoint: 1 turn(s)", task["auto_checkpoint"])
        # The record carrying the new field still satisfies schema and validator.
        self.assertEqual(0, self.run_cli("validate").returncode)

    def test_handoff_and_capsule_lead_with_manual_progress(self) -> None:
        self.start("TASK-CKPT-RENDER")
        self.run_cli(
            "update", "--task-id", "TASK-CKPT-RENDER", "--revision", "auto",
            "--progress", "Manual narrative.", "--json",
        )
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        result = self.run_cli(
            "turn", "--task-id", "TASK-CKPT-RENDER", "--flush", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)

        state = self.handoff_state()
        self.assertTrue(state.startswith("Manual narrative."), state)
        self.assertIn("\nSince checkpoint: Auto-checkpoint: 1 turn(s)", state)

        capsule = json.loads(
            self.run_cli(
                "retrieve", "cobalt", "--task-id", "TASK-CKPT-RENDER", "--json"
            ).stdout
        )
        working_progress = capsule["working"]["progress"]
        self.assertTrue(
            working_progress.startswith("Manual narrative."), working_progress
        )
        self.assertIn("Since checkpoint: Auto-checkpoint:", working_progress)

    def test_a_checkpoint_alone_does_not_pose_as_manual_progress(self) -> None:
        self.start("TASK-CKPT-ONLY")
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        result = self.run_cli(
            "turn", "--task-id", "TASK-CKPT-ONLY", "--flush", "--json"
        )
        self.assertEqual(0, result.returncode, result.stderr)

        task = json.loads(
            self.run_cli("get", "--task-id", "TASK-CKPT-ONLY", "--json").stdout
        )
        self.assertEqual("", task["progress"])
        state = self.handoff_state()
        # Without manual progress the checkpoint stands under its own
        # `Auto-checkpoint:` label instead of wearing the operator's words.
        self.assertTrue(state.startswith("Auto-checkpoint: 1 turn(s)"), state)
        self.assertNotIn("Since checkpoint:", state)

    def test_a_record_without_a_checkpoint_renders_as_before(self) -> None:
        self.start("TASK-NO-CKPT")
        self.run_cli(
            "update", "--task-id", "TASK-NO-CKPT", "--revision", "auto",
            "--progress", "Manual only.", "--json",
        )

        record = json.loads(
            next(
                (self.repository / "project-brain/dynamic/tasks").glob("*.md")
            ).read_text(encoding="utf-8").split("---")[1]
        )
        self.assertNotIn("auto_checkpoint", record)
        self.assertEqual("Manual only.", self.handoff_state())
        self.assertEqual(0, self.run_cli("validate").returncode)

    def test_only_a_task_may_record_an_auto_checkpoint(self) -> None:
        record = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", "FIND-CKPT",
                "--title", "Not a task", "--source", "specs/authority.md", "--json",
            ).stdout
        )
        with self.assertRaises(brain.BrainError):
            brain.update_record(
                self.repository,
                record["id"],
                expected_revision=1,
                progress=None,
                next_steps=[],
                files=[],
                sources=[],
                actor=record["owner"],
                auto_checkpoint="Auto-checkpoint: forged onto a finding",
            )

        # A forged field written straight to disk is caught on the next read.
        path = next(
            (self.repository / "project-brain/dynamic/findings").glob("*.md")
        )
        metadata, body = brain.parse_markdown_record(path)
        metadata["auto_checkpoint"] = "Auto-checkpoint: forged"
        path.write_text(
            brain.render_markdown_record(metadata, body), encoding="utf-8"
        )
        self.assertEqual(1, self.run_cli("validate").returncode)


class ExportBundleTest(RuntimeHarness):
    """Export is the one operation that moves content off this installation."""

    def setUp(self) -> None:
        super().setUp()
        self.destination = Path(self.temporary.name) / "outbound"

    def chunk(self, identifier: str, slug: str, status: str, *, tags=None, body="Durable text.") -> Path:
        path = self.repository / "memory-bank/chunks" / f"{identifier}-{slug}.md"
        metadata = {
            "id": identifier,
            "title": f"Chunk {identifier}",
            "type": "convention",
            "status": status,
            "scope": ["accelerator"],
            "tags": tags or [],
            "created": "2026-01-01",
            "last_verified": "2026-01-02",
            "review_after": "2027-01-02",
            "sources": ["specs/authority.md"],
            "supersedes": [],
            "superseded_by": None,
        }
        path.write_text(
            "---\n" + json.dumps(metadata, indent=2) + "\n---\n\n# Chunk\n\n" + body + "\n",
            encoding="utf-8",
        )
        return path

    def export(self, **kwargs) -> dict:
        return context_cli.export_bundle(self.repository, self.destination, **kwargs)

    def manifest(self) -> dict:
        return json.loads((self.destination / "MANIFEST.json").read_text(encoding="utf-8"))

    def test_private_and_restricted_records_never_leave_the_installation(self) -> None:
        brain.create_record(
            self.repository, "finding", "F-PUBLIC", "Shareable finding.",
            [], ["specs/authority.md"], owner="alice", privacy="public",
        )
        brain.create_record(
            self.repository, "finding", "F-TEAM", "Team finding.",
            [], ["specs/authority.md"], owner="alice", privacy="team",
        )
        brain.create_record(
            self.repository, "finding", "F-PRIVATE", "Private finding.",
            [], ["specs/authority.md"], owner="alice", privacy="private",
        )
        brain.create_record(
            self.repository, "finding", "F-RESTRICTED", "Restricted finding.",
            [], ["specs/authority.md"], owner="alice", privacy="restricted",
        )

        result = self.export()

        self.assertEqual(2, result["brain_records"])
        exported = {item["external_id"] for item in self.manifest()["included"]
                    if item["kind"] == "brain-record"}
        self.assertEqual({"F-PUBLIC", "F-TEAM"}, exported)

        # The bundle must not merely omit them - it must say what it omitted.
        reasons = {item["reason"] for item in self.manifest()["excluded"]}
        self.assertEqual(2, len(self.manifest()["excluded"]))
        self.assertTrue(all("allowed_privacy" in reason for reason in reasons))

        written = "\n".join(
            path.read_text(encoding="utf-8")
            for path in self.destination.rglob("*.md")
        )
        self.assertNotIn("Private finding.", written)
        self.assertNotIn("Restricted finding.", written)

    def test_superseded_chunks_are_excluded_until_asked_for(self) -> None:
        self.chunk("MEM-0100", "active-one", "active")
        self.chunk("MEM-0101", "superseded-one", "superseded")

        default = self.export()
        self.assertEqual(1, default["memory_chunks"])
        self.assertEqual(1, default["excluded"])

        widened = self.export(include_superseded=True, force=True)
        self.assertEqual(2, widened["memory_chunks"])
        self.assertEqual(0, widened["excluded"])

    def test_auto_promoted_chunks_are_counted_and_flagged(self) -> None:
        self.chunk("MEM-0200", "reviewed", "active")
        self.chunk("MEM-0201", "unreviewed", "active", tags=["auto-promoted"])

        result = self.export()

        self.assertEqual(1, result["auto_promoted_chunks"])
        flagged = {
            item["id"]: item["auto_promoted"]
            for item in self.manifest()["included"]
            if item["kind"] == "memory-chunk"
        }
        self.assertEqual({"MEM-0200": False, "MEM-0201": True}, flagged)

    def test_a_secret_aborts_the_whole_bundle_without_echoing_it(self) -> None:
        self.chunk("MEM-0300", "clean", "active")
        self.chunk(
            "MEM-0301", "leaky", "active",
            body="Deploy with AKIAIOSFODNN7EXAMPLE for now.",
        )

        with self.assertRaises(context_cli.ContextError) as caught:
            self.export()

        message = str(caught.exception)
        self.assertIn("MEM-0301", message)
        self.assertNotIn("AKIAIOSFODNN7EXAMPLE", message)
        # Fail closed: nothing at all is written, not even the clean chunk.
        self.assertFalse(self.destination.exists(), "aborted export left a partial bundle")

        # And the abort is not a one-off: removing the secret exports both.
        self.chunk("MEM-0301", "leaky", "active", body="Deploy with the rotated key.")
        self.assertEqual(2, self.export()["memory_chunks"])

    def test_a_non_empty_destination_is_refused_without_force(self) -> None:
        self.chunk("MEM-0400", "one", "active")
        self.export()

        with self.assertRaises(context_cli.ContextError) as caught:
            self.export()
        self.assertIn("not empty", str(caught.exception))

        self.assertEqual(1, self.export(force=True)["memory_chunks"])

    def test_bundle_states_its_provenance_and_promotion_mode(self) -> None:
        self.chunk("MEM-0500", "one", "active")
        self.export()
        manifest = self.manifest()

        self.assertEqual(context_cli.EXPORT_SCHEMA_VERSION, manifest["schema_version"])
        self.assertEqual("governed", manifest["source"]["mode"])
        self.assertIn("automatic_promotion", manifest["source"])
        self.assertEqual(["public", "team"], manifest["filters"]["allowed_privacy"])

        readme = (self.destination / "README.md").read_text(encoding="utf-8")
        self.assertIn("not authoritative", readme)
        self.assertIn("auto-promoted", readme)
        self.assertIn("authorized owner", readme)


class ShippedRuntimeConfigTest(unittest.TestCase):
    """Assert the config this edition actually ships, not a fixture's rewrite.

    Every other automation test calls ``enable_automation``, which overwrites
    ``runtime.json`` with the flags that test needs. That leaves the shipped
    defaults - the only ones a user ever gets - covered by nothing. A flipped
    default is a behavioral change to every installation, so it fails here
    rather than being discovered in a consuming project.
    """

    SHIPPED = EDITION / "project-brain" / "config" / "runtime.json"

    def setUp(self) -> None:
        self.config = json.loads(self.SHIPPED.read_text(encoding="utf-8"))

    def test_only_safe_memory_maintenance_is_automatic_by_default(self) -> None:
        for flag in ("automatic_promotion", "automatic_compaction"):
            with self.subTest(flag=flag):
                self.assertIs(True, self.config.get(flag))
        self.assertIs(
            False,
            self.config.get("automatic_completion"),
            "merge evidence may suggest completion but must never close a task",
        )

    def test_mode_and_provider_are_the_documented_defaults(self) -> None:
        self.assertEqual("governed", self.config.get("mode"))
        self.assertEqual("sqlite-fts5", self.config.get("provider"))

    def test_telemetry_stays_disabled(self) -> None:
        self.assertIs(False, self.config.get("telemetry_enabled"))

    def test_private_records_are_not_retrievable_by_default(self) -> None:
        self.assertNotIn("private", self.config.get("allowed_privacy", []))


class MetadataTelemetryTest(RuntimeHarness):
    def configure(self, enabled: bool) -> None:
        config = self.repository / "project-brain/config"
        config.mkdir(parents=True, exist_ok=True)
        config.joinpath("telemetry.json").write_text(
            json.dumps(
                {
                    "enabled": enabled,
                    "metadata_only": True,
                    "prohibited_fields": [
                        "prompt", "response", "source_body", "tool_payload",
                        "secret", "customer_data", "raw_log",
                    ],
                    "schema": "../schemas/token-usage-event.schema.json",
                }
            ),
            encoding="utf-8",
        )
        config.joinpath("runtime.json").write_text(
            json.dumps({"telemetry_enabled": enabled}),
            encoding="utf-8",
        )

    def test_disabled_telemetry_produces_no_event(self) -> None:
        self.configure(False)
        result = telemetry_runtime.write_metadata_event(
            self.repository,
            {"provider": "native", "operation": "retrieve"},
        )
        self.assertIsNone(result)
        self.assertEqual([], list(self.repository.glob("**/telemetry/*.json")))

    def test_unavailable_metrics_are_explicitly_na(self) -> None:
        self.configure(True)
        path = telemetry_runtime.write_metadata_event(
            self.repository,
            {
                "provider": "native",
                "model": "host-managed",
                "operation": "retrieve",
                "input_tokens": None,
                "output_tokens": None,
                "context_tokens": None,
                "cached_input_tokens": None,
                "cost_usd": None,
                "ttfr_ms": None,
                "task_id": None,
            },
        )
        self.assertIsNotNone(path)
        event = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual("N/A", event["availability"])
        for field in telemetry_runtime.METRIC_FIELDS:
            self.assertIsNone(event[field])

    def test_telemetry_metrics_match_integer_and_finite_number_schema(self) -> None:
        self.configure(True)
        invalid = (
            ("input_tokens", 1.5),
            ("output_tokens", True),
            ("context_tokens", -1),
            ("cost_usd", float("nan")),
            ("ttfr_ms", float("inf")),
        )
        for field, value in invalid:
            with self.subTest(field=field, value=value):
                with self.assertRaises(telemetry_runtime.TelemetryError):
                    telemetry_runtime.write_metadata_event(
                        self.repository,
                        {
                            "provider": "native",
                            "operation": "retrieve",
                            field: value,
                        },
                    )
        self.assertEqual([], list(self.repository.glob("**/telemetry/*.json")))

    def test_telemetry_canonicalizes_task_uuid(self) -> None:
        self.configure(True)
        supplied = "F47AC10B-58CC-4372-A567-0E02B2C3D479"
        path = telemetry_runtime.write_metadata_event(
            self.repository,
            {
                "provider": "native",
                "operation": "retrieve",
                "task_id": supplied,
            },
        )
        event = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(supplied.lower(), event["task_id"])

    def test_private_and_payload_fields_cannot_be_recorded(self) -> None:
        self.configure(True)
        for field in (
            "prompt", "response", "source_body", "tool_payload",
            "secret", "customer_data", "raw_log",
        ):
            with self.subTest(field=field):
                with self.assertRaisesRegex(
                    telemetry_runtime.TelemetryError, "prohibited"
                ):
                    telemetry_runtime.write_metadata_event(
                        self.repository,
                        {
                            "provider": "native",
                            "operation": "retrieve",
                            field: "must-not-be-written",
                        },
                    )
        self.assertEqual([], list(self.repository.glob("**/telemetry/*.json")))


if __name__ == "__main__":
    unittest.main()
