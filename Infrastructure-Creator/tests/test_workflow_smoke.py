#!/usr/bin/env python3
"""Model-free reliability workflow and generated-edition parity rehearsal."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests" / "fixtures" / "workflow-smoke" / "input.json"
RUNTIME_ASSETS = (
    ROOT / ".agents" / "skills" / "memory-seed" / "assets" / "scripts"
)
BOOTSTRAP_SCRIPTS = (
    ROOT / ".agents" / "skills" / "bootstrap-verifier" / "scripts"
)
sys.path.insert(0, str(BOOTSTRAP_SCRIPTS))
SKILL_TEST_PATH = ROOT / "tests" / "test_skill_quality.py"
SKILL_TEST_SPEC = importlib.util.spec_from_file_location(
    "workflow_skill_quality_fixture", SKILL_TEST_PATH
)
assert SKILL_TEST_SPEC and SKILL_TEST_SPEC.loader
skill_test_module = importlib.util.module_from_spec(SKILL_TEST_SPEC)
sys.modules[SKILL_TEST_SPEC.name] = skill_test_module
SKILL_TEST_SPEC.loader.exec_module(skill_test_module)
SkillQualityFixture = skill_test_module.SkillQualityFixture
validator = skill_test_module.validator
PUBLICATION_SPEC = importlib.util.spec_from_file_location(
    "workflow_publication", BOOTSTRAP_SCRIPTS / "publish_staging.py"
)
assert PUBLICATION_SPEC and PUBLICATION_SPEC.loader
publication = importlib.util.module_from_spec(PUBLICATION_SPEC)
sys.modules[PUBLICATION_SPEC.name] = publication
PUBLICATION_SPEC.loader.exec_module(publication)


def blocking(diagnostics: list) -> list:
    return [item for item in diagnostics if item.severity == "error"]


class WorkflowSmokeTests(unittest.TestCase):
    def load_fixture(self) -> dict:
        return json.loads(FIXTURE.read_text(encoding="utf-8"))

    def run_plan_sequence(self) -> dict:
        case = SkillQualityFixture(methodName="runTest")
        case.setUp()
        try:
            case.use_schema_1_4()
            case.rewrite()
            approval = case.plan_diagnostics()
            self.assertFalse(blocking(approval))

            second = case.plan["skills"][1]["name"]
            second_path = case.skills / second / "SKILL.md"
            second_text = second_path.read_text(encoding="utf-8")
            second_path.unlink()
            partial = validator.validate(
                case.skills,
                case.plan_path,
                case.target,
                case.registry_path,
                allow_partial_skills=True,
            )
            self.assertFalse(blocking(partial))
            final_incomplete = validator.validate(
                case.skills, case.plan_path, case.target, case.registry_path
            )
            self.assertIn(
                "SKILL_FILE_MISSING", [item.code for item in final_incomplete]
            )

            second_path.parent.mkdir(parents=True, exist_ok=True)
            second_path.write_text(second_text, encoding="utf-8")
            final = validator.validate(
                case.skills, case.plan_path, case.target, case.registry_path
            )
            self.assertFalse(blocking(final))
            return {
                "approval": [item.as_dict() for item in approval],
                "partial": [item.as_dict() for item in partial],
                "final_incomplete": [item.as_dict() for item in final_incomplete],
                "final": [item.as_dict() for item in final],
            }
        finally:
            case.doCleanups()

    def make_parity_target(self, root: Path, editions: list[str]) -> dict:
        scripts = root / "memory-bank" / "scripts"
        scripts.mkdir(parents=True)
        for name in (
            "context.py",
            "brain_runtime.py",
            "context_retrieval.py",
            "validate.py",
        ):
            shutil.copy2(RUNTIME_ASSETS / name, scripts / name)

        canonical = (
            ".agents"
            if "codex" in editions
            else ".claude"
            if "claude" in editions
            else ".cursor"
        )
        config = root / "project-brain" / "config"
        config.mkdir(parents=True)
        (config / "runtime.json").write_text(
            json.dumps(
                {
                    "framework": "generic",
                    "canonical_edition": canonical,
                    "mode": "governed",
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        content = (
            "---\nname: workflow-check\n"
            "description: Verify deterministic generated-edition parity.\n"
            "---\n# Workflow Check\n"
        )
        roots = {"claude": ".claude", "cursor": ".cursor", "codex": ".agents"}
        for edition in editions:
            path = root / roots[edition] / "skills" / "workflow-check" / "SKILL.md"
            path.parent.mkdir(parents=True)
            path.write_text(content, encoding="utf-8")

        command = [
            sys.executable,
            str(scripts / "context.py"),
            "parity",
            "--json",
        ]
        result = subprocess.run(
            command, cwd=root, text=True, capture_output=True, check=False
        )
        self.assertEqual(result.returncode, 0, result.stderr or result.stdout)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["valid"])
        self.assertEqual(payload["canonical_edition"], canonical)
        return payload

    def run_publication_rehearsal(self, root: Path, fixture: dict) -> dict:
        target = root / "published-target"
        staging = root / "staging"
        journal = root / "rollback"
        generated = Path(fixture["generated_file"])
        team = Path(fixture["team_file"])
        target_generated = target / generated
        target_team = target / team
        staged_generated = staging / generated
        staged_manifest = staging / ".infra-manifest.json"
        target_generated.parent.mkdir(parents=True)
        target_team.parent.mkdir(parents=True, exist_ok=True)
        staged_generated.parent.mkdir(parents=True)
        target = target.resolve()
        staging = staging.resolve()
        journal = journal.resolve()
        target_generated = target / generated
        target_team = target / team
        staged_generated = staging / generated
        staged_manifest = staging / ".infra-manifest.json"
        target_generated.write_text("previous policy\n", encoding="utf-8")
        target_generated.chmod(0o640)
        target_team.write_text(fixture["team_content"], encoding="utf-8")
        staged_generated.write_text(fixture["generated_content"], encoding="utf-8")
        staged_manifest.write_text(
            json.dumps(
                {
                    "manifest_version": 1,
                    "files": {
                        generated.as_posix(): hashlib.sha256(
                            fixture["generated_content"].encode("utf-8")
                        ).hexdigest()
                    },
                }
            )
            + "\n",
            encoding="utf-8",
        )

        paths = [generated.as_posix(), ".infra-manifest.json"]
        watched = [team.as_posix()]
        snapshot = publication.build_snapshot(target, paths, watched)
        target_team.write_text("concurrent team edit\n", encoding="utf-8")
        with self.assertRaises(publication.PublicationError):
            publication.publish(
                target, staging, paths, snapshot, journal, baseline_only=watched
            )
        journal_after_drift = journal.exists()
        self.assertFalse(journal_after_drift)
        target_team.write_text(fixture["team_content"], encoding="utf-8")
        snapshot = publication.build_snapshot(target, paths, watched)
        publication.publish(
            target, staging, paths, snapshot, journal, baseline_only=watched
        )
        published_digest = hashlib.sha256(target_generated.read_bytes()).hexdigest()
        watched_digest = hashlib.sha256(target_team.read_bytes()).hexdigest()
        watched_backup_exists = (journal / "backups" / team).exists()
        self.assertEqual(
            target_generated.read_text(encoding="utf-8"),
            fixture["generated_content"],
        )
        self.assertEqual(
            target_team.read_text(encoding="utf-8"), fixture["team_content"]
        )
        self.assertFalse(watched_backup_exists)

        publication.restore(target, journal)
        restored_bytes = target_generated.read_bytes()
        restored_mode = oct(target_generated.stat().st_mode & 0o777)
        manifest_after_restore = (target / ".infra-manifest.json").exists()
        self.assertEqual(restored_bytes.decode("utf-8"), "previous policy\n")
        self.assertEqual(restored_mode, "0o640")
        self.assertFalse(manifest_after_restore)
        # Every value below is observed from the rehearsal filesystem so the
        # repeated-run hash compares recomputed outcomes, not constants.
        return {
            "drift_journal_exists": journal_after_drift,
            "published_digest": published_digest,
            "watched_digest": watched_digest,
            "watched_backup_exists": watched_backup_exists,
            "restored_digest": hashlib.sha256(restored_bytes).hexdigest(),
            "restored_mode": restored_mode,
            "manifest_exists_after_restore": manifest_after_restore,
        }

    def run_scenario(self) -> dict:
        fixture = self.load_fixture()
        with tempfile.TemporaryDirectory(prefix="workflow-smoke-") as temporary:
            base = Path(temporary)
            parity = []
            for index, editions in enumerate(fixture["editions"]):
                parity.append(
                    self.make_parity_target(base / f"target-{index}", editions)
                )
            publication_result = self.run_publication_rehearsal(base, fixture)
            return {
                "plan": self.run_plan_sequence(),
                "parity": parity,
                "publication": publication_result,
            }

    def test_workflow_is_repeatable_and_all_editions_reach_parity(self) -> None:
        first = json.dumps(self.run_scenario(), sort_keys=True)
        second = json.dumps(self.run_scenario(), sort_keys=True)
        self.assertEqual(
            hashlib.sha256(first.encode()).hexdigest(),
            hashlib.sha256(second.encode()).hexdigest(),
        )


if __name__ == "__main__":
    unittest.main()
