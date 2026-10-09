"""Attached accelerators: a registered project gets an edition lent from this clone, nothing copied into it."""

import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "harness/src"))
from harness import sessions  # noqa: E402
from harness import providers as provider_commands  # noqa: E402
from harness.accelerators import Accelerators  # noqa: E402
from harness.knowledge import KnowledgeManager  # noqa: E402
from harness import memory_use  # noqa: E402

LARAVEL = ROOT / "Laravel"


def snapshot(root):
    return sorted(path.relative_to(root).as_posix() for path in root.rglob("*"))


class AttachedAcceleratorTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "shop"
        (self.project / "app").mkdir(parents=True)
        (self.project / "composer.json").write_text(json.dumps({"require": {"laravel/framework": "^11.0"}}), encoding="utf-8")
        (self.project / "app/Order.php").write_text("<?php\n", encoding="utf-8")
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": name, "name": name, "available": True, "executable": f"/never-executed/{name}", "detail": ""}
            for name in ("claude", "codex", "cursor")])
        discovery.start()
        self.addCleanup(discovery.stop)
        worker = patch.object(sessions.Sessions, "_worker", return_value=None)
        worker.start()
        self.addCleanup(worker.stop)
        self.store = sessions.Sessions(self.root / "state", [self.project])
        self.addCleanup(self.store.close)
        self.project_id = next(iter(self.store.projects))
        self.before = snapshot(self.project)

    def assert_project_untouched(self):
        self.assertEqual(self.before, snapshot(self.project))

    def test_registration_attaches_the_detected_edition(self):
        info = self.store.accelerators.get(self.project_id)
        self.assertEqual("attached", info["mode"])
        self.assertEqual("Laravel", info["edition"])
        self.assertEqual(str(LARAVEL), info["home"])
        self.assertEqual(str(self.root / "state" / "attached" / self.project_id), info["state"])
        listed = next(item for item in self.store.list_projects() if item["id"] == self.project_id)
        self.assertEqual({"mode": "attached", "edition": "Laravel"}, listed["accelerator"])
        self.assert_project_untouched()

    def test_a_project_with_its_own_accelerator_is_left_alone(self):
        installed = self.root / "installed"
        (installed / "memory-bank/scripts").mkdir(parents=True)
        (installed / "memory-bank/scripts/context.py").write_text("# installed\n", encoding="utf-8")
        (installed / "composer.json").write_text(json.dumps({"require": {"laravel/framework": "^11.0"}}), encoding="utf-8")
        project = self.store.add_project({"path": str(installed)})
        self.assertEqual("installed", self.store.accelerators.get(project["id"])["mode"])
        self.assertIsNone(self.store.accelerators.overlay(project["id"], "claude", installed))
        with self.assertRaises(sessions.SessionError):
            self.store.accelerators.attach(project["id"], "Laravel")

    def test_a_project_without_php_is_attached_only_on_request(self):
        other = self.root / "notes"
        other.mkdir()
        project = self.store.add_project({"path": str(other)})
        self.assertIsNone(self.store.accelerators.get(project["id"])["mode"])
        with self.assertRaises(sessions.SessionError):
            self.store.accelerators.attach(project["id"])
        self.assertEqual("attached", self.store.accelerators.attach(project["id"], "PHP Core")["mode"])
        detached = self.store.accelerators.detach(project["id"])
        self.assertIsNone(detached["mode"])

    def test_claude_launch_carries_the_edition_from_the_clone(self):
        command = provider_commands.build_command("claude", "/never-executed/claude", self.project, "Hello",
                                                  mode="edit", agents_enabled=False)
        overlay = self.store.accelerators.overlay(self.project_id, "claude", self.project)
        launched = Accelerators.apply("claude", command, overlay, first_turn=True)

        self.assertEqual(str(LARAVEL), launched[launched.index("--add-dir") + 1])
        policy = Path(launched[launched.index("--append-system-prompt-file") + 1])
        self.assertIn((LARAVEL / "AGENTS.md").read_text(encoding="utf-8")[:200], policy.read_text(encoding="utf-8"))
        self.assertTrue(policy.is_relative_to(self.root / "state"))
        settings = json.loads(launched[launched.index("--settings") + 1])
        # The Harness's own per-launch settings survive the merge.
        self.assertTrue(settings["disableWorkflows"])
        self.assertIn("CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH", settings["env"])
        start = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        self.assertEqual(f"{LARAVEL.as_posix()}/.claude/hooks/local-context.sh", start)
        self.assertIn(f"Edit(//{LARAVEL.as_posix().lstrip('/')}/**)", settings["permissions"]["deny"])
        self.assertEqual({"ACCELERATOR_HOME": str(LARAVEL), "ACCELERATOR_STATE_DIR": str(self.root / "state/attached" / self.project_id),
                          "ACCELERATOR_PROJECT_DIR": str(self.project), "ACCELERATOR_EDITION": "Laravel"}, overlay.environment)
        self.assert_project_untouched()

    def test_codex_launch_puts_policy_skills_and_hooks_before_exec(self):
        command = provider_commands.build_command("codex", "/never-executed/codex", self.project, "Hello", mode="plan")
        overlay = self.store.accelerators.overlay(self.project_id, "codex", self.project)
        launched = Accelerators.apply("codex", command, overlay, first_turn=True)

        exec_index = launched.index("exec")
        settings = [launched[index + 1] for index, value in enumerate(launched[:exec_index]) if value == "-c"]
        instructions = next(value for value in settings if value.startswith("developer_instructions="))
        self.assertIn("Laravel accelerator: attached, not installed", instructions)
        self.assertIn(".agents/skills/eloquent/SKILL.md", instructions)
        hooks = [value for value in settings if value.startswith("hooks.")]
        self.assertTrue(any(value.startswith("hooks.SessionStart=") and "local-context.sh" in value for value in hooks))
        self.assertIn(str(self.root / "state/attached" / self.project_id), launched[:exec_index])
        self.assert_project_untouched()

    def test_cursor_launch_loads_the_plugin_and_names_the_policy_once(self):
        first = provider_commands.build_command("cursor", "/never-executed/cursor", self.project, "Hello", mode="plan")
        overlay = self.store.accelerators.overlay(self.project_id, "cursor", self.project)
        launched = Accelerators.apply("cursor", first, overlay, first_turn=True)
        self.assertEqual(str(LARAVEL), launched[launched.index("--plugin-dir") + 1])
        self.assertLess(launched.index("--plugin-dir"), launched.index("--"))
        self.assertTrue(launched[-1].endswith("Hello"))
        self.assertIn(str(LARAVEL / "AGENTS.md"), launched[-1])

        resumed = provider_commands.build_command("cursor", "/never-executed/cursor", self.project, "Again",
                                                  mode="plan", session_id="chat-1")
        self.assertEqual("Again", Accelerators.apply("cursor", resumed, overlay, first_turn=False)[-1])

    def test_composer_lists_the_attached_skills(self):
        codex = self.store.accelerators.skills(self.project_id, "codex")
        self.assertIn("eloquent", [skill["name"] for skill in codex])
        self.assertTrue(all(Path(skill["path"]).is_relative_to(LARAVEL / ".agents/skills") for skill in codex))
        self.assertEqual(LARAVEL, self.store.accelerators.home(self.project_id, self.project))

    def test_detaching_keeps_the_projects_memory(self):
        state = Path(self.store.accelerators.get(self.project_id)["state"])
        self.store.accelerators.overlay(self.project_id, "claude", self.project)
        self.assertTrue((state / "accelerator-attach.json").is_file())
        self.store.accelerators.detach(self.project_id)
        self.assertTrue((state / "accelerator-attach.json").is_file())
        self.assertIsNone(self.store.accelerators.overlay(self.project_id, "claude", self.project))


    def test_knowledge_reads_the_state_and_runs_the_clones_runtime(self):
        manager = KnowledgeManager(self.store)
        self.addCleanup(manager.close)
        info = manager.info(self.project_id)
        state = self.root / "state/attached" / self.project_id
        self.assertEqual("memory-bank", info["bank_id"])
        self.assertTrue(info["runtime_available"])
        self.assertTrue(info["brain_available"])
        self.assertEqual({"edition": "Laravel", "home": str(LARAVEL), "state": str(state)}, info["attached"])

        result = manager.run(self.project_id, {"action": "status"})
        self.assertTrue(result["ok"], result)
        self.assertEqual("attached", result["result"]["workspace"]["layout"])
        self.assertEqual(str(self.project), result["result"]["workspace"]["project"])
        self.assertTrue(manager.run(self.project_id, {"action": "index"})["ok"])
        found = manager.run(self.project_id, {"action": "search", "query": "eloquent relationships"})
        self.assertTrue(found["ok"], found)
        self.assertTrue(any(item["path"].startswith(LARAVEL.as_posix()) for item in found["result"]["documents"]))

        self.assertEqual("memory-bank", self.store.memory(self.project_id)["bank_id"])
        self.assertIsNotNone(memory_use.read(manager, self.project_id)["chunks"])
        self.assert_project_untouched()


    def test_an_attached_session_records_a_learning_about_a_project_file(self):
        # The runtime's root is the state; the learning cites the project.
        manager = KnowledgeManager(self.store)
        self.addCleanup(manager.close)
        started = manager.run(self.project_id, {"action": "start", "task_id": "TASK-ATTACHED", "goal": "Check orders"})
        self.assertTrue(started["ok"], started)
        recorded = manager.run(self.project_id, {
            "action": "record-result", "task_id": "TASK-ATTACHED", "result_id": "attached-1",
            "revision": started["result"]["revision"], "attestation": "agent",
            "request": {"progress": "Orders checked.", "next_steps": [], "verified": True,
                        "learnings": [{"type": "finding", "title": "Orders are final",
                                       "consequence": "Extend an order by composition.",
                                       "sources": ["app/Order.php"]}]}})
        self.assertTrue(recorded["ok"], recorded)
        self.assertEqual(["created"], [item["state"] for item in recorded["result"]["records"]])
        state = self.root / "state/attached" / self.project_id
        self.assertEqual(1, len(list((state / "project-brain/dynamic/findings").glob("*.md"))))
        self.assert_project_untouched()

    def test_capsule_paths_may_name_the_attached_edition_only(self):
        from harness.task_context import TaskContext
        skill = (LARAVEL / ".agents/skills/eloquent/SKILL.md").as_posix()
        self.assertEqual(skill, TaskContext._visible_path(skill, LARAVEL))
        self.assertEqual("app/Order.php", TaskContext._visible_path("app/Order.php", LARAVEL))
        for path in ("/etc/passwd", LARAVEL.as_posix() + "/../secrets.txt"):
            with self.assertRaises(sessions.SessionError):
                TaskContext._visible_path(path, LARAVEL)
        with self.assertRaises(sessions.SessionError):
            TaskContext._visible_path(skill, None)


    def test_runners_receive_the_overlays_as_request_data(self):
        sys.path.insert(0, str(ROOT / "scripts"))
        import accelerator_attach
        overlays = self.store.accelerators.overlays(self.project_id, ["claude", "codex", "claude"], self.project)
        self.assertEqual(["claude", "codex"], list(overlays))
        overlays = json.loads(json.dumps(overlays))  # what the runner reads back from its request file
        command = provider_commands.build_command("codex", "/never-executed/codex", self.project, "Review", mode="plan",
                                                  session_id="thread-1")
        launched = accelerator_attach.apply_overlay("codex", command, overlays["codex"], first_turn=False)
        self.assertLess(launched.index("--add-dir"), launched.index("exec"))
        claude = provider_commands.build_command("claude", "/never-executed/claude", self.project, "Review", mode="plan")
        launched = accelerator_attach.apply_overlay("claude", claude, overlays["claude"])
        self.assertIn("hooks", json.loads(launched[launched.index("--settings") + 1]))
        self.assertEqual(str(self.project), overlays["codex"]["environment"]["ACCELERATOR_PROJECT_DIR"])


