#!/usr/bin/env python3
"""Synthetic clean-install tests for deterministic edition inventories."""

from __future__ import annotations

import contextlib
import errno
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

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
    "memory-bank/scripts/mcp_config.py",
    "memory-bank/scripts/mcp_server.py",
    "memory-bank/MCP.md",
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
    "claude": (".mcp.json", ".claude/hooks/bash-validator.sh", ".claude/skills/memory-bank/SKILL.md"),
    "cursor": (".cursor/mcp.json", ".cursor/hooks/bash-validator.sh", ".cursor/skills/memory-bank/SKILL.md"),
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
        digest.update((ROOT / EDITION_PATHS[edition] / source).read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def source_status() -> str:
    result = run("git", "status", "--porcelain=v1", "--untracked-files=all")
    if result.returncode:
        raise AssertionError(result.stderr)
    return result.stdout


# The files each client reads its hook wiring from, relative to a project root.
HOOK_WIRING = (".claude/settings.json", ".cursor/hooks.json", ".codex/hooks.json")


# The hook scripts one wiring command runs; the routes gate parses every form
# the editions use (bare, "${CLAUDE_PROJECT_DIR}"-anchored, Codex launcher).
sys.path.insert(0, str(ROOT / "scripts"))
from check_routes import wired_hook_scripts as scripts_named  # noqa: E402
import install_accelerator  # noqa: E402  - in-process runs, for the injected faults below


def wired_hook_scripts(project: Path) -> list[str]:
    """Every project-relative hook script a client's wiring runs."""
    scripts: list[str] = []

    def walk(node: object) -> None:
        if isinstance(node, dict):
            for key, value in node.items():
                if key == "command" and isinstance(value, str):
                    for script in scripts_named(value):
                        if script not in scripts:
                            scripts.append(script)
                else:
                    walk(value)
        elif isinstance(node, list):
            for item in node:
                walk(item)

    for wiring in HOOK_WIRING:
        path = project / wiring
        if path.is_file():
            walk(json.loads(path.read_text(encoding="utf-8")).get("hooks", {}))
    return scripts


def is_executable(path: Path) -> bool:
    return bool(path.stat().st_mode & stat.S_IXUSR) and os.access(path, os.X_OK)


def files_under(root: Path) -> dict[str, bytes]:
    """Every file below `root` with its bytes, to show nothing there changed."""
    return {
        path.relative_to(root).as_posix(): path.read_bytes()
        for path in sorted(root.rglob("*"))
        if path.is_file()
    }


def run_installer(*args: str) -> tuple[int, str, str]:
    """`install_accelerator.py *args` in this process, so a test can inject a fault."""
    out, err = io.StringIO(), io.StringIO()
    with patch.object(sys, "argv", [str(INSTALLER), *args]), contextlib.redirect_stdout(
        out
    ), contextlib.redirect_stderr(err):
        code = install_accelerator.main()
    return code, out.getvalue(), err.getvalue()


def install_quietly(target: Path, edition: str, tools: list[str], merge: bool = False) -> None:
    with contextlib.redirect_stdout(io.StringIO()):
        code = install_accelerator.install(ROOT, edition, target, tools, False, False, merge)
    if code != 0:
        raise AssertionError(f"install into {target} returned {code}")


# A write's temporary file beside the file it replaces: `.<name>.accelerator-<pid>-<hex>`.
TEMPORARY = re.compile(r"\..+\.accelerator-\d+-[0-9a-f]{8}")


def is_temporary(name: str, marker: str) -> bool:
    return TEMPORARY.fullmatch(name) is not None and marker in name


def temporaries(root: Path) -> list[str]:
    """Temporary files a write left below `root`."""
    return sorted(path.name for path in root.rglob("*") if TEMPORARY.fullmatch(path.name))


@contextlib.contextmanager
def swapped_while_written(project: Path, folder: str, outside: Path, marker: str, how: str):
    """Make `folder` a link to `outside` as a write creates its temporary file for `marker`.

    That is after every look the write takes at the path and before its bytes
    exist anywhere. `how`: "moved" renames the real folder elsewhere in the
    project, "removed" deletes it. Yields the swaps made (at most one).
    """
    real_open = os.open
    swaps: list[str] = []

    def racing_open(path, flags, *args, **kwargs):
        name = os.path.basename(os.fsdecode(path))
        if not swaps and flags & os.O_CREAT and flags & os.O_EXCL and is_temporary(name, marker):
            real = project / folder
            if how == "moved":
                real.rename(real.with_name(real.name + "-moved"))
            else:
                shutil.rmtree(real)
            real.symlink_to(outside, target_is_directory=True)
            swaps.append(name)
        return real_open(path, flags, *args, **kwargs)

    with patch.object(os, "open", racing_open):
        yield swaps


@contextlib.contextmanager
def disk_full_while_written(marker: str, also=None):
    """The disk fills after the first byte a write puts in its temporary file for `marker`.

    What a full disk or an interrupted run does to a write: the merge used to
    be written over the file in place, so the first byte was all that was
    left of it. `also` runs at that moment, as another process would.
    """
    real_open, real_write = os.open, os.write
    doomed: set[int] = set()
    hit: list[str] = []

    def tracking_open(path, flags, *args, **kwargs):
        descriptor = real_open(path, flags, *args, **kwargs)
        name = os.path.basename(os.fsdecode(path))
        if not hit and flags & os.O_CREAT and flags & os.O_EXCL and is_temporary(name, marker):
            doomed.add(descriptor)
            hit.append(name)
        return descriptor

    def full_disk(descriptor, data):
        if descriptor in doomed:
            doomed.discard(descriptor)
            real_write(descriptor, bytes(data[:1]))
            if also is not None:
                also()
            raise OSError(errno.ENOSPC, os.strerror(errno.ENOSPC))
        return real_write(descriptor, data)

    with patch.object(os, "open", tracking_open), patch.object(os, "write", full_disk):
        yield hit


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
                    ".gitattributes": ".install/gitattributes",
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
            self.assertIn("*.sh text eol=lf\n", attributes)
            # The monorepo's mirror marking stays home: a client reviewing an
            # edited hook must see its diff, not "Binary files differ".
            self.assertNotIn("-diff", attributes)

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

    def test_claude_install_imports_the_policy_beside_a_project_claude_md(self) -> None:
        """Claude Code reads AGENTS.md itself only while no CLAUDE.md exists;
        a project with its own CLAUDE.md (Laravel Boost writes one) would
        otherwise never load the policy."""
        with tempfile.TemporaryDirectory(prefix="install claude md ") as raw:
            target = Path(raw).resolve()
            (target / "CLAUDE.md").write_text("# Team notes\n", encoding="utf-8")
            (target / ".claude").mkdir()
            existing = "# Existing Claude notes\n\nUse PHP 8.3.\n"
            (target / ".claude" / "CLAUDE.md").write_text(existing, encoding="utf-8")
            command = (
                sys.executable, str(INSTALLER), "--edition", "Laravel",
                "--target", str(target), "--tool", "claude",
            )

            refused = run(*command, "--dry-run")
            self.assertEqual(2, refused.returncode)
            self.assertIn("COLLISION\tclaude\t.claude/CLAUDE.md", refused.stderr)

            installed = run(*command, "--merge-existing")
            self.assertEqual(0, installed.returncode, installed.stderr)
            self.assertIn("MERGE\tclaude\t.claude/CLAUDE.md", installed.stdout)
            merged = (target / ".claude" / "CLAUDE.md").read_text(encoding="utf-8")
            self.assertTrue(merged.startswith(existing.rstrip()))
            self.assertIn("\n@../AGENTS.md\n", merged)
            self.assertEqual("# Team notes\n", (target / "CLAUDE.md").read_text(encoding="utf-8"))

            repeated = run(*command, "--merge-existing")
            self.assertEqual(0, repeated.returncode, repeated.stderr)
            self.assertIn("UNCHANGED\tclaude\t.claude/CLAUDE.md", repeated.stdout)

    def test_fresh_install_gitattributes_keeps_hooks_lf_and_diffable(self) -> None:
        for edition in EDITION_PATHS:
            with self.subTest(edition=edition), tempfile.TemporaryDirectory(
                prefix="install attributes "
            ) as raw:
                target = Path(raw).resolve()
                result = run(
                    sys.executable, str(INSTALLER), "--edition", edition,
                    "--target", str(target), "--tool", "claude",
                )
                self.assertEqual(0, result.returncode, result.stderr)
                attributes = (target / ".gitattributes").read_text(encoding="utf-8")
                self.assertIn("*.sh text eol=lf\n", attributes)
                self.assertNotIn("-diff", attributes)

    def test_reinstall_keeps_runtime_state_the_project_owns(self) -> None:
        """A reinstall over a project that has used the accelerator must not
        collide on, or reset, the state its runtime and team own."""
        with tempfile.TemporaryDirectory(prefix="install seed ") as raw:
            target = Path(raw).resolve()
            run("git", "init", "--quiet", str(target))
            command = (
                sys.executable, str(INSTALLER), "--edition", "Laravel",
                "--target", str(target), "--tool", "codex",
            )
            first = run(*command)
            self.assertEqual(0, first.returncode, first.stderr)
            started = run(
                sys.executable, "memory-bank/scripts/context.py", "start",
                "--task-id", "seed-only-smoke", "--goal", "Prove reinstall keeps state",
                "--source", "AGENTS.md", cwd=target,
            )
            self.assertEqual(0, started.returncode, started.stderr)
            counter = target / "tasks" / ".task-counter"
            counter.write_text("7\n", encoding="utf-8")
            manifest = target / "specs" / "MANIFEST.md"
            manifest.write_text(manifest.read_text(encoding="utf-8") + "| team spec |\n", encoding="utf-8")
            state = {
                path: (target / path).read_bytes()
                for path in ("tasks/.task-counter", "specs/MANIFEST.md", "project-brain/indexes/active.json")
            }

            for mode in ("--merge-existing", "--overwrite"):
                with self.subTest(mode=mode):
                    again = run(*command, mode)
                    self.assertEqual(0, again.returncode, again.stderr)
                    self.assertIn("KEPT\tshared\ttasks/.task-counter", again.stdout)
                    for path, content in state.items():
                        self.assertEqual(content, (target / path).read_bytes(), path)
                    validated = run(
                        sys.executable, "project-brain/scripts/validate.py", "--root", ".", cwd=target
                    )
                    self.assertEqual(0, validated.returncode, validated.stdout + validated.stderr)

    def test_every_claude_install_ships_the_policy_import(self) -> None:
        for edition in EDITION_PATHS:
            with self.subTest(edition=edition):
                shipped = (ROOT / EDITION_PATHS[edition] / ".claude" / "CLAUDE.md").read_text(
                    encoding="utf-8"
                )
                imports = [line for line in shipped.splitlines() if line.startswith("@")]
                self.assertEqual(["@../AGENTS.md"], imports)

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


class InstallSyncTest(unittest.TestCase):
    """An installed project follows the clone without anyone reinstalling.

    On six real installations none carried the memory fixes of the week
    before: the installer copies once and nothing ever updated the copy.
    """

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="install sync ")
        self.addCleanup(self._tmp.cleanup)
        base = Path(self._tmp.name)
        # A clone of its own, so a test can publish a new release into it.
        self.clone = base / "clone"
        shutil.copytree(ROOT / "install" / "inventories", self.clone / "install" / "inventories")
        shutil.copytree(ROOT / "Symfony", self.clone / "Symfony")
        self.target = base / "project"
        self.target.mkdir()
        run("git", "init", "--quiet", str(self.target))
        installed = run(
            sys.executable, str(INSTALLER), "--source-root", str(self.clone),
            "--edition", "Symfony", "--tool", "claude", "--tool", "codex",
            "--target", str(self.target),
        )
        self.assertEqual(0, installed.returncode, installed.stderr)

    def sync(self, *extra: str) -> dict:
        returncode, report = self.sync_result(*extra)
        self.assertEqual(0, returncode, report)
        return report

    def sync_result(self, *extra: str) -> tuple[int, dict]:
        result = run(
            sys.executable, str(INSTALLER), "--source-root", str(self.clone),
            "--sync", "--target", str(self.target), *extra,
        )
        self.assertEqual("", result.stderr)
        return result.returncode, json.loads(result.stdout)

    def link(self, relative: str, outside: Path, move: bool) -> None:
        """Put a symbolic link to `outside` where the project's folder was.

        `move` takes the folder's files along; otherwise `outside` is empty.
        """
        folder = self.target / relative
        if move:
            folder.rename(outside)
        else:
            shutil.rmtree(folder)
            outside.mkdir()
        folder.symlink_to(outside, target_is_directory=True)

    def release(self, path: str, text: str) -> None:
        source = self.clone / "Symfony" / path
        source.write_text(source.read_text(encoding="utf-8") + text, encoding="utf-8")

    def actions(self, report: dict) -> dict:
        return {item["path"]: item["action"] for item in report["changed"]}

    def kept(self, report: dict) -> dict:
        return {item["path"]: item["reason"] for item in report["kept"]}

    def test_a_fresh_install_is_already_current(self) -> None:
        report = self.sync()
        self.assertEqual(("Symfony", None), (report["edition"], report["error"]))
        self.assertEqual([], report["changed"])
        self.assertTrue((self.target / "memory-bank/local/accelerator-install.json").is_file())

    def test_a_new_release_reaches_untouched_files_only(self) -> None:
        self.sync()  # records what the install wrote
        skill = ".agents/skills/memory/SKILL.md"
        edited = ".agents/skills/checkpoint/SKILL.md"
        self.release(skill, "\nNew release note.\n")
        self.release(edited, "\nAnother release note.\n")
        local = self.target / edited
        local.write_text(local.read_text(encoding="utf-8") + "\nTeam rule.\n", encoding="utf-8")

        report = self.sync()
        self.assertEqual("updated", self.actions(report).get(skill))
        self.assertIn("New release note.", (self.target / skill).read_text(encoding="utf-8"))
        self.assertEqual("edited in the project", self.kept(report).get(edited))
        self.assertIn("Team rule.", local.read_text(encoding="utf-8"))

    def test_the_runtime_follows_the_release_over_a_local_edit(self) -> None:
        runtime = "memory-bank/scripts/context.py"
        local = self.target / runtime
        local.write_text(local.read_text(encoding="utf-8") + "\n# local patch\n", encoding="utf-8")
        report = self.sync()
        self.assertEqual("updated over a local edit", self.actions(report).get(runtime))
        self.assertEqual(
            (self.clone / "Symfony" / runtime).read_bytes(), local.read_bytes()
        )
        backup = self.target / report["backups"][0]
        self.assertTrue(backup.relative_to(self.target).as_posix().startswith(
            "memory-bank/local/accelerator-sync/"
        ))
        self.assertIn("# local patch", backup.read_text(encoding="utf-8"))

    def test_what_a_person_owns_is_never_written(self) -> None:
        # Codex trusts hooks by the hash of their definitions, the project's
        # Git history is the team's, and seeded state is the project's own.
        hooks = self.target / ".codex/hooks.json"
        hooks.write_text(hooks.read_text(encoding="utf-8").replace("\n", "\n ", 1), encoding="utf-8")
        tracked = ".agents/skills/memory/SKILL.md"
        run("git", "-C", str(self.target), "add", "--", tracked)
        self.release(tracked, "\nRelease edit of a tracked file.\n")
        runtime_config = self.target / "project-brain/config/runtime.json"
        runtime_config.write_text('{"mode": "governed"}\n', encoding="utf-8")

        report = self.sync()
        kept = self.kept(report)
        self.assertIn("re-approval", kept.get(".codex/hooks.json", ""))
        self.assertIn("tracked", kept.get(tracked, ""))
        self.assertNotIn("Release edit", (self.target / tracked).read_text(encoding="utf-8"))
        self.assertEqual('{"mode": "governed"}\n', runtime_config.read_text(encoding="utf-8"))
        self.assertNotIn("project-brain/config/runtime.json", self.actions(report))

    def test_codex_wiring_is_rewritten_only_for_a_caller_that_approves_it_again(self) -> None:
        self.sync()  # records what the install wrote
        wiring = ".codex/hooks.json"
        self.release(wiring, "\n")
        released = (self.clone / "Symfony" / wiring).read_bytes()
        self.assertIn("re-approval", self.kept(self.sync()).get(wiring, ""))
        self.assertNotEqual(released, (self.target / wiring).read_bytes())

        report = self.sync("--rewire-codex")
        self.assertEqual("updated", self.actions(report).get(wiring))
        self.assertEqual(released, (self.target / wiring).read_bytes())

        # A team's own edit stays, approval or not.
        local = self.target / wiring
        local.write_text(local.read_text(encoding="utf-8") + " ", encoding="utf-8")
        self.release(wiring, "\n")
        report = self.sync("--rewire-codex")
        self.assertEqual("edited in the project", self.kept(report).get(wiring))

    def test_codex_project_settings_follow_the_release(self) -> None:
        # Not trust-bound: Codex approves hook definitions, not this file. It
        # carries the AGENTS.md budget the policy needs to be read whole.
        self.sync()
        config = ".codex/config.toml"
        self.release(config, "\n# release note\n")
        report = self.sync()
        self.assertEqual("updated", self.actions(report).get(config))
        self.assertIn("project_doc_max_bytes", (self.target / config).read_text(encoding="utf-8"))
        local = self.target / config
        local.write_text(local.read_text(encoding="utf-8") + "model = \"team\"\n", encoding="utf-8")
        self.release(config, "# another note\n")
        # Appended after the managed block, the team's key is TOML of the
        # memory server's own table: the release's merger refuses that table,
        # and the file stays the project's to reconcile.
        reason = self.kept(self.sync()).get(config, "")
        self.assertIn("memory MCP configuration not merged", reason)
        self.assertIn('model = "team"', local.read_text(encoding="utf-8"))

    def test_rewiring_codex_needs_a_sync(self) -> None:
        result = run(
            sys.executable, str(INSTALLER), "--source-root", str(self.clone),
            "--edition", "Symfony", "--rewire-codex", "--target", str(self.target), "--dry-run",
        )
        self.assertEqual(2, result.returncode)
        self.assertIn("--rewire-codex requires --sync", result.stderr)

    def test_only_the_managed_policy_block_is_replaced(self) -> None:
        agents = self.target / "AGENTS.md"
        agents.write_text(
            "# Project rules\n\nKeep this.\n\n<!-- BEGIN ACCELERATOR MANAGED POLICY -->\n"
            "old policy\n<!-- END ACCELERATOR MANAGED POLICY -->\n",
            encoding="utf-8",
        )
        report = self.sync()
        self.assertEqual("managed block updated", self.actions(report).get("AGENTS.md"))
        text = agents.read_text(encoding="utf-8")
        self.assertTrue(text.startswith("# Project rules\n\nKeep this."))
        self.assertNotIn("old policy", text)
        self.assertIn((self.clone / "Symfony/AGENTS.md").read_text(encoding="utf-8").strip(), text)

    def test_a_missing_policy_import_is_added_and_no_tool_is_installed_unasked(self) -> None:
        (self.target / ".claude/CLAUDE.md").unlink()
        report = self.sync()
        self.assertEqual("added", self.actions(report).get(".claude/CLAUDE.md"))
        self.assertIn("@../AGENTS.md", (self.target / ".claude/CLAUDE.md").read_text(encoding="utf-8"))
        # Cursor was not installed, so the release does not install it (the
        # shared component's .cursor/README.md is all an install puts there).
        self.assertFalse((self.target / ".cursor/hooks.json").exists())
        self.assertFalse((self.target / ".cursor/skills").exists())

    def test_a_dry_run_writes_nothing(self) -> None:
        runtime = self.target / "memory-bank/scripts/context.py"
        runtime.write_text("# edited\n", encoding="utf-8")
        report = self.sync("--dry-run")
        self.assertTrue(report["dry_run"])
        self.assertIn("memory-bank/scripts/context.py", self.actions(report))
        self.assertEqual("# edited\n", runtime.read_text(encoding="utf-8"))
        self.assertFalse((self.target / "memory-bank/local/accelerator-install.json").exists())

    def test_a_linked_tool_folder_is_not_written_through(self) -> None:
        # A checkout can carry a link where `.cursor` was. Only the last
        # component was checked, so the missing `.cursor/README.md` was
        # "added" - through the link, outside the project.
        outside = Path(self._tmp.name) / "outside"
        self.link(".cursor", outside, move=False)
        for extra in (("--dry-run",), ()):
            with self.subTest(dry_run=bool(extra)):
                report = self.sync(*extra)
                self.assertIsNone(report["error"])
                self.assertNotIn(".cursor/README.md", self.actions(report))
                self.assertIn(
                    ".cursor is a symbolic link", self.kept(report).get(".cursor/README.md", "")
                )
                self.assertEqual({}, files_under(outside))
        # A file where a folder belongs is no way through either.
        (self.target / ".cursor").unlink()
        (self.target / ".cursor").write_text("the project's file\n", encoding="utf-8")
        report = self.sync()
        self.assertEqual(".cursor is not a folder", self.kept(report).get(".cursor/README.md"))
        self.assertEqual("the project's file\n", (self.target / ".cursor").read_text(encoding="utf-8"))

    def test_a_linked_memory_bank_is_not_synced_through(self) -> None:
        # Through the link, the runtime "was" installed: a local edit outside
        # the project was replaced, and its backup and the sync's record were
        # written outside too.
        outside = Path(self._tmp.name) / "outside-memory"
        self.link("memory-bank", outside, move=True)
        runtime = outside / "scripts/context.py"
        runtime.write_text(runtime.read_text(encoding="utf-8") + "\n# local marker\n", encoding="utf-8")
        before = files_under(outside)
        for extra in (("--dry-run",), ()):
            with self.subTest(dry_run=bool(extra)):
                returncode, report = self.sync_result(*extra)
                self.assertEqual(1, returncode)
                self.assertIn("memory-bank is a symbolic link", report["error"])
                self.assertEqual(([], [], []), (report["changed"], report["kept"], report["backups"]))
                self.assertEqual(before, files_under(outside))

    def test_backups_and_the_record_stay_inside_the_project(self) -> None:
        # With `memory-bank` itself in place the runtime is the project's,
        # but its backups and the sync's record live under memory-bank/local.
        self.sync()  # records what the install wrote
        outside = Path(self._tmp.name) / "outside-local"
        self.link("memory-bank/local", outside, move=True)
        before = files_under(outside)
        runtime = self.target / "memory-bank/scripts/context.py"
        edited = runtime.read_text(encoding="utf-8") + "\n# local patch\n"
        runtime.write_text(edited, encoding="utf-8")
        skill = self.target / ".agents/skills/memory/SKILL.md"
        skill.unlink()
        for extra in (("--dry-run",), ()):
            with self.subTest(dry_run=bool(extra)):
                report = self.sync(*extra)
                kept = self.kept(report)
                # Without its backup the local edit would be lost, so it stays.
                self.assertIn(
                    "backup cannot be written inside the project",
                    kept.get("memory-bank/scripts/context.py", ""),
                )
                self.assertIn(
                    "memory-bank/local is a symbolic link",
                    kept.get("memory-bank/local/accelerator-install.json", ""),
                )
                self.assertEqual([], report["backups"])
                self.assertEqual(edited, runtime.read_text(encoding="utf-8"))
                self.assertEqual(before, files_under(outside))
                # The rest of the project still follows the release.
                self.assertEqual("added", self.actions(report).get(".agents/skills/memory/SKILL.md"))
        self.assertTrue(skill.is_file())

    def test_a_folder_without_an_install_is_refused(self) -> None:
        empty = Path(self._tmp.name) / "empty"
        empty.mkdir()
        result = run(
            sys.executable, str(INSTALLER), "--source-root", str(self.clone),
            "--sync", "--target", str(empty),
        )
        self.assertEqual(1, result.returncode)
        self.assertEqual("not an installed accelerator", json.loads(result.stdout)["error"])

    def test_released_history_identifies_an_untouched_old_copy(self) -> None:
        # With no manifest yet - every install made before syncs existed - the
        # clone's own history says whether a file is an untouched release.
        log = run(
            "git", "log", "--format=%H", "-n", "2", "--",
            "Symfony/.agents/skills/memory/SKILL.md",
        )
        commits = log.stdout.split()
        if log.returncode != 0 or len(commits) < 2:
            self.skipTest("the clone has no older release of this file (shallow history)")
        older = run("git", "show", f"{commits[1]}:Symfony/.agents/skills/memory/SKILL.md")
        if older.stdout == (ROOT / "Symfony/.agents/skills/memory/SKILL.md").read_text(encoding="utf-8"):
            self.skipTest("the older release is identical")
        target = self.target / ".agents/skills/memory/SKILL.md"
        target.write_text(older.stdout, encoding="utf-8")
        sys.path.insert(0, str(ROOT / "scripts"))
        try:
            import install_accelerator
        finally:
            sys.path.pop(0)
        report = install_accelerator.sync_installation(ROOT, self.target, dry_run=True)
        self.assertEqual(
            "updated",
            {item["path"]: item["action"] for item in report["changed"]}.get(
                ".agents/skills/memory/SKILL.md"
            ),
        )


