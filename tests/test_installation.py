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
EDITION_PATHS = {
    "Laravel": Path("Laravel"),
    "Symfony": Path("Symfony"),
    "PHP Core": Path("PHP Core"),
    "WordPress": Path("Cms/wordpress"),
}
EDITIONS = tuple(EDITION_PATHS)
TOOLS = ("claude", "cursor", "codex")
TIMEOUT = 90
REQUIRED_SHARED = (
    "AGENTS.md",
    "memory-bank/README.md",
    "memory-bank/INDEX.md",
    "memory-bank/scripts/context.py",
    "memory-bank/scripts/context_continuity.py",
    "memory-bank/scripts/context_handoff.py",
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
CONTINUITY_HOOKS = {
    "claude": ".claude/hooks/context-continuity.sh",
    "cursor": ".cursor/hooks/context-continuity.sh",
    "codex": ".codex/hooks/context-continuity.sh",
}
CONTINUITY_REGISTRATIONS = {
    "claude": {
        "SessionStart": (".claude/hooks/context-continuity.sh restore",),
        "UserPromptSubmit": (".claude/hooks/context-continuity.sh capture",),
        "Stop": (".claude/hooks/context-continuity.sh capture",),
    },
    "cursor": {
        "sessionStart": (".cursor/hooks/context-continuity.sh restore",),
        "beforeSubmitPrompt": (".cursor/hooks/context-continuity.sh capture",),
        "afterAgentResponse": (".cursor/hooks/context-continuity.sh capture",),
    },
    "codex": {
        "SessionStart": (".codex/hooks/context-continuity.sh restore",),
        "UserPromptSubmit": (".codex/hooks/context-continuity.sh capture",),
        "Stop": (".codex/hooks/context-continuity.sh capture",),
    },
}
for _tool, _hook in CONTINUITY_HOOKS.items():
    REQUIRED_TOOLS[_tool] += (_hook,)
# Every selected tool must carry the portable continuation entry points.
for _tool, _skill_root in (("claude", ".claude"), ("cursor", ".cursor"), ("codex", ".agents")):
    REQUIRED_TOOLS[_tool] += tuple(
        f"{_skill_root}/skills/{name}/SKILL.md" for name in ("context-save", "context-load")
    )
    if _tool != "codex":
        REQUIRED_TOOLS[_tool] += tuple(
            f".{_tool}/commands/{name}.md" for name in ("context-save", "context-load")
        )

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


def run(
    *args: str,
    cwd: Path = ROOT,
    env: dict[str, str] | None = None,
    input_text: str | None = None,
):
    return subprocess.run(
        list(args),
        cwd=cwd,
        env=env,
        input=input_text,
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
        digest.update((ROOT / EDITION_PATHS[edition] / source).read_bytes())
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
            source_dir = EDITION_PATHS[edition].as_posix()
            tracked_result = run("git", "ls-files", "-z", "--", source_dir)
            self.assertEqual(0, tracked_result.returncode, tracked_result.stderr)
            prefix = source_dir + "/"
            tracked = {
                path[len(prefix) :]
                for path in tracked_result.stdout.split("\0")
                if path.startswith(prefix)
            }
            for path in REQUIRED_SOURCE_EXCLUSIONS:
                with self.subTest(edition=edition, path=path):
                    self.assertIn(path, excluded)
                    self.assertTrue((ROOT / EDITION_PATHS[edition] / path).is_file())
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
        with tempfile.TemporaryDirectory(prefix="regenerated inventories ") as raw:
            regenerated = Path(raw).resolve() / "inventories"
            generated = run(
                sys.executable,
                str(INSTALLER),
                "--write-inventories",
                "--inventory-out",
                str(regenerated),
            )
            self.assertEqual(0, generated.returncode, generated.stderr)
            self.assertEqual(
                before,
                {
                    path: (regenerated / path.name).read_bytes()
                    for path in inventory_paths
                },
            )
        # Regeneration reads the index, so an untracked working tree can neither
        # change the committed inventories nor be written into them.
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
            # Every file the accelerator's own runtime rewrites here installs
            # from a pristine source instead of the developer's working copy.
            self.assertEqual(
                {
                    "memory-bank/INDEX.md": "memory-bank/.install/INDEX.md",
                    "project-brain/indexes/active.json": "project-brain/.install/active.json",
                    "project-brain/indexes/archive.json": "project-brain/.install/archive.json",
                },
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


class UntrackedSourceTest(unittest.TestCase):
    """Untracked working-tree content must never reach a shipped inventory."""

    LEAKS = (
        ".env",
        "client-notes.txt",
        "Task/app/.env",
        "Task/app/var/cache/dev/ContainerSynthetic.php",
    )
    MARKER = "CLIENT_SECRET=must-not-ship"

    def _write_editions(self, base: Path) -> None:
        for edition_path in EDITION_PATHS.values():
            edition_root = base / edition_path
            (edition_root / "memory-bank" / ".install").mkdir(parents=True)
            (edition_root / ".claude").mkdir(parents=True)
            (edition_root / "VERSION").write_text("0.0.0\n", encoding="utf-8")
            (edition_root / "AGENTS.md").write_text("# policy\n", encoding="utf-8")
            (edition_root / "CHANGELOG.md").write_text("# history\n", encoding="utf-8")
            (edition_root / "memory-bank" / "INDEX.md").write_text(
                "# source index\n", encoding="utf-8"
            )
            (edition_root / "memory-bank" / ".install" / "INDEX.md").write_text(
                "# production index\n", encoding="utf-8"
            )
            (edition_root / ".claude" / "settings.json").write_text(
                "{}\n", encoding="utf-8"
            )

    def _plant_untracked_files(self, base: Path) -> None:
        for relative in self.LEAKS:
            leak = base / "Symfony" / relative
            leak.parent.mkdir(parents=True, exist_ok=True)
            leak.write_text(self.MARKER + "\n", encoding="utf-8")

    def test_untracked_files_are_absent_from_generated_inventories(self) -> None:
        with tempfile.TemporaryDirectory(prefix="untracked source ") as raw:
            base = Path(raw).resolve()
            self._write_editions(base)
            initialized = run(
                "git", "-c", "init.defaultBranch=main", "init", "-q", str(base)
            )
            self.assertEqual(0, initialized.returncode, initialized.stderr)
            # Staged and never committed: the index alone defines the payload.
            staged = run(
                "git",
                "add",
                "--",
                *(path.as_posix() for path in EDITION_PATHS.values()),
                cwd=base,
            )
            self.assertEqual(0, staged.returncode, staged.stderr)
            self._plant_untracked_files(base)

            generated = run(
                sys.executable,
                str(INSTALLER),
                "--source-root",
                str(base),
                "--write-inventories",
                cwd=base,
            )
            self.assertEqual(0, generated.returncode, generated.stderr)

            written = base / "install" / "inventories" / "symfony.json"
            raw_text = written.read_text(encoding="utf-8")
            self.assertNotIn(self.MARKER, raw_text)
            data = json.loads(raw_text)
            installed = [
                path for paths in data["installed"].values() for path in paths
            ]
            recorded = set(installed) | set(data["excluded_tracked_paths"])
            for relative in self.LEAKS:
                with self.subTest(path=relative):
                    self.assertNotIn(relative, recorded)
            self.assertEqual(
                ["AGENTS.md", "VERSION", "memory-bank/INDEX.md"],
                data["installed"]["shared"],
            )
            self.assertEqual([".claude/settings.json"], data["installed"]["claude"])
            self.assertEqual(
                ["CHANGELOG.md", "memory-bank/.install/INDEX.md"],
                data["excluded_tracked_paths"],
            )

            # Verification reads the same index, so the dirty working tree keeps
            # the installation gate green instead of failing on client files.
            verified = run(
                sys.executable,
                str(INSTALLER),
                "--source-root",
                str(base),
                "--verify-inventories",
                cwd=base,
            )
            self.assertEqual(0, verified.returncode, verified.stderr)
            self.assertIn("VERIFIED\tSymfony", verified.stdout)

    def test_generation_without_a_git_checkout_fails_instead_of_scanning(self) -> None:
        with tempfile.TemporaryDirectory(prefix="ungit source ") as raw:
            base = Path(raw).resolve()
            self._write_editions(base)
            self._plant_untracked_files(base)
            result = run(
                sys.executable,
                str(INSTALLER),
                "--source-root",
                str(base),
                "--write-inventories",
                cwd=base,
            )
            self.assertEqual(1, result.returncode, result.stdout)
            self.assertIn("install-accelerator:", result.stderr)
            self.assertFalse((base / "install").exists())


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

    def test_local_runtime_state_never_ships_into_an_install(self) -> None:
        """A developer's own Brain index must not reach a target.

        The accelerator's runtime rewrites `project-brain/indexes/active.json`
        in this repository whenever a task is opened here, and the records it
        then lists are this repository's - untracked, and never installed. A
        target that received that index held one pointing at files it does not
        have, which its own `context.py validate` reports as stale. Measured on
        a working checkout: three clean-install subtests failed for that reason
        alone, with nothing wrong in any committed file.
        """
        index = ROOT / "Symfony" / "project-brain" / "indexes" / "active.json"
        pristine = index.read_bytes()
        index.write_text(
            json.dumps(
                [{"type": "task", "external_id": "local-session",
                  "path": "project-brain/dynamic/tasks/local-session.md"}],
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )
        self.addCleanup(index.write_bytes, pristine)
        with tempfile.TemporaryDirectory(prefix="runtime state install ") as raw:
            target = Path(raw).resolve()
            installed = run(
                sys.executable,
                str(INSTALLER),
                "--edition",
                "Symfony",
                "--tool",
                "claude",
                "--target",
                str(target),
            )
            self.assertEqual(0, installed.returncode, installed.stderr)
            shipped = (target / "project-brain" / "indexes" / "active.json").read_text(
                encoding="utf-8"
            )
        self.assertEqual("[]", shipped.strip())

    def _continuity_registrations(self, target: Path, tool: str) -> dict[str, list[str]]:
        settings = target / (".claude/settings.json" if tool == "claude" else f".{tool}/hooks.json")
        parsed = json.loads(settings.read_text(encoding="utf-8"))
        registered: dict[str, list[str]] = {}
        for event, groups in parsed["hooks"].items():
            commands: list[str] = []
            for group in groups:
                hooks = group.get("hooks", [group])
                commands.extend(
                    hook["command"] for hook in hooks if isinstance(hook.get("command"), str)
                )
            registered[event] = commands
        return registered

    def _continuity_text(self, tool: str, output: str) -> str:
        payload = json.loads(output)
        if tool == "cursor":
            return payload["additional_context"]
        return payload["hookSpecificOutput"]["additionalContext"]

    def _run_continuity_hook(
        self, target: Path, tool: str, action: str, payload: dict[str, str]
    ) -> subprocess.CompletedProcess[str]:
        # Hooks are registered as a project-relative executable.  Invoke that
        # exact installed file from outside the project to prove it derives its
        # root from its own location rather than the caller's current directory.
        return run(
            str(target / CONTINUITY_HOOKS[tool]), action,
            cwd=target.parent,
            input_text=json.dumps(payload),
        )

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
            gitignore = target / ".gitignore"
            gitignore.write_text("*.md\n/docs/\n", encoding="utf-8")
            source_path = "docs/install-memory-source.md"
            source = target / source_path
            source.parent.mkdir()
            source.write_text(
                "# Install memory source\n\nThe copper rule is authoritative.\n",
                encoding="utf-8",
            )
            initialized = run(
                "git",
                "-c",
                "init.defaultBranch=main",
                "init",
                "-q",
                str(target),
            )
            self.assertEqual(0, initialized.returncode, initialized.stderr)
            tracked = run(
                "git",
                "add",
                "-f",
                "--",
                ".gitignore",
                source_path,
                cwd=target,
            )
            self.assertEqual(0, tracked.returncode, tracked.stderr)
            ignored_without_index = run(
                "git", "check-ignore", "--no-index", "--quiet", "--", source_path,
                cwd=target,
            )
            self.assertEqual(0, ignored_without_index.returncode)
            tracked_despite_ignore = run(
                "git", "check-ignore", "--quiet", "--", source_path, cwd=target
            )
            self.assertEqual(1, tracked_despite_ignore.returncode)
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
                    "--merge-existing",
                )
                self.assertEqual(0, result.returncode, result.stderr)
                expected = len(data["installed"]["shared"]) + len(
                    data["installed"][tool]
                )
                copies = [
                    line for line in result.stdout.splitlines() if line.startswith("COPY\t")
                ]
                self.assertEqual(expected - 1, len(copies))
                self.assertIn(
                    "MERGE\tshared\t.gitignore\t.gitignore",
                    result.stdout.splitlines(),
                )
                self.assertEqual(
                    "COMPLETE\t{}\ttools={}\tfiles={}\tdry_run=false".format(
                        edition, tool, expected
                    ),
                    result.stdout.splitlines()[-1],
                )
                for path in (*REQUIRED_SHARED, *REQUIRED_TOOLS[tool]):
                    self.assertTrue((target / path).is_file(), path)
                self.assertTrue(
                    os.access(target / CONTINUITY_HOOKS[tool], os.X_OK),
                    "registered continuity hook must be executable",
                )

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
                self.assertIn("*.md\n", gitignore.read_text(encoding="utf-8"))
                self.assertIn("/docs/\n", gitignore.read_text(encoding="utf-8"))
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
                context_command = (
                    sys.executable,
                    "memory-bank/scripts/context.py",
                    "--db",
                    str(local_db),
                )
                # Exercise the installed facade and imported module before
                # another command creates SQLite.
                handoff_input = target.parent / "handoff-input.json"
                handoff_input.write_text(json.dumps({
                    "goal": "Continue checking the copper rule",
                    "summary": "The contract is ready for review",
                    "next_steps": ["Check the current source"],
                    "files": [source_path],
                }), encoding="utf-8")
                transcript = target.parent / "visible-export.txt"
                marker = "Verbatim conversation sentinel"
                transcript.write_bytes((marker + "\r\nCopper rule discussion.\r\n").encode())
                for detail in ("summary", "topic", "full"):
                    handoff = target / "tasks/TASK-001" / f"context-save-{detail}.md"
                    extra = ("--topic", "copper rule") if detail == "topic" else ()
                    if detail == "full":
                        extra = ("--transcript", str(transcript))
                    saved = run(
                        *context_command, "context-save", "--input", str(handoff_input),
                        "--output", str(handoff), "--detail", detail,
                        "--source-client", tool, "--json", *extra,
                        cwd=target, env=command_env,
                    )
                    self.assertEqual(0, saved.returncode, saved.stderr)
                    self.assertTrue(handoff.is_file())
                    loaded = run(
                        *context_command, "context-load", "--input", str(handoff), "--json",
                        cwd=target, env=command_env,
                    )
                    self.assertEqual(0, loaded.returncode, loaded.stderr)
                    self.assertIn("The contract is ready for review", loaded.stdout)
                    self.assertNotIn(marker, loaded.stdout)
                    if detail == "full":
                        full = run(
                            *context_command, "context-load", "--input", str(handoff),
                            "--include-transcript", "--json", cwd=target, env=command_env,
                        )
                        self.assertEqual(0, full.returncode, full.stderr)
                        self.assertIn(marker, full.stdout)
                self.assertFalse(local_db.exists(), "save/load must not create SQLite")

                registrations = self._continuity_registrations(target, tool)
                for event, commands in CONTINUITY_REGISTRATIONS[tool].items():
                    for command in commands:
                        self.assertIn(command, registrations.get(event, []), (tool, event, command))
                self.assertIn(".context-handoff", gitignore.read_text(encoding="utf-8"))

                session = "clean-install-continuity"
                prompt_marker = "Automatic continuity prompt sentinel for {}".format(tool)
                response_marker = "Automatic continuity response sentinel for {}".format(tool)
                prompt_payload = {
                    "session_id": session,
                    "conversation_id": session,
                    "prompt": prompt_marker,
                }
                response_payload = {
                    "session_id": session,
                    "conversation_id": session,
                    "last_assistant_message": response_marker,
                    "text": response_marker,
                }
                prompt_capture = self._run_continuity_hook(
                    target, tool, "capture", prompt_payload
                )
                self.assertEqual(0, prompt_capture.returncode, prompt_capture.stderr)
                self.assertEqual("", prompt_capture.stdout)
                response_capture = self._run_continuity_hook(
                    target, tool, "capture", response_payload
                )
                self.assertEqual(0, response_capture.returncode, response_capture.stderr)
                self.assertEqual("", response_capture.stdout)
                storage = target / ".context-handoff"
                snapshots = sorted(storage.glob("*.json"))
                self.assertEqual(1, len(snapshots))
                ignored_snapshot = run(
                    "git", "check-ignore", "--quiet", "--", str(snapshots[0].relative_to(target)),
                    cwd=target,
                )
                self.assertEqual(0, ignored_snapshot.returncode, ignored_snapshot.stderr)
                self.assertFalse(local_db.exists(), "continuity hooks must not create SQLite")

                restored = self._run_continuity_hook(target, tool, "restore", {})
                self.assertEqual(0, restored.returncode, restored.stderr)
                restored_text = self._continuity_text(tool, restored.stdout)
                self.assertIn(prompt_marker, restored_text)
                self.assertIn(response_marker, restored_text)

                other_host = "cursor" if tool != "cursor" else "claude"
                cross_host = run(
                    sys.executable, str(target / "memory-bank/scripts/context_continuity.py"),
                    "--root", str(target), "--host", other_host,
                    "--event", "restore", "--json", cwd=target.parent,
                )
                self.assertEqual(0, cross_host.returncode, cross_host.stderr)
                self.assertIn(response_marker, self._continuity_text(other_host, cross_host.stdout))

                for key, value in (("user.email", "install@example.test"), ("user.name", "Install Test")):
                    configured = run("git", "config", key, value, cwd=target)
                    self.assertEqual(0, configured.returncode, configured.stderr)
                staged = run("git", "add", "-f", "--", ".gitignore", source_path, cwd=target)
                self.assertEqual(0, staged.returncode, staged.stderr)
                committed = run("git", "commit", "-qm", "continuity test baseline", cwd=target)
                self.assertEqual(0, committed.returncode, committed.stderr)
                switched = run("git", "switch", "-q", "-c", "continuity-isolation", cwd=target)
                self.assertEqual(0, switched.returncode, switched.stderr)
                foreign = self._run_continuity_hook(target, tool, "restore", {})
                self.assertEqual(0, foreign.returncode, foreign.stderr)
                self.assertEqual("", foreign.stdout, "a different branch must not restore prior context")
                malformed = run(
                    str(target / CONTINUITY_HOOKS[tool]), "capture",
                    cwd=target.parent, input_text="{not JSON",
                )
                self.assertEqual(0, malformed.returncode, malformed.stderr)
                self.assertEqual("", malformed.stdout)
                returned = run("git", "switch", "-q", "main", cwd=target)
                self.assertEqual(0, returned.returncode, returned.stderr)
                restored_main = self._run_continuity_hook(target, tool, "restore", {})
                self.assertEqual(0, restored_main.returncode, restored_main.stderr)
                self.assertIn(response_marker, self._continuity_text(tool, restored_main.stdout))

                commands = (
                    (sys.executable, "memory-bank/scripts/validate.py"),
                    (
                        sys.executable,
                        "project-brain/scripts/validate.py",
                        "--root",
                        ".",
                    ),
                    (*context_command, "status", "--json"),
                    (*context_command, "validate", "--json"),
                    (*context_command, "index", "--json"),
                )
                for command in commands:
                    smoke = run(*command, cwd=target, env=command_env)
                    self.assertEqual(0, smoke.returncode, smoke.stderr)
                    self.assertNotIn("must-not-be-read", smoke.stdout + smoke.stderr)

                start = run(
                    *context_command,
                    "start",
                    "--task-id",
                    "install-memory-smoke",
                    "--goal",
                    "Prove installed governed memory",
                    "--source",
                    source_path,
                    "--json",
                    cwd=target,
                    env=command_env,
                )
                self.assertEqual(0, start.returncode, start.stderr)
                started = json.loads(start.stdout)
                self.assertEqual("project-brain", started["authority"])
                self.assertEqual([source_path], started["sources"])

                create = run(
                    *context_command,
                    "brain-create",
                    "finding",
                    "--external-id",
                    "install-memory-finding",
                    "--title",
                    "Installed runtime remembers the copper rule",
                    "--source",
                    source_path,
                    "--json",
                    cwd=target,
                    env=command_env,
                )
                self.assertEqual(0, create.returncode, create.stderr)
                created = json.loads(create.stdout)
                self.assertEqual("finding", created["type"])
                self.assertEqual([source_path], created["sources"])
                self.assertEqual(source_path, created["source_fingerprints"][0]["path"])

                reindex = run(
                    *context_command,
                    "index",
                    "--json",
                    cwd=target,
                    env=command_env,
                )
                self.assertEqual(0, reindex.returncode, reindex.stderr)
                self.assertGreaterEqual(json.loads(reindex.stdout)["brain"], 2)

                links = run(
                    *context_command,
                    "links",
                    "--path",
                    source_path,
                    "--json",
                    cwd=target,
                    env=command_env,
                )
                self.assertEqual(0, links.returncode, links.stderr)
                linked = json.loads(links.stdout)
                self.assertEqual(source_path, linked["path"])
                self.assertTrue(
                    {"brain-task", "brain-finding"}
                    <= {item["kind"] for item in linked["documents"]}
                )
                self.assertTrue(
                    all(item["ref_path"] == source_path for item in linked["documents"])
                )
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