if __name__ == "__main__":
    unittest.main()


class InstalledAcceleratorSyncTests(unittest.TestCase):
    """A project with its own copy is kept at this clone's version by itself."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "shop"
        self.project.mkdir()
        sys.path.insert(0, str(ROOT / "scripts"))
        try:
            import install_accelerator
        finally:
            sys.path.pop(0)
        self.installer = install_accelerator
        self.assertEqual(0, install_accelerator.install(ROOT, "Laravel", self.project, ["claude"], False, False, False))
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[])
        discovery.start()
        self.addCleanup(discovery.stop)
        for name in ("_worker", "_keep_accelerators_current"):
            stub = patch.object(sessions.Sessions, name, return_value=None)
            stub.start()
            self.addCleanup(stub.stop)
        self.store = sessions.Sessions(self.root / "state", [self.project])
        self.addCleanup(self.store.close)
        self.project_id = next(iter(self.store.projects))

    def test_an_installed_runtime_is_brought_up_to_the_clone_once_per_version(self):
        runtime = self.project / "memory-bank/scripts/context.py"
        runtime.write_text("# an old runtime\n", encoding="utf-8")
        accelerators = self.store.accelerators
        with patch.object(Accelerators, "source_version", return_value="release-1"):
            report = accelerators.keep_current(self.project_id)
            self.assertIsNone(report["error"])
            self.assertIn("memory-bank/scripts/context.py", {item["path"] for item in report["changed"]})
            self.assertEqual((ROOT / "Laravel/memory-bank/scripts/context.py").read_bytes(), runtime.read_bytes())
            # The same clone version is not synced twice.
            self.assertIsNone(accelerators.keep_current(self.project_id))
        info = accelerators.get(self.project_id)
        self.assertEqual("installed", info["mode"])
        self.assertEqual(("release-1", "Laravel"), (info["sync"]["source"], info["sync"]["edition"]))
        self.assertTrue(info["sync"]["backups"])
        runtime.write_text("# edited again\n", encoding="utf-8")
        with patch.object(Accelerators, "source_version", return_value="release-2"):
            again = accelerators.keep_current(self.project_id)
        self.assertIn("memory-bank/scripts/context.py", {item["path"] for item in again["changed"]})

    def test_the_conversation_is_told_when_the_project_was_updated(self):
        notice = sessions.accelerator_sync_notice({
            "edition": "Laravel", "release": "2.0.0",
            "changed": [{"path": "a", "action": "updated"}], "kept": [{"path": "b", "reason": "x"}]})
        self.assertIn("brought up to Laravel 2.0.0: 1 file(s) updated", notice)
        self.assertIn("1 left as they are", notice)

    def test_an_attached_project_is_never_written(self):
        other = self.root / "attached"
        (other / "app").mkdir(parents=True)
        (other / "composer.json").write_text(json.dumps({"require": {"laravel/framework": "^11.0"}}), encoding="utf-8")
        project = self.store.add_project({"path": str(other)})
        before = snapshot(other)
        self.assertIsNone(self.store.accelerators.keep_current(project["id"]))
        self.assertEqual(before, snapshot(other))


class InstalledCodexHookTrustTests(unittest.TestCase):
    """Only the accelerator's own hook definitions, running its own scripts, are approved."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.project = Path(temporary.name) / "shop"
        (self.project / ".codex/hooks").mkdir(parents=True)
        for name in ("working-memory-read.sh", "working-memory-write.sh", "local-context.sh"):
            (self.project / ".codex/hooks" / name).write_bytes((LARAVEL / ".codex/hooks" / name).read_bytes())
        (self.project / ".codex/hooks.json").write_bytes((LARAVEL / ".codex/hooks.json").read_bytes())
        canonical = json.loads((LARAVEL / ".codex/hooks.json").read_text(encoding="utf-8"))["hooks"]
        self.commands = {event: canonical[event][0]["hooks"][0]["command"]
                         for event in ("SessionStart", "UserPromptSubmit", "Stop")}
        self.requests = []
        self.listing = []

    def hook(self, key, event, command=None, status="untrusted", source="project", path=None):
        return {"key": key, "eventName": event, "command": command or self.commands[event],
                "currentHash": "sha256:" + key, "trustStatus": status, "source": source,
                "sourcePath": str(path or self.project / ".codex/hooks.json")}

    def app_server(self, executable, arguments, cwd, requests, timeout=60):
        self.requests.append(requests)
        if requests[0]["method"] == "hooks/list":
            return [{"result": {"data": [{"hooks": self.listing}]}}]
        return [{"result": {}}]

    def trust(self):
        import accelerator_attach
        with patch.object(accelerator_attach, "_app_server", side_effect=self.app_server):
            return accelerator_attach.trust_installed_codex_hooks("/never-executed/codex", "Laravel", self.project)

    def test_the_accelerators_own_hooks_are_approved_and_nothing_else(self):
        script = self.project / ".codex/hooks/local-context.sh"
        script.write_bytes(script.read_bytes() + b"# a team's edit\n")
        self.listing = [
            self.hook("read", "UserPromptSubmit"),
            self.hook("write", "Stop", status="trusted"),
            self.hook("edited", "SessionStart"),
            self.hook("team", "Stop", command="sh -c ./scripts/notify.sh"),
            self.hook("user", "Stop", source="user", path=Path.home() / ".codex/hooks.json"),
            self.hook("nested", "Stop", path=self.project / "packages/a/.codex/hooks.json"),
        ]
        self.assertEqual({"approved": 1, "already": 1, "left": ["edited", "team"]}, self.trust())
        [_, written] = self.requests
        self.assertEqual([{"keyPath": "hooks.state", "mergeStrategy": "upsert",
                           "value": {"read": {"trusted_hash": "sha256:read"}}}], written[0]["params"]["edits"])

    def test_nothing_is_written_when_every_hook_is_approved(self):
        self.listing = [self.hook("read", "UserPromptSubmit", status="trusted")]
        self.assertEqual({"approved": 0, "already": 1, "left": []}, self.trust())
        self.assertEqual(1, len(self.requests))