class McpConfigSyncTest(unittest.TestCase):
    """The memory server's entry follows the release beside a project's own servers.

    Install merges these configurations instead of copying them, so one that
    also holds a team's server is never the release's bytes, and a sync that
    treated it as an edited file kept its memory server at the install's
    version forever.
    """

    MCP = {"claude": ".mcp.json", "cursor": ".cursor/mcp.json", "codex": ".codex/config.toml"}
    TEAM = {"command": "node", "args": ["team-server.js"]}
    TEAM_TABLE = '\n[mcp_servers.team]\ncommand = "node"\nargs = ["team-server.js"]\n'
    BLOCK = re.compile(rb"# BEGIN HARNESS MEMORY MCP\n.*?# END HARNESS MEMORY MCP\n?", re.S)

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="mcp sync ")
        self.addCleanup(self._tmp.cleanup)
        base = Path(self._tmp.name)
        self.clone = base / "clone"
        shutil.copytree(ROOT / "install" / "inventories", self.clone / "install" / "inventories")
        shutil.copytree(ROOT / "Symfony", self.clone / "Symfony")
        self.target = base / "project"
        self.target.mkdir()
        run("git", "init", "--quiet", str(self.target))
        for host in ("claude", "cursor"):
            config = self.target / self.MCP[host]
            config.parent.mkdir(exist_ok=True)
            config.write_text(json.dumps({"mcpServers": {"team-server": self.TEAM}}, indent=2) + "\n")
        installed = run(
            sys.executable, str(INSTALLER), "--source-root", str(self.clone),
            "--edition", "Symfony", "--tool", "claude", "--tool", "cursor", "--tool", "codex",
            "--merge-existing", "--target", str(self.target),
        )
        self.assertEqual(0, installed.returncode, installed.stderr)
        # The team adds a server of its own to the Codex configuration too: a
        # table after the managed block, so it is TOML of its own.
        codex = self.target / self.MCP["codex"]
        codex.write_text(codex.read_text(encoding="utf-8") + self.TEAM_TABLE, encoding="utf-8")
        self.sync()  # records what the install wrote

    def sync(self, *extra: str) -> dict:
        result = run(
            sys.executable, str(INSTALLER), "--source-root", str(self.clone),
            "--sync", "--target", str(self.target), *extra,
        )
        self.assertEqual(0, result.returncode, result.stderr)
        return json.loads(result.stdout)

    def released(self):
        """The release's MCP merger, as the sync loads it from the clone."""
        spec = importlib.util.spec_from_file_location(
            "released_mcp_config", self.clone / "Symfony/memory-bank/scripts/mcp_config.py"
        )
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def publish_bootstrap(self, version: int):
        """A release that changes the memory server's launcher, shipped as install ships it."""
        path = self.clone / "Symfony/memory-bank/scripts/mcp_config.py"
        text = path.read_text(encoding="utf-8")
        current = re.search(r"# Harness memory bootstrap v\d+\n", text).group(0)
        path.write_text(
            text.replace(current, f"# Harness memory bootstrap v{version}\n# release {version}\n", 1),
            encoding="utf-8",
        )
        released = self.released()
        (self.clone / "Symfony" / self.MCP["claude"]).write_bytes(released.template("claude"))
        shipped = self.clone / "Symfony" / self.MCP["codex"]
        block = self.BLOCK.search(shipped.read_bytes()).group(0)
        shipped.write_bytes(shipped.read_bytes().replace(block, released.codex_block().encode()))
        return released

    def servers(self, host: str) -> dict:
        return json.loads((self.target / self.MCP[host]).read_text(encoding="utf-8"))["mcpServers"]

    def mcp_actions(self, report: dict) -> dict:
        return {item["path"]: item["action"] for item in report["changed"] if item["path"] in self.MCP.values()}

    def mcp_kept(self, report: dict) -> dict:
        return {item["path"]: item["reason"] for item in report["kept"] if item["path"] in self.MCP.values()}

    def test_the_memory_server_follows_the_release_beside_the_projects_servers(self) -> None:
        released = self.publish_bootstrap(2)
        python = released.python_command()
        # Cursor's entry has no bootstrap; an install on a machine whose
        # Python answered to another name holds an older entry of ours.
        other = next(candidate for candidate in released.PYTHONS if candidate != python)
        cursor = self.target / self.MCP["cursor"]
        data = json.loads(cursor.read_text(encoding="utf-8"))
        data["mcpServers"]["harness-memory"] = released.entry("cursor", other)
        cursor.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
        before = {path: (self.target / path).read_bytes() for path in self.MCP.values()}
        expected = {path: "memory server updated" for path in self.MCP.values()}

        preview = self.sync("--dry-run")
        self.assertEqual(expected, self.mcp_actions(preview))
        self.assertEqual(before, {path: (self.target / path).read_bytes() for path in self.MCP.values()})

        report = self.sync()
        self.assertEqual(expected, self.mcp_actions(report))
        for host in ("claude", "cursor"):
            with self.subTest(host=host):
                servers = self.servers(host)
                self.assertEqual(released.entry(host, python), servers["harness-memory"])
                self.assertEqual(self.TEAM, servers["team-server"])
        codex = (self.target / self.MCP["codex"]).read_text(encoding="utf-8")
        self.assertIn(released.codex_block(python), codex)
        self.assertIn("bootstrap v2", codex)
        self.assertTrue(codex.endswith(self.TEAM_TABLE))
        self.assertIn("hooks = true", codex)

        # A merged file is the project's, not an untouched release: the next
        # release merges into it again instead of replacing it whole.
        released = self.publish_bootstrap(3)
        report = self.sync()
        self.assertEqual(
            {self.MCP["claude"]: "memory server updated", self.MCP["codex"]: "memory server updated"},
            self.mcp_actions(report),
        )
        self.assertEqual({self.MCP["cursor"]: "edited in the project"}, self.mcp_kept(report))
        self.assertEqual(self.TEAM, self.servers("claude")["team-server"])
        self.assertIn("bootstrap v3", self.servers("claude")["harness-memory"]["args"][1])
        codex = (self.target / self.MCP["codex"]).read_text(encoding="utf-8")
        self.assertIn("bootstrap v3", codex)
        self.assertTrue(codex.endswith(self.TEAM_TABLE))

        # Current, the files are left alone and named as the project's.
        report = self.sync()
        self.assertEqual({}, self.mcp_actions(report))
        self.assertEqual({path: "edited in the project" for path in self.MCP.values()}, self.mcp_kept(report))

    def test_a_project_config_without_the_memory_server_gets_it(self) -> None:
        claude = self.target / self.MCP["claude"]
        claude.write_text(json.dumps({"mcpServers": {"team-server": self.TEAM}}, indent=2) + "\n")
        report = self.sync()
        self.assertEqual({self.MCP["claude"]: "memory server updated"}, self.mcp_actions(report))
        released = self.released()
        self.assertEqual(
            {"team-server": self.TEAM, "harness-memory": released.entry("claude", released.python_command())},
            self.servers("claude"),
        )

    def test_a_malformed_config_or_a_foreign_memory_server_is_left_alone(self) -> None:
        self.publish_bootstrap(2)
        codex = self.target / self.MCP["codex"]
        broken = {
            # Not JSON any more.
            self.MCP["claude"]: (b'{"mcpServers": {"team-server": ', "Invalid MCP JSON configuration"),
            # Another server under the memory server's name.
            self.MCP["cursor"]: (
                json.dumps({"mcpServers": {"harness-memory": self.TEAM}}, indent=2).encode() + b"\n",
                "belongs to another server",
            ),
            # The managed block edited by hand.
            self.MCP["codex"]: (
                codex.read_bytes().replace(b"enabled = true\n# END", b"enabled = false\n# END"),
                "Managed memory MCP block was edited",
            ),
        }
        for path, (content, _) in broken.items():
            (self.target / path).write_bytes(content)
        for extra in (("--dry-run",), ()):
            with self.subTest(dry_run=bool(extra)):
                report = self.sync(*extra)
                self.assertEqual({}, self.mcp_actions(report))
                kept = self.mcp_kept(report)
                for path, (content, reason) in broken.items():
                    self.assertIn("memory MCP configuration not merged", kept.get(path, ""))
                    self.assertIn(reason, kept.get(path, ""))
                    self.assertEqual(content, (self.target / path).read_bytes())


