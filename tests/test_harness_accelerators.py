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
