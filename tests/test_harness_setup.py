"""Owned project registration and snapshot-based accelerator setup checks."""

import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import sessions
from harness.setup import SetupManager


def setup_source_fixture(root):
    """A small real inventory using production merge and source-override rules."""
    files = {
        ".gitignore": b"memory-bank/local/\n",
        ".gitattributes": b".cursor/skills/** linguist-generated\n",
        "AGENTS.md": b"# Accelerator policy\n\nVerify project sources.\n",
        "README.md": b"# Accelerator documentation\n",
        "VERSION": b"2.0.0\n",
        "memory-bank/.install/INDEX.md": b"# Pristine memory index\n",
        "memory-bank/scripts/context.py": b"raise AssertionError('Fixture runtime must never execute during setup')\n",
        "memory-bank/scripts/brain_runtime.py": b"# Static runtime prerequisite fixture\n",
        "memory-bank/scripts/context_retrieval.py": b"# Static retrieval prerequisite fixture\n",
        "project-brain/PROTOCOL.md": b"# Governed fixture protocol\n",
        "project-brain/config/runtime.json": b'{"mode":"governed"}\n',
        "project-brain/scripts/validate.py": b"raise AssertionError('Fixture validator must never execute during setup')\n",
        ".claude/skills/fixture/SKILL.md": b"---\nname: fixture\ndescription: Claude fixture\n---\n",
        ".claude/settings.json": b"{}\n",
        ".cursor/skills/fixture/SKILL.md": b"---\nname: fixture\ndescription: Cursor fixture\n---\n",
        ".cursor/hooks.json": b"{}\n",
        ".agents/skills/fixture/SKILL.md": b"---\nname: fixture\ndescription: Codex fixture\n---\n",
        ".codex/hooks/fixture.sh": b"#!/bin/sh\nexit 0\n",
        ".codex/config.toml": b"# Static Codex configuration fixture\n",
    }
    edition = root / "PHP Core"
    for name, payload in files.items():
        path = edition / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)
    (edition / ".codex/hooks/fixture.sh").chmod(0o755)
    components = {
        "shared": sorted(name for name in files if not name.startswith((".claude/", ".cursor/", ".agents/", ".codex/"))
                         and name != "memory-bank/.install/INDEX.md") + ["memory-bank/INDEX.md"],
        "claude": [".claude/settings.json", ".claude/skills/fixture/SKILL.md"],
        "cursor": [".cursor/hooks.json", ".cursor/skills/fixture/SKILL.md"],
        "codex": [".agents/skills/fixture/SKILL.md", ".codex/config.toml", ".codex/hooks/fixture.sh"],
    }
    components["shared"].sort()
    inventory = {"schema_version": 2, "inventory_version": 2, "edition": "PHP Core", "release": "2.0.0",
                 "scope": "Owned fixture", "installed": components,
                 "excluded_tracked_paths": ["memory-bank/.install/INDEX.md"],
                 "source_overrides": {"memory-bank/INDEX.md": "memory-bank/.install/INDEX.md"}}
    inventories = root / "install/inventories"
    inventories.mkdir(parents=True)
    (inventories / "php-core.json").write_text(json.dumps(inventory, indent=2), encoding="utf-8")
    return root


class SetupTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        self.source = setup_source_fixture(self.root / "source")
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": "claude", "name": "Fixture Claude", "available": True, "executable": "/never-executed/claude",
             "detail": "Executable present; authentication not checked"},
            {"id": "codex", "name": "Fixture Codex", "available": False, "executable": None,
             "detail": "Executable unavailable"},
        ])
        discovery.start()
        self.addCleanup(discovery.stop)
        worker = patch.object(sessions.Sessions, "_worker", return_value=None)
        worker.start()
        self.addCleanup(worker.stop)
        native = patch.object(sessions.providers, "build_command", side_effect=AssertionError("Setup must not call models"))
        self.native = native.start()
        self.addCleanup(native.stop)
        self.stores, self.managers = [], []
        self.addCleanup(self.close_managers)
        self.store = self.new_store()
        self.manager = SetupManager(self.store, source_root=self.source)
        self.managers.append(self.manager)
        self.project_id = next(iter(self.store.projects))

    def new_store(self):
        store = sessions.Sessions(self.root / "state", [self.project])
        self.stores.append(store)
        return store

    def close_managers(self):
        for manager in self.managers:
            manager.close()
        for store in self.stores:
            store.close()

    def options(self, **changes):
        return {"project_id": self.project_id, "edition": "PHP Core", "tools": ["cursor"], **changes}

    @staticmethod
    def snapshot(root):
        return {path.relative_to(root).as_posix(): (path.read_bytes(), path.stat().st_mode & 0o777)
                for path in root.rglob("*") if path.is_file() and not path.is_symlink()}

    def test_registration_is_idempotent_persistent_and_rejects_redirected_or_missing_paths(self):
        added = self.root / "additional project"
        added.mkdir()
        project = self.store.add_project({"path": str(added)})
        repeated = self.store.add_project({"path": str(added)})
        self.assertEqual(repeated, project)
        self.assertEqual(len(self.store.list_projects()), 2)
        linked = self.root / "linked"
        linked.symlink_to(added, target_is_directory=True)
        for path in (str(linked), str(linked / "child"), str(self.root / "missing"), "relative", [], None):
            with self.subTest(path=path), self.assertRaises(sessions.SessionError):
                self.store.add_project({"path": path})
        with self.assertRaises(sessions.SessionError):
            self.store.add_project({"path": str(added), "extra": True})
        self.assertEqual(len(self.store.list_projects()), 2)
        self.manager.close()
        self.store.close()
        self.managers.remove(self.manager)
        self.stores.remove(self.store)
        restarted = self.new_store()
        self.assertEqual({item["id"] for item in restarted.list_projects()}, {self.project_id, project["id"]})
        self.assertEqual(restarted.project(project["id"])["path"], str(added))
        restarted.close()
        self.stores.remove(restarted)
        added.rmdir()
        missing = self.new_store()
        self.assertIn(self.project_id, {item["id"] for item in missing.list_projects()})
        missing_project = next(item for item in missing.list_projects() if item["id"] == project["id"])
        self.assertFalse(missing_project["available"])
        self.assertFalse(added.exists(), "Restart recreated a missing registered project")
        self.native.assert_not_called()

    def test_status_is_readonly_and_reports_installed_components_without_claiming_authentication(self):
        before = self.snapshot(self.project)
        empty = self.manager.status(self.project_id)
        self.assertEqual(empty["project_id"], self.project_id)
        self.assertEqual(empty["path"], str(self.project))
        self.assertFalse(any(empty["readiness"].values()))
        self.assertFalse(empty["git"]["is_git"])
        self.assertIsNone(empty["installed_edition"])
        self.assertEqual(self.snapshot(self.project), before)
        self.assertFalse(any(item.get("authenticated") is True for item in empty["providers"]))
        self.assertFalse(any("executable" in item for item in empty["providers"]))
        self.assertTrue(any("authentication is not checked" in item for item in empty["diagnostics"]))
        preview = self.manager.preview(self.options(tools=["codex"]))
        self.manager.install({"preview_id": preview["preview_id"]})
        status = self.manager.status(self.project_id)
        self.assertTrue(status["readiness"]["policy"])
        self.assertTrue(status["readiness"]["memory_runtime"])
        self.assertTrue(status["readiness"]["brain_runtime"])
        self.assertEqual({item["id"] for item in status["tools"] if item["installed"]}, {"codex"})
        self.assertEqual(status["installed_edition"], "PHP Core")
        self.assertTrue(status["payload_verified"])
        self.assertEqual((self.project / ".codex/hooks/fixture.sh").stat().st_mode & 0o777, 0o755)
        self.assertFalse((self.project / "memory-bank/local").exists())
        (self.project / "AGENTS.md").write_text("User policy changed after installation\n")
        changed = self.manager.status(self.project_id)
        self.assertTrue(changed["readiness"]["policy"])
        self.assertFalse(changed["payload_verified"])
        self.native.assert_not_called()

    @unittest.skipUnless(shutil.which('git'), 'Git is required for metadata safety verification')
    def test_setup_git_metadata_does_not_run_clean_filters_or_fsmonitor(self):
        marker, monitor_marker = self.root / 'clean-ran', self.root / 'monitor-ran'
        clean, monitor = self.root / 'clean.py', self.root / 'monitor.py'
        clean.write_text('import pathlib,sys\npathlib.Path(sys.argv[1]).write_text("ran")\n'
                         'sys.stdout.write(sys.stdin.read())\n')
        monitor.write_text('#!' + sys.executable + '\nimport pathlib\npathlib.Path('
                           + repr(str(monitor_marker)) + ').write_text("ran")\n')
        monitor.chmod(0o755)
        environment = {key: value for key, value in os.environ.items() if not key.startswith('GIT_')}
        environment.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM='1')

        def git(*arguments):
            return subprocess.run(['git', '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgSign=false',
                                   '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                                   '-C', str(self.project), *arguments], env=environment,
                                  capture_output=True, check=True)

        git('init', '-q')
        git('config', 'filter.fixture.clean', shlex.join([sys.executable, str(clean), str(marker)]))
        (self.project / '.gitattributes').write_text('sample.txt filter=fixture\n')
        source = self.project / 'sample.txt'
        source.write_text('unchanged tracked content\n')
        git('add', '.')
        git('commit', '-qm', 'Owned fixture')
        git('config', 'core.fsmonitor', str(monitor))
        marker.unlink(missing_ok=True)
        previous = source.stat()
        os.utime(source, ns=(previous.st_atime_ns, previous.st_mtime_ns + 5_000_000_000))
        with patch.dict(os.environ, environment, clear=True):
            result = self.manager.status(self.project_id)
            self.assertTrue(result['git']['is_git'])
            self.assertIsNone(result['git']['dirty'])
            self.assertTrue(any('not inspected' in message for message in result['diagnostics']))
            self.assertFalse(marker.exists(), 'Setup executed a project clean filter')
            self.assertFalse(monitor_marker.exists(), 'Setup executed a project fsmonitor')
            # The fixture actually exercises the filter path. Workflow callers
            # keep their existing dirty check rather than silently losing it.
            workflow = sessions.git_details(self.project)
            self.assertIs(workflow['dirty'], False)
            self.assertTrue(marker.exists())
            self.assertFalse(monitor_marker.exists())
        self.native.assert_not_called()

    def test_preview_installs_only_reviewed_tools_preserves_user_merges_and_reruns_identically(self):
        originals = {"AGENTS.md": "# Project policy\n\nKeep my policy.\n", "README.md": "# My application\n",
                     ".gitignore": ".env\n/vendor/\n", ".gitattributes": "*.lock binary\n"}
        for name, content in originals.items():
            (self.project / name).write_text(content)
        before = self.snapshot(self.project)
        options = self.options()
        preview = self.manager.preview(options)
        self.assertTrue(preview["can_install"], preview)
        self.assertEqual(preview["collisions"], [])
        self.assertEqual(self.snapshot(self.project), before)
        self.assertTrue(any(item["action"] == "merge" and item["diff"] for item in preview["files"]))
        self.assertTrue(any(item["action"] == "copy-as" for item in preview["files"]))
        self.assertEqual(preview["file_count"], len(preview["files"]))
        options["tools"].append("claude")
        options["edition"] = "Laravel"
        result = self.manager.install({"preview_id": preview["preview_id"]})
        self.assertTrue(result["ok"], result)
        self.assertEqual((self.project / "README.md").read_text(), originals["README.md"])
        self.assertEqual((self.project / "ACCELERATOR.md").read_bytes(), (self.source / "PHP Core/README.md").read_bytes())
        self.assertTrue((self.project / "AGENTS.md").read_text().startswith(originals["AGENTS.md"].rstrip()))
        self.assertIn("BEGIN ACCELERATOR MANAGED POLICY", (self.project / "AGENTS.md").read_text())
        self.assertIn(".env\n", (self.project / ".gitignore").read_text())
        self.assertIn("memory-bank/local/", (self.project / ".gitignore").read_text())
        self.assertTrue((self.project / ".cursor/skills/fixture/SKILL.md").is_file())
        self.assertFalse((self.project / ".claude").exists())
        self.assertFalse((self.project / ".agents").exists())
        self.assertEqual((self.project / "memory-bank/INDEX.md").read_bytes(), b"# Pristine memory index\n")
        with self.assertRaises(sessions.SessionError):
            self.manager.install({"preview_id": preview["preview_id"]})
        installed = self.snapshot(self.project)
        again = self.manager.preview(self.options())
        self.assertTrue(again["can_install"])
        self.assertEqual(again["changed_count"], 0)
        repeated = self.manager.install({"preview_id": again["preview_id"]})
        self.assertTrue(repeated["ok"])
        self.assertEqual(repeated["installed"], [])
        self.assertEqual(repeated["merged"], [])
        self.assertEqual(self.snapshot(self.project), installed)

    def test_conflicts_source_or_target_edits_invalidate_install_without_partial_project_writes(self):
        collision = self.project / "memory-bank/INDEX.md"
        collision.parent.mkdir()
        collision.write_text("My durable index\n")
        before = self.snapshot(self.project)
        blocked = self.manager.preview(self.options())
        self.assertFalse(blocked["can_install"])
        self.assertTrue(any(item["path"] == "memory-bank/INDEX.md" for item in blocked["collisions"]))
        with self.assertRaises(sessions.SessionError):
            self.manager.install({"preview_id": blocked["preview_id"]})
        self.assertEqual(self.snapshot(self.project), before)
        collision.unlink()
        for changed in ("target", "source", "source_mode", "inventory"):
            with self.subTest(changed=changed):
                preview = self.manager.preview(self.options())
                original_mode = None
                if changed == "target":
                    path = self.project / "AGENTS.md"
                    original = None
                    path.write_text("Policy created after preview\n")
                elif changed in ("source", "source_mode"):
                    path = self.source / "PHP Core/AGENTS.md"
                    original = path.read_bytes()
                    if changed == "source_mode":
                        original_mode = path.stat().st_mode & 0o777
                        path.chmod(original_mode ^ 0o100)
                    else:
                        path.write_bytes(original + b"\nChanged source policy\n")
                else:
                    path = self.source / "install/inventories/php-core.json"
                    original = path.read_bytes()
                    data = json.loads(original)
                    data["release"] = "2.0.1"
                    path.write_text(json.dumps(data, indent=2))
                expected = self.snapshot(self.project)
                try:
                    with self.assertRaises(sessions.SessionError):
                        self.manager.install({"preview_id": preview["preview_id"]})
                    self.assertEqual(self.snapshot(self.project), expected)
                finally:
                    if original is None:
                        path.unlink()
                    else:
                        path.write_bytes(original)
                    if original_mode is not None:
                        path.chmod(original_mode)
        parent = self.project / ".cursor"
        parent.mkdir()
        preview = self.manager.preview(self.options())
        original_parent = self.project / ".cursor-original"
        parent.rename(original_parent)
        parent.mkdir()
        before = self.snapshot(self.project)
        with self.assertRaises(sessions.SessionError):
            self.manager.install({"preview_id": preview["preview_id"]})
        self.assertEqual(self.snapshot(self.project), before)
        self.assertEqual(list(parent.iterdir()), [])
        self.assertEqual(list(original_parent.iterdir()), [])

    def test_partial_write_failure_reports_exact_files_preserves_user_policy_and_can_be_previewed_again(self):
        policy = "# My policy\n\nUser authority remains.\n"
        (self.project / "AGENTS.md").write_text(policy)
        preview = self.manager.preview(self.options())
        first_copy = next(item["path"] for item in preview["files"] if item["action"] == "copy")
        link = os.link
        calls = []

        def fail_second_link(*args, **kwargs):
            calls.append(args[1])
            if len(calls) == 2:
                raise OSError("Owned publication failure")
            return link(*args, **kwargs)

        with patch("harness.setup.os.link", side_effect=fail_second_link):
            result = self.manager.install({"preview_id": preview["preview_id"]})
        self.assertFalse(result["ok"])
        self.assertEqual(len(calls), 2)
        self.assertEqual(result["installed"], [first_copy])
        self.assertEqual(result["merged"], [])
        self.assertEqual(result["unchanged"], [])
        self.assertIn("partial", result["error"])
        self.assertEqual((self.project / "AGENTS.md").read_text(), policy)
        self.assertEqual(set(self.snapshot(self.project)), {"AGENTS.md", first_copy})
        self.assertEqual(list(self.project.rglob(".harness-setup-*")), [])
        retry = self.manager.preview(self.options())
        self.assertTrue(retry["can_install"])
        result = self.manager.install({"preview_id": retry["preview_id"]})
        self.assertTrue(result["ok"], result)
        self.assertTrue((self.project / "AGENTS.md").read_text().startswith(policy.rstrip()))
        self.assertTrue((self.project / ".cursor/skills/fixture/SKILL.md").is_file())

    def test_unsafe_paths_and_active_sessions_block_setup_without_touching_referents(self):
        outside = self.root / "outside.md"
        outside.write_text("OUTSIDE CONTENT\n")
        for kind in ("symlink", "hardlink"):
            with self.subTest(kind=kind):
                destination = self.project / "AGENTS.md"
                if kind == "symlink":
                    destination.symlink_to(outside)
                else:
                    os.link(outside, destination)
                try:
                    try:
                        preview = self.manager.preview(self.options())
                    except sessions.SessionError:
                        pass
                    else:
                        self.assertFalse(preview["can_install"])
                        with self.assertRaises(sessions.SessionError):
                            self.manager.install({"preview_id": preview["preview_id"]})
                    self.assertEqual(outside.read_text(), "OUTSIDE CONTENT\n")
                    self.assertFalse((self.project / "memory-bank").exists())
                finally:
                    destination.unlink()
        outside_dir = self.root / "outside-dir"
        outside_dir.mkdir()
        link = self.project / ".cursor"
        link.symlink_to(outside_dir, target_is_directory=True)
        try:
            try:
                preview = self.manager.preview(self.options())
            except sessions.SessionError:
                pass
            else:
                self.assertFalse(preview["can_install"])
            self.assertEqual(list(outside_dir.iterdir()), [])
        finally:
            link.unlink()
        preview = self.manager.preview(self.options())
        sid = self.store.create({"project_id": self.project_id, "provider": "claude", "prompt": "Queued fixture"})["id"]
        with self.assertRaises(sessions.SessionError):
            self.manager.install({"preview_id": preview["preview_id"]})
        self.assertEqual(list(self.project.iterdir()), [])
        self.assertEqual(self.store.get(sid)["status"], "queued")
        self.native.assert_not_called()


if __name__ == "__main__":
    unittest.main()