@unittest.skipUnless(
    install_accelerator._DESCRIPTOR_WALK,
    "the descriptor walk needs os.supports_dir_fd; native Windows looks at each component first",
)
class SyncRaceTest(unittest.TestCase):
    """A folder swapped for a link between the sync's look at a path and its write.

    The sync looked at every folder of a path and then wrote by the path: a
    `.cursor` replaced by a link in between sent `.cursor/mcp.json` to the
    link's target. Each case swaps the folder the moment the write creates
    its temporary file - after every look, before the bytes exist - once by
    moving the real folder elsewhere in the project and once by deleting it,
    and runs `--sync` as the command line does.
    """

    HOW = ("moved", "removed")

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="sync race ")
        self.addCleanup(self._tmp.cleanup)

    def project(self, name: str) -> tuple[Path, Path]:
        base = Path(self._tmp.name) / name
        target, outside = base / "project", base / "outside"
        target.mkdir(parents=True)
        outside.mkdir()
        run("git", "init", "--quiet", str(target))
        install_quietly(target, "Laravel", ["claude", "cursor", "codex"])
        return target, outside

    def sync(self, target: Path, outside: Path, folder: str, marker: str, how: str) -> dict:
        with swapped_while_written(target, folder, outside, marker, how) as swaps:
            code, out, err = run_installer("--source-root", str(ROOT), "--sync", "--target", str(target))
        self.assertEqual(1, len(swaps), "the write never created its temporary file")
        self.assertEqual((0, ""), (code, err))
        return json.loads(out)

    @staticmethod
    def kept(report: dict) -> dict:
        return {item["path"]: item["reason"] for item in report["kept"]}

    def test_a_tool_folder_swapped_while_the_sync_writes_into_it(self) -> None:
        for how in self.HOW:
            with self.subTest(how=how):
                target, outside = self.project(f"tool {how}")
                (target / ".cursor/mcp.json").unlink()  # the sync adds it back
                report = self.sync(target, outside, ".cursor", "mcp.json.", how)
                self.assertEqual({}, files_under(outside))
                self.assertNotIn(".cursor/mcp.json", {item["path"] for item in report["changed"]})
                self.assertIn(".cursor is a symbolic link", self.kept(report).get(".cursor/mcp.json", ""))
                self.assertEqual([], temporaries(target))

    def test_the_backup_folder_swapped_while_a_local_edit_is_saved(self) -> None:
        for how in self.HOW:
            with self.subTest(how=how):
                target, outside = self.project(f"backup {how}")
                install_accelerator.sync_installation(ROOT, target)  # the sync's record
                runtime = target / "memory-bank/scripts/context.py"
                edited = runtime.read_text(encoding="utf-8") + "\n# local patch\n"
                runtime.write_text(edited, encoding="utf-8")
                report = self.sync(target, outside, "memory-bank/local", "context.py.", how)
                kept = self.kept(report)
                self.assertEqual({}, files_under(outside))
                # Without its backup the edit would be lost, so it stays.
                reason = kept.get("memory-bank/scripts/context.py", "")
                self.assertIn("backup cannot be written inside the project", reason)
                self.assertIn("memory-bank/local is a symbolic link", reason)
                self.assertEqual(edited, runtime.read_text(encoding="utf-8"))
                self.assertEqual([], report["backups"])
                self.assertIn(
                    "memory-bank/local is a symbolic link",
                    kept.get(install_accelerator.SYNC_MANIFEST, ""),
                )
                self.assertEqual([], temporaries(target))

    def test_the_record_folder_swapped_while_the_record_is_written(self) -> None:
        for how in self.HOW:
            with self.subTest(how=how):
                target, outside = self.project(f"record {how}")
                report = self.sync(target, outside, "memory-bank/local", "accelerator-install.json.", how)
                self.assertEqual({}, files_under(outside))
                self.assertIn(
                    "memory-bank/local is a symbolic link",
                    self.kept(report).get(install_accelerator.SYNC_MANIFEST, ""),
                )
                self.assertEqual([], temporaries(target))


