#!/usr/bin/env python3
"""Synthetic clean-install tests for deterministic edition inventories."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INSTALLER = ROOT / "scripts" / "install_accelerator.py"
EDITIONS = ("Laravel", "Symfony", "PHP Core")
TOOLS = ("claude", "cursor", "codex")
TIMEOUT = 90
REQUIRED_SHARED = (
    "AGENTS.md",
    "memory-bank/README.md",
    "memory-bank/INDEX.md",
    "memory-bank/scripts/context.py",
    "memory-bank/scripts/validate.py",
    "project-brain/PROTOCOL.md",
    "project-brain/config/runtime.json",
    "project-brain/scripts/validate.py",
)
REQUIRED_TOOLS = {
    "claude": (".claude/hooks/bash-validator.sh", ".claude/skills/memory-bank/SKILL.md"),
    "cursor": (".cursor/hooks/bash-validator.sh", ".cursor/skills/memory-bank/SKILL.md"),
    "codex": (".codex/hooks/bash-validator.sh", ".agents/skills/memory-bank/SKILL.md"),
}
REQUIRED_SOURCE_EXCLUSIONS = (
    "CHANGELOG.md",
    "examples/completed-task/writing-plans-plan.md",
    "examples/context-summary.md",
    "examples/pr-description.md",
    "memory-bank/.memory-counter",
    "memory-bank/chunks/MEM-0001-cross-edition-sync.md",
    "memory-bank/tests/test_validate.py",
    "project-brain/tests/test_runtime.py",
)


def run(*args: str, cwd: Path = ROOT, env: dict[str, str] | None = None):
    return subprocess.run(
        list(args),
        cwd=cwd,
        env=env,
        capture_output=True,
        text=True,
        timeout=TIMEOUT,
    )


def inventory(edition: str) -> dict:
    name = edition.lower().replace(" ", "-") + ".json"
    return json.loads((ROOT / "install" / "inventories" / name).read_text())


def source_digest(edition: str, data: dict) -> str:
    digest = hashlib.sha256()
    for path in sorted(path for values in data["installed"].values() for path in values):
        source = data["source_overrides"].get(path, path)
        digest.update(path.encode())
        digest.update(b"\0")
        digest.update((ROOT / edition / source).read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def source_status() -> str:
    result = run("git", "status", "--porcelain=v1", "--untracked-files=all")
    if result.returncode:
        raise AssertionError(result.stderr)
    return result.stdout


class InventoryTest(unittest.TestCase):
    def test_inventories_match_repository_distribution(self) -> None:
        result = run(sys.executable, str(INSTALLER), "--verify-inventories")
        self.assertEqual(0, result.returncode, result.stderr)
        for edition in EDITIONS:
            self.assertIn("VERIFIED\t{}".format(edition), result.stdout)

    def test_source_only_files_remain_in_repository_contract(self) -> None:
        for edition in EDITIONS:
            data = inventory(edition)
            excluded = set(data["excluded_tracked_paths"])
            tracked_result = run("git", "ls-files", "-z", "--", edition)
            self.assertEqual(0, tracked_result.returncode, tracked_result.stderr)
            prefix = edition + "/"
            tracked = {
                path[len(prefix) :]
                for path in tracked_result.stdout.split("\0")
                if path.startswith(prefix)
            }
            for path in REQUIRED_SOURCE_EXCLUSIONS:
                with self.subTest(edition=edition, path=path):
                    self.assertIn(path, excluded)
                    self.assertTrue((ROOT / edition / path).is_file())
                    self.assertIn(path, tracked)
            self.assertTrue(any(path.startswith("Task/") for path in excluded))

    def test_inventory_metadata_is_deterministic(self) -> None:
        inventory_paths = tuple(
            ROOT
            / "install"
            / "inventories"
            / (edition.lower().replace(" ", "-") + ".json")
            for edition in EDITIONS
        )
        before = {path: path.read_bytes() for path in inventory_paths}
        generated = run(sys.executable, str(INSTALLER), "--write-inventories")
        self.assertEqual(0, generated.returncode, generated.stderr)
        self.assertEqual(before, {path: path.read_bytes() for path in inventory_paths})

        for edition in EDITIONS:
            data = inventory(edition)
            installed = [
                path for component in data["installed"].values() for path in component
            ]
            excluded = data["excluded_tracked_paths"]
            self.assertEqual(len(installed), len(set(installed)))
            self.assertEqual(excluded, sorted(set(excluded)))
            self.assertTrue(set(installed).isdisjoint(excluded))
            self.assertEqual(
                {"memory-bank/INDEX.md": "memory-bank/.install/INDEX.md"},
                data["source_overrides"],
            )

    def test_generation_ignores_untracked_working_tree_files(self) -> None:
        # An inventory is a committed contract that the installer copies
        # verbatim, so anything the generator picks up ships to consumers.
        # Generating from the working tree once absorbed 8586 untracked
        # `vendor/` paths from a locally built app into an edition's
        # distribution list. Verification stays permissive on purpose - it is
        # meant to warn about a file not committed yet - but generation reads
        # tracked files only.
        probe = ROOT / "Symfony" / ".claude" / "untracked-generation-probe.md"
        self.assertFalse(probe.exists(), "probe path is already in use")
        inventory_paths = tuple(
            ROOT
            / "install"
            / "inventories"
            / (edition.lower().replace(" ", "-") + ".json")
            for edition in EDITIONS
        )
        before = {path: path.read_bytes() for path in inventory_paths}
        probe.write_text("untracked\n", encoding="utf-8")
        try:
            generated = run(sys.executable, str(INSTALLER), "--write-inventories")
            self.assertEqual(0, generated.returncode, generated.stderr)
            self.assertEqual(
                before, {path: path.read_bytes() for path in inventory_paths}
            )
            self.assertNotIn(
                probe.name, json.dumps(inventory("Symfony"))
            )
            # The delta is what makes a wrong inventory visible before it is
            # committed, so an unchanged run has to say so rather than stay
            # silent.
            self.assertIn("\t+0\t-0", generated.stdout)
        finally:
            probe.unlink()

    def test_malformed_exclusion_and_override_metadata_is_rejected(self) -> None:
        mutations = {
            "unsafe exclusion": lambda data: data["excluded_tracked_paths"].append(
                "../outside"
            ),
            "overlapping exclusion": lambda data: data[
                "excluded_tracked_paths"
            ].append("AGENTS.md"),
            "unknown override destination": lambda data: data[
                "source_overrides"
            ].update({"not-installed.md": "CHANGELOG.md"}),
            "installed override source": lambda data: data[
                "source_overrides"
            ].update({"memory-bank/INDEX.md": "README.md"}),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory(
                prefix="malformed inventory "
            ) as raw:
                source_root = Path(raw)
                inventory_dir = source_root / "install" / "inventories"
                inventory_dir.mkdir(parents=True)
                data = inventory("PHP Core")
                mutate(data)
                if label in {"unsafe exclusion", "overlapping exclusion"}:
                    data["excluded_tracked_paths"].sort()
                (inventory_dir / "php-core.json").write_text(
                    json.dumps(data), encoding="utf-8"
                )
                result = run(
                    sys.executable,
                    str(INSTALLER),
                    "--source-root",
                    str(source_root),
                    "--edition",
                    "PHP Core",
                    "--target",
                    str(source_root / "target"),
                )
                self.assertEqual(1, result.returncode)
                self.assertIn("install-accelerator:", result.stderr)

    def test_excluded_paths_are_absent_after_clean_install(self) -> None:
        for edition in EDITIONS:
            with self.subTest(edition=edition), tempfile.TemporaryDirectory(
                prefix="production boundary "
            ) as raw:
                target = Path(raw).resolve()
                data = inventory(edition)
                result = run(
                    sys.executable,
                    str(INSTALLER),
                    "--edition",
                    edition,
                    "--target",
                    str(target),
                    "--tool",
                    "cursor",
                )
                self.assertEqual(0, result.returncode, result.stderr)
                for path in data["excluded_tracked_paths"]:
                    self.assertFalse((target / path).exists(), path)
                links = run(
                    sys.executable,
                    str(ROOT / "scripts" / "check_links.py"),
                    "--root",
                    str(target),
                )
                self.assertEqual(0, links.returncode, links.stdout + links.stderr)

    def test_dry_run_reports_collision_and_refuses_all_writes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="install collision ") as raw:
            target = Path(raw).resolve()
            collision = target / "AGENTS.md"
            collision.write_text("project-owned\n", encoding="utf-8")
            result = run(
                sys.executable,
                str(INSTALLER),
                "--edition",
                "PHP Core",
                "--target",
                str(target),
                "--tool",
                "cursor",
                "--dry-run",
            )
            self.assertEqual(2, result.returncode)
            self.assertIn("COLLISION\tshared\tAGENTS.md", result.stderr)
            self.assertIn("no files copied", result.stderr)
            self.assertEqual("project-owned\n", collision.read_text(encoding="utf-8"))
            self.assertFalse((target / "memory-bank").exists())

    def test_merge_existing_handles_standard_root_files_and_is_idempotent(self) -> None:
        with tempfile.TemporaryDirectory(prefix="install merge ") as raw:
            target = Path(raw).resolve()
            originals = {
                ".gitattributes": "*.lock binary\n",
                ".gitignore": ".env\n/vendor/\n",
                "AGENTS.md": "# Project policy\n\nKeep project behavior.\n",
                "README.md": "# Existing application\n",
            }
            for path, content in originals.items():
                (target / path).write_text(content, encoding="utf-8")

            command = (
                sys.executable,
                str(INSTALLER),
                "--edition",
                "PHP Core",
                "--target",
                str(target),
                "--tool",
                "cursor",
                "--merge-existing",
            )
            preview = run(*command, "--dry-run")
            self.assertEqual(0, preview.returncode, preview.stderr)
            self.assertIn("WOULD_MERGE\tshared\t.gitignore\t.gitignore", preview.stdout)
            self.assertIn(
                "WOULD_MERGE\tshared\t.gitattributes\t.gitattributes",
                preview.stdout,
            )
            self.assertIn("WOULD_MERGE\tshared\tAGENTS.md\tAGENTS.md", preview.stdout)
            self.assertIn(
                "WOULD_COPY_AS\tshared\tREADME.md\tACCELERATOR.md",
                preview.stdout,
            )
            for path, content in originals.items():
                self.assertEqual(content, (target / path).read_text(encoding="utf-8"))
            self.assertFalse((target / "ACCELERATOR.md").exists())
            self.assertFalse((target / "memory-bank").exists())

            installed = run(*command)
            self.assertEqual(0, installed.returncode, installed.stderr)
            self.assertEqual(
                originals["README.md"],
                (target / "README.md").read_text(encoding="utf-8"),
            )
            self.assertEqual(
                (ROOT / "PHP Core" / "README.md").read_bytes(),
                (target / "ACCELERATOR.md").read_bytes(),
            )
            agents = (target / "AGENTS.md").read_text(encoding="utf-8")
            self.assertTrue(agents.startswith(originals["AGENTS.md"].rstrip()))
            self.assertIn("BEGIN ACCELERATOR MANAGED POLICY", agents)
            self.assertIn("# AGENTS.md - Policy Rules", agents)
            gitignore = (target / ".gitignore").read_text(encoding="utf-8")
            self.assertIn(".env\n", gitignore)
            self.assertIn("memory-bank/local/\n", gitignore)
            attributes = (target / ".gitattributes").read_text(encoding="utf-8")
            self.assertIn("*.lock binary\n", attributes)
            self.assertIn(".cursor/skills/", attributes)

            merged_digests = {
                path: hashlib.sha256((target / path).read_bytes()).hexdigest()
                for path in (*originals, "ACCELERATOR.md")
            }
            repeated = run(*command)
            self.assertEqual(0, repeated.returncode, repeated.stderr)
            self.assertIn("UNCHANGED\tshared\tAGENTS.md", repeated.stdout)
            self.assertIn("UNCHANGED\tshared\tREADME.md", repeated.stdout)
            for path, digest in merged_digests.items():
                self.assertEqual(digest, hashlib.sha256((target / path).read_bytes()).hexdigest())

    def test_merge_existing_still_refuses_unsupported_collision_atomically(self) -> None:
        with tempfile.TemporaryDirectory(prefix="install unsupported merge ") as raw:
            target = Path(raw).resolve()
            collision = target / "memory-bank" / "README.md"
            collision.parent.mkdir()
            collision.write_text("project-owned memory\n", encoding="utf-8")
            result = run(
                sys.executable,
                str(INSTALLER),
                "--edition",
                "PHP Core",
                "--target",
                str(target),
                "--tool",
                "cursor",
                "--merge-existing",
            )
            self.assertEqual(2, result.returncode)
            self.assertIn(
                "COLLISION\tshared\tmemory-bank/README.md\texisting-file",
                result.stderr,
            )
            self.assertEqual(
                "project-owned memory\n", collision.read_text(encoding="utf-8")
            )
            self.assertFalse((target / "AGENTS.md").exists())

    def test_parent_obstruction_is_preflighted_before_copy(self) -> None:
        with tempfile.TemporaryDirectory(prefix="install obstruction ") as raw:
            target = Path(raw).resolve()
            obstruction = target / "memory-bank"
            obstruction.write_text("project-owned path\n", encoding="utf-8")
            result = run(
                sys.executable,
                str(INSTALLER),
                "--edition",
                "Laravel",
                "--target",
                str(target),
                "--tool",
                "claude",
            )
            self.assertEqual(2, result.returncode)
            self.assertIn("parent-obstruction", result.stderr)
            self.assertIn("no files copied", result.stderr)
            self.assertEqual(
                "project-owned path\n", obstruction.read_text(encoding="utf-8")
            )
            self.assertFalse((target / "AGENTS.md").exists())

    def test_overwrite_refuses_final_symlink_without_touching_referent(self) -> None:
        with tempfile.TemporaryDirectory(prefix="install symlink ") as raw:
            base = Path(raw).resolve()
            target = base / "target"
            target.mkdir()
            outside = base / "outside-policy.md"
            outside.write_text("outside\n", encoding="utf-8")
            (target / "AGENTS.md").symlink_to(outside)

            result = run(
                sys.executable,
                str(INSTALLER),
                "--edition",
                "PHP Core",
                "--target",
                str(target),
                "--tool",
                "cursor",
                "--overwrite",
            )

            self.assertEqual(2, result.returncode)
            self.assertIn("COLLISION\tshared\tAGENTS.md\tsymlink", result.stderr)
            self.assertIn("no files copied", result.stderr)
            self.assertEqual("outside\n", outside.read_text(encoding="utf-8"))
            self.assertFalse((target / "memory-bank").exists())

    def test_overwrite_refuses_symlinked_parent_without_external_writes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="install parent symlink ") as raw:
            base = Path(raw).resolve()
            target = base / "target"
            outside = base / "outside"
            target.mkdir()
            outside.mkdir()
            (target / "memory-bank").symlink_to(outside, target_is_directory=True)

            result = run(
                sys.executable,
                str(INSTALLER),
                "--edition",
                "Laravel",
                "--target",
                str(target),
                "--tool",
                "claude",
                "--overwrite",
            )

            self.assertEqual(2, result.returncode)
            self.assertIn("parent-obstruction", result.stderr)
            self.assertIn("no files copied", result.stderr)
            self.assertEqual([], list(outside.iterdir()))
            self.assertFalse((target / "AGENTS.md").exists())

    def test_symlinked_target_root_is_refused_without_external_writes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="install root symlink ") as raw:
            base = Path(raw).resolve()
            outside = base / "outside"
            outside.mkdir()
            linked_target = base / "target"
            linked_target.symlink_to(outside, target_is_directory=True)

            result = run(
                sys.executable,
                str(INSTALLER),
                "--edition",
                "PHP Core",
                "--target",
                str(linked_target),
                "--tool",
                "cursor",
                "--overwrite",
            )

            self.assertEqual(1, result.returncode)
            self.assertIn("target path contains a symlink", result.stderr)
            self.assertEqual([], list(outside.iterdir()))

    def test_paths_with_spaces_and_exact_dry_run_transcript(self) -> None:
        data = inventory("PHP Core")
        expected = len(data["installed"]["shared"]) + len(
            data["installed"]["cursor"]
        )
        with tempfile.TemporaryDirectory(prefix="target with spaces ") as raw:
            target = Path(raw).resolve()
            result = run(
                sys.executable,
                str(INSTALLER),
                "--edition",
                "PHP Core",
                "--target",
                str(target),
                "--tool",
                "cursor",
                "--dry-run",
            )
            self.assertEqual(0, result.returncode, result.stderr)
            lines = result.stdout.splitlines()
            copies = [line for line in lines if line.startswith("WOULD_COPY\t")]
            self.assertEqual(expected, len(copies))
            self.assertEqual(
                "COMPLETE\tPHP Core\ttools=cursor\tfiles={}\tdry_run=true".format(
                    expected
                ),
                lines[-1],
            )
            self.assertFalse(any(target.iterdir()))


class CleanInstallTest(unittest.TestCase):
    def test_every_edition_and_tool_clean_install(self) -> None:
        baseline_status = source_status()
        for edition in EDITIONS:
            data = inventory(edition)
            baseline_digest = source_digest(edition, data)
            for tool in TOOLS:
                with self.subTest(edition=edition, tool=tool):
                    self._run_clean_install(edition, tool, data)
                    self.assertEqual(baseline_digest, source_digest(edition, data))
                    self.assertEqual(baseline_status, source_status())

    def _run_clean_install(self, edition: str, tool: str, data: dict) -> None:
        with tempfile.TemporaryDirectory(prefix="clean install ") as raw:
            target = Path(raw).resolve()
            env_file = target / ".env"
            env_file.write_text("TOP_SECRET=must-not-be-read\n", encoding="utf-8")
            app_db = target / "application.sqlite"
            app_db.write_bytes(b"synthetic-application-database")
            app_db_digest = hashlib.sha256(app_db.read_bytes()).hexdigest()
            composer = target / "composer.json"
            composer.write_text(
                json.dumps(
                    {
                        "name": "synthetic/no-application-execution",
                        "scripts": {"post-install-cmd": ["touch APPLICATION_EXECUTED"]},
                    }
                ),
                encoding="utf-8",
            )
            changelog = target / "CHANGELOG.md"
            changelog.write_text("# Existing project history\n", encoding="utf-8")
            initialized = run(
                "git",
                "-c",
                "init.defaultBranch=main",
                "init",
                "-q",
                str(target),
            )
            self.assertEqual(0, initialized.returncode, initialized.stderr)
            if os.name != "nt":
                env_file.chmod(0)
                app_db.chmod(0)
            try:
                result = run(
                    sys.executable,
                    str(INSTALLER),
                    "--edition",
                    edition,
                    "--target",
                    str(target),
                    "--tool",
                    tool,
                )
                self.assertEqual(0, result.returncode, result.stderr)
                expected = len(data["installed"]["shared"]) + len(
                    data["installed"][tool]
                )
                copies = [
                    line for line in result.stdout.splitlines() if line.startswith("COPY\t")
                ]
                self.assertEqual(expected, len(copies))
                self.assertEqual(
                    "COMPLETE\t{}\ttools={}\tfiles={}\tdry_run=false".format(
                        edition, tool, expected
                    ),
                    result.stdout.splitlines()[-1],
                )
                for path in (*REQUIRED_SHARED, *REQUIRED_TOOLS[tool]):
                    self.assertTrue((target / path).is_file(), path)

                for path in data["excluded_tracked_paths"]:
                    if path == "CHANGELOG.md":
                        continue
                    self.assertFalse((target / path).exists(), path)
                self.assertFalse((target / "memory-bank" / ".memory-counter").exists())
                installed_index = (target / "memory-bank" / "INDEX.md").read_text(
                    encoding="utf-8"
                )
                self.assertNotIn("MEM-0001", installed_index)
                self.assertNotIn("chunks/", installed_index)
                self.assertEqual(
                    "# Existing project history\n",
                    changelog.read_text(encoding="utf-8"),
                )

                command_env = dict(os.environ)
                command_env.update(
                    {
                        "DATABASE_URL": "sqlite:///{}".format(app_db),
                        "DB_DATABASE": str(app_db),
                    }
                )
                local_db = target / "memory-bank" / "local" / "install-test.db"
                commands = (
                    (sys.executable, "memory-bank/scripts/validate.py"),
                    (
                        sys.executable,
                        "project-brain/scripts/validate.py",
                        "--root",
                        ".",
                    ),
                    (
                        sys.executable,
                        "memory-bank/scripts/context.py",
                        "--db",
                        str(local_db),
                        "status",
                        "--json",
                    ),
                    (
                        sys.executable,
                        "memory-bank/scripts/context.py",
                        "--db",
                        str(local_db),
                        "validate",
                        "--json",
                    ),
                    (
                        sys.executable,
                        "memory-bank/scripts/context.py",
                        "--db",
                        str(local_db),
                        "index",
                        "--json",
                    ),
                )
                for command in commands:
                    smoke = run(*command, cwd=target, env=command_env)
                    self.assertEqual(0, smoke.returncode, smoke.stderr)
                    self.assertNotIn("must-not-be-read", smoke.stdout + smoke.stderr)
                self.assertFalse((target / "APPLICATION_EXECUTED").exists())
            finally:
                if os.name != "nt":
                    env_file.chmod(0o600)
                    app_db.chmod(0o600)
            self.assertEqual(
                app_db_digest, hashlib.sha256(app_db.read_bytes()).hexdigest()
            )


if __name__ == "__main__":
    unittest.main()
