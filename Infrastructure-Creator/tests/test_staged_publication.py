#!/usr/bin/env python3
"""Regression tests for explicit, rollback-capable staged publication."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / ".agents/skills/bootstrap-verifier/scripts"
sys.path.insert(0, str(SCRIPTS))

import publish_staging as publication  # noqa: E402
import infra_ownership as ownership  # noqa: E402
from publish_staging import (  # noqa: E402
    PublicationError,
    build_snapshot,
    planned_paths,
    publish,
    restore,
    verify_baseline,
)


class StagedPublicationTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="infra-publication-")
        root = Path(self.temp.name).resolve()
        self.target = root / "target"
        self.staging = root / "staging"
        self.target.mkdir()
        self.staging.mkdir()
        self.plan = root / "publication-plan.txt"
        self.plan.write_text("config/policy.md\nruntime/seed.md\n", encoding="utf-8")
        (self.target / "config").mkdir()
        (self.target / "config/policy.md").write_text("old\n", encoding="utf-8")
        for rel, content in (
            ("config/policy.md", "new\n"),
            ("runtime/seed.md", "seed\n"),
        ):
            path = self.staging / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        self.refresh_staged_manifest()

    def tearDown(self) -> None:
        self.temp.cleanup()

    def refresh_staged_manifest(self) -> None:
        """Write a staged manifest owning every manifest-ownable staged file."""
        files = {
            path.relative_to(self.staging).as_posix(): ownership.sha256_file(path)
            for path in sorted(self.staging.rglob("*"))
            if path.is_file()
            and path.name != ownership.MANIFEST_NAME
            and ownership.may_be_manifest_owned(
                path.relative_to(self.staging).as_posix()
            )
        }
        (self.staging / ownership.MANIFEST_NAME).write_text(
            json.dumps({"manifest_version": 1, "files": files}, indent=2) + "\n",
            encoding="utf-8",
        )

    def write_target_manifest(self, files: dict) -> None:
        (self.target / ownership.MANIFEST_NAME).write_text(
            json.dumps({"manifest_version": 1, "files": files}, indent=2) + "\n",
            encoding="utf-8",
        )

    def test_publish_then_rollback_restores_exact_baseline(self) -> None:
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "journal").resolve()

        publish(self.target, self.staging, paths, snapshot, journal)

        self.assertEqual(
            (self.target / "config/policy.md").read_text(encoding="utf-8"), "new\n"
        )
        self.assertEqual(
            (self.target / "runtime/seed.md").read_text(encoding="utf-8"), "seed\n"
        )
        self.assertTrue((self.target / ".infra-manifest.json").is_file())
        metadata = json.loads(
            (journal / "journal.json").read_text(encoding="utf-8")
        )
        self.assertEqual(metadata["status"], "published-pending-verification")

        restore(self.target, journal)

        self.assertEqual(
            (self.target / "config/policy.md").read_text(encoding="utf-8"), "old\n"
        )
        self.assertFalse((self.target / "runtime/seed.md").exists())
        self.assertFalse((self.target / "runtime").exists())
        self.assertTrue((self.target / "config").is_dir())
        self.assertFalse((self.target / ".infra-manifest.json").exists())

    def test_changed_file_rollback_restores_exact_bytes_and_mode(self) -> None:
        changed = self.target / "config/policy.md"
        original = b"\xffteam\r\nwithout-final-newline"
        changed.write_bytes(original)
        os.chmod(changed, 0o640)
        staged = self.staging / "config/policy.md"
        os.chmod(staged, 0o755)
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "mode-journal").resolve()

        publish(self.target, self.staging, paths, snapshot, journal)
        self.assertNotEqual(changed.read_bytes(), original)
        self.assertEqual(changed.stat().st_mode & 0o777, 0o755)

        restore(self.target, journal)
        self.assertEqual(changed.read_bytes(), original)
        self.assertEqual(changed.stat().st_mode & 0o777, 0o640)

    def test_target_drift_refuses_publication(self) -> None:
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        (self.target / "config/policy.md").write_text("team edit\n", encoding="utf-8")

        with self.assertRaisesRegex(PublicationError, "target changed"):
            verify_baseline(self.target, paths, snapshot)

    def test_manifest_is_always_part_of_publication(self) -> None:
        paths = planned_paths(self.plan)
        self.assertEqual(paths[-1], ".infra-manifest.json")

    def test_unplanned_team_file_is_preserved(self) -> None:
        team_file = self.target / "config/team-owned.md"
        team_file.write_text("team\n", encoding="utf-8")
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "merge-journal").resolve()

        publish(self.target, self.staging, paths, snapshot, journal)

        self.assertEqual(team_file.read_text(encoding="utf-8"), "team\n")

    def test_baseline_only_path_is_checked_but_never_published_or_journaled(self) -> None:
        watched = self.target / ".gitignore"
        watched.write_bytes(b"team\r\n")
        os.chmod(watched, 0o640)
        staged_watch = self.staging / ".gitignore"
        staged_watch.write_text("must not publish\n", encoding="utf-8")
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths, [".gitignore"])
        journal = (Path(self.temp.name) / "watch-journal").resolve()

        publish(
            self.target,
            self.staging,
            paths,
            snapshot,
            journal,
            baseline_only=[".gitignore"],
        )

        self.assertEqual(watched.read_bytes(), b"team\r\n")
        self.assertEqual(watched.stat().st_mode & 0o777, 0o640)
        metadata = json.loads(
            (journal / "journal.json").read_text(encoding="utf-8")
        )
        self.assertNotIn(".gitignore", metadata["paths"])
        self.assertNotIn(".gitignore", metadata["baseline"])
        self.assertFalse((journal / "backups/.gitignore").exists())

    def test_watched_drift_aborts_before_manifest_publication(self) -> None:
        watched = self.target / ".gitignore"
        watched.write_text("baseline\n", encoding="utf-8")
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths, [".gitignore"])
        watched.write_text("team drift\n", encoding="utf-8")
        journal = (Path(self.temp.name) / "drift-journal").resolve()

        with self.assertRaisesRegex(PublicationError, "target changed"):
            publish(
                self.target,
                self.staging,
                paths,
                snapshot,
                journal,
                baseline_only=[".gitignore"],
            )

        self.assertFalse((self.target / ".infra-manifest.json").exists())
        self.assertFalse(journal.exists())
        self.assertEqual(
            (self.target / "config/policy.md").read_text(encoding="utf-8"), "old\n"
        )

    def test_baseline_only_overlap_is_rejected(self) -> None:
        paths = planned_paths(self.plan)
        with self.assertRaisesRegex(PublicationError, "overlap baseline-only"):
            build_snapshot(self.target, paths, ["config/policy.md"])

    def test_copy_failure_rolls_back_partial_publication(self) -> None:
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "failed-journal").resolve()
        real_copy = publication.atomic_copy

        def fail_on_seed(source: Path, destination: Path) -> None:
            if destination.name == "seed.md":
                raise OSError("injected copy failure")
            real_copy(source, destination)

        with mock.patch.object(publication, "atomic_copy", side_effect=fail_on_seed):
            with self.assertRaisesRegex(OSError, "injected copy failure"):
                publish(self.target, self.staging, paths, snapshot, journal)

        self.assertEqual(
            (self.target / "config/policy.md").read_text(encoding="utf-8"), "old\n"
        )
        self.assertFalse((self.target / "runtime/seed.md").exists())
        self.assertFalse((self.target / ".infra-manifest.json").exists())

    def test_approved_removal_is_rollback_capable(self) -> None:
        obsolete = self.target / "config/obsolete.md"
        obsolete.write_text("restore me\n", encoding="utf-8")
        self.write_target_manifest(
            {"config/obsolete.md": ownership.sha256_file(obsolete)}
        )
        paths = planned_paths(self.plan)
        removals = ["config/obsolete.md"]
        snapshot = build_snapshot(self.target, paths + removals)
        journal = (Path(self.temp.name) / "removal-journal").resolve()

        publish(
            self.target,
            self.staging,
            paths,
            snapshot,
            journal,
            removals,
        )
        self.assertFalse(obsolete.exists())

        restore(self.target, journal)
        self.assertEqual(obsolete.read_text(encoding="utf-8"), "restore me\n")

    def test_publish_refuses_to_overwrite_runtime_state(self) -> None:
        memory = self.target / "memory-bank/local/notes.md"
        memory.parent.mkdir(parents=True)
        memory.write_text("your live memory\n", encoding="utf-8")
        staged = self.staging / "memory-bank/local/notes.md"
        staged.parent.mkdir(parents=True)
        staged.write_text("generator overwrote your memory\n", encoding="utf-8")
        self.refresh_staged_manifest()
        self.plan.write_text(
            "config/policy.md\nruntime/seed.md\nmemory-bank/local/notes.md\n",
            encoding="utf-8",
        )
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "runtime-journal").resolve()

        with self.assertRaisesRegex(
            PublicationError, "non-ownable runtime state"
        ):
            publish(self.target, self.staging, paths, snapshot, journal)

        self.assertEqual(
            memory.read_text(encoding="utf-8"), "your live memory\n"
        )
        self.assertEqual(
            (self.target / "config/policy.md").read_text(encoding="utf-8"), "old\n"
        )
        self.assertFalse(journal.exists())
        self.assertFalse((self.target / ".infra-manifest.json").exists())

    def test_publish_seeds_runtime_state_only_into_absent_paths(self) -> None:
        staged = self.staging / "memory-bank/local/notes.md"
        staged.parent.mkdir(parents=True)
        staged.write_text("initial seed\n", encoding="utf-8")
        self.refresh_staged_manifest()
        self.plan.write_text(
            "config/policy.md\nruntime/seed.md\nmemory-bank/local/notes.md\n",
            encoding="utf-8",
        )
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "seed-journal").resolve()

        publish(self.target, self.staging, paths, snapshot, journal)

        self.assertEqual(
            (self.target / "memory-bank/local/notes.md").read_text(
                encoding="utf-8"
            ),
            "initial seed\n",
        )

    def test_publish_refuses_paths_absent_from_staged_manifest(self) -> None:
        extra = self.staging / "unowned-extra.md"
        extra.write_text("orphan\n", encoding="utf-8")
        self.plan.write_text(
            "config/policy.md\nruntime/seed.md\nunowned-extra.md\n",
            encoding="utf-8",
        )
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "unowned-journal").resolve()

        with self.assertRaisesRegex(
            PublicationError, "missing from the staged manifest: unowned-extra.md"
        ):
            publish(self.target, self.staging, paths, snapshot, journal)

        self.assertFalse((self.target / "unowned-extra.md").exists())
        self.assertEqual(
            (self.target / "config/policy.md").read_text(encoding="utf-8"), "old\n"
        )
        self.assertFalse(journal.exists())

    def test_publish_refuses_removal_of_unmanifested_team_file(self) -> None:
        team_file = self.target / "config/team-owned.md"
        team_file.write_text("team\n", encoding="utf-8")
        self.write_target_manifest({})
        paths = planned_paths(self.plan)
        removals = ["config/team-owned.md"]
        snapshot = build_snapshot(self.target, paths + removals)
        journal = (Path(self.temp.name) / "team-removal-journal").resolve()

        with self.assertRaisesRegex(
            PublicationError,
            "not owned by the target manifest: config/team-owned.md",
        ):
            publish(self.target, self.staging, paths, snapshot, journal, removals)

        self.assertEqual(team_file.read_text(encoding="utf-8"), "team\n")
        self.assertFalse(journal.exists())

    def test_publish_refuses_removals_when_target_has_no_manifest(self) -> None:
        team_file = self.target / "config/team-owned.md"
        team_file.write_text("team\n", encoding="utf-8")
        paths = planned_paths(self.plan)
        removals = ["config/team-owned.md"]
        snapshot = build_snapshot(self.target, paths + removals)
        journal = (Path(self.temp.name) / "legacy-removal-journal").resolve()

        with self.assertRaisesRegex(PublicationError, "target manifest missing"):
            publish(self.target, self.staging, paths, snapshot, journal, removals)

        self.assertEqual(team_file.read_text(encoding="utf-8"), "team\n")
        self.assertFalse(journal.exists())

    def test_rollback_removes_directory_chains_publish_created(self) -> None:
        staged = self.staging / "deep/nested/dir/file.md"
        staged.parent.mkdir(parents=True)
        staged.write_text("generated\n", encoding="utf-8")
        self.refresh_staged_manifest()
        self.plan.write_text(
            "config/policy.md\nruntime/seed.md\ndeep/nested/dir/file.md\n",
            encoding="utf-8",
        )
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "dirs-journal").resolve()

        publish(self.target, self.staging, paths, snapshot, journal)
        self.assertTrue((self.target / "deep/nested/dir/file.md").is_file())

        restore(self.target, journal)
        self.assertFalse((self.target / "deep").exists())
        self.assertFalse((self.target / "runtime").exists())
        self.assertTrue((self.target / "config").is_dir())

    def test_rollback_keeps_created_directory_holding_team_content(self) -> None:
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "kept-dir-journal").resolve()

        publish(self.target, self.staging, paths, snapshot, journal)
        team_file = self.target / "runtime/team-added.md"
        team_file.write_text("team\n", encoding="utf-8")

        restore(self.target, journal)
        self.assertFalse((self.target / "runtime/seed.md").exists())
        self.assertEqual(team_file.read_text(encoding="utf-8"), "team\n")
        self.assertTrue((self.target / "runtime").is_dir())

    def test_rollback_preserves_third_party_edits_and_reports_conflict(self) -> None:
        paths = planned_paths(self.plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "conflict-journal").resolve()

        publish(self.target, self.staging, paths, snapshot, journal)
        edited = self.target / "config/policy.md"
        edited.write_text("post-publish team edit\n", encoding="utf-8")

        with self.assertRaisesRegex(
            PublicationError, "third-party edits.*config/policy.md"
        ):
            restore(self.target, journal)

        self.assertEqual(
            edited.read_text(encoding="utf-8"), "post-publish team edit\n"
        )
        self.assertFalse((self.target / "runtime/seed.md").exists())
        self.assertFalse((self.target / ".infra-manifest.json").exists())
        metadata = json.loads(
            (journal / "journal.json").read_text(encoding="utf-8")
        )
        self.assertEqual(metadata["status"], "rolled-back-with-conflicts")
        self.assertEqual(metadata["conflicts"], ["config/policy.md"])

    def test_update_cli_persists_keep_and_merge_final_hashes(self) -> None:
        kept_rel = ".claude/hooks/local-context.sh"
        merged_rel = ".claude/settings.json"
        agents_rel = "AGENTS.md"
        kept = self.target / kept_rel
        kept.parent.mkdir(parents=True)
        kept.write_text("team-owned hook\n", encoding="utf-8")
        rejected = self.staging / kept_rel
        rejected.parent.mkdir(parents=True)
        rejected.write_text("generated hook v2\n", encoding="utf-8")
        merged_target = self.target / merged_rel
        merged_target.write_text('{"team": true}\n', encoding="utf-8")
        merged = self.staging / merged_rel
        merged.write_text(
            '{"team": true, "generated": true}\n', encoding="utf-8"
        )
        agents = self.staging / agents_rel
        agents.write_text(
            "<!-- Generated by Infrastructure-Creator v2.5.0 | "
            "TASK-002 | 2026-08-14 -->\npolicy\n",
            encoding="utf-8",
        )
        write_plan = [agents_rel, kept_rel, merged_rel]
        decisions = {
            kept_rel: {
                "decision": "kept",
                "rejected_sha256": ownership.sha256_file(rejected),
                "task": "TASK-002",
            },
            merged_rel: {
                "decision": "merged",
                "rejected_sha256": "b" * 64,
                "task": "TASK-002",
            },
        }
        update_plan = Path(self.temp.name).resolve() / "update-write-plan.txt"
        update_plan.write_text("\n".join(write_plan) + "\n", encoding="utf-8")
        decisions_path = Path(self.temp.name).resolve() / "decisions.json"
        decisions_path.write_text(
            json.dumps(decisions) + "\n", encoding="utf-8"
        )
        source_map_path = Path(self.temp.name).resolve() / "sources.json"
        source_map_path.write_text(
            json.dumps({kept_rel: "target"}) + "\n", encoding="utf-8"
        )
        result = subprocess.run(
            [
                sys.executable,
                str(SCRIPTS / "infra_ownership.py"),
                "manifest",
                "--target",
                str(self.staging),
                "--write-plan",
                str(update_plan),
                "--version",
                "2.5.0",
                "--task",
                "TASK-002",
                "--profile",
                "tasks/TASK-002/infra-scan-project-profile.md",
                "--editions",
                "claude",
                "--mode",
                "full",
                "--source-target",
                str(self.target),
                "--source-map",
                str(source_map_path),
                "--decisions",
                str(decisions_path),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        publication_plan = Path(self.temp.name).resolve() / "update-publication.txt"
        publication_plan.write_text(
            f"{agents_rel}\n{merged_rel}\n", encoding="utf-8"
        )
        paths = planned_paths(publication_plan)
        snapshot = build_snapshot(self.target, paths)
        journal = (Path(self.temp.name) / "decision-journal").resolve()

        publish(self.target, self.staging, paths, snapshot, journal)

        refreshed = ownership.load_manifest(self.target)
        self.assertEqual(
            refreshed["files"][kept_rel], ownership.sha256_file(kept)
        )
        self.assertEqual(kept.read_text(encoding="utf-8"), "team-owned hook\n")
        self.assertEqual(
            merged_target.read_text(encoding="utf-8"),
            '{"team": true, "generated": true}\n',
        )
        self.assertEqual(
            refreshed["files"][merged_rel], ownership.sha256_file(merged_target)
        )
        self.assertEqual(
            refreshed["decisions"][merged_rel]["decision"], "merged"
        )
        rows = ownership.classify_update(self.target, self.staging, refreshed)
        self.assertEqual(
            [item["path"] for item in rows],
            sorted(
                [
                    agents_rel,
                    kept_rel,
                    merged_rel,
                    "config/policy.md",
                    "runtime/seed.md",
                ]
            ),
            "the staged manifest itself must never surface as a classify row",
        )
        row = next(item for item in rows if item["path"] == kept_rel)
        self.assertEqual(row["classification"], "standing-decision-honored")


if __name__ == "__main__":
    unittest.main()