class SyncWithoutDescriptorsTest(unittest.TestCase):
    """A platform whose os.supports_dir_fd lacks the calls, as native Windows does.

    The sync looked at each component with lstat there and then wrote by the
    path, and a `.cursor` swapped for a link in between sent `.cursor/mcp.json`
    to the link's target (a review reproduced it on this same fallback under
    Linux). Windows now walks by NT handles; where neither descriptors nor
    handles are available - here: descriptors taken away, and Linux has no
    handle calls - nothing is written and each file is reported in `kept`.
    """

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="sync without descriptors ")
        self.addCleanup(self._tmp.cleanup)
        self.base = Path(self._tmp.name)
        self.target = self.base / "project"
        self.target.mkdir()
        run("git", "init", "--quiet", str(self.target))
        install_quietly(self.target, "Laravel", ["claude", "cursor", "codex"])
        self.outside = self.base / "outside"
        self.outside.mkdir()

    @staticmethod
    def without_descriptors():
        return patch.object(install_accelerator, "_DESCRIPTOR_WALK", False)

    def test_a_folder_swapped_while_the_sync_writes_takes_no_write(self) -> None:
        (self.target / ".cursor/mcp.json").unlink()  # the sync adds it back
        with self.without_descriptors(), swapped_while_written(
            self.target, ".cursor", self.outside, "mcp.json.", "moved"
        ) as swaps:
            code, out, err = run_installer("--source-root", str(ROOT), "--sync", "--target", str(self.target))
        self.assertEqual((0, ""), (code, err))
        report = json.loads(out)
        self.assertEqual({}, files_under(self.outside))
        # Nothing was even begun: no temporary file for the swap to race.
        self.assertEqual([], swaps)
        kept = {item["path"]: item["reason"] for item in report["kept"]}
        self.assertEqual(install_accelerator.NO_SAFE_WRITE, kept.get(".cursor/mcp.json"))
        self.assertFalse((self.target / ".cursor/mcp.json").exists())
        self.assertEqual([], temporaries(self.target))

    def test_nothing_is_written_and_every_write_is_reported(self) -> None:
        runtime = self.target / "memory-bank/scripts/context.py"
        edited = runtime.read_text(encoding="utf-8") + "\n# local patch\n"
        runtime.write_text(edited, encoding="utf-8")
        (self.target / ".claude/CLAUDE.md").unlink()
        shutil.rmtree(self.target / ".codex")
        (self.target / ".codex").symlink_to(self.outside, target_is_directory=True)
        before = files_under(self.target)

        with self.without_descriptors():
            report = install_accelerator.sync_installation(ROOT, self.target)
        kept = {item["path"]: item["reason"] for item in report["kept"]}
        self.assertIsNone(report["error"])
        self.assertEqual(([], []), (report["changed"], report["backups"]))
        self.assertEqual(before, files_under(self.target))
        self.assertEqual({}, files_under(self.outside))
        # Reads still look at each component first: the link is named.
        self.assertIn(".codex is a symbolic link", kept.get(".codex/hooks.json", ""))
        self.assertEqual(install_accelerator.NO_SAFE_WRITE, kept.get(".claude/CLAUDE.md"))
        # Without its backup a local edit would be lost: it stays.
        self.assertEqual(
            "not replaced: its backup cannot be written inside the project "
            f"({install_accelerator.NO_SAFE_WRITE})",
            kept.get("memory-bank/scripts/context.py"),
        )
        self.assertEqual(
            f"the sync's record is not written ({install_accelerator.NO_SAFE_WRITE})",
            kept.get(install_accelerator.SYNC_MANIFEST),
        )

    def test_an_install_writes_no_merge_by_path(self) -> None:
        mcp = self.target / ".mcp.json"
        team = json.dumps({"mcpServers": {"team-server": {"command": "node", "args": []}}}, indent=2) + "\n"
        mcp.write_text(team, encoding="utf-8")
        with self.without_descriptors():
            code, out, err = run_installer(
                "--edition", "Laravel", "--target", str(self.target), "--tool", "claude", "--merge-existing"
            )
        self.assertEqual(1, code)
        self.assertIn(install_accelerator.NO_SAFE_WRITE, err)
        self.assertEqual(team, mcp.read_text(encoding="utf-8"))
        self.assertEqual([], temporaries(self.target))


