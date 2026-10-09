#!/usr/bin/env python3
"""Contract tests for the combined Project Brain + Local Context Engine."""

from __future__ import annotations

import hashlib
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
    import memory_results
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
        # "plover" names the document it matched - it is its title - and so
        # admits it alone; "zorkmid" is merely a rare word in the README.
        self.assertRegex(rendered.stdout, r"specs/plover\.md — [^\n]*\(weak match\)")
        self.assertNotIn("README.md", rendered.stdout)

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
        self.assertEqual("distinctive", strengths.get("specs/plover.md"), capsule)
        self.assertNotIn("README.md", strengths)
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        self.assertEqual(
            "distinctive",
            {item["path"]: item["match"] for item in manifest["selected"]}["specs/plover.md"],
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
        # A hook without a prompt (Cursor's stop and session-start renders)
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

    def test_a_stale_citation_does_not_stop_compaction(self) -> None:
        # Records cite living files, so every repository soon holds one whose
        # source moved on. Compaction refused all of it from that moment.
        active = brain.create_record(
            self.repository, "decision", "DEC-LIVE", "Cobalt stays canonical",
            [], ["specs/authority.md"], owner="alice",
        )
        resolved = brain.create_record(
            self.repository, "finding", "FIND-DONE", "Cobalt guard fixed",
            [], ["specs/authority.md"], owner="alice",
        )
        resolved = brain.update_record(
            self.repository, resolved["id"], expected_revision=resolved["revision"],
            progress="Guard added.", next_steps=[], files=[], sources=[],
            actor="alice", transition_to="investigating", reason="Investigating",
        )
        brain.update_record(
            self.repository, resolved["id"], expected_revision=resolved["revision"],
            progress=None, next_steps=[], files=[], sources=[],
            actor="alice", transition_to="resolved", reason="Resolved",
        )
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nThe cobalt authority rule now covers writes.\n",
            encoding="utf-8",
        )
        stale = [
            error for error in brain.validate_repository(self.repository)
            if "source fingerprint is stale" in error
        ]
        self.assertEqual(2, len(stale), stale)

        self.assertEqual({"moved": 1}, brain.compact(self.repository))

        # The terminal record is history now; the live one stays where it was.
        self.assertTrue(
            self.repository.joinpath(
                "project-brain/archive/finding", f"{resolved['id']}.md"
            ).is_file()
        )
        self.assertTrue(
            self.repository.joinpath(
                "project-brain/dynamic/decisions", f"{active['id']}.md"
            ).is_file()
        )
        # Staleness is still reported, and still keeps the record out of reach.
        self.assertEqual(
            2,
            sum(
                "source fingerprint is stale" in error
                for error in brain.validate_repository(self.repository)
            ),
        )
        self.assertEqual(
            [], brain.validate_repository(self.repository, check_freshness=False)
        )

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
        # Clean before the write and failing after it: a write is refused for
        # what it introduced, not for what the bank already carried.
        with mock.patch.object(
            brain, "validate_bank", side_effect=[[], ["injected validation failure"]]
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

    def test_a_failed_commit_takes_the_completion_event_with_it(self) -> None:
        # The event is written before the SQLite commit, and the rollback
        # restored only files that existed beforehand: a commit that failed on
        # disk or I/O reopened the task but left a "Completed" event outside
        # the restored index, so `validate` failed, and the retry could not
        # write the event of the completion that did happen.
        task = self.start("TASK-COMMIT-FAILS")
        events = self.repository / "project-brain/dynamic/events"
        path, record, _ = brain.find_task(self.repository, "TASK-COMMIT-FAILS")
        tracked = [
            path,
            brain.handoff_path(self.repository, record["id"]),
            *brain.index_paths(self.repository),
        ]
        before = {item: item.read_bytes() for item in tracked}
        real_connect = sqlite3.connect

        class FailingCommit(sqlite3.Connection):
            def commit(self) -> None:
                # Only the completion commits with its event already on disk.
                if any(events.glob("*.md")):
                    raise sqlite3.OperationalError("disk I/O error")
                super().commit()

        with mock.patch.object(
            sqlite3,
            "connect",
            lambda *arguments, **keywords: real_connect(
                *arguments, factory=FailingCommit, **keywords
            ),
        ):
            code, _, error = self.run_main(
                "complete", "--task-id", "TASK-COMMIT-FAILS",
                "--revision", str(task["revision"]),
                "--outcome", self.OUTCOME, "--verification", self.CHECK,
            )
        self.assertEqual(1, code)
        self.assertIn("disk I/O error", error)
        self.assertEqual([], list(events.glob("*.md")))
        self.assertEqual(before, {item: item.read_bytes() for item in tracked})
        self.assertEqual(
            "active", brain.get_task(self.repository, "TASK-COMMIT-FAILS")["status"]
        )
        validation = self.run_cli("validate", "--json")
        self.assertEqual(0, validation.returncode, validation.stdout)
        status = json.loads(self.run_cli("status", "--json").stdout)
        self.assertEqual((1, 0), (status["working"], status["episodes"]))

        retry = self.run_cli(
            "complete", "--task-id", "TASK-COMMIT-FAILS",
            "--revision", str(task["revision"]),
            "--outcome", self.OUTCOME, "--verification", self.CHECK, "--json",
        )
        self.assertEqual(0, retry.returncode, retry.stderr)
        self.assertIsNotNone(json.loads(retry.stdout)["event_id"], retry.stdout)
        self.assertEqual(1, len(list(events.glob("*.md"))))


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

    def test_rendered_path_link_contains_the_filtered_body(self) -> None:
        payload = self.capsule("--path", self.SOURCE)
        result = self.run_cli("retrieve", self.SOURCE_WORDS, "--task-id", "TASK-PATH",
                              "--path", self.SOURCE)
        self.assertEqual(0, result.returncode, result.stderr)
        for item in payload["semantic"]:
            if item["path"] in self.chunk_paths():
                # The excerpt quotes the body; the heading names the item.
                phrase = " ".join(item["snippet"].split())
                phrase = phrase.removeprefix("# " + str(item["title"])).strip()
                self.assertIn(phrase[:80], result.stdout)

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

    def test_a_chunk_whose_source_changed_stays_flagged_for_checking(
        self,
    ) -> None:
        # An edit to a cited file used to evict the chunk until someone ran
        # bank-reverify by hand, and nobody did: 65 of 70 resolved findings on
        # a real project were lost that way. The chunk now stays, ranked down
        # and marked, and the reader checks the file it names.
        self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa")
        indexed = self.run_cli("index", "--json")
        self.assertEqual(0, indexed.returncode, indexed.stderr)
        self.start("TASK-DIGEST")
        chunk_path = "memory-bank/chunks/MEM-20260101-aaaaaaaa-old-rule.md"
        found = json.loads(
            self.run_cli(
                "retrieve", "cerulean rollout", "--task-id", "TASK-DIGEST",
                "--ephemeral", "--json",
            ).stdout
        )
        fresh = {item["path"]: item for item in found["selected"]}
        self.assertIn(chunk_path, fresh, found)
        self.assertNotIn("source_changed", fresh[chunk_path])

        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nThe cobalt authority rule was replaced.\n",
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        capsule = json.loads(
            self.run_cli(
                "retrieve", "cerulean rollout", "--task-id", "TASK-DIGEST",
                "--ephemeral", "--json",
            ).stdout
        )
        changed = {item["path"]: item for item in capsule["selected"]}
        self.assertIn(chunk_path, changed, capsule)
        self.assertEqual(
            ["specs/authority.md"], changed[chunk_path]["source_changed"]
        )
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        listed = {item["path"]: item for item in manifest["selected"]}
        self.assertTrue(listed[chunk_path]["source_changed"])
        self.assertLess(
            listed[chunk_path]["score"],
            {item["path"]: item for item in json.loads(
                (self.repository / found["manifest"]).read_text(encoding="utf-8")
            )["selected"]}[chunk_path]["score"],
        )

    def test_a_chunk_whose_source_is_gone_leaves_retrieval(self) -> None:
        self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa")
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        chunk_path = "memory-bank/chunks/MEM-20260101-aaaaaaaa-old-rule.md"
        self.repository.joinpath("specs/authority.md").unlink()
        filtered, excluded = retrieval._runtime_filter(
            self.repository,
            [
                {
                    "path": chunk_path, "privacy": "team", "owner": "local",
                    "authority": "verified", "kind": "memory",
                    "lifecycle": "active",
                    "source_hash": retrieval._content_hash(
                        self.chunk.read_text(encoding="utf-8")
                    ),
                    "source_fingerprints": self.digests_of(self.chunk),
                }
            ],
            {"allowed_privacy": ["team"], "allowed_authority": ["verified"],
             "owners": ["*"]},
        )
        self.assertEqual([], filtered)
        self.assertEqual(
            [{"path": chunk_path, "reason": "source-missing"}], excluded
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


class OverdueBankWriteTest(BankFixture):
    """A chunk past its review date leaves retrieval; it must not lock the bank."""

    def age(self, memory_id: str, slug: str, body: str) -> Path:
        today = datetime.now(timezone.utc).date()
        verified = (today - timedelta(days=400)).isoformat()
        return self.write_chunk(
            memory_id, slug, body,
            created=verified, last_verified=verified,
            review_after=(today - timedelta(days=35)).isoformat(),
        )

    def two_overdue(self) -> None:
        self.age("MEM-20260101-aaaaaaaa", "old-rule", "The cerulean rollout is current.")
        self.age("MEM-20260102-bbbbbbbb", "older-rule", "The vermilion rollout is current.")
        self.assertEqual(0, self.run_cli("reindex-bank").returncode)
        self.assertEqual(
            2, sum("overdue for review" in error for error in self.bank_errors())
        )

    def test_two_overdue_chunks_can_each_be_reverified(self) -> None:
        # Each re-attestation validated the whole bank and failed on the other
        # chunk's review date, so neither could be repaired through the CLI.
        self.two_overdue()
        for memory_id in ("MEM-20260101-aaaaaaaa", "MEM-20260102-bbbbbbbb"):
            result = self.run_cli("bank-reverify", "--id", memory_id)
            self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual([], self.bank_errors())

    def test_an_overdue_chunk_does_not_block_retiring_another(self) -> None:
        self.two_overdue()
        yesterday = (datetime.now(timezone.utc).date() - timedelta(days=1)).isoformat()
        result = self.run_cli(
            "bank-retire", "--id", "MEM-20260101-aaaaaaaa", "--valid-to", yesterday
        )
        self.assertEqual(0, result.returncode, result.stderr)
        # Still reported where it always was.
        self.assertEqual(
            1, sum("overdue for review" in error for error in self.bank_errors())
        )
        audit = json.loads(self.run_cli("bank-audit", "--json").stdout)
        self.assertEqual(
            ["MEM-20260102-bbbbbbbb"],
            [item["id"] for item in audit["overdue_review"]],
            audit,
        )

    def test_reverify_still_holds_the_attested_chunk_to_the_contract(self) -> None:
        # A write no longer pays for other chunks' problems; the chunk it
        # attests still answers for its own.
        self.repository.joinpath("specs/authority.md").unlink()
        result = self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("source path does not exist", result.stderr)

    def test_bank_messages_do_not_carry_the_machine_path(self) -> None:
        self.two_overdue()
        for error in self.bank_errors():
            self.assertTrue(error.startswith("memory-bank/chunks/"), error)
            self.assertNotIn(str(self.repository), error)

    def test_refresh_names_the_chunks_it_stopped_serving(self) -> None:
        # Writes no longer fail on a review date, which was the only place the
        # lapse was ever said out loud; the chunk still leaves retrieval.
        self.two_overdue()
        rendered = self.run_cli("refresh")
        self.assertIn(
            "memory review: 2 chunk(s) overdue, not served until re-verified",
            rendered.stdout,
        )
        self.assertEqual(
            2, json.loads(self.run_cli("refresh", "--json").stdout)["overdue_review"]
        )
        self.run_cli("bank-reverify", "--id", "MEM-20260101-aaaaaaaa")
        self.run_cli("bank-reverify", "--id", "MEM-20260102-bbbbbbbb")
        self.assertNotIn("memory review:", self.run_cli("refresh").stdout)


class WorkingStateCapsuleTest(RuntimeHarness):
    """What the rendered capsule says about the task, and what it stops repeating."""

    def started(self) -> str:
        task = self.start("TASK-STATE")
        update = self.run_cli(
            "update", "--task-id", "TASK-STATE", "--revision", "auto",
            "--progress", "Cobalt guard wired into the request path.",
            "--next-step", "Cover the read path.",
            "--next-step", "Cover the write path.",
            "--file", "src/Guard.php", "--file", "src/Request.php", "--json",
        )
        self.assertEqual(0, update.returncode, update.stderr)
        return str(task["task_uuid"])

    def test_the_rendered_capsule_says_where_the_work_stopped(self) -> None:
        # The JSON capsule carried this all along; the rendered one - the only
        # form Claude Code, Codex and Cursor read - printed the goal alone.
        uuid = self.started()
        rendered = self.run_cli(
            "retrieve", "continue where we left off",
            "--task-id", "TASK-STATE", "--ephemeral",
        )
        self.assertEqual(0, rendered.returncode, rendered.stderr)
        lines = rendered.stdout.splitlines()
        # The Cursor hooks accept a capsule only if it opens with "working:".
        self.assertTrue(lines[0].startswith("working: TASK-STATE"), rendered.stdout)
        self.assertIn("progress: Cobalt guard wired into the request path.", lines)
        self.assertIn("next: Cover the read path.", lines)
        self.assertIn("next: Cover the write path.", lines)
        self.assertIn("recent files: src/Guard.php, src/Request.php", lines)
        self.assertIn(f"task record: project-brain/dynamic/tasks/{uuid}.md", lines)

    def test_the_rendered_progress_is_bounded(self) -> None:
        self.start("TASK-LONG")
        update = self.run_cli(
            "update", "--task-id", "TASK-LONG", "--revision", "auto",
            "--progress", "cobalt " * 400, "--json",
        )
        self.assertEqual(0, update.returncode, update.stderr)
        rendered = self.run_cli(
            "retrieve", "cobalt", "--task-id", "TASK-LONG", "--ephemeral"
        )
        line = next(
            line for line in rendered.stdout.splitlines()
            if line.startswith("progress: ")
        )
        self.assertLessEqual(
            len(line), len("progress: ") + context_cli.RENDERED_PROGRESS_LIMIT
        )
        self.assertGreater(len(line), context_cli.RENDERED_PROGRESS_LIMIT // 2)
        self.assertTrue(line.endswith("…"), line)

    def test_the_task_does_not_point_at_its_own_record(self) -> None:
        # It matched every prompt that shared a word with its own goal and took
        # a semantic slot to point at the state the capsule already leads with.
        uuid = self.started()
        own = f"project-brain/dynamic/tasks/{uuid}.md"
        payload = json.loads(
            self.run_cli(
                "retrieve", "apply the cobalt authority rule",
                "--task-id", "TASK-STATE", "--ephemeral", "--json",
            ).stdout
        )
        self.assertNotIn(own, [item["path"] for item in payload["selected"]])
        self.assertEqual(own, payload["task_record"])
        manifest = json.loads(
            (self.repository / payload["manifest"]).read_text(encoding="utf-8")
        )
        self.assertIn({"path": own, "reason": "working-task"}, manifest["excluded"])

    def test_host_loaded_instructions_leave_the_capsule(self) -> None:
        self.start("TASK-HOST")
        self.repository.joinpath("CLAUDE.md").write_text(
            "# Notes\n\nThe vermilion lattice convention. Policy: @AGENTS.md\n",
            encoding="utf-8",
        )
        self.repository.joinpath("AGENTS.md").write_text(
            "# Policy\n\nThe vermilion lattice rule applies.\n", encoding="utf-8"
        )

        def procedural(host: str) -> tuple[set, list]:
            payload = json.loads(
                self.run_cli(
                    "retrieve", "vermilion lattice", "--task-id", "TASK-HOST",
                    "--ephemeral", "--gate", "off", "--host", host, "--json",
                ).stdout
            )
            manifest = json.loads(
                (self.repository / payload["manifest"]).read_text(encoding="utf-8")
            )
            return {item["path"] for item in payload["procedural"]}, manifest["excluded"]

        paths, _ = procedural("cli")
        # One procedural slot: the better of the two, both being candidates.
        self.assertEqual(1, len(paths))
        self.assertLessEqual(paths, {"AGENTS.md", "CLAUDE.md"})
        paths, excluded = procedural("claude")
        self.assertEqual(set(), paths)
        self.assertIn({"path": "CLAUDE.md", "reason": "host-loaded"}, excluded)
        # Claude Code also loads what CLAUDE.md imports.
        self.assertIn({"path": "AGENTS.md", "reason": "host-loaded"}, excluded)
        paths, excluded = procedural("codex")
        self.assertEqual({"CLAUDE.md"}, paths)
        self.assertIn({"path": "AGENTS.md", "reason": "host-loaded"}, excluded)

    def test_claude_imports_skip_code_and_paths_outside_the_repository(self) -> None:
        docs = self.repository / "docs"
        docs.mkdir()
        docs.joinpath("a.md").write_text("# A\n", encoding="utf-8")
        docs.joinpath("b.md").write_text("# B\n\nSee @c.md for details.\n", encoding="utf-8")
        docs.joinpath("c.md").write_text("# C\n", encoding="utf-8")
        self.repository.joinpath("AGENTS.md").write_text("# Policy\n", encoding="utf-8")
        self.repository.joinpath("CLAUDE.md").write_text(
            "# Notes\n\n"
            "Inline `@AGENTS.md` is code, not an import.\n"
            "```\n@docs/a.md\n```\n"
            "Real import: @docs/b.md.\n"
            "Outside: @../outside.md, and mail someone@example.com.\n",
            encoding="utf-8",
        )
        self.assertEqual(
            {"docs/b.md", "docs/c.md"}, retrieval._claude_imports(self.repository)
        )

    def test_claude_reads_agents_md_itself_only_without_a_claude_md(self) -> None:
        self.repository.joinpath("AGENTS.md").write_text("# Policy\n", encoding="utf-8")
        self.assertIn("AGENTS.md", retrieval.host_loaded_paths(self.repository, "claude"))
        self.repository.joinpath("CLAUDE.local.md").write_text("# Mine\n", encoding="utf-8")
        self.assertNotIn(
            "AGENTS.md", retrieval.host_loaded_paths(self.repository, "claude")
        )

    def test_shipped_claude_dir_import_loads_agents_md(self) -> None:
        self.repository.joinpath("AGENTS.md").write_text("# Policy\n", encoding="utf-8")
        self.repository.joinpath("CLAUDE.md").write_text("# Team notes\n", encoding="utf-8")
        claude_dir = self.repository / ".claude"
        claude_dir.mkdir()
        claude_dir.joinpath("CLAUDE.md").write_text(
            "Policy lives in AGENTS.md.\n\n@../AGENTS.md\n", encoding="utf-8"
        )
        loaded = retrieval.host_loaded_paths(self.repository, "claude")
        self.assertIn("AGENTS.md", loaded)
        self.assertIn(".claude/CLAUDE.md", loaded)
        self.assertEqual({"AGENTS.md"}, retrieval._claude_imports(self.repository))

    def test_done_next_steps_can_leave_the_task(self) -> None:
        uuid = self.started()
        replaced = self.run_cli(
            "update", "--task-id", "TASK-STATE", "--revision", "auto",
            "--replace-next-steps", "--next-step", "Open the pull request.",
            "--file", "src/Guard.php", "--json",
        )
        self.assertEqual(0, replaced.returncode, replaced.stderr)
        task = json.loads(replaced.stdout)
        self.assertEqual(["Open the pull request."], task["next_steps"])
        # Touched again, so newest again.
        self.assertEqual(["src/Request.php", "src/Guard.php"], task["files"])
        cleared = self.run_cli(
            "brain-update", "--record-id", uuid, "--revision", "auto",
            "--replace-next-steps", "--json",
        )
        self.assertEqual(0, cleared.returncode, cleared.stderr)
        self.assertEqual([], json.loads(cleared.stdout)["next_steps"])


class CapsuleNoiseTest(RuntimeHarness):
    def capsule(self, query: str) -> dict:
        result = self.run_cli("refresh", "--query", query, "--task-id", "TASK-NOISE",
                              "--host", "codex", "--ephemeral", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)["capsule"]

    def test_render_delivers_the_filtered_excerpt_and_preserves_source_warning(self) -> None:
        capsule = {"working": None, "warnings": [], "procedural": [], "episodic": [],
                   "semantic": [{"path": "specs/cobalt.md", "title": "Cobalt",
                                 "snippet": "The cobalt allocation requires one owner.",
                                 "source_changed": ["src/Guard.php"]}]}
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            context_cli.print_capsule(capsule, {
                "specs/cobalt.md": "The cobalt allocation requires one owner; ask person@example.test."})
        self.assertTrue(output.getvalue().startswith("working:"))
        self.assertIn("allocation requires one owner", output.getvalue())
        # The excerpt is document text on its way into a prompt: screened too.
        self.assertNotIn("person@example.test", output.getvalue())
        self.assertIn("cited file changed", output.getvalue())

    def test_excerpt_preserves_array_syntax_from_the_source(self) -> None:
        self.repository.joinpath("specs/array.md").write_text(
            "# Allocation\n\nCobalt values[owner] must exist before allocation.\n")
        result = self.run_cli("refresh", "--query", "cobalt values", "--task-id", "TASK-NOISE", "--ephemeral")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("values[owner]", result.stdout)

    def test_render_is_bounded_and_never_publishes_private_excerpt_text(self) -> None:
        capsule = {"working": {"task_id": "TASK-NOISE", "goal": "Apply cobalt",
                               "progress": "p" * 2000, "next_steps": ["s" * 2000] * 3,
                               "files": ["src/" + "f" * 300] * 5}, "warnings": [],
                   "procedural": [], "episodic": [],
                   "semantic": [{"path": "specs/cobalt.md", "title": "Cobalt",
                                 "snippet": "cobalt " * 300 + "person@example.test"}]}
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            context_cli.print_capsule(capsule, {"specs/cobalt.md": capsule["semantic"][0]["snippet"]})
        self.assertLessEqual(len(output.getvalue()), 3600)
        self.assertNotIn("person@example.test", output.getvalue())
        self.assertTrue(output.getvalue().startswith("working: TASK-NOISE"))

    def test_chat_does_not_retrieve_documents_or_history(self) -> None:
        self.repository.joinpath("specs/chat.md").write_text("# Chat\n\nThanks, looks good.\n")
        capsule = self.capsule("thanks looks good")
        self.assertFalse(any(capsule[layer] for layer in context_cli.DOCUMENT_LAYERS), capsule)

    def test_weak_skills_are_omitted_but_an_exact_identifier_survives(self) -> None:
        skill = self.repository / ".agents/skills/xanthic/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text("---\nname: xanthic\ndescription: xanthic workflows\n---\n# Xanthic\n")
        self.repository.joinpath("specs/other.md").write_text("# Rollover\n\nRollover is separate.\n")
        self.repository.joinpath("specs/identifier.md").write_text("# Allocation\n\nCMS768 needs one owner.\n")
        for number in range(15):
            self.repository.joinpath(f"specs/filler-{number}.md").write_text(
                f"# Filler {number}\n\nOrdinary records.\n")
        self.assertEqual([], self.capsule("xanthic rollover")["procedural"])
        capsule = self.capsule("CMS768 rollover")
        self.assertIn("specs/identifier.md", [item["path"] for item in capsule["semantic"]])

    def test_weak_incidental_word_is_not_evidence_of_the_requested_topic(self) -> None:
        self.repository.joinpath("specs/unrelated.md").write_text("# Allocation\n\nMentions xanthic.\n")
        self.repository.joinpath("specs/other.md").write_text("# Rollover\n\nRollover is separate.\n")
        for number in range(15):
            self.repository.joinpath(f"specs/filler-{number}.md").write_text(f"# Filler {number}\n\nOrdinary records.\n")
        capsule = self.capsule("xanthic rollover")
        self.assertNotIn("specs/unrelated.md", [item["path"] for item in capsule["semantic"]])

    def test_lightweight_task_query_does_not_restore_a_weak_skill(self) -> None:
        skill = self.repository / ".agents/skills/xanthic/SKILL.md"
        skill.parent.mkdir(parents=True)
        skill.write_text("---\nname: xanthic\ndescription: xanthic workflows\n---\n# Xanthic\n")
        self.repository.joinpath("specs/rollover.md").write_text("# Rollover\n\nRollover is separate.\n")
        for number in range(15):
            self.repository.joinpath(f"specs/filler-{number}.md").write_text(f"# Filler {number}\n\nOrdinary records.\n")
        started = self.run_cli("--mode", "lightweight", "start", "--task-id", "TASK-LITE",
                               "--goal", "xanthic rollover")
        self.assertEqual(0, started.returncode, started.stderr)
        result = self.run_cli("--mode", "lightweight", "context", "cobalt",
                              "--task-id", "TASK-LITE", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual([], json.loads(result.stdout)["procedural"])

    def test_lightweight_acknowledgement_keeps_state_without_goal_fallback_retrieval(self) -> None:
        started = self.run_cli("--mode", "lightweight", "start", "--task-id", "TASK-LITE",
                               "--goal", "cobalt authority")
        self.assertEqual(0, started.returncode, started.stderr)
        result = self.run_cli("--mode", "lightweight", "context", "thanks looks good",
                              "--task-id", "TASK-LITE", "--json")
        self.assertEqual(0, result.returncode, result.stderr)
        capsule = json.loads(result.stdout)
        self.assertEqual("cobalt authority", capsule["working"]["goal"])
        self.assertFalse(any(capsule[layer] for layer in context_cli.DOCUMENT_LAYERS), capsule)

    def test_only_one_strong_skill_is_delivered(self) -> None:
        for name in ("first", "second"):
            skill = self.repository / f".agents/skills/{name}/SKILL.md"
            skill.parent.mkdir(parents=True)
            skill.write_text(f"---\nname: {name}\ndescription: cobalt allocation\n---\n# {name}\n\ncobalt allocation\n")
        self.assertEqual(1, len(self.capsule("cobalt allocation")["procedural"]))

    def test_relative_floor_is_per_layer_and_keeps_the_best_memory(self) -> None:
        rows = []
        for path, layer, title, repetitions in (
            (".agents/skills/strong/SKILL.md", "procedural", "Cobalt allocation", 1),
            ("specs/best.md", "semantic", "Cobalt allocation", 1),
            ("specs/tail.md", "semantic", "Unrelated", 400)):
            content = "cobalt allocation " + "ordinary material " * repetitions
            source = self.repository / path
            source.parent.mkdir(parents=True, exist_ok=True)
            source.write_text(content)
            rows.append((path, layer, "skill" if layer == "procedural" else "spec", title, title, content))
        connection = context_cli.connect(self.repository / "memory-bank/local/context.db")
        try:
            retrieval.index_documents(connection, self.repository, rows)
            candidates, _ = retrieval._candidates(connection, "cobalt allocation")
        finally:
            connection.close()
        paths = {item["path"] for item in candidates}
        self.assertIn("specs/best.md", paths)
        self.assertNotIn("specs/tail.md", paths)

    def test_next_question_receives_a_different_excerpt_from_the_same_file(self) -> None:
        self.repository.joinpath("specs/allocation.md").write_text(
            "# Allocation\n\nCobalt allocation requires one owner.\n\n"
            + "Ordinary background. " * 200
            + "\n\nRelease requires the dragonfly token.\n")
        first = self.run_cli("refresh", "--query", "cobalt allocation", "--task-id", "TASK-NOISE",
                             "--ephemeral")
        second = self.run_cli("refresh", "--query", "dragonfly release", "--task-id", "TASK-NOISE",
                              "--ephemeral")
        self.assertEqual(0, first.returncode, first.stderr)
        self.assertEqual(0, second.returncode, second.stderr)
        self.assertIn("allocation requires one owner", first.stdout)
        self.assertIn("Release requires the dragonfly token", second.stdout)

    def test_automatic_query_sanitization_keeps_the_technical_subject(self) -> None:
        safe = context_cli.sanitize_automatic_query(
            "Fix CMS768 allocation for person@example.test, call +370 600 12345; Customer id: 10492")
        self.assertIn("CMS768 allocation", safe)
        for private in ("person@example.test", "12345", "10492"):
            self.assertNotIn(private, safe)
        for private_only in ("person@example.test", "Customer email: person@example.test",
                             "Call +370 600 12345", "Patient name: Alice Smith",
                             "Client address: Main Street 7, Vilnius",
                             "Client address: Main Street 7\nVilnius",
                             "Patient name: Alice; Smith; Fix CMS768",
                             "Client address: Building A; Vilnius; Fix CMS768"):
            self.assertEqual("", context_cli.sanitize_automatic_query(private_only))
        self.assertIn("CMS768 allocation", context_cli.sanitize_automatic_query(
            "Fix CMS768 allocation; Patient name: Alice; Smith"))
        # A secret is cut out rather than costing the turn its memory; a prompt
        # that was nothing else leaves nothing to search. A transcript role
        # prefix goes and the words after it stay, and a private key goes
        # with its body, not only its header line.
        self.assertEqual("", context_cli.sanitize_automatic_query("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"))
        self.assertEqual("copied request", context_cli.sanitize_automatic_query("User: copied request"))
        self.assertEqual("why does deploy fail", context_cli.sanitize_automatic_query(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA\n-----END RSA PRIVATE KEY-----\nwhy does deploy fail"))
        self.assertEqual("", context_cli.sanitize_automatic_query(
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjE truncated without its footer"))
        # A quoted value goes whole, whatever the quote, and an unclosed one
        # to the end of its line: the space inside it is not its end.
        for quote in ("'", '"', "`"):
            cleaned = context_cli.sanitize_automatic_query(
                f"Fix deploy password={quote}alpha beta gamma{quote} after migration")
            self.assertEqual("Fix deploy   after migration", cleaned)
        self.assertEqual("next line stays", context_cli.sanitize_automatic_query(
            "api_key: 'alpha beta\nnext line stays"))
        # An HTTP credential goes whatever shape its token takes; a
        # placeholder in documentation is not one.
        token = "Zx81Kq0vLm2Np3Qr4St5Uv6Wx7"
        self.assertEqual("curl fails with   on /orders", context_cli.sanitize_automatic_query(
            f"curl fails with Authorization: Bearer {token} on /orders"))
        self.assertEqual("use   for the deploy", context_cli.sanitize_automatic_query(f"use bearer {token} for the deploy"))
        self.assertEqual("is rejected", context_cli.sanitize_automatic_query(
            "Authorization: Basic dXNlcjpwYXNzMTIz is rejected"))
        for placeholder in ("Authorization: Bearer YOUR_API_TOKEN", "Authorization: Bearer <token>",
                            "Authorization: Bearer $TOKEN", "Bearer authentication for the API"):
            self.assertEqual(placeholder, context_cli.sanitize_automatic_query(placeholder))


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

    def test_refresh_retrieves_before_the_branch_task_exists(self) -> None:
        """A read-only session, and the first turns of every branch, have no
        governed task yet. The capsule used to be a bare warning then; it now
        carries the retrieval layers and no working state."""
        result = self.run_cli(
            "refresh", "--query", "cobalt authority", "--task-id", "TASK-ABSENT",
            "--ephemeral", "--json",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual("updated", report["semantic"])
        capsule = report["capsule"]
        self.assertIsNotNone(capsule, report["warnings"])
        self.assertIsNone(capsule["working"])
        self.assertEqual("warming", capsule["kind"])
        self.assertIn(
            "specs/authority.md",
            [item["path"] for item in capsule["semantic"]],
        )
        self.assertFalse(
            any("Capsule unavailable" in item for item in report["warnings"]),
            report["warnings"],
        )
        # No governed history is written for a task that does not exist.
        governed = self.repository / "project-brain" / "control" / "retrieval-manifests"
        self.assertEqual([], sorted(governed.glob("*.json")) if governed.is_dir() else [])

    def test_unprovisioned_capsule_renders_its_state(self) -> None:
        result = self.run_cli(
            "refresh", "--query", "cobalt authority", "--task-id", "TASK-ABSENT",
            "--ephemeral",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("working: not recorded yet", result.stdout)

    def test_explicit_retrieve_still_requires_the_task(self) -> None:
        result = self.run_cli("retrieve", "cobalt", "--task-id", "TASK-ABSENT")
        self.assertNotEqual(0, result.returncode)
        self.assertIn("Working task not found", result.stderr)

    def test_subagent_completion_provisions_the_branch_task(self) -> None:
        result = self.run_cli(
            "msg-dispatch", "--task-id", "feature/new-branch",
            "--agent", "coder-agent", "--event", "complete",
        )
        self.assertEqual(0, result.returncode, result.stderr)
        listed = self.run_cli("msg-read", "--task-id", "feature/new-branch", "--json")
        self.assertEqual(0, listed.returncode, listed.stderr)
        self.assertEqual(
            ["completion"], [message["type"] for message in json.loads(listed.stdout)]
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

    def commit(self, name: str, subject: str) -> None:
        self.repository.joinpath(name).write_text(f"{subject}\n", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-qm", subject)

    def checkpoint_of(self, task_id: str) -> str:
        return json.loads(
            self.run_cli("get", "--task-id", task_id, "--json").stdout
        )["auto_checkpoint"]

    def test_a_checkpoint_names_the_branch_commits(self) -> None:
        # Paths said where the work happened; nothing said what it was, though
        # every commit already carried a person's one-line summary of it.
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/export")
        self.commit("exporter.txt", "Add the invoice exporter")
        self.commit("controller.txt", "Wire the export controller")
        self.git("checkout", "-q", "main")
        self.commit("main.txt", "Main moved on")
        self.git("checkout", "-q", "feature/export")
        self.git("merge", "-q", "--no-ff", "main", "-m", "Merge main into the branch")
        self.repository.joinpath("pending.txt").write_text("x\n", encoding="utf-8")

        result = self.run_cli("turn", "--task-id", "feature/export", "--flush", "--json")

        self.assertEqual(0, result.returncode, result.stderr)
        checkpoint = self.checkpoint_of("feature/export")
        self.assertTrue(checkpoint.startswith("Auto-checkpoint: 1 turn(s)"), checkpoint)
        self.assertIn(
            "Branch commits, newest first: Wire the export controller; "
            "Add the invoice exporter.",
            checkpoint,
        )
        # The branch's own story: no merges, nothing the default branch did.
        self.assertNotIn("Merge main", checkpoint)
        self.assertNotIn("Main moved on", checkpoint)

    def test_a_turn_that_ends_in_a_commit_still_counts(self) -> None:
        # A clean tree used to read as a turn with nothing in it, so the most
        # finished work - committed work - never reached the checkpoint.
        self.repository.joinpath(".gitignore").write_text(
            "memory-bank/local/\n", encoding="utf-8"
        )
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/clean")
        visit = json.loads(
            self.run_cli("turn", "--task-id", "feature/clean", "--json").stdout
        )
        # Visiting a branch still mints nothing.
        self.assertIsNone(visit["delta_id"])
        self.commit("work.txt", "Finish the clean-tree change")

        result = json.loads(
            self.run_cli("turn", "--task-id", "feature/clean", "--flush", "--json").stdout
        )

        self.assertTrue(result["flushed"], result)
        self.assertEqual(0, result["files"])
        self.assertIn("Finish the clean-tree change", self.checkpoint_of("feature/clean"))
        # An unchanged HEAD on a clean tree is still nothing.
        again = json.loads(
            self.run_cli("turn", "--task-id", "feature/clean", "--json").stdout
        )
        self.assertIsNone(again["delta_id"])

    def test_a_checkpoint_leaves_out_subjects_a_privacy_gate_refuses(self) -> None:
        self.commit_main()
        self.git("checkout", "-q", "-b", "feature/gated")
        self.commit("one.txt", "Add the ledger")
        self.commit("two.txt", "Ping alice@example.com about totals")
        self.commit("three.txt", "Set password=hunter2hunter2 for staging")
        self.repository.joinpath("pending.txt").write_text("x\n", encoding="utf-8")

        result = self.run_cli("turn", "--task-id", "feature/gated", "--flush", "--json")

        self.assertEqual(0, result.returncode, result.stderr)
        checkpoint = self.checkpoint_of("feature/gated")
        self.assertIn("Branch commits, newest first: Add the ledger (+2 more).", checkpoint)
        self.assertNotIn("example.com", checkpoint)
        self.assertNotIn("hunter2", checkpoint)

    def test_work_on_the_default_branch_lists_no_branch_commits(self) -> None:
        self.commit_main()
        self.commit("main.txt", "Direct work on main")
        self.repository.joinpath("pending.txt").write_text("x\n", encoding="utf-8")

        self.run_cli("turn", "--task-id", "main", "--flush", "--json")

        self.assertNotIn("Branch commits", self.checkpoint_of("main"))

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

    def test_promote_auto_runs_automatic_promotion_without_a_turn(self) -> None:
        # Knowledge recorded deliberately, as a reviewed Harness session does,
        # should not wait for the turn counter to reach its boundary.
        record_id = self.accepted_decision()
        off = self.run_cli("promote-auto", "--json")
        self.assertEqual(0, off.returncode, off.stderr)
        self.assertEqual((False, []), (json.loads(off.stdout)["enabled"], json.loads(off.stdout)["promoted"]))

        self.enable_automatic_promotion()
        result = json.loads(self.run_cli("promote-auto", "--json").stdout)

        self.assertEqual([record_id], [item["record_id"] for item in result["promoted"]])
        chunk = next((self.repository / "memory-bank/chunks").glob(
            f"{result['promoted'][0]['memory_id']}-*.md"
        ))
        self.assertIn("auto-promoted", chunk.read_text(encoding="utf-8"))
        # Once is enough: the record is bound to its promotion now.
        self.assertEqual([], json.loads(self.run_cli("promote-auto", "--json").stdout)["promoted"])

    def test_turn_promotes_resolved_verified_knowledge_without_review(self) -> None:
        # Resolved while promotion was off, so the turn boundary is what
        # promotes it - the path every record written before promotion was
        # switched on, or through the runtime API, still takes.
        record_id = self.accepted_decision()
        self.enable_automatic_promotion()
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

    def test_turns_after_the_branch_task_closed_do_not_jam_the_boundary(self) -> None:
        # Work on a branch went on after its task was completed. Every flush
        # tried to reattach the closed task and failed, so the buffer grew and
        # the promotion riding on the boundary never ran again.
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        first = json.loads(
            self.run_cli("turn", "--task-id", "feature/closed", "--flush", "--json").stdout
        )
        self.assertTrue(first["provisioned"])
        current = json.loads(
            self.run_cli("get", "--task-id", "feature/closed", "--json").stdout
        )
        done = self.run_cli(
            "complete", "--task-id", "feature/closed",
            "--revision", str(current["revision"]), "--outcome", "Done.",
            "--verification", "checked by hand", "--source", "specs/authority.md",
            "--json",
        )
        self.assertEqual(0, done.returncode, done.stderr)

        record_id = self.accepted_decision()
        self.enable_automatic_promotion()
        self.repository.joinpath("app.txt").write_text("more work\n", encoding="utf-8")
        after = self.run_cli("turn", "--task-id", "feature/closed", "--flush", "--json")
        self.assertEqual(0, after.returncode, after.stderr)
        payload = json.loads(after.stdout)
        self.assertEqual(current["task_uuid"], payload["closed_task"])
        self.assertEqual(1, payload["discarded_turns"])
        self.assertEqual(0, payload["pending"])
        self.assertEqual([record_id], [item["record_id"] for item in payload["promoted"]])

    def test_the_resolving_update_promotes_at_once(self) -> None:
        # Waiting for the turn boundary let the fix that resolved a finding
        # edit the files it cites first, and the record was never promoted.
        self.enable_automatic_promotion()
        record = json.loads(
            self.run_cli(
                "brain-create", "decision", "--external-id", "TASK-NOW-D1",
                "--title", "Cobalt rule is canonical",
                "--source", "specs/authority.md", "--authority", "verified",
                "--json",
            ).stdout
        )
        accepted = self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--progress", "Cobalt applies to every request path, not only reads.",
            "--transition", "accepted", "--reason", "Accepted", "--json",
        )
        self.assertEqual(0, accepted.returncode, accepted.stderr)
        payload = json.loads(accepted.stdout)
        self.assertEqual("accepted", payload["status"])
        promoted = payload["promotion"]["promoted"]
        self.assertEqual([record["id"]], [item["record_id"] for item in promoted])
        self.assertTrue(
            list((self.repository / "memory-bank/chunks").glob(
                f"{promoted[0]['memory_id']}-*.md"
            ))
        )
        # Nothing is left for the boundary to do.
        self.assertEqual(
            [], json.loads(self.run_cli("promote-auto", "--json").stdout)["promoted"]
        )

    def test_an_update_that_does_not_resolve_promotes_nothing(self) -> None:
        self.enable_automatic_promotion()
        record = json.loads(
            self.run_cli(
                "brain-create", "finding", "--external-id", "TASK-OPEN-F1",
                "--title", "Open finding", "--source", "specs/authority.md",
                "--authority", "verified", "--json",
            ).stdout
        )
        updated = json.loads(
            self.run_cli(
                "brain-update", "--record-id", record["id"], "--revision", "auto",
                "--progress", "Still investigating the cobalt guard.", "--json",
            ).stdout
        )
        self.assertNotIn("promotion", updated)
        self.assertEqual(
            [], list((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        )

    def test_knowledge_whose_cited_file_changed_is_promoted_and_marked(self) -> None:
        # Resolved, then the cited file was edited before promotion ran: the
        # record used to be blocked for good. It is promoted with the digest
        # taken when it was verified, and retrieval marks it for checking.
        record_id = self.accepted_decision()
        verified_digest = brain.fingerprint(self.repository, "specs/authority.md")
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nThe cobalt rule was rewritten after the decision.\n",
            encoding="utf-8",
        )
        self.enable_automatic_promotion()
        result = json.loads(self.run_cli("promote-auto", "--json").stdout)
        self.assertEqual([record_id], [item["record_id"] for item in result["promoted"]])
        chunk = next((self.repository / "memory-bank/chunks").glob(
            f"{result['promoted'][0]['memory_id']}-*.md"
        ))
        metadata, _ = brain.parse_markdown_record(chunk)
        self.assertIn(verified_digest, metadata["source_digests"])
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        self.start("TASK-MARKED")
        capsule = json.loads(
            self.run_cli(
                "retrieve", "cobalt request path", "--task-id", "TASK-MARKED",
                "--ephemeral", "--json",
            ).stdout
        )
        marked = {
            item["path"]: item.get("source_changed") for item in capsule["selected"]
        }
        chunk_path = chunk.relative_to(self.repository).as_posix()
        self.assertEqual(["specs/authority.md"], marked.get(chunk_path), capsule)

    def test_knowledge_whose_cited_file_is_gone_stays_blocked(self) -> None:
        self.accepted_decision()
        self.repository.joinpath("specs/authority.md").unlink()
        self.enable_automatic_promotion()
        result = json.loads(self.run_cli("promote-auto", "--json").stdout)
        self.assertEqual([], result["promoted"])
        self.assertIn(
            "cited source changed", " ".join(item["reason"] for item in result["blocked"])
        )

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
        # Promoted by the resolving update itself, not at the turn boundary.
        promoted = json.loads(resolved.stdout)["promotion"]["promoted"]
        self.assertEqual(1, len(promoted), promoted)

        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        flushed = self.run_cli(
            "turn", "--task-id", "TASK-MEMORY-E2E", "--flush", "--json"
        )
        self.assertEqual(0, flushed.returncode, flushed.stderr)
        self.assertEqual([], json.loads(flushed.stdout)["promoted"])
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
        # Resolved before automation was on, so nothing was promoted at
        # resolution and the capped boundary run has all seven to place.
        for index in range(7):
            self.resolved_finding(index)
        self.enable_automation(
            automatic_promotion=True, automatic_compaction=True,
            compaction_threshold=1,
        )
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

    def test_a_stale_citation_does_not_stop_automatic_compaction(self) -> None:
        # The hook path: from the first edit to a file any live record cited,
        # every turn reported "compaction failed" and nothing was archived.
        self.enable_automation(automatic_compaction=True, compaction_threshold=1)
        live = self.run_cli(
            "brain-create", "decision", "--external-id", "DEC-LIVE",
            "--title", "Cobalt stays canonical", "--source", "specs/authority.md",
            "--json",
        )
        self.assertEqual(0, live.returncode, live.stderr)
        self.resolved_finding(0)
        self.repository.joinpath("specs/authority.md").write_text(
            "# Authority\n\nThe cobalt authority rule now covers writes.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")

        result = json.loads(
            self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json").stdout
        )

        self.assertEqual((1, 0), (result["archived"], result["archivable_pending"]))
        report = json.loads(
            (self.repository / "memory-bank/local/last-turn-report.json").read_text(
                encoding="utf-8"
            )
        )
        self.assertEqual([], report["compaction_errors"])

    def age_chunk(self, chunk: Path) -> None:
        """Move a chunk's review date into the past, as a year of use would."""
        _, frontmatter, body = chunk.read_text(encoding="utf-8").split("---\n", 2)
        metadata = json.loads(frontmatter)
        today = datetime.now(timezone.utc).date()
        verified = (today - timedelta(days=400)).isoformat()
        metadata.update(
            created=verified, last_verified=verified,
            review_after=(today - timedelta(days=35)).isoformat(),
        )
        chunk.write_text(
            "---\n" + json.dumps(metadata, indent=2) + "\n---\n" + body,
            encoding="utf-8",
        )
        self.assertEqual(0, self.run_cli("reindex-bank").returncode)

    def test_an_overdue_chunk_does_not_stop_automatic_promotion(self) -> None:
        # One chunk reaching its review date failed every later promotion in
        # the repository - a year after automatic promotion wrote the first.
        self.enable_automatic_promotion()
        self.resolved_finding(0)
        self.age_chunk(next((self.repository / "memory-bank/chunks").glob("MEM-*.md")))

        # Promoted by its own resolving update, past the overdue chunk.
        self.resolved_finding(1)
        self.assertEqual(
            2, len(list((self.repository / "memory-bank/chunks").glob("MEM-*.md")))
        )
        self.repository.joinpath("more.txt").write_text("more\n", encoding="utf-8")
        result = json.loads(
            self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json").stdout
        )
        self.assertEqual([], result["promotion_failed"])

    def test_an_overdue_promoted_chunk_does_not_stop_compaction(self) -> None:
        # Compaction repoints the citation of a chunk promoted from a record it
        # archives. The rewrite is mechanical, and the chunk's review date is
        # not its business.
        self.enable_automatic_promotion()
        self.resolved_finding(0)
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        self.run_cli("turn", "--task-id", "feature/x", "--flush", "--json")
        chunk = next((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        self.age_chunk(chunk)

        result = self.run_cli("compact")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("project-brain/archive/finding/", chunk.read_text(encoding="utf-8"))
        self.assertEqual(
            1,
            sum(
                "overdue for review" in error
                for error in brain.validate_bank(self.repository / "memory-bank")
            ),
        )


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

        # The automated flush does not reattach it either. It used to fail on
        # every boundary and keep its buffer, so promotion and compaction never
        # ran again; the turns after completion are now dropped, and the next
        # capsule says so.
        self.repository.joinpath("app.txt").write_text("work\n", encoding="utf-8")
        turn = self.run_cli(
            "turn", "--task-id", "feature/finished", "--flush", "--json"
        )
        self.assertEqual(0, turn.returncode, turn.stderr)
        payload = json.loads(turn.stdout)
        self.assertEqual(task["task_uuid"], payload["closed_task"])
        self.assertFalse(payload["flushed"])
        self.assertEqual(0, payload["pending"])
        report = json.loads(self.report_path().read_text(encoding="utf-8"))
        self.assertFalse(report["flushed"])
        self.assertEqual(1, report["discarded_turns"])

        other = self.start("TASK-WITNESS")
        self.assertTrue(other["task_uuid"])
        text = self.run_cli(
            "refresh", "--query", "cobalt authority",
            "--task-id", "TASK-WITNESS", "--ephemeral",
        )
        self.assertEqual(0, text.returncode, text.stderr)
        self.assertIn("Last turn: this branch's task is closed", text.stdout)


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


class LineEndingFingerprintTest(RuntimeHarness):
    """A CRLF checkout of the same commit keeps every record fresh."""

    def test_crlf_checkout_of_a_cited_source_stays_fresh(self) -> None:
        record = brain.create_record(
            self.repository, "finding", "FIND-CRLF", "Cobalt rule finding",
            [], ["specs/authority.md"], owner="alice",
        )
        source = self.repository / "specs" / "authority.md"
        source.write_bytes(source.read_bytes().replace(b"\n", b"\r\n"))

        self.assertTrue(brain.sources_are_fresh(self.repository, record))
        self.assertEqual([], brain.validate_repository(self.repository))

        source.write_bytes(b"# Authority\r\n\r\nA different rule.\r\n")
        self.assertFalse(brain.sources_are_fresh(self.repository, record))

    def test_lf_digest_is_the_plain_file_digest(self) -> None:
        source = self.repository / "specs" / "authority.md"
        self.assertEqual(
            hashlib.sha256(source.read_bytes()).hexdigest(),
            brain.fingerprint(self.repository, "specs/authority.md")["sha256"],
        )

    def test_binary_source_is_digested_as_is(self) -> None:
        binary = self.repository / "specs" / "logo.bin"
        binary.write_bytes(b"\x89PNG\0\r\n\x1a\n")
        self.assertEqual(
            hashlib.sha256(binary.read_bytes()).hexdigest(),
            brain.fingerprint(self.repository, "specs/logo.bin")["sha256"],
        )


class SharedTextGuardTest(RuntimeHarness):
    """Governed writes refuse the secrets and personal data they introduce.

    Records, handoffs and agent messages are Git-tracked and travel into Task
    Capsules, so the governed path holds the same line the lightweight path
    already held. A refusal names the kind of data and never the value.
    """

    TOKEN = "ghp_" + "ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"
    UNSAFE = (
        ("email", "Fix login for person@example.test", "person@example.test"),
        ("token", f"Use {TOKEN} for the deploy", "ABCDEFGHIJ"),
        ("env credential", "Works locally with DB_PASSWORD=SuperS3cret!", "SuperS3cret"),
        ("phone", "Call the owner at +370 600 12345", "600 12345"),
    )

    def task(self, external_id: str = "TASK-GUARD") -> dict:
        return brain.create_task(
            self.repository,
            external_id,
            "Apply the cobalt authority rule.",
            [],
            ["specs/authority.md"],
            owner="alice",
        )

    def records_on_disk(self) -> list:
        return sorted(self.repository.glob("project-brain/dynamic/**/*.md"))

    def write_bypassing_guard(self, record: dict) -> None:
        """A record as one written before the guard existed would look."""
        brain._write_record(
            self.repository, record, brain.dynamic_path(self.repository, record)
        )

    def test_create_refuses_unsafe_goal_and_stores_nothing(self) -> None:
        for kind, goal, value in self.UNSAFE:
            with self.subTest(kind=kind):
                with self.assertRaises(brain.BrainError) as caught:
                    brain.create_task(
                        self.repository, "TASK-GUARD", goal, [],
                        ["specs/authority.md"], owner="alice",
                    )
                message = str(caught.exception)
                self.assertIn("nothing was stored", message)
                self.assertNotIn(value, message)
                self.assertEqual([], self.records_on_disk())

    def test_update_refuses_unsafe_text_and_leaves_the_record_unchanged(self) -> None:
        task = self.task()
        for kind, text, value in self.UNSAFE:
            for field in ("progress", "next_steps", "reason"):
                with self.subTest(kind=kind, field=field):
                    changes = {
                        "progress": None, "next_steps": [], "reason": "Task updated",
                    }
                    changes[field] = [text] if field == "next_steps" else text
                    with self.assertRaises(brain.BrainError) as caught:
                        brain.update_record(
                            self.repository, task["id"],
                            expected_revision=task["revision"],
                            files=[], sources=[], actor="alice", **changes,
                        )
                    self.assertNotIn(value, str(caught.exception))
        stored = brain.get_record(self.repository, task["id"])
        self.assertEqual(task["revision"], stored["revision"])
        self.assertEqual("", stored["progress"])

    def test_record_written_before_the_guard_stays_updatable(self) -> None:
        task = self.task()
        legacy = brain.get_record(self.repository, task["id"])
        legacy["progress"] = "Reported by person@example.test"
        self.write_bypassing_guard(legacy)

        updated = brain.update_record(
            self.repository, task["id"],
            expected_revision=legacy["revision"],
            progress=None, next_steps=["Confirm the cobalt rule."],
            files=[], sources=[], actor="alice",
        )

        self.assertEqual(legacy["revision"] + 1, updated["revision"])

    def test_agent_message_refuses_personal_data(self) -> None:
        task = self.task()
        with self.assertRaises(brain.BrainError) as caught:
            brain.append_message(
                self.repository, task["id"],
                from_actor="coder", to_actor="orchestrator",
                message_type="finding", body="Ask person@example.test for access",
            )
        self.assertNotIn("person@example.test", str(caught.exception))
        self.assertFalse(brain.messages_path(self.repository, task["id"]).exists())

    def test_promotion_refuses_a_legacy_record_carrying_personal_data(self) -> None:
        finding = brain.create_record(
            self.repository, "finding", "FIND-GUARD",
            "Cobalt authority finding", [], ["specs/authority.md"], owner="alice",
        )
        resolved = self.resolve_verified_finding(finding)
        config = brain.load_config(self.repository)
        self.assertIsNone(
            brain.promotion_eligibility_error(self.repository, resolved, config)
        )
        resolved["progress"] = "Verified with person@example.test"
        self.write_bypassing_guard(resolved)

        self.assertEqual(
            "record content contains personal data (email address)",
            brain.promotion_eligibility_error(self.repository, resolved, config),
        )

    def test_validation_reports_a_stored_secret_without_echoing_it(self) -> None:
        task = self.task()
        legacy = brain.get_record(self.repository, task["id"])
        legacy["progress"] = f"Deploy token {self.TOKEN}"
        self.write_bypassing_guard(legacy)

        errors = brain.validate_repository(self.repository)

        self.assertTrue(any("possible GitHub token" in error for error in errors), errors)
        self.assertFalse(any("ABCDEFGHIJ" in error for error in errors))

    def test_validation_does_not_fail_on_legacy_personal_data(self) -> None:
        task = self.task()
        legacy = brain.get_record(self.repository, task["id"])
        legacy["progress"] = "Reported by person@example.test"
        self.write_bypassing_guard(legacy)

        self.assertEqual([], brain.validate_repository(self.repository))

    def test_governed_cli_start_refuses_personal_data(self) -> None:
        result = self.run_cli(
            "--mode", "governed", "start", "--task-id", "TASK-CLI-GUARD",
            "--goal", "Fix login for person@example.test",
            "--source", "specs/authority.md",
        )
        self.assertNotEqual(0, result.returncode)
        self.assertNotIn("person@example.test", result.stdout + result.stderr)
        self.assertEqual([], self.records_on_disk())


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



class DeliveryTest(RuntimeHarness):
    """What a host actually puts in front of the model on each turn."""

    def enable_automatic_promotion(self) -> None:
        config = self.repository / "project-brain/config/runtime.json"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(
            json.dumps({"mode": "governed", "automatic_promotion": True}),
            encoding="utf-8",
        )

    def accept_money_decision(self) -> str:
        self.repository.joinpath("src").mkdir(exist_ok=True)
        self.repository.joinpath("src/Money.php").write_text(
            "<?php\nfinal class Money {}\n", encoding="utf-8"
        )
        record = json.loads(
            self.run_cli(
                "brain-create", "decision", "--external-id", "DEC-MONEY",
                "--title", "Store money as integer cents",
                "--source", "src/Money.php", "--authority", "verified", "--json",
            ).stdout
        )
        accepted = self.run_cli(
            "brain-update", "--record-id", record["id"], "--revision", "auto",
            "--progress",
            "Totals and discounts are kept in integer cents; floats never hold "
            "an amount, and rounding happens only when an amount is displayed.",
            "--transition", "accepted", "--reason", "Agreed", "--json",
        )
        self.assertEqual(0, accepted.returncode, accepted.stderr)
        return str(record["id"])

    def refresh(self, query: str, *extra: str) -> dict:
        result = self.run_cli(
            "refresh", "--query", query, "--task-id", "TASK-DELIVER",
            "--ephemeral", "--json", *extra,
        )
        self.assertIn(result.returncode, (0, 1), result.stderr)
        return json.loads(result.stdout)

    def test_the_capsule_carries_the_answer_not_a_pointer(self) -> None:
        self.enable_automatic_promotion()
        record_id = self.accept_money_decision()
        self.start("TASK-DELIVER")
        result = self.refresh("order totals rounding discount cents")
        text = result["capsule_text"]
        self.assertTrue(text.startswith("working: TASK-DELIVER"), text)
        self.assertIn("rounding happens only when an amount is displayed", text)
        # The promoted chunk and the record it came from said the same thing
        # in two slots; the record yields to its chunk.
        semantic = [item["path"] for item in result["capsule"]["semantic"]]
        self.assertEqual(1, sum("store-money-as-integer-cents" in path for path in semantic), semantic)
        self.assertNotIn(f"project-brain/dynamic/decisions/{record_id}.md", semantic)
        manifest = json.loads(
            (self.repository / result["capsule"]["manifest"]).read_text(encoding="utf-8")
        )
        self.assertIn(
            {"path": f"project-brain/dynamic/decisions/{record_id}.md",
             "reason": "promoted-to-chunk"},
            manifest["excluded"],
        )
        self.assertLessEqual(len(text), context_cli.RENDERED_CAPSULE_LIMIT)

    def test_a_conversation_is_not_handed_the_same_item_twice(self) -> None:
        self.enable_automatic_promotion()
        self.accept_money_decision()
        self.start("TASK-DELIVER")
        query = "order totals rounding discount cents"
        first = self.refresh(query, "--session-id", "conversation-1")
        handed = {item["path"] for item in first["capsule"]["selected"]}
        self.assertTrue(handed)
        second = self.refresh(query, "--session-id", "conversation-1")
        self.assertEqual([], second["capsule"]["selected"])
        self.assertEqual(len(handed), second["capsule"]["repeated"])
        self.assertIn("handed earlier in this conversation", second["capsule_text"])
        # The slots are not refilled with weaker candidates.
        self.assertNotIn("memory (retrieved", second["capsule_text"])
        # Another conversation is handed everything.
        other = self.refresh(query, "--session-id", "conversation-2")
        self.assertEqual(handed, {item["path"] for item in other["capsule"]["selected"]})
        # After the novelty window the item comes back: a conversation that
        # was compacted meanwhile no longer holds it.
        for _ in range(retrieval.SESSION_NOVELTY_TURNS - 2):
            self.refresh(query, "--session-id", "conversation-1")
        again = self.refresh(query, "--session-id", "conversation-1")
        self.assertEqual(handed, {item["path"] for item in again["capsule"]["selected"]})

    def test_enforce_hands_another_conversation_what_it_was_never_handed(self) -> None:
        # The gate's repeat baseline is the task's, not the conversation's:
        # a second conversation asking the same thing was a "repeat" of the
        # first and enforce withheld its whole capsule, although nothing had
        # been handed to it. What a conversation holds is its own record's
        # call, and a repeat inside one conversation is still left out.
        self.enable_automatic_promotion()
        self.accept_money_decision()
        self.start("TASK-DELIVER")
        query = "order totals rounding discount cents"
        first = self.refresh(query, "--session-id", "conversation-1", "--gate", "enforce")
        handed = {item["path"] for item in first["capsule"]["selected"]}
        self.assertTrue(handed, first["capsule"])
        other = self.refresh(query, "--session-id", "conversation-2", "--gate", "enforce")
        self.assertEqual("retrieve", other["capsule"]["gate"]["decision"], other["capsule"]["gate"])
        self.assertEqual(handed, {item["path"] for item in other["capsule"]["selected"]})
        self.assertIn("rounding happens only when an amount is displayed", other["capsule_text"])
        again = self.refresh(query, "--session-id", "conversation-1", "--gate", "enforce")
        self.assertEqual("skip", again["capsule"]["gate"]["decision"], again["capsule"]["gate"])
        self.assertEqual([], again["capsule"]["selected"])
        self.assertEqual(len(handed), again["capsule"]["repeated"])
        # A caller with no conversation keeps the task's baseline.
        self.refresh(query, "--gate", "enforce")
        alone = self.refresh(query, "--gate", "enforce")
        self.assertEqual("repeat-retrieval", alone["capsule"]["gate"]["reason"], alone["capsule"]["gate"])
        self.assertEqual([], alone["capsule"]["selected"])

    def test_enforce_hands_back_what_the_conversation_no_longer_holds(self) -> None:
        # Past the novelty window, or after a compaction, the conversation's
        # record hands an item again; the gate saw the selection of a turn
        # that had delivered it and withheld it on that turn and every later
        # one, since a withheld turn records nothing as handed.
        self.enable_automatic_promotion()
        self.accept_money_decision()
        self.start("TASK-DELIVER")
        query = "order totals rounding discount cents"
        first = self.refresh(query, "--session-id", "c-novelty", "--gate", "enforce")
        handed = {item["path"] for item in first["capsule"]["selected"]}
        self.assertTrue(handed, first["capsule"])
        for _ in range(retrieval.SESSION_NOVELTY_TURNS - 1):
            held = self.refresh(query, "--session-id", "c-novelty", "--gate", "enforce")
            self.assertEqual([], held["capsule"]["selected"])
        again = self.refresh(query, "--session-id", "c-novelty", "--gate", "enforce")
        self.assertEqual("retrieve", again["capsule"]["gate"]["decision"], again["capsule"]["gate"])
        self.assertEqual(handed, {item["path"] for item in again["capsule"]["selected"]})

        folder = tempfile.TemporaryDirectory(prefix="transcript-")
        self.addCleanup(folder.cleanup)
        transcript = Path(folder.name) / "transcript.jsonl"
        transcript.write_text('{"type":"user"}\n', encoding="utf-8")
        conversation = ("--session-id", "c-compact", "--transcript", str(transcript), "--gate", "enforce")
        self.assertTrue(self.refresh(query, *conversation)["capsule"]["selected"])
        with transcript.open("a", encoding="utf-8") as handle:
            handle.write('{"type":"system","subtype":"compact_boundary"}\n')
        compacted = self.refresh(query, *conversation)
        self.assertEqual("retrieve", compacted["capsule"]["gate"]["decision"], compacted["capsule"]["gate"])
        self.assertEqual(handed, {item["path"] for item in compacted["capsule"]["selected"]})
        self.assertIn("rounding happens only when an amount is displayed", compacted["capsule_text"])

    def test_the_structured_capsule_quotes_what_the_text_quotes(self) -> None:
        # The JSON capsule's snippet was FTS snippet() over the whole file, so
        # a promoted chunk carried its frontmatter where the rendered capsule
        # quoted the decision, and its token estimate counted the JSON keys.
        self.enable_automatic_promotion()
        self.accept_money_decision()
        self.start("TASK-DELIVER")
        result = self.refresh("order totals rounding discount cents")
        chunk = next(
            item for item in result["capsule"]["semantic"]
            if "store-money-as-integer-cents" in item["path"]
        )
        self.assertEqual(
            "Totals and discounts are kept in integer cents; floats never hold an "
            "amount, and rounding happens only when an amount is displayed.",
            chunk["snippet"],
        )
        self.assertIn(chunk["snippet"], result["capsule_text"])
        self.assertEqual(
            retrieval._estimate_tokens(chunk["title"] + chunk["snippet"]),
            chunk["estimated_tokens"],
        )

    def test_a_match_only_in_frontmatter_quotes_the_opening_of_the_body(self) -> None:
        # The query's words are in the description, not the prose: the
        # snippet used to be that frontmatter, and the body's window, scored
        # nothing everywhere, would have been its last sentence.
        self.repository.joinpath("specs/ledger.md").write_text(
            "---\ndescription: Cobalt allocation ledger\n---\n# Bookkeeping\n\n"
            + " ".join(f"Entry rule {index} keeps the books balanced." for index in range(1, 40))
            + "\n",
            encoding="utf-8",
        )
        self.start("TASK-DELIVER")
        result = self.refresh("cobalt allocation ledger")
        ledger = next(
            item for item in result["capsule"]["semantic"] if item["path"] == "specs/ledger.md"
        )
        self.assertTrue(
            ledger["snippet"].startswith("Entry rule 1 keeps the books balanced."), ledger["snippet"]
        )
        self.assertNotIn("description", ledger["snippet"])
        self.assertIn("Entry rule 1 keeps the books balanced.", result["capsule_text"])

    def test_a_section_the_query_never_matched_gives_its_opening(self) -> None:
        text = "# Guide\n\n" + " ".join(
            f"Step {index} is described here." for index in range(1, 60)
        ) + "\n"
        section = retrieval.quoted_section(text, "Guide")
        window = retrieval.excerpt_window(section["units"], 120)
        self.assertTrue(window.startswith("Step 1 is described here."), window)
        self.assertLessEqual(len(window), 120)
        marked = text.replace("Step 30 is", "\x02Step\x03 30 is")
        window = retrieval.excerpt_window(retrieval.quoted_section(marked, "Guide")["units"], 120)
        self.assertTrue(window.startswith("… Step 30 is described here."), window)

    def test_a_damaged_repeat_record_costs_a_repeat_not_the_turn(self) -> None:
        self.enable_automatic_promotion()
        self.accept_money_decision()
        self.start("TASK-DELIVER")
        query = "order totals rounding discount cents"
        first = self.refresh(query, "--session-id", "conversation-1")
        self.assertTrue(first["capsule"]["selected"])
        for damaged in ({"conversation-1": {"turn": 1, "items": ["bad"]}},
                        {"conversation-1": {"turn": "bad", "items": {}}},
                        {"conversation-1": "bad", "conversation-2": {"turn": 2, "items": {"k": "bad"}}}):
            with self.subTest(damaged=damaged):
                connection = context_cli.connect(context_cli.default_database(self.repository))
                try:
                    with connection:
                        retrieval.store_index_state(
                            connection, {retrieval.SESSION_DELIVERIES_KEY: json.dumps(damaged)})
                finally:
                    connection.close()
                again = self.refresh(query, "--session-id", "conversation-1")
                self.assertTrue(again["capsule"]["selected"] or again["capsule"]["repeated"])

    def test_small_talk_retrieves_nothing(self) -> None:
        self.enable_automatic_promotion()
        self.accept_money_decision()
        self.start("TASK-DELIVER")
        for prompt in ("thanks, looks good", "ok continue", "спасибо, отлично"):
            with self.subTest(prompt=prompt):
                result = self.refresh(prompt)
                self.assertEqual([], result["capsule"]["selected"])
                self.assertNotIn("memory (retrieved", result["capsule_text"])

    def test_an_automatic_prompt_is_cleaned_instead_of_refused(self) -> None:
        self.enable_automatic_promotion()
        self.accept_money_decision()
        self.start("TASK-DELIVER")
        prompt = (
            "Order totals are off by a cent after a discount, see the log:\n"
            "stderr: rounding mismatch for jane.doe@example.test\n"
            "Customer id: 10492 called about it"
        )
        refused = self.run_cli(
            "refresh", "--query", prompt, "--task-id", "TASK-DELIVER",
            "--ephemeral", "--json",
        )
        self.assertNotEqual(0, refused.returncode)
        self.assertEqual("", refused.stdout)
        cleaned = self.run_cli(
            "refresh", "--query", prompt, "--task-id", "TASK-DELIVER",
            "--ephemeral", "--sanitize", "--json",
        )
        self.assertIn(cleaned.returncode, (0, 1), cleaned.stderr)
        result = json.loads(cleaned.stdout)
        self.assertIn("rounding happens only when an amount is displayed", result["capsule_text"])
        manifest = (self.repository / result["capsule"]["manifest"]).read_text(encoding="utf-8")
        for private in ("jane.doe", "example.test", "10492", "stderr"):
            self.assertNotIn(private, manifest)
            self.assertNotIn(private, result["capsule_text"])

    def test_the_excerpt_is_the_section_that_matches(self) -> None:
        content = (
            "---\n{\"id\": \"x\"}\n---\n# Payments\n\nGeneral notes about payments.\n\n"
            "## Refunds\n\nRefunds are issued to the original card within five days.\n\n"
            "## Webhooks\n\nWebhook signatures rotate monthly.\n"
        )
        excerpt = context_cli.best_excerpt(content, {"refunds", "card"}, 200, "Payments")
        self.assertTrue(excerpt.startswith("Refunds: Refunds are issued"), excerpt)
        self.assertNotIn("Webhook", excerpt)
        self.assertLessEqual(len(context_cli.best_excerpt(content, set(), 30)), 30)

    def write_auth_guide(self) -> None:
        filler = " ".join(
            ["The session store keeps one record per login and expires it on a schedule."] * 12
        )
        self.repository.joinpath("specs").mkdir(exist_ok=True)
        self.repository.joinpath("specs/auth.md").write_text(
            "# Auth guide\n\n## Sessions\n\n"
            f"{filler} To revoke every session of a user, call SessionStore::purge "
            "after the password reset.\n\n"
            "## Recovery codes\n\nRecovery codes are single-use: a used code is struck "
            "from the list, and after the third the user prints a new set.\n",
            encoding="utf-8",
        )
        self.repository.joinpath("specs/cleanup.md").write_text(
            "# Cleanup\n\n## Login\n\nA session starts at login.\n\n"
            "## Nightly job\n\nSessions are purged nightly by the cleanup job.\n",
            encoding="utf-8",
        )
        for index in range(6):
            self.repository.joinpath(f"specs/other-{index}.md").write_text(
                f"# Other {index}\n\nunrelated text {index}.\n", encoding="utf-8"
            )

    def test_the_excerpt_is_the_stretch_that_answers(self) -> None:
        # The right section, but its answer sat past the first 600 characters.
        self.write_auth_guide()
        self.start("TASK-DELIVER")
        text = self.refresh("how do I revoke sessions after a password reset")["capsule_text"]
        self.assertIn("specs/auth.md § Sessions", text)
        self.assertIn("SessionStore::purge after the password reset", text)

    def test_the_excerpt_matches_words_as_the_index_does(self) -> None:
        # The request says "session", the answering section "Sessions": the
        # index matched them, the excerpt counted them as different words.
        self.write_auth_guide()
        self.start("TASK-DELIVER")
        text = self.refresh("when does a session get purged")["capsule_text"]
        self.assertIn("specs/cleanup.md § Nightly job", text)
        self.assertIn("purged nightly by the cleanup job", text)

    def test_another_section_of_a_handed_document_is_still_handed(self) -> None:
        self.write_auth_guide()
        self.start("TASK-DELIVER")
        first = self.refresh("how do I revoke sessions after a password reset", "--session-id", "c-1")
        self.assertIn("specs/auth.md § Sessions", first["capsule_text"])
        again = self.refresh("revoke the sessions after a password reset", "--session-id", "c-1")
        self.assertNotIn("specs/auth.md", again["capsule_text"])
        other = self.refresh("what happens to used recovery codes", "--session-id", "c-1")
        self.assertIn("specs/auth.md § Recovery codes", other["capsule_text"])
        self.assertIn("struck from the list", other["capsule_text"])

    def test_a_compacted_conversation_is_handed_its_memory_again(self) -> None:
        self.write_auth_guide()
        self.start("TASK-DELIVER")
        # A folder of its own: parallel suites share the repository's parent.
        folder = tempfile.TemporaryDirectory(prefix="transcript-")
        self.addCleanup(folder.cleanup)
        transcript = Path(folder.name) / "transcript.jsonl"
        transcript.write_text('{"type":"user"}\n', encoding="utf-8")
        query = "how do I revoke sessions after a password reset"
        conversation = ("--session-id", "c-1", "--transcript", str(transcript))
        self.assertIn("SessionStore::purge", self.refresh(query, *conversation)["capsule_text"])

        def append(record: str) -> None:
            with transcript.open("a", encoding="utf-8") as handle:
                handle.write(record + "\n")

        append('{"type":"assistant"}')
        self.assertNotIn("SessionStore::purge", self.refresh(query, *conversation)["capsule_text"])
        # Claude Code's compaction record, then Codex's.
        for record in (
            '{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}',
            '{"timestamp":"2026-10-08T10:00:00Z","type":"compacted","payload":{}}',
        ):
            with self.subTest(record=record):
                append(record)
                self.assertIn("SessionStore::purge", self.refresh(query, *conversation)["capsule_text"])
                self.assertNotIn("SessionStore::purge", self.refresh(query, *conversation)["capsule_text"])

    def test_one_rare_word_admits_a_document_only_when_it_names_something(self) -> None:
        # In a small index most words are rare; one of them shared with the
        # request is not evidence unless it is an identifier or the document's
        # own subject.
        specs = self.repository / "specs"
        specs.mkdir(exist_ok=True)
        specs.joinpath("weather.md").write_text(
            "# Weather\n\nThe falcon flew over the harbour.\n", encoding="utf-8")
        specs.joinpath("exporter.md").write_text(
            "# Exports\n\nOrderExporter writes one line per order.\n", encoding="utf-8")
        specs.joinpath("ticket.md").write_text(
            "# Release notes\n\nShipped the fix for ticket 4711.\n", encoding="utf-8")
        specs.joinpath("falcon.md").write_text(
            "# Falcon dispatch\n\nRoutes are assigned nightly.\n", encoding="utf-8")
        specs.joinpath("cobalt.md").write_text(
            "# Cobalt\n\nCobalt budgets are reviewed monthly.\n", encoding="utf-8")
        # Enough documents for a word in two of them to count as rare.
        for index in range(24):
            specs.joinpath(f"filler-{index}.md").write_text(
                f"# Filler {index}\n\nunrelated text {index}.\n", encoding="utf-8")
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        connection = context_cli.connect(context_cli.default_database(self.repository))
        try:
            def admitted(query: str) -> set[str]:
                candidates, _ = retrieval._candidates(connection, query)
                return {item["path"] for item in candidates}

            # Two informative terms, so one alone is a weak match.
            self.assertNotIn("specs/weather.md", admitted("falcon cobalt"))
            self.assertIn("specs/falcon.md", admitted("falcon cobalt"))
            self.assertIn("specs/exporter.md", admitted("OrderExporter cobalt"))
            self.assertIn("specs/ticket.md", admitted("4711 cobalt"))
        finally:
            connection.close()

    def test_a_weak_tail_in_a_layer_is_left_out(self) -> None:
        self.repository.joinpath("specs/quartz.md").write_text(
            "# Quartz falcon\n\nThe quartz falcon dispatch rule. Quartz falcon "
            "retries stop after three attempts.\n",
            encoding="utf-8",
        )
        filler = " ".join(f"word{index}" for index in range(1500))
        self.repository.joinpath("specs/long.md").write_text(
            f"# Long\n\n{filler} quartz {filler} falcon {filler}\n",
            encoding="utf-8",
        )
        for index in range(10):
            self.repository.joinpath(f"specs/other-{index}.md").write_text(
                f"# Other {index}\n\nunrelated text {index}.\n", encoding="utf-8"
            )
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        connection = context_cli.connect(context_cli.default_database(self.repository))
        try:
            candidates, _ = retrieval._candidates(connection, "quartz falcon")
        finally:
            connection.close()
        paths = [item["path"] for item in candidates]
        self.assertIn("specs/quartz.md", paths)
        self.assertNotIn("specs/long.md", paths)

    def test_a_plural_and_its_singular_weigh_the_same_in_the_excerpt(self) -> None:
        # The index's Porter tokenizer matched "classes" to "class"; the
        # excerpt read them as "class" and "clas", so the rare word that
        # answered fell to the lowest weight and a section sharing two common
        # words with the request was quoted instead.
        pairs = {
            "ledger-1": ("classes", "class"),
            "ledger-2": ("policy", "policies"),
            "ledger-3": ("businesses", "business"),
            "ledger-4": ("stopped", "stop"),
        }
        specs = self.repository / "specs"
        for stem, (_, written) in pairs.items():
            specs.joinpath(f"{stem}.md").write_text(
                f"# Ledger {stem[-1]}\n\n"
                f"## Rare details\n\nThe InvoiceMapper {written} is what the "
                "container resolves first.\n\n"
                "## Common details\n\nThe billing module wiring lists every "
                "listener of the billing module.\n",
                encoding="utf-8",
            )
        for index in range(8):
            specs.joinpath(f"common-{index}.md").write_text(
                f"# Common {index}\n\nThe billing module note number {index}.\n",
                encoding="utf-8",
            )
        for index in range(10):
            specs.joinpath(f"other-{index}.md").write_text(
                f"# Other {index}\n\nunrelated text {index}.\n", encoding="utf-8"
            )
        self.start("TASK-DELIVER")
        for stem, (asked, written) in pairs.items():
            with self.subTest(asked=asked, written=written):
                forms = retrieval.term_forms([asked, written])
                self.assertEqual(forms[asked], forms[written], forms)
                text = self.refresh(f"billing module {asked}")["capsule_text"]
                self.assertIn(f"specs/{stem}.md § Rare details", text)
                self.assertIn(f"The InvoiceMapper {written} is what", text)

    def record_episode(self) -> None:
        recorded = self.run_cli(
            "record", "--summary", "Amber viaduct rollout",
            "--outcome",
            "The amber viaduct rollout finished after the canary held for an hour.",
            "--json",
        )
        self.assertEqual(0, recorded.returncode, recorded.stderr)

    def test_a_recorded_episode_is_handed_once_per_revision(self) -> None:
        # A recorded episode took no part in the conversation's record: every
        # turn handed the same one again and counted no repeat.
        self.start("TASK-DELIVER")
        self.record_episode()
        query = "amber viaduct rollout canary"
        first = self.refresh(query, "--session-id", "repeat-local")
        self.assertEqual(
            ["Amber viaduct rollout"],
            [item.get("summary") for item in first["capsule"]["episodic"]],
        )
        self.assertEqual(0, first["capsule"]["repeated"])
        second = self.refresh(query, "--session-id", "repeat-local")
        self.assertEqual([], second["capsule"]["episodic"])
        self.assertEqual(1, second["capsule"]["repeated"])
        self.assertIn("1 item(s) handed earlier", second["capsule_text"])
        manifest = json.loads(
            (self.repository / second["capsule"]["manifest"]).read_text(encoding="utf-8")
        )
        self.assertEqual(0, manifest["local_episode_count"])
        self.assertIn(
            {"path": "episode:1", "reason": "delivered-this-session"},
            manifest["excluded"],
        )
        # Another conversation is handed it.
        other = self.refresh(query, "--session-id", "repeat-other")
        self.assertEqual(1, len(other["capsule"]["episodic"]))
        # An episode whose content changed is new to the conversation.
        connection = sqlite3.connect(self.repository / "memory-bank/local/context.db")
        try:
            with connection:
                connection.execute(
                    "UPDATE episodes SET outcome = ? WHERE rowid = 1",
                    ("The amber viaduct rollout was rolled back when the canary failed.",),
                )
        finally:
            connection.close()
        changed = self.refresh(query, "--session-id", "repeat-local")
        self.assertEqual(
            ["The amber viaduct rollout was rolled back when the canary failed."],
            [item.get("outcome") for item in changed["capsule"]["episodic"]],
        )
        self.assertEqual(0, changed["capsule"]["repeated"])

    def test_a_compacted_conversation_is_handed_its_episode_again(self) -> None:
        self.start("TASK-DELIVER")
        self.record_episode()
        folder = tempfile.TemporaryDirectory(prefix="transcript-")
        self.addCleanup(folder.cleanup)
        transcript = Path(folder.name) / "transcript.jsonl"
        transcript.write_text('{"type":"user"}\n', encoding="utf-8")
        conversation = ("--session-id", "c-episode", "--transcript", str(transcript))
        query = "amber viaduct rollout canary"

        def append(record: str) -> None:
            with transcript.open("a", encoding="utf-8") as handle:
                handle.write(record + "\n")

        self.assertEqual(1, len(self.refresh(query, *conversation)["capsule"]["episodic"]))
        append('{"type":"assistant"}')
        self.assertEqual([], self.refresh(query, *conversation)["capsule"]["episodic"])
        append('{"type":"system","subtype":"compact_boundary","content":"Conversation compacted"}')
        again = self.refresh(query, *conversation)
        self.assertEqual(1, len(again["capsule"]["episodic"]))
        self.assertEqual(0, again["capsule"]["repeated"])

    DISPATCH_NOTES = ("alpha", "bravo", "charlie")

    def write_dispatch_notes(self, depth: int, title_words: int) -> None:
        specs = self.repository / "specs"
        for name in self.DISPATCH_NOTES:
            folder = specs / f"{name}-{'routing-' * depth}"
            folder.mkdir(parents=True)
            folder.joinpath("dispatch.md").write_text(
                f"# {name.capitalize()} quartz falcon dispatch {'handbook ' * title_words}\n\n"
                f"The {name} quartz falcon dispatch retries stop after three attempts.\n",
                encoding="utf-8",
            )
        for index in range(12):
            specs.joinpath(f"other-{index}.md").write_text(
                f"# Other {index}\n\nunrelated text {index}.\n", encoding="utf-8"
            )

    def shown_notes(self, result: dict) -> list[str]:
        return [
            name for name in self.DISPATCH_NOTES
            if f"specs/{name}-" in result["capsule_text"]
        ]

    def assert_recorded_as_shown(self, result: dict) -> str:
        """The capsule, its manifest and its estimates name what the text
        shows; returns the note that was left out."""
        shown = self.shown_notes(result)
        self.assertEqual(2, len(shown), result["capsule_text"])
        (left_out,) = set(self.DISPATCH_NOTES) - set(shown)
        capsule = result["capsule"]
        selected = sorted(item["path"] for item in capsule["selected"])
        self.assertEqual(
            shown, [path[len("specs/"):].split("-")[0] for path in selected]
        )
        manifest = json.loads(
            (self.repository / capsule["manifest"]).read_text(encoding="utf-8")
        )
        self.assertEqual(selected, sorted(item["path"] for item in manifest["selected"]))
        self.assertEqual(
            ["capsule-limit"],
            [
                item["reason"]
                for item in manifest["excluded"]
                if item["path"].startswith(f"specs/{left_out}-")
            ],
        )
        self.assertEqual(
            sum(item["estimated_tokens"] for item in manifest["selected"]),
            manifest["token_estimates"]["total"],
        )
        self.assertEqual(manifest["token_estimates"], capsule["token_estimates"])
        return left_out

    def test_an_item_the_capsule_leaves_out_is_handed_on_the_next_turn(self) -> None:
        # Paths and titles long enough that the capsule's JSON holds two of
        # the three notes. The third was recorded as handed all the same, and
        # the next turns left it out as an item that "still applies". (Each
        # snippet is the note's sentence, not its long heading, hence the
        # title length; the path is near the file system's name limit.)
        self.write_dispatch_notes(depth=28, title_words=34)
        self.start("TASK-DELIVER")
        query = "quartz falcon dispatch retries"
        first = self.refresh(query, "--session-id", "c-limit")
        left_out = self.assert_recorded_as_shown(first)
        second = self.refresh(query, "--session-id", "c-limit")
        self.assertEqual([left_out], self.shown_notes(second))
        self.assertEqual(2, second["capsule"]["repeated"])

    def test_an_item_the_rendered_text_leaves_out_is_handed_on_the_next_turn(self) -> None:
        # The rendered text drops whole entries once the excerpts are gone,
        # and an entry it dropped was recorded as handed. A ceiling that holds
        # the working state and two of the three item lines, measured on a
        # capsule that shows all three, makes the renderer the one to drop.
        self.write_dispatch_notes(depth=8, title_words=1)
        self.start("TASK-DELIVER")
        arguments = (
            "refresh", "--query", "quartz falcon dispatch retries",
            "--task-id", "TASK-DELIVER", "--ephemeral", "--json",
        )

        def refresh_in_process(*extra: str) -> dict:
            code, stdout, stderr = self.run_main(*arguments, *extra)
            self.assertIn(code, (0, 1), stderr)
            return json.loads(stdout)

        lines = refresh_in_process()["capsule_text"].splitlines()
        header = lines.index(context_cli.MEMORY_RENDER_HEADER)
        items = [line for line in lines[header + 1:] if line.startswith("- ")]
        self.assertEqual(3, len(items), lines)
        ceiling = sum(len(line) + 1 for line in [*lines[: header + 1], *items[:2]]) + 10
        with mock.patch.object(context_cli, "RENDERED_CAPSULE_LIMIT", ceiling):
            first = refresh_in_process("--session-id", "c-render")
        self.assertLessEqual(len(first["capsule_text"]), ceiling)
        left_out = self.assert_recorded_as_shown(first)
        self.assertEqual(1, first["capsule"]["omitted"]["semantic"])
        second = refresh_in_process("--session-id", "c-render")
        self.assertEqual([left_out], self.shown_notes(second))
        self.assertEqual(2, second["capsule"]["repeated"])

    def test_the_capsule_keeps_only_the_items_its_text_shows(self) -> None:
        # At the real ceiling: a full working state, two warnings and a
        # last-turn line leave room for some of three long item lines. What
        # stays in the capsule is exactly what its text shows, each with its
        # attestation notice.
        def item(index: int) -> dict:
            return {
                "path": f"specs/{'x' * 240}/item-{index}.md", "layer": "semantic",
                "kind": "spec", "title": f"Item {index} " + "t" * 160, "snippet": "",
                "category": "evidence", "estimated_tokens": 50, "record_id": None,
                "conflicts": [], "attestation": "agent",
            }

        items = [item(index) for index in range(3)]
        capsule = {
            "query": "",
            "working": {
                "task_id": "TASK-" + "w" * 120, "goal": "g" * 200, "phase": None,
                "progress": "p" * 400, "next_steps": ["n" * 200] * 3,
                "files": [f"src/{'f' * 100}{index}.php" for index in range(5)],
                "sources": [],
            },
            "task_record": "project-brain/dynamic/tasks/00000000-0000-4000-8000-000000000000.md",
            "warnings": ["w" * 160] * 2, "last_turn": "l" * 600,
            "procedural": [], "semantic": items, "episodic": [],
            "selected": [dict(entry) for entry in items],
            "categories": {"evidence": [dict(entry) for entry in items]},
            "repeated": 0, "no_match": [],
            "omitted": {"procedural": 0, "semantic": 0, "episodic": 0},
        }
        full = "\n".join(context_cli.render_capsule_lines(capsule, {}))
        connection = context_cli.connect(context_cli.default_database(self.repository))
        try:
            context_cli.shown_in_render(connection, capsule)
        finally:
            connection.close()
        kept = [entry["path"] for entry in capsule["semantic"]]
        self.assertTrue(0 < len(kept) < len(items), kept)
        self.assertEqual(kept, [entry["path"] for entry in items if entry["path"] in full])
        self.assertEqual(kept, [entry["path"] for entry in capsule["selected"]])
        self.assertEqual(kept, [entry["path"] for entry in capsule["categories"]["evidence"]])
        self.assertEqual(len(items) - len(kept), capsule["omitted"]["semantic"])
        text = "\n".join(context_cli.render_capsule_lines(capsule, {}))
        self.assertLessEqual(len(text), context_cli.RENDERED_CAPSULE_LIMIT)
        for path in kept:
            line = next(line for line in text.splitlines() if path in line)
            self.assertIn("agent-attested, not reviewed by a person", line)



class RecordResultTest(RuntimeHarness):
    """One replayable write path for a run's result: MCP, the Harness and the CLI."""

    LEARNING = {
        "type": "finding",
        "title": "Cobalt authority is canonical",
        "consequence": "The cobalt authority rule decides allocation; nothing overrides it.",
        "sources": ["specs/authority.md"],
    }

    def setUp(self) -> None:
        super().setUp()
        config = self.repository / "project-brain/config/runtime.json"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(json.dumps({"mode": "governed", "automatic_promotion": True}), encoding="utf-8")
        started = self.run_cli("start", "--task-id", "TASK-RESULT", "--goal", "Check the cobalt authority", "--json")
        self.assertEqual(0, started.returncode, started.stderr)

    def task(self) -> dict:
        return brain.find_record(self.repository, "TASK-RESULT", record_type="task")[1]

    def records(self, record_type: str = "finding") -> list[dict]:
        return [record for _, record, _ in brain.iter_records(self.repository) if record["type"] == record_type]

    def record(self, result_id: str, data: dict, revision: int | None = None, task: str = "TASK-RESULT") -> dict:
        return memory_results.record_result(
            self.repository, task, result_id,
            self.task()["revision"] if revision is None else revision, data,
            owner="local", reason="Saved by the test; agent-attested, not reviewed by a person",
        )

    def result(self, **changes) -> dict:
        return {"progress": "Authority checked.", "next_steps": ["Cover release"],
                "learnings": [self.LEARNING], "verified": True, **changes}

    def test_a_result_closes_its_learnings_and_says_the_agent_attested_them(self) -> None:
        saved = self.record("run-1", self.result())
        self.assertEqual([("finding", "resolved", "created")],
                         [(item["type"], item["status"], item["state"]) for item in saved["records"]])
        self.assertFalse(saved["replayed"])
        finding = self.records()[0]
        self.assertEqual("verified", finding["authority"])
        self.assertEqual("agent", brain.record_attestation(finding))
        self.assertEqual(("Authority checked.", ["Cover release"]),
                         (self.task()["progress"], self.task()["next_steps"]))
        chunk = next((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        self.assertIn("agent-attested", chunk.read_text(encoding="utf-8"))

    def test_a_replay_writes_nothing_and_other_content_under_the_same_id_is_refused(self) -> None:
        self.record("run-1", self.result())
        revision = self.task()["revision"]
        replay = self.record("run-1", self.result())
        self.assertTrue(replay["replayed"])
        self.assertEqual(revision, self.task()["revision"])
        self.assertEqual(1, len(self.records()))
        with self.assertRaisesRegex(brain.BrainError, "different content"):
            self.record("run-1", self.result(progress="Something else."))
        # A lost receipt: the task already says what the result would write.
        shutil.rmtree(self.repository / "project-brain/local/results")
        self.assertTrue(self.record("run-1", self.result())["replayed"])
        self.assertEqual(revision, self.task()["revision"])

    def test_a_save_that_stopped_after_writing_a_learning_is_finished_by_its_replay(self) -> None:
        identifier = memory_results.learning_id(self.task()["id"], self.LEARNING)
        brain.create_record(self.repository, "finding", identifier, self.LEARNING["title"], [],
                            self.LEARNING["sources"], owner="local", authority="observed",
                            goal=self.LEARNING["consequence"])
        saved = self.record("run-1", self.result())
        self.assertEqual(["completed"], [item["state"] for item in saved["records"]])
        self.assertEqual([("resolved", "verified")], [(r["status"], r["authority"]) for r in self.records()])
        self.assertEqual("Authority checked.", self.task()["progress"])

    def test_a_learning_someone_took_further_is_left_as_it_is(self) -> None:
        # Only a record still in its initial state is a result's to finish:
        # one under investigation since is someone's later work.
        identifier = memory_results.learning_id(self.task()["id"], self.LEARNING)
        record = brain.create_record(self.repository, "finding", identifier, self.LEARNING["title"], [],
                                     self.LEARNING["sources"], owner="local", authority="observed",
                                     goal=self.LEARNING["consequence"])
        brain.update_record(self.repository, record["id"], expected_revision=record["revision"],
                            progress=None, next_steps=[], files=[], sources=[], actor="local",
                            transition_to="investigating", reason="Under investigation")
        saved = self.record("run-1", self.result())
        self.assertEqual(["existing"], [item["state"] for item in saved["records"]])
        self.assertEqual("investigating", self.records()[0]["status"])
        self.assertEqual("Authority checked.", self.task()["progress"])

    def test_a_stale_revision_writes_nothing(self) -> None:
        revision = self.task()["revision"]
        moved = self.run_cli("update", "--task-id", "TASK-RESULT", "--revision", "auto",
                             "--progress", "Moved on.", "--json")
        self.assertEqual(0, moved.returncode, moved.stderr)
        with self.assertRaisesRegex(brain.BrainError, "Stale task revision.*Nothing was written"):
            self.record("run-1", self.result(), revision=revision)
        self.assertEqual([], self.records())
        self.assertEqual("Moved on.", self.task()["progress"])

    def test_the_same_learning_from_a_later_result_names_the_same_record(self) -> None:
        self.record("run-1", self.result())
        later = self.record("run-2", self.result(progress="Release covered."))
        self.assertEqual(["existing"], [item["state"] for item in later["records"]])
        self.assertEqual(1, len(self.records()))
        self.assertEqual("Release covered.", self.task()["progress"])

    def test_sources_outside_the_project_or_in_derived_memory_are_refused_before_writes(self) -> None:
        for source in ("../outside.md", ".env", "memory-bank/chunks/x.md", "specs/missing.md"):
            with self.subTest(source=source), self.assertRaises(brain.BrainError):
                self.record("run-1", self.result(learnings=[{**self.LEARNING, "sources": [source]}]))
        with self.assertRaisesRegex(brain.BrainError, "verified=true"):
            self.record("run-1", self.result(verified=False))
        self.assertEqual([], self.records())

    def test_the_external_id_and_the_uuid_name_one_result(self) -> None:
        self.record("run-1", self.result())
        uuid = self.task()["id"]
        revision = self.task()["revision"]
        self.assertTrue(self.record("run-1", self.result(), task=uuid)["replayed"])
        with self.assertRaisesRegex(brain.BrainError, "different content"):
            self.record("run-1", self.result(progress="Something else."), task=uuid)
        self.assertEqual(1, len(self.records()))
        self.assertEqual(revision, self.task()["revision"])

    def test_an_archived_learning_is_not_written_again(self) -> None:
        self.record("run-1", self.result())
        brain.compact(self.repository)
        self.assertEqual([], self.records())
        later = self.record("run-2", self.result(progress="Release covered."))
        self.assertEqual(["archived"], [item["state"] for item in later["records"]])
        self.assertEqual([], self.records())
        archived = [record for _, record, _ in brain.iter_records(self.repository, include_archive=True)
                    if record["type"] == "finding"]
        self.assertEqual(1, len(archived))
        self.assertEqual("Release covered.", self.task()["progress"])

    def test_a_stale_revision_refuses_a_result_of_learnings_alone(self) -> None:
        revision = self.task()["revision"]
        moved = self.run_cli("update", "--task-id", "TASK-RESULT", "--revision", "auto",
                             "--progress", "Moved on.", "--json")
        self.assertEqual(0, moved.returncode, moved.stderr)
        with self.assertRaisesRegex(brain.BrainError, "Stale task revision"):
            self.record("run-1", {"learnings": [self.LEARNING], "verified": True}, revision=revision)
        self.assertEqual([], self.records())

    def test_a_replay_finishes_a_promotion_that_failed(self) -> None:
        with mock.patch.object(memory_results, "auto_promote", side_effect=brain.BrainError("disk full")):
            with self.assertRaisesRegex(brain.BrainError, "disk full"):
                self.record("run-1", self.result())
        self.assertEqual([], list((self.repository / "memory-bank/chunks").glob("MEM-*.md")))
        replay = self.record("run-1", self.result())
        self.assertTrue(replay["replayed"])
        self.assertEqual(1, len(replay["promotion"]["promoted"]))
        self.assertEqual(1, len(list((self.repository / "memory-bank/chunks").glob("MEM-*.md"))))

    def test_an_archived_agent_attested_record_keeps_its_mark_when_promoted(self) -> None:
        config = self.repository / "project-brain/config/runtime.json"
        config.write_text(json.dumps({"mode": "governed", "automatic_promotion": False}), encoding="utf-8")
        self.record("run-1", self.result())
        finding = self.records()[0]
        brain.compact(self.repository)
        proposal = brain.create_promotion(self.repository, [finding["id"]], "Cobalt authority",
                                          "The cobalt authority rule decides allocation.", proposer="local")
        brain.review_promotion(self.repository, proposal["id"], reviewer="human-reviewer", approve=True)
        brain.apply_promotion(self.repository, proposal["id"])
        chunk = next((self.repository / "memory-bank/chunks").glob("MEM-*.md"))
        self.assertIn("agent-attested", chunk.read_text(encoding="utf-8"))

    def test_a_pasted_log_or_transcript_line_is_refused(self) -> None:
        for data in (self.result(progress="stderr: synthetic migration failure"),
                     self.result(next_steps=["user: please rerun it"]),
                     self.result(learnings=[{**self.LEARNING, "consequence": "log: retries were exhausted"}])):
            with self.subTest(data=data), self.assertRaisesRegex(brain.BrainError, "log or transcript"):
                self.record("run-1", data)
        self.assertEqual([], self.records())
        self.assertEqual("", self.task()["progress"])

    def test_a_bearer_token_is_refused_by_the_writer_and_a_direct_query(self) -> None:
        token = "Zx81Kq0vLm2Np3Qr4St5Uv6Wx7"
        with self.assertRaises(brain.BrainError):
            self.record("run-1", self.result(progress=f"Called the API with Authorization: Bearer {token}."))
        with self.assertRaises(brain.BrainError):
            self.record("run-2", self.result(learnings=[{**self.LEARNING, "consequence": f"Send bearer {token} first."}]))
        self.assertEqual([], self.records())
        refused = self.run_cli("retrieve", f"why does Authorization: Bearer {token} fail", "--task-id", "TASK-RESULT")
        self.assertNotEqual(0, refused.returncode)
        manifests = self.repository / "project-brain/control/retrieval-manifests"
        for manifest in manifests.glob("*.json") if manifests.is_dir() else []:
            self.assertNotIn(token, manifest.read_text(encoding="utf-8"))

    def test_a_slack_token_or_a_customer_number_never_reaches_shared_memory(self) -> None:
        # Built here, so no literal token sits in the repository.
        token = "xox" + "b-" + "123456789012-1234567890123-" + "AbCdEfGhIjKlMnOpQrStUvWx"
        for data in (self.result(progress=f"Notified the channel with {token}."),
                     self.result(progress="Fixed the checkout timeout for customer ID 10492."),
                     self.result(learnings=[{**self.LEARNING, "consequence": "Customer #10492 needs a retry."}])):
            with self.subTest(data=data), self.assertRaises(brain.BrainError):
                self.record("run-1", data)
        self.assertEqual([], self.records())
        for query in (f"why does {token} fail", "why does customer ID 10492 time out"):
            refused = self.run_cli("retrieve", query, "--task-id", "TASK-RESULT")
            self.assertNotEqual(0, refused.returncode, query)
        prompt = f"Fix the cobalt authority timeout for customer ID 10492; notify with {token}"
        cleaned = self.run_cli("refresh", "--query", prompt, "--task-id", "TASK-RESULT", "--sanitize", "--json")
        self.assertIn(cleaned.returncode, (0, 1), cleaned.stderr)
        stored = "".join(path.read_text(encoding="utf-8") for path in self.repository.rglob("*")
                         if path.is_file() and path.suffix in (".md", ".json")
                         and ".git" not in path.relative_to(self.repository).parts)
        self.assertTrue(list((self.repository / "project-brain/control/retrieval-manifests").glob("*.json")))
        for text in (stored, cleaned.stdout):
            self.assertNotIn(token, text)
            self.assertNotIn("10492", text)

    def test_a_revised_consequence_updates_what_an_agent_attested(self) -> None:
        self.record("run-1", self.result())
        revised = self.record("run-2", self.result(learnings=[{**self.LEARNING, "consequence": "Retry once, then stop."}]))
        self.assertEqual(["updated"], [item["state"] for item in revised["records"]])
        self.assertFalse(revised["replayed"])
        self.assertEqual(1, len(self.records()))
        self.assertEqual("Retry once, then stop.", self.records()[0]["progress"])

    def test_an_agent_does_not_overwrite_what_a_person_verified(self) -> None:
        memory_results.record_result(self.repository, "TASK-RESULT", "run-1", self.task()["revision"],
                                     self.result(), owner="local", reason="Reviewed by a person",
                                     attestation="person")
        revised = self.record("run-2", self.result(learnings=[{**self.LEARNING, "consequence": "Retry once, then stop."}]))
        self.assertEqual(["differs"], [item["state"] for item in revised["records"]])
        self.assertFalse(revised["replayed"])
        self.assertEqual(self.LEARNING["consequence"], self.records()[0]["progress"])
        brain.compact(self.repository)
        archived = self.record("run-3", self.result(learnings=[{**self.LEARNING, "consequence": "Retry twice."}]))
        self.assertEqual(["archived-differs"], [item["state"] for item in archived["records"]])
        self.assertFalse(archived["replayed"])

    def test_the_cli_records_a_result_from_a_file(self) -> None:
        request = self.repository / "result.json"
        request.write_text(json.dumps(self.result()), encoding="utf-8")
        first = self.run_cli("record-result", "--task-id", "TASK-RESULT", "--result-id", "cli-1",
                             "--revision", "auto", "--input", str(request), "--json")
        self.assertEqual(0, first.returncode, first.stderr)
        self.assertFalse(json.loads(first.stdout)["replayed"])
        second = self.run_cli("record-result", "--task-id", "TASK-RESULT", "--result-id", "cli-1",
                              "--revision", "auto", "--input", str(request))
        self.assertEqual(0, second.returncode, second.stderr)
        self.assertIn("Result already recorded", second.stdout)
        self.assertEqual(1, len(self.records()))

    def test_the_capsule_says_which_knowledge_only_an_agent_checked(self) -> None:
        self.record("run-1", self.result())
        refreshed = self.run_cli("refresh", "--query", "cobalt authority canonical allocation",
                                 "--task-id", "TASK-RESULT", "--ephemeral")
        self.assertEqual(0, refreshed.returncode, refreshed.stderr)
        self.assertRegex(refreshed.stdout, r"Cobalt authority is canonical[^\n]*agent-attested, not reviewed by a person")

    def chunks(self) -> dict:
        found = {}
        for path in sorted((self.repository / "memory-bank/chunks").glob("MEM-*.md")):
            metadata, body = brain._parse_chunk(path)
            found[metadata["id"]] = {**metadata, "body": body}
        return found

    def automatic(self, enabled: bool) -> None:
        config = self.repository / "project-brain/config/runtime.json"
        config.write_text(json.dumps({"mode": "governed", "automatic_promotion": enabled}), encoding="utf-8")

    def stalled(self, finding: dict) -> dict:
        proposal = brain.create_promotion(self.repository, [finding["id"]], finding["title"],
                                          brain.promotion_content(finding), proposer="local",
                                          review_mode="automatic")
        return brain.auto_review_promotion(self.repository, proposal["id"])

    def test_a_delayed_replay_of_an_older_result_leaves_the_later_revision(self) -> None:
        self.record("run-1", self.result())
        self.record("run-2", self.result(learnings=[{**self.LEARNING, "consequence": "Retry once, then stop."}]))
        revised = self.records()[0]
        replay = self.record("run-1", self.result())
        self.assertTrue(replay["replayed"])
        self.assertEqual(["differs"], [item["state"] for item in replay["records"]])
        self.assertEqual(("Retry once, then stop.", revised["revision"]),
                         (self.records()[0]["progress"], self.records()[0]["revision"]))

    def test_a_persons_correction_is_theirs_and_no_agent_overwrites_it(self) -> None:
        self.record("run-1", self.result())
        corrected = {**self.LEARNING, "consequence": "Retry once, then stop."}
        person = memory_results.record_result(
            self.repository, "TASK-RESULT", "run-2", self.task()["revision"],
            self.result(learnings=[corrected]), owner="local", reason="Corrected by a person",
            attestation="person")
        self.assertEqual(["updated"], [item["state"] for item in person["records"]])
        finding = self.records()[0]
        self.assertEqual("person", brain.record_attestation(finding))
        brain.validate_record(finding)
        later = self.record("run-3", self.result(learnings=[{**self.LEARNING, "consequence": "Retry forever."}]))
        self.assertEqual(["differs"], [item["state"] for item in later["records"]])
        self.assertEqual("Retry once, then stop.", self.records()[0]["progress"])

    def test_a_revised_learning_supersedes_the_chunk_promoted_before(self) -> None:
        self.record("run-1", self.result())
        [first] = self.chunks()
        revised = {**self.LEARNING,
                   "consequence": "Retry once, then stop; only after that does the cobalt rule decide allocation."}
        saved = self.record("run-2", self.result(learnings=[revised]))
        self.assertEqual(1, len(saved["promotion"]["promoted"]), saved["promotion"])
        successor = saved["promotion"]["promoted"][0]["memory_id"]
        chunks = self.chunks()
        self.assertEqual({first, successor}, set(chunks))
        self.assertEqual(("superseded", successor), (chunks[first]["status"], chunks[first]["superseded_by"]))
        self.assertEqual(("active", [first]), (chunks[successor]["status"], chunks[successor]["supersedes"]))
        self.assertIn("Retry once, then stop", chunks[successor]["body"])
        self.assertIn("agent-attested", chunks[successor]["tags"])
        self.assertEqual([], brain.validate_bank(self.repository / "memory-bank"))
        replay = self.record("run-2", self.result(learnings=[revised]))
        self.assertEqual(([], [], []), (replay["promotion"]["promoted"], replay["promotion"]["failed"],
                                        replay["promotion"]["closed"]))
        self.assertEqual({first, successor}, set(self.chunks()))

    def test_a_revision_of_what_a_person_promoted_waits_for_a_person(self) -> None:
        self.automatic(False)
        self.record("run-1", self.result())
        finding = self.records()[0]
        proposal = brain.create_promotion(self.repository, [finding["id"]], finding["title"],
                                          brain.promotion_content(finding), proposer="local")
        brain.review_promotion(self.repository, proposal["id"], reviewer="human-reviewer", approve=True)
        brain.apply_promotion(self.repository, proposal["id"])
        self.automatic(True)
        saved = self.record("run-2", self.result(learnings=[{**self.LEARNING, "consequence": "Retry once, then stop."}]))
        self.assertEqual([], saved["promotion"]["promoted"])
        self.assertTrue(any("person's review" in item["reason"] for item in saved["promotion"]["blocked"]),
                        saved["promotion"]["blocked"])
        self.assertEqual(["active"], [chunk["status"] for chunk in self.chunks().values()])

    def test_two_promotion_runs_at_once_write_one_proposal_and_one_chunk(self) -> None:
        self.automatic(False)
        self.record("run-1", self.result())
        self.automatic(True)
        original = brain.create_promotion
        other: dict = {}

        def racing(*arguments, **keywords):
            # The other run starts while this one is choosing; before the
            # fix it finished in the gap and both wrote a proposal.
            if "thread" not in other:
                other["thread"] = threading.Thread(
                    target=lambda: other.update(result=brain.auto_promote(self.repository, owner="local")))
                other["thread"].start()
                other["thread"].join(timeout=0.5)
            return original(*arguments, **keywords)

        with mock.patch.object(brain, "create_promotion", side_effect=racing):
            first = brain.auto_promote(self.repository, owner="local")
        other["thread"].join(timeout=30)
        self.assertFalse(other["thread"].is_alive())
        self.assertEqual(["applied"], [proposal["status"] for _, proposal in brain.iter_promotions(self.repository)])
        self.assertEqual(1, len(self.chunks()))
        self.assertEqual([], first["failed"] + other["result"]["failed"])
        self.assertEqual(1, len(first["promoted"] + other["result"]["promoted"]))

    def test_a_stalled_promotion_whose_record_moved_on_is_withdrawn(self) -> None:
        self.automatic(False)
        self.record("run-1", self.result())
        proposal = self.stalled(self.records()[0])
        self.record("run-2", self.result(learnings=[{**self.LEARNING, "consequence": "Retry once, then stop."}]))
        self.automatic(True)
        first = brain.auto_promote(self.repository, owner="local")
        self.assertEqual([], first["failed"])
        self.assertEqual([proposal["id"]], [item["promotion_id"] for item in first["closed"]])
        self.assertEqual(1, len(first["promoted"]))
        second = brain.auto_promote(self.repository, owner="local")
        self.assertEqual(([], [], []), (second["promoted"], second["failed"], second["closed"]))
        self.assertEqual(["applied", "rejected"],
                         sorted(proposal["status"] for _, proposal in brain.iter_promotions(self.repository)))
        self.assertIn("Retry once, then stop", next(iter(self.chunks().values()))["body"])

    def test_a_second_proposal_of_a_promoted_revision_is_withdrawn_not_retried(self) -> None:
        self.automatic(False)
        self.record("run-1", self.result())
        finding = self.records()[0]
        winner, loser = self.stalled(finding), self.stalled(finding)
        brain.apply_promotion(self.repository, winner["id"])
        self.automatic(True)
        result = brain.auto_promote(self.repository, owner="local")
        self.assertEqual(([], []), (result["failed"], result["promoted"]))
        self.assertEqual([loser["id"]], [item["promotion_id"] for item in result["closed"]])
        self.assertEqual(1, len(self.chunks()))
        self.assertEqual([], brain.auto_promote(self.repository, owner="local")["failed"])

    def test_an_older_unattended_save_still_reads_as_the_agents(self) -> None:
        record = {"transitions": [
            {"from": None, "to": "open", "reason": "Finding created"},
            {"from": "observed", "to": "verified",
             "reason": "Saved automatically when a Harness run completed; agent-attested, not reviewed by a person"},
        ]}
        self.assertEqual("agent", brain.record_attestation(record))
        record["transitions"][-1]["reason"] = "Verified by Dana [attestation:person]"
        self.assertEqual("person", brain.record_attestation(record))
        record["transitions"].pop()
        self.assertEqual("", brain.record_attestation(record))


class SmallIndexRarityTest(RuntimeHarness):
    def test_an_identifier_in_one_document_of_a_small_index_is_still_rare(self) -> None:
        self.repository.joinpath("specs/allocation.md").write_text(
            "# Allocation\n\nCMS768 needs one owner.\n", encoding="utf-8")
        self.repository.joinpath("specs/other.md").write_text(
            "# Other\n\nUnrelated notes about allocation.\n", encoding="utf-8")
        self.assertEqual(0, self.run_cli("index", "--json").returncode)
        connection = context_cli.connect(context_cli.default_database(self.repository))
        try:
            total = connection.execute("SELECT COUNT(*) FROM documents").fetchone()[0]
            self.assertLess(total, 10)
            coverage, distinctive = retrieval.token_coverage(connection, ["cms768", "owner"])
        finally:
            connection.close()
        self.assertIn("specs/allocation.md", distinctive)


if __name__ == "__main__":
    unittest.main()