class InstalledCodexSyncTests(unittest.TestCase):
    """Codex's hook wiring follows the clone only where the new wiring is approved in the same pass."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "shop"
        self.project.mkdir()
        sys.path.insert(0, str(ROOT / "scripts"))
        try:
            import accelerator_attach
            import install_accelerator
        finally:
            sys.path.pop(0)
        self.attach = accelerator_attach
        with patch("sys.stdout"):
            self.assertEqual(0, install_accelerator.install(ROOT, "Laravel", self.project, ["codex"], False, False, False))
        # The wiring an older release installed, untouched since.
        self.hooks = self.project / ".codex/hooks.json"
        self.old = b'{"hooks": {}}\n'
        self.hooks.write_bytes(self.old)
        manifest = self.project / install_accelerator.SYNC_MANIFEST
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(json.dumps({"schema": 1, "files": {
            ".codex/hooks.json": install_accelerator.git_blob_id(self.old)}}), encoding="utf-8")
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": "codex", "name": "codex", "available": True, "executable": "/never-executed/codex", "detail": ""}])
        discovery.start()
        self.addCleanup(discovery.stop)
        for name in ("_worker", "_keep_accelerators_current"):
            stub = patch.object(sessions.Sessions, name, return_value=None)
            stub.start()
            self.addCleanup(stub.stop)
        environment = patch.dict(os.environ, {"HARNESS_CODEX_HOOK_TRUST": ""})
        environment.start()
        self.addCleanup(environment.stop)
        self.store = sessions.Sessions(self.root / "state", [self.project])
        self.addCleanup(self.store.close)
        self.project_id = next(iter(self.store.projects))

    def keep_current(self, **trust):
        with patch.object(self.attach, "trust_installed_codex_hooks", **trust) as approve, \
                patch.object(Accelerators, "source_version", return_value="release-1"):
            return self.store.accelerators.keep_current(self.project_id), approve

    def test_the_new_wiring_is_written_and_approved_in_one_pass(self):
        report, approve = self.keep_current(return_value={"approved": 6, "already": 0, "left": []})
        approve.assert_called_once_with("/never-executed/codex", "Laravel", self.project)
        self.assertEqual((LARAVEL / ".codex/hooks.json").read_bytes(), self.hooks.read_bytes())
        self.assertIn(".codex/hooks.json", {item["path"] for item in report["changed"]})
        self.assertIn("6 accelerator hook(s) approved for Codex", sessions.accelerator_sync_notice(report))
        self.assertEqual(6, self.store.accelerators.get(self.project_id)["sync"]["codex_trust"]["approved"])

    def test_wiring_codex_could_not_approve_goes_back(self):
        report, _ = self.keep_current(side_effect=self.attach.AttachError("Codex could not list hooks."))
        self.assertEqual(self.old, self.hooks.read_bytes())
        self.assertNotIn(".codex/hooks.json", {item["path"] for item in report["changed"]})
        self.assertIn("could not approve", {item["path"]: item["reason"] for item in report["kept"]}[".codex/hooks.json"])
        self.assertEqual({"error": "Codex could not list hooks."}, report["codex_trust"])

    def test_with_approval_turned_off_the_wiring_waits_for_a_person(self):
        with patch.dict(os.environ, {"HARNESS_CODEX_HOOK_TRUST": "0"}):
            report, approve = self.keep_current(return_value={"approved": 6, "already": 0, "left": []})
        approve.assert_not_called()
        self.assertEqual(self.old, self.hooks.read_bytes())
        self.assertIn("re-approval", {item["path"]: item["reason"] for item in report["kept"]}[".codex/hooks.json"])
        # Turned on again, the same clone version runs once more.
        report, approve = self.keep_current(return_value={"approved": 6, "already": 0, "left": []})
        approve.assert_called_once()
        self.assertEqual((LARAVEL / ".codex/hooks.json").read_bytes(), self.hooks.read_bytes())


class AttachedCodexHookTrustTests(unittest.TestCase):
    """The hooks lent to an attached project are approved without a click; the project is never written."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "shop"
        (self.project / "app").mkdir(parents=True)
        (self.project / "composer.json").write_text(json.dumps({"require": {"laravel/framework": "^11.0"}}), encoding="utf-8")
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": "codex", "name": "codex", "available": True, "executable": "/never-executed/codex", "detail": ""}])
        discovery.start()
        self.addCleanup(discovery.stop)
        for name in ("_worker", "_keep_accelerators_current"):
            stub = patch.object(sessions.Sessions, name, return_value=None)
            stub.start()
            self.addCleanup(stub.stop)
        environment = patch.dict(os.environ, {"HARNESS_CODEX_HOOK_TRUST": ""})
        environment.start()
        self.addCleanup(environment.stop)
        self.store = sessions.Sessions(self.root / "state", [self.project])
        self.addCleanup(self.store.close)
        self.project_id = next(iter(self.store.projects))
        import accelerator_attach
        self.attach = accelerator_attach

    def test_untrusted_lent_hooks_are_approved_once_per_clone_version(self):
        before = snapshot(self.project)
        listed = [{"key": "a", "status": "untrusted"}, {"key": "b", "status": "trusted"}]
        approved = [{"key": "a", "status": "trusted"}, {"key": "b", "status": "trusted"}]
        with patch.object(self.attach, "codex_hook_trust", return_value=listed), \
                patch.object(self.attach, "trust_codex_hooks", return_value=approved) as approve, \
                patch.object(Accelerators, "source_version", return_value="release-1"):
            report = self.store.accelerators.keep_current(self.project_id)
            self.assertIsNone(self.store.accelerators.keep_current(self.project_id))
        approve.assert_called_once_with("/never-executed/codex", "Laravel", self.project)
        self.assertEqual({"approved": 1, "already": 1, "left": []}, report["codex_trust"])
        self.assertEqual(before, snapshot(self.project))

    def test_without_codex_there_is_nothing_to_keep_current(self):
        self.store.providers["codex"] = {"id": "codex", "available": False, "executable": None}
        with patch.object(self.attach, "codex_hook_trust") as listing:
            self.assertIsNone(self.store.accelerators.keep_current(self.project_id))
        listing.assert_not_called()


if __name__ == "__main__":
    unittest.main()