class StandInCalls:
    """The descriptor calls standing in for another platform's (Windows: NT handles).

    They keep the contract a walk relies on - one component at a time,
    relative to an open folder - and record what they are asked.
    """

    def __init__(self) -> None:
        self._real = install_accelerator._DescriptorCalls()
        self.used: dict[str, int] = {}
        self.names: list[str] = []

    def __getattr__(self, attribute: str):
        call = getattr(self._real, attribute)

        def recorded(*arguments):
            self.used[attribute] = self.used.get(attribute, 0) + 1
            if attribute != "open_project":
                self.names.extend(argument for argument in arguments if isinstance(argument, str))
            return call(*arguments)

        return recorded


class SyncThroughStandInCallsTest(unittest.TestCase):
    """Where descriptors cannot walk, the walk takes whatever calls stand in for them.

    On native Windows those are `_HandleCalls`, which cannot run here; the
    same walk through the stand-in shows that every read, write, backup and
    the record go through those calls - never by path below the project -
    and that the swap which took `.cursor/mcp.json` outside takes nothing.
    """

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="sync through stand-in calls ")
        self.addCleanup(self._tmp.cleanup)
        self.calls = StandInCalls()

    def project(self, name: str) -> tuple[Path, Path]:
        base = Path(self._tmp.name) / name
        target, outside = base / "project", base / "outside"
        target.mkdir(parents=True)
        outside.mkdir()
        run("git", "init", "--quiet", str(target))
        install_quietly(target, "Laravel", ["claude", "cursor", "codex"])
        return target, outside

    @contextlib.contextmanager
    def through_stand_in(self):
        with patch.object(install_accelerator, "_DESCRIPTOR_WALK", False), patch.object(
            install_accelerator, "_HANDLE_CALLS", self.calls
        ):
            yield

    def assert_one_component_each(self) -> None:
        for name in self.calls.names:
            self.assertTrue(name and "/" not in name and os.sep not in name, name)

    def test_a_folder_swapped_while_written_takes_no_write_outside(self) -> None:
        for how in ("moved", "removed"):
            with self.subTest(how=how):
                target, outside = self.project(how)
                (target / ".cursor/mcp.json").unlink()  # the sync adds it back
                with self.through_stand_in(), swapped_while_written(
                    target, ".cursor", outside, "mcp.json.", how
                ) as swaps:
                    report = install_accelerator.sync_installation(ROOT, target)
                self.assertEqual(1, len(swaps), "the write never created its temporary file")
                self.assertEqual({}, files_under(outside))
                kept = {item["path"]: item["reason"] for item in report["kept"]}
                self.assertIn(".cursor is a symbolic link", kept.get(".cursor/mcp.json", ""))
                self.assertNotIn(".cursor/mcp.json", {item["path"] for item in report["changed"]})
                self.assertEqual([], temporaries(target))
        self.assertTrue(self.calls.used.get("create_file") and self.calls.used.get("unlink"))
        self.assert_one_component_each()

    def test_reads_writes_backups_and_the_record_go_through_the_calls(self) -> None:
        target, _ = self.project("whole")
        runtime = target / "memory-bank/scripts/context.py"
        runtime.write_text(runtime.read_text(encoding="utf-8") + "\n# local patch\n", encoding="utf-8")
        (target / ".claude/CLAUDE.md").unlink()
        project = os.fspath(target)
        real_open = os.open
        by_path: list[str] = []

        def watching_open(path, flags, *arguments, **options):
            text = os.fsdecode(path)
            if os.path.isabs(text) and text != project and text.startswith(project + os.sep):
                by_path.append(text)
            return real_open(path, flags, *arguments, **options)

        with self.through_stand_in(), patch.object(os, "open", watching_open):
            report = install_accelerator.sync_installation(ROOT, target)
        self.assertIsNone(report["error"])
        self.assertEqual([], by_path)
        changed = {item["path"]: item["action"] for item in report["changed"]}
        self.assertEqual("updated over a local edit", changed.get("memory-bank/scripts/context.py"))
        self.assertEqual("added", changed.get(".claude/CLAUDE.md"))
        [backup] = report["backups"]
        self.assertIn("# local patch", (target / backup).read_text(encoding="utf-8"))
        self.assertTrue((target / install_accelerator.SYNC_MANIFEST).is_file())
        self.assertEqual(
            (ROOT / "Laravel/memory-bank/scripts/context.py").read_bytes(), runtime.read_bytes()
        )
        for call in ("open_project", "open_folder", "make_folder", "lstat", "open_file", "create_file",
                     "fstat", "chmod", "set_times", "rename", "close"):
            self.assertTrue(self.calls.used.get(call), call)
        self.assert_one_component_each()


class InterruptedMergeTest(unittest.TestCase):
    """An install that merges into a project's file and fails part way leaves that file whole.

    The merge was written over the file in place: a disk that filled after
    the first byte left `{` where a team's `.mcp.json` had been, and every
    later install refused to merge into it
    (`cannot-merge:memory-mcp-configuration`).
    """

    TEAM = {"command": "node", "args": ["team-server.js"]}
    # A time the install would never give a file, to see it put back.
    MTIME = 1_577_880_000_000_000_000

    def setUp(self) -> None:
        self._tmp = tempfile.TemporaryDirectory(prefix="interrupted merge ")
        self.addCleanup(self._tmp.cleanup)
        self.target = Path(self._tmp.name) / "project"
        self.target.mkdir()

    def own(self, relative: str, content: str, mode: int) -> None:
        """A file the project had before the install."""
        path = self.target / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        os.chmod(path, mode)
        os.utime(path, ns=(self.MTIME, self.MTIME))

    def team_config(self) -> str:
        return json.dumps({"mcpServers": {"team-server": self.TEAM}}, indent=2) + "\n"

    def state(self, paths) -> dict:
        return {
            path: (
                (self.target / path).read_bytes(),
                stat.S_IMODE((self.target / path).stat().st_mode),
                (self.target / path).stat().st_mtime_ns,
            )
            for path in paths
        }

    @staticmethod
    def records(output: str, action: str) -> list[str]:
        """The inventory paths of an install's `action` records, sorted."""
        return sorted(line.split("\t")[2] for line in output.splitlines() if line.startswith(action + "\t"))

    def install(self, *tools: str) -> list[str]:
        command = ["--edition", "Laravel", "--target", str(self.target), "--merge-existing"]
        for tool in tools:
            command += ["--tool", tool]
        return command

    def test_a_merge_that_runs_out_of_space_leaves_the_config_whole(self) -> None:
        self.own(".mcp.json", self.team_config(), 0o640)
        before = self.state([".mcp.json"])
        command = self.install("claude")
        with disk_full_while_written("mcp.json.") as hit:
            code, out, err = run_installer(*command)
        self.assertTrue(hit, "the merge never created its temporary file")
        self.assertEqual(1, code)
        self.assertIn("No space left on device", err)
        self.assertEqual(before, self.state([".mcp.json"]))
        self.assertEqual([], temporaries(self.target))

        again = run(sys.executable, str(INSTALLER), *command)
        self.assertEqual(0, again.returncode, again.stderr)
        self.assertIn("MERGE\tclaude\t.mcp.json\t.mcp.json", again.stdout)
        servers = json.loads((self.target / ".mcp.json").read_text(encoding="utf-8"))["mcpServers"]
        self.assertEqual(self.TEAM, servers["team-server"])
        self.assertIn("harness-memory", servers)
        self.assertEqual(0o640, stat.S_IMODE((self.target / ".mcp.json").stat().st_mode))

    def project_with_own_files(self) -> list[str]:
        self.own("AGENTS.md", "# Project policy\n\nKeep project behavior.\n", 0o644)
        self.own(".gitignore", ".env\n/vendor/\n", 0o600)
        self.own(".mcp.json", self.team_config(), 0o640)
        self.own(".cursor/mcp.json", self.team_config(), 0o644)
        return ["AGENTS.md", ".gitignore", ".mcp.json", ".cursor/mcp.json"]

    def test_a_failed_install_puts_back_the_files_it_had_merged(self) -> None:
        # The Codex configuration is the last MCP file in inventory order, so
        # the project's policy, ignore list and both MCP configurations have
        # been merged by the time it fails.
        owned = self.project_with_own_files()
        before = self.state(owned)
        command = self.install("claude", "cursor", "codex")
        with disk_full_while_written("config.toml.") as hit:
            code, out, err = run_installer(*command)
        self.assertTrue(hit, "the install never wrote the Codex configuration")
        self.assertEqual(1, code)
        self.assertEqual(sorted(owned), self.records(out, "MERGE"))
        self.assertEqual(sorted(owned), self.records(out, "RESTORED"))
        self.assertEqual(before, self.state(owned))
        self.assertFalse((self.target / ".codex/config.toml").exists())
        self.assertEqual([], temporaries(self.target))

        again = run(sys.executable, str(INSTALLER), *command)
        self.assertEqual(0, again.returncode, again.stderr)
        self.assertEqual(sorted(owned), self.records(again.stdout, "MERGE"))
        self.assertTrue((self.target / ".codex/config.toml").is_file())

    def test_a_file_changed_after_the_install_wrote_it_is_not_put_back(self) -> None:
        owned = self.project_with_own_files()
        command = self.install("claude", "cursor", "codex")
        edited = '{"mcpServers": {"edited-meanwhile": {"command": "node", "args": []}}}\n'

        def someone_else_edits():
            (self.target / ".mcp.json").write_text(edited, encoding="utf-8")

        with disk_full_while_written("config.toml.", also=someone_else_edits):
            code, out, err = run_installer(*command)
        self.assertEqual(1, code)
        self.assertEqual(edited, (self.target / ".mcp.json").read_text(encoding="utf-8"))
        self.assertIn(
            "NOT_RESTORED\tclaude\t.mcp.json\t.mcp.json\tchanged after the install wrote it", err
        )
        self.assertEqual(sorted(set(owned) - {".mcp.json"}), self.records(out, "RESTORED"))


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
                    # The project-brain skill tells agents to run parity, so
                    # every tool selection must pass it, including the
                    # single-tool installs that ship one skill tree.
                    (*context_command, "parity"),
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


@unittest.skipIf(os.name == "nt", "the filesystem has no executable bit to assert")
class ExecutableHookInstallTest(unittest.TestCase):
    """Every hook a client runs directly must arrive executable.

    Cursor wires `.cursor/hooks/subagent-dispatch.sh` (subagentStop) as a
    direct command. It once shipped 100644 in Cursor and Codex mirrors of all
    four editions, so every installed project got exit status 126 there and
    the write-agent lock subagent-gate takes was never released: the next
    write agent was refused until the lock's 30-minute TTL ran out.
    """

    def test_every_wired_hook_is_executable_after_install(self) -> None:
        for edition in EDITIONS:
            with self.subTest(edition=edition), tempfile.TemporaryDirectory(
                prefix="executable hooks "
            ) as raw:
                target = Path(raw).resolve()
                result = run(
                    sys.executable,
                    str(INSTALLER),
                    "--edition",
                    edition,
                    "--target",
                    str(target),
                )
                self.assertEqual(0, result.returncode, result.stderr)
                wired = wired_hook_scripts(target)
                # Each tool wires several hooks; an empty list would mean the
                # wiring moved and this test silently checked nothing.
                for wiring in HOOK_WIRING:
                    tool_dir = wiring.split("/", 1)[0] + "/"
                    self.assertTrue(
                        any(script.startswith(tool_dir) for script in wired),
                        f"no hook wired in {wiring}",
                    )
                for script in wired:
                    path = target / script
                    self.assertTrue(path.is_file(), f"{script} is wired but not installed")
                    self.assertTrue(is_executable(path), f"{script} is not executable")
                installed_hooks = [
                    path
                    for path in target.rglob("*.sh")
                    if "hooks" in path.relative_to(target).parts[:-1]
                ]
                self.assertTrue(installed_hooks)
                for path in installed_hooks:
                    self.assertTrue(
                        is_executable(path),
                        f"{path.relative_to(target).as_posix()} is not executable",
                    )


@unittest.skipIf(os.name == "nt", "the filesystem has no executable bit to assert")
class ExecutableBitSourceTest(unittest.TestCase):
    """The executable bit installs from the source index, not the working tree.

    The synthetic source reproduces the shipped defect: the Cursor and Codex
    hooks are 100644 in the index, and no file carries the bit on disk - the
    state of a checkout made on Windows, or with `core.fileMode=false`.
    """

    HOOKS = (".claude/hooks/gate.sh", ".cursor/hooks/gate.sh", ".codex/hooks/gate.sh")
    INDEX_EXECUTABLE = ".agents/skills/demo/scripts/run.py"
    PLAIN_SCRIPT = ".agents/skills/demo/scripts/helper.py"

    def _write_source(self, base: Path) -> None:
        def command(path: str) -> dict:
            return {"type": "command", "command": path}

        for edition_path in EDITION_PATHS.values():
            files = {
                "VERSION": "0.0.0\n",
                "AGENTS.md": "# policy\n",
                ".claude/settings.json": json.dumps(
                    {"hooks": {"Stop": [{"hooks": [command(self.HOOKS[0])]}]}}
                ),
                ".cursor/hooks.json": json.dumps(
                    {"version": 1, "hooks": {"stop": [{"command": self.HOOKS[1]}]}}
                ),
                ".codex/hooks.json": json.dumps(
                    {"hooks": {"Stop": [{"hooks": [command(self.HOOKS[2])]}]}}
                ),
                self.INDEX_EXECUTABLE: "#!/usr/bin/env python3\n",
                self.PLAIN_SCRIPT: "#!/usr/bin/env python3\n",
            }
            for hook in self.HOOKS:
                files[hook] = "#!/usr/bin/env bash\nexit 0\n"
            for relative, text in files.items():
                path = base / edition_path / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")
                path.chmod(0o644)

    def _git_source(self, base: Path) -> None:
        self._write_source(base)
        for command in (
            ("git", "-c", "init.defaultBranch=main", "init", "-q", str(base)),
            ("git", "add", "--", *(path.as_posix() for path in EDITION_PATHS.values())),
            (
                "git",
                "update-index",
                "--chmod=+x",
                "--",
                *(
                    f"{path.as_posix()}/{relative}"
                    for path in EDITION_PATHS.values()
                    for relative in (self.HOOKS[0], self.INDEX_EXECUTABLE)
                ),
            ),
        ):
            result = run(*command, cwd=base)
            self.assertEqual(0, result.returncode, result.stderr)
        generated = run(
            sys.executable,
            str(INSTALLER),
            "--source-root",
            str(base),
            "--write-inventories",
            cwd=base,
        )
        self.assertEqual(0, generated.returncode, generated.stderr)

    def _install(self, base: Path, target: Path, *extra: str):
        return run(
            sys.executable,
            str(INSTALLER),
            "--source-root",
            str(base),
            "--edition",
            "PHP Core",
            "--target",
            str(target),
            *extra,
            cwd=base,
        )

    def test_index_bit_wins_over_a_working_tree_without_it(self) -> None:
        with tempfile.TemporaryDirectory(prefix="mode source ") as raw:
            base = Path(raw).resolve() / "source"
            target = Path(raw).resolve() / "target"
            target.mkdir()
            self._git_source(base)
            listed = run("git", "ls-files", "--stage", "--", "PHP Core", cwd=base)
            index_modes = {
                line.split("\t", 1)[1]: line.split(" ", 1)[0]
                for line in listed.stdout.splitlines()
            }
            self.assertEqual("100755", index_modes["PHP Core/" + self.HOOKS[0]])
            self.assertEqual("100644", index_modes["PHP Core/" + self.HOOKS[1]])
            self.assertEqual("100755", index_modes["PHP Core/" + self.INDEX_EXECUTABLE])
            self.assertTrue(
                all(
                    not is_executable(path)
                    for path in (base / "PHP Core").rglob("*")
                    if path.is_file()
                ),
                "the synthetic source must carry no executable bit on disk",
            )

            result = self._install(base, target)
            self.assertEqual(0, result.returncode, result.stderr)
            self.assertEqual(sorted(self.HOOKS), sorted(wired_hook_scripts(target)))
            for hook in self.HOOKS:
                # The .claude hook is 100755 in the index; the other two are
                # 100644 there, as on the commit that shipped the defect, and
                # still install executable because they are hook scripts.
                self.assertTrue(is_executable(target / hook), hook)
            self.assertTrue(is_executable(target / self.INDEX_EXECUTABLE))
            self.assertFalse(is_executable(target / self.PLAIN_SCRIPT))
            self.assertFalse(is_executable(target / "AGENTS.md"))

    def test_without_git_the_filesystem_bit_and_the_hook_rule_decide(self) -> None:
        with tempfile.TemporaryDirectory(prefix="mode archive ") as raw:
            base = Path(raw).resolve() / "source"
            target = Path(raw).resolve() / "target"
            target.mkdir()
            self._git_source(base)
            # An extracted archive: inventories, files, no index to ask.
            shutil.rmtree(base / ".git")
            (base / "PHP Core" / self.PLAIN_SCRIPT).chmod(0o755)

            result = self._install(base, target)
            self.assertEqual(0, result.returncode, result.stderr)
            for hook in self.HOOKS:
                self.assertTrue(is_executable(target / hook), hook)
            self.assertTrue(is_executable(target / self.PLAIN_SCRIPT))
            self.assertFalse(is_executable(target / self.INDEX_EXECUTABLE))

    def test_identical_non_executable_hook_gets_only_its_bit_back(self) -> None:
        with tempfile.TemporaryDirectory(prefix="mode reinstall ") as raw:
            base = Path(raw).resolve() / "source"
            target = Path(raw).resolve() / "target"
            target.mkdir()
            self._git_source(base)
            first = self._install(base, target)
            self.assertEqual(0, first.returncode, first.stderr)
            # A project installed before the fix: identical bytes, no bit.
            broken = target / self.HOOKS[1]
            broken.chmod(0o644)
            content, modified = broken.read_bytes(), broken.stat().st_mtime_ns

            preview = self._install(base, target, "--merge-existing", "--dry-run")
            self.assertEqual(0, preview.returncode, preview.stderr)
            self.assertIn(
                f"WOULD_FIX_MODE\tcursor\t{self.HOOKS[1]}\t{self.HOOKS[1]}",
                preview.stdout.splitlines(),
            )
            self.assertEqual(0o644, stat.S_IMODE(broken.stat().st_mode))

            repeated = self._install(base, target, "--merge-existing")
            self.assertEqual(0, repeated.returncode, repeated.stderr)
            lines = repeated.stdout.splitlines()
            self.assertIn(f"FIX_MODE\tcursor\t{self.HOOKS[1]}\t{self.HOOKS[1]}", lines)
            self.assertNotIn(f"UNCHANGED\tcursor\t{self.HOOKS[1]}", lines)
            self.assertEqual(0o755, stat.S_IMODE(broken.stat().st_mode))
            # Only the mode changed: the file was not rewritten.
            self.assertEqual(content, broken.read_bytes())
            self.assertEqual(modified, broken.stat().st_mtime_ns)
            self.assertIn(f"UNCHANGED\tclaude\t{self.HOOKS[0]}", lines)
            self.assertIn(f"UNCHANGED\tcodex\t{self.HOOKS[2]}", lines)
            self.assertEqual("", repeated.stderr)

            again = self._install(base, target, "--merge-existing")
            self.assertEqual(0, again.returncode, again.stderr)
            self.assertIn(f"UNCHANGED\tcursor\t{self.HOOKS[1]}", again.stdout.splitlines())
            self.assertNotIn("FIX_MODE", again.stdout)

    def test_install_from_an_earlier_release_gets_its_hook_bit_back(self) -> None:
        # An install from the release before executable bits were enforced:
        # the hook is byte-identical but 0644, and a file this release
        # changed (here the Cursor wiring) now collides.
        with tempfile.TemporaryDirectory(prefix="mode upgrade ") as raw:
            base = Path(raw).resolve() / "source"
            target = Path(raw).resolve() / "target"
            target.mkdir()
            self._git_source(base)
            first = self._install(base, target)
            self.assertEqual(0, first.returncode, first.stderr)
            broken = target / self.HOOKS[1]
            broken.chmod(0o644)
            wiring = base / "PHP Core" / ".cursor/hooks.json"
            wiring.write_text(wiring.read_text(encoding="utf-8") + "\n", encoding="utf-8")

            refused = self._install(base, target, "--merge-existing")
            self.assertEqual(2, refused.returncode, refused.stdout)
            self.assertIn(
                "COLLISION\tcursor\t.cursor/hooks.json\texisting-file",
                refused.stderr.splitlines(),
            )
            # A refused run writes nothing, modes included.
            self.assertEqual(0o644, stat.S_IMODE(broken.stat().st_mode))

            upgraded = self._install(base, target, "--overwrite")
            self.assertEqual(0, upgraded.returncode, upgraded.stderr)
            lines = upgraded.stdout.splitlines()
            self.assertIn("OVERWRITE\tcursor\t.cursor/hooks.json", lines)
            self.assertIn(f"FIX_MODE\tcursor\t{self.HOOKS[1]}\t{self.HOOKS[1]}", lines)
            self.assertEqual(0o755, stat.S_IMODE(broken.stat().st_mode))
            self.assertTrue(all(is_executable(target / hook) for hook in self.HOOKS))

if __name__ == "__main__":
    unittest.main()
