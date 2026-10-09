"""Offline session-to-Brain context binding with the copied project runtime."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import context_usage, sessions
from harness.knowledge import KnowledgeManager
from harness.task_context import TaskContext
from tests.test_harness_knowledge import install_knowledge_fixture, metadata


FAKE_NATIVE = r'''
import json, os, pathlib, sys
receipt = pathlib.Path(sys.argv[1])
receipt.write_text(json.dumps({"cwd": os.getcwd(), "prompt": sys.stdin.read(),
                              "task_id": os.environ.get("CONTEXT_TASK_ID"),
                              "delivered": os.environ.get("CONTEXT_CAPSULE_DELIVERED")}))
print(json.dumps({"type": "thread.started", "thread_id": "native-context-fixture"}), flush=True)
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": "RAW ASSISTANT FIXTURE OUTPUT"}}), flush=True)
print(json.dumps({"type": "turn.completed"}), flush=True)
'''


FAKE_DRAFT = r'''
import json, os, pathlib, sys
receipt = pathlib.Path(sys.argv[1])
receipt.write_text(json.dumps({"cwd": os.getcwd(), "prompt": sys.stdin.read(),
                              "task_id": os.environ.get("CONTEXT_TASK_ID"),
                              "delivered": os.environ.get("CONTEXT_CAPSULE_DELIVERED")}))
draft = {"progress": "The cobalt rule is checked at allocation.", "next_steps": ["Cover the release path."],
         "learnings": [{"type": "finding", "title": "Cobalt allocation needs one owner",
                        "consequence": "Every cobalt allocation names exactly one owner.",
                        "sources": ["specs/authority.md"]}]}
text = "Checked the rule.\n\n```memory-draft\n" + json.dumps(draft) + "\n```"
print(json.dumps({"type": "thread.started", "thread_id": "native-context-fixture"}), flush=True)
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": text}}), flush=True)
print(json.dumps({"type": "turn.completed"}), flush=True)
'''


FAKE_CURSOR = r'''
import json, os, pathlib, sys
receipt = pathlib.Path(sys.argv[1])
receipt.write_text(json.dumps({"cwd": os.getcwd(), "prompt": sys.stdin.read(),
                              "task_id": os.environ.get("CONTEXT_TASK_ID"),
                              "delivered": os.environ.get("CONTEXT_CAPSULE_DELIVERED")}))
print(json.dumps({"type": "system", "subtype": "init", "session_id": "native-cursor-fixture"}), flush=True)
print(json.dumps({"type": "assistant", "message": {"role": "assistant", "content": [
    {"type": "text", "text": "Checked the rule."}]}}), flush=True)
print(json.dumps({"type": "result", "subtype": "success", "is_error": False,
                  "session_id": "native-cursor-fixture", "result": "Checked the rule."}), flush=True)
'''


FAKE_READS = r"""
import json, os, pathlib, sys
receipt = pathlib.Path(sys.argv[1])
receipt.write_text(json.dumps({"cwd": os.getcwd(), "prompt": sys.stdin.read()}))
draft = {"progress": "The cobalt rule is checked at allocation.", "next_steps": [], "learnings": [],
         "used_memory": ["specs/authority.md"]}
text = "Checked the rule.\n\n```memory-draft\n" + json.dumps(draft) + "\n```"
def emit(event):
    print(json.dumps(event), flush=True)
emit({"type": "system", "subtype": "init", "session_id": "native-reads-fixture"})
# Forty-one other files first: the receipt lists forty opened paths, and the delivered spec comes after them.
reads = [("app/F%d.php" % number, False) for number in range(41)] + [("specs/authority.md", False)]
if os.environ.get("FIXTURE_READ_FAILS"):
    reads = [("specs/authority.md", True)]
for number, (path, failed) in enumerate(reads):
    call = "toolu_%d" % number
    emit({"type": "assistant", "parent_tool_use_id": None, "session_id": "native-reads-fixture", "message": {
        "id": "msg_" + call, "role": "assistant", "content": [
            {"type": "tool_use", "id": call, "name": "Read", "input": {"file_path": os.path.join(os.getcwd(), path)}}]}})
    emit({"type": "user", "parent_tool_use_id": None, "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": call, "content": "file text", "is_error": failed}]}})
emit({"type": "assistant", "parent_tool_use_id": None, "session_id": "native-reads-fixture", "message": {
    "id": "msg_end", "role": "assistant", "content": [{"type": "text", "text": text}]}})
emit({"type": "result", "subtype": "success", "is_error": False, "session_id": "native-reads-fixture", "result": text})
"""


FAKE_SHELL = r"""
import json, os, pathlib, sys
receipt = pathlib.Path(sys.argv[1])
receipt.write_text(json.dumps({"cwd": os.getcwd(), "prompt": sys.stdin.read()}))
draft = {"progress": "The cobalt rule is checked at allocation.", "next_steps": [], "learnings": [],
         "used_memory": ["specs/authority.md"]}
text = "Checked the rule.\n\n```memory-draft\n" + json.dumps(draft) + "\n```"
command = "/bin/bash -lc \"sed -n '1,5p' specs/authority.md\""
for event in ({"type": "thread.started", "thread_id": "native-shell-fixture"},
              {"type": "item.started", "item": {"id": "item_1", "type": "command_execution", "command": command,
                                                "status": "in_progress"}},
              {"type": "item.completed", "item": {"id": "item_1", "type": "command_execution", "command": command,
                                                  "status": "completed", "exit_code": 0,
                                                  "aggregated_output": "# Cobalt authority"}},
              {"type": "item.completed", "item": {"type": "agent_message", "text": text}},
              {"type": "turn.completed"}):
    print(json.dumps(event), flush=True)
"""


# Codex writes each call's fill to the thread's rollout; the recovery resumes the thread, so its calls follow there.
FAKE_ROLLOUT = r'''
import datetime, json, os, pathlib, sqlite3, sys
sys.stdin.read()
home = pathlib.Path(os.environ["CODEX_HOME"])
rollout = home / "sessions" / "rollout-native-context-fixture.jsonl"
def record(kind, payload):
    return json.dumps({"type": kind, "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                       "payload": payload}) + "\n"
def count(fill):
    return record("event_msg", {"type": "token_count", "info": {"model_context_window": 258400,
                                                                "last_token_usage": {"input_tokens": fill}}})
if os.environ.get("CONTEXT_MEMORY_RECOVERY"):
    # Resumed near the window, the thread compacts before the recovery's call.
    with rollout.open("a") as handle:
        handle.write(record("compacted", {"message": ""}) + count(24000))
    text = "```memory-draft\n" + json.dumps({"progress": "Cobalt checked", "next_steps": [], "learnings": []}) + "\n```"
else:
    rollout.parent.mkdir(parents=True)
    meta = {"type": "session_meta", "payload": {"id": "native-context-fixture", "cwd": os.getcwd(), "source": "exec"}}
    rollout.write_text(json.dumps(meta) + "\n" + count(12000) + count(240000))
    with sqlite3.connect(home / "state_5.sqlite") as db:
        db.execute("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, cwd TEXT)")
        db.execute("INSERT INTO threads VALUES (?,?,?)", ("native-context-fixture", str(rollout), os.getcwd()))
    text = "Checked the rule."
for event in ({"type": "thread.started", "thread_id": "native-context-fixture"},
              {"type": "item.completed", "item": {"type": "agent_message", "text": text}},
              {"type": "turn.completed", "usage": {"input_tokens": 100, "cached_input_tokens": 50, "output_tokens": 10}}):
    print(json.dumps(event), flush=True)
'''


class TaskContextTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        install_knowledge_fixture(self.project)
        self.fake = self.root / "native.py"
        self.fake.write_text(FAKE_NATIVE)
        self.calls, self.managers, self.knowledge_managers = [], [], []
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": "codex", "name": "Offline fixture", "available": True, "executable": str(self.fake)}])
        discovery.start()
        self.addCleanup(discovery.stop)
        catalog = patch.object(sessions.providers, "model_options", return_value={
            "models": [], "efforts": [], "detail": "Offline fixture"})
        catalog.start()
        self.addCleanup(catalog.stop)
        builder = patch.object(sessions.providers, "build_command", side_effect=self.build_command)
        builder.start()
        self.addCleanup(builder.stop)
        self.addCleanup(self.close_managers)

    def close_managers(self):
        for manager in self.knowledge_managers:
            manager.close()
        for manager in self.managers:
            manager.close()

    def build_command(self, provider, executable, project, prompt, **options):
        receipt = self.root / f"native-{len(self.calls)}.json"
        self.calls.append({"provider": provider, "project": str(project), "prompt": prompt,
                           "receipt": receipt, **options})
        return [sys.executable, "-u", str(self.fake), str(receipt)]

    def manager(self):
        store = sessions.Sessions(self.root / "state", [self.project], timeout=10)
        self.managers.append(store)
        return store

    def knowledge(self, store):
        manager = KnowledgeManager(store)
        self.knowledge_managers.append(manager)
        return manager

    def options(self, store, **changes):
        return {"project_id": next(iter(store.projects)), "provider": "codex", "mode": "plan",
                "prompt": "Check the cobalt allocation rule", "project_context": False,
                "brain": {"bank": "memory-bank", "task_id": "TASK-CONTEXT", "query": "cobalt allocation",
                          "create": True, "goal": "Verify the cobalt allocation rule", "review": True}, **changes}

    def wait_status(self, store, sid, status, timeout=15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            current = store.get(sid)
            if current["status"] == status and store.jobs.unfinished_tasks == 0:
                return current
            if current["status"] == "failed" and status != "failed" and store.jobs.unfinished_tasks == 0:
                self.fail(f"Context session failed: {store.events(sid)}")
            time.sleep(.01)
        self.fail(f"Context session did not reach {status}: {store.get(sid)}, {store.events(sid)}")

    @staticmethod
    def records_text(root):
        return "\n".join(path.read_text(encoding="utf-8") for path in root.rglob("*.md"))

    def git(self, *arguments):
        environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        environment.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                            "GIT_AUTHOR_NAME": "Harness fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
                            "GIT_COMMITTER_NAME": "Harness fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
                            "GIT_TERMINAL_PROMPT": "0"})
        return subprocess.run(["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgSign=false",
                               "-C", str(self.project), *arguments], env=environment,
                              stdin=subprocess.DEVNULL, capture_output=True, text=True, check=True).stdout.strip()

    def test_linked_task_identity_reaches_native_hooks(self):
        store = self.manager()
        with patch.dict(os.environ, {"CONTEXT_TASK_ID": "UNRELATED-PARENT-TASK"}):
            sid = store.create(self.options(store))["id"]
            waiting = self.wait_status(store, sid, "awaiting_context")
            store.run_context(sid, waiting["brain"]["context_id"])
            self.wait_status(store, sid, "completed")
        receipt = json.loads(self.calls[-1]["receipt"].read_text())
        self.assertEqual("TASK-CONTEXT", receipt["task_id"])

    def linked_run(self, store):
        sid = store.create(self.options(store))["id"]
        waiting = self.wait_status(store, sid, "awaiting_context")
        store.run_context(sid, waiting["brain"]["context_id"])
        self.wait_status(store, sid, "completed")
        return sid

    def test_linked_runs_ask_for_a_memory_draft_and_unlinked_runs_do_not(self):
        from harness import memory_draft
        store = self.manager()
        self.linked_run(store)
        self.assertIn(memory_draft.instruction(), self.calls[-1]["prompt"])
        unlinked = {key: value for key, value in self.options(store).items() if key != "brain"}
        sid = store.create(unlinked)["id"]
        self.wait_status(store, sid, "completed")
        self.assertNotIn("memory-draft", self.calls[-1]["prompt"])

    def test_a_reviewed_run_saves_its_draft_when_it_completes(self):
        # Review covers the context before each turn; the run's draft is saved like any other, with no form to fill.
        from harness import memory_draft
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager()
        sid = self.linked_run(store)
        info = store.brain_info(sid)
        self.assertEqual("drafted", info["memory_draft"]["state"])
        self.assertEqual(("The cobalt rule is checked at allocation.", ["Cover the release path."]),
                         (info["task"]["progress"], info["task"]["next_steps"]))
        findings = [record for record in info["records"] if record["type"] == "finding"]
        self.assertEqual([("Cobalt allocation needs one owner", "resolved", "verified")],
                         [(record["title"], record["status"], record["authority"]) for record in findings])
        self.assertEqual([memory_draft.AUTOMATIC_REASON + " [attestation:agent]", memory_draft.AUTOMATIC_REASON],
                         [item["reason"] for item in findings[0]["transitions"][-2:]])
        saved = self.memory_events(store, sid)[-1]
        self.assertTrue(saved["ok"], saved)
        self.assertIn("Saved to project memory: the task's progress and next steps; "
                      "finding “Cobalt allocation needs one owner”.", saved["text"])
        chunk = next((self.project / "memory-bank/chunks").glob("MEM-*-*.md"), None)
        self.assertIsNotNone(chunk)
        self.assertIn("auto-promoted", chunk.read_text(encoding="utf-8"))

    def test_the_save_api_records_a_checked_draft_through_the_runtime(self):
        # The page no longer saves by hand; the API that records a person-checked draft still does.
        store = self.manager()
        sid = self.linked_run(store)
        before = store.brain_info(sid)
        learning = {"type": "finding", "title": "Cobalt allocation needs one owner",
                    "consequence": "Every cobalt allocation names exactly one owner.", "sources": ["specs/authority.md"]}
        result = store.save_memory(sid, {"progress": "The cobalt rule is checked at allocation.",
                                         "next_steps": ["Cover the release path."], "learnings": [learning], "verified": True})
        self.assertTrue(result["ok"], result)
        saved = result["saved"]
        self.assertGreater(saved["task"]["revision"], before["task"]["revision"])
        self.assertEqual([("finding", "resolved")], [(item["type"], item["status"]) for item in saved["records"]])
        task = store.brain_info(sid)["task"]
        self.assertEqual("The cobalt rule is checked at allocation.", task["progress"])
        # The draft is the current plan, so it replaced the steps rather than appending.
        self.assertEqual(["Cover the release path."], task["next_steps"])
        finding = next(record for record in store.brain_info(sid)["records"] if record["id"] == saved["records"][0]["id"])
        self.assertEqual(("verified", "Every cobalt allocation names exactly one owner."),
                         (finding["authority"], finding["progress"]))
        promotion = saved["promotion"]
        self.assertTrue(promotion["enabled"], promotion)
        self.assertEqual(1, len(promotion["promoted"]), promotion)

    def test_saved_progress_with_empty_plan_removes_finished_next_steps(self):
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager(); sid = self.linked_run(store)
        draft = store.brain_info(sid)['memory_draft']['draft']
        self.assertTrue(store.save_memory(sid, {**draft, 'verified': True})['ok'])
        self.assertTrue(store.brain_info(sid)['task']['next_steps'])
        result = store._task_context().save_memory(store.get(sid),
            {'progress': 'Release coverage finished.', 'next_steps': [], 'learnings': []}, automatic=True)
        self.assertTrue(result['ok'], result)
        self.assertEqual([], store.brain_info(sid)['task']['next_steps'])

    LEARNING = {"type": "finding", "title": "Cobalt allocation needs one owner",
                "consequence": "Every cobalt allocation names exactly one owner.", "sources": ["specs/authority.md"]}

    def findings(self, store, sid):
        return [record for record in store.brain_info(sid)["records"]
                if record["type"] == "finding" and record["title"] == self.LEARNING["title"]]

    def test_a_repeated_save_of_one_run_writes_each_learning_once(self):
        # The same run's save again - a retry, or a replay after a crash
        # between its writes - finds what the first attempt wrote.
        store = self.manager(); sid = self.linked_run(store)
        context = store._task_context()
        draft = {"progress": "The cobalt rule is checked.", "next_steps": [], "learnings": [self.LEARNING],
                 "verified": True}
        generation = store.generations.get(sid)
        first = context.save_memory(store.get(sid), draft, automatic=True, generation=generation)
        revision = store.brain_info(sid)["task"]["revision"]
        second = context.save_memory(store.get(sid), draft, automatic=True, generation=generation)
        self.assertTrue(first["ok"] and second["ok"], (first, second))
        self.assertEqual(1, len(self.findings(store, sid)))
        self.assertEqual(revision, store.brain_info(sid)["task"]["revision"])
        # A later run restating the same learning names the same record too.
        third = context.save_memory(store.get(sid), {**draft, "progress": "Release path covered."}, automatic=True)
        self.assertTrue(third["ok"], third)
        self.assertEqual(1, len(self.findings(store, sid)))
        self.assertEqual("Release path covered.", store.brain_info(sid)["task"]["progress"])

    def test_a_task_moved_on_since_it_was_read_is_saved_on_the_second_try(self):
        # A Stop hook's checkpoint can land between reading the task and saving.
        store = self.manager(); sid = self.linked_run(store)
        context = store._task_context()
        stale = store.brain_info(sid)["task"]
        moved = store.brain_action(sid, {"action": "brain-update", "revision": stale["revision"],
                                         "progress": "A checkpoint moved the task on."})
        self.assertTrue(moved["ok"], moved)
        original = context._knowledge
        reads = []
        def first_read_is_stale(operation, *arguments, **options):
            if operation == "inspect" and options.get("task_only") and not reads:
                reads.append(True)
                return {"task": stale}
            return original(operation, *arguments, **options)
        with patch.object(context, "_knowledge", side_effect=first_read_is_stale):
            result = context.save_memory(store.get(sid), {"progress": "Saved after the checkpoint.", "next_steps": [],
                                                          "learnings": [self.LEARNING], "verified": True}, automatic=True)
        self.assertTrue(result["ok"], result)
        self.assertEqual("Saved after the checkpoint.", store.brain_info(sid)["task"]["progress"])
        self.assertEqual(1, len(self.findings(store, sid)))

    def test_a_runtime_without_record_result_saves_command_by_command(self):
        from harness import knowledge
        store = self.manager(); sid = self.linked_run(store)
        context = store._task_context()
        original = context._call
        actions = []
        def older_runtime(session, options, workspace, action, **fields):
            actions.append(action)
            if action == "record-result":
                raise sessions.SessionError(knowledge.OLDER_RUNTIME)
            return original(session, options, workspace, action, **fields)
        with patch.object(context, "_call", side_effect=older_runtime):
            result = context.save_memory(store.get(sid), {"progress": "Saved the old way.", "next_steps": [],
                                                          "learnings": [self.LEARNING], "verified": True}, automatic=True)
        self.assertTrue(result["ok"], result)
        self.assertEqual("record-result", actions[0])
        self.assertIn("brain-create", actions)
        self.assertEqual([("finding", "resolved")], [(item["type"], item["status"]) for item in result["saved"]["records"]])

    def test_saving_refuses_unconfirmed_or_unsourced_learnings_and_a_finished_task(self):
        store = self.manager()
        sid = self.linked_run(store)
        learning = {"type": "decision", "title": "Owners are explicit", "consequence": "Name the owner.",
                    "sources": ["specs/authority.md"]}
        for data, message in (({"learnings": [learning]}, "Confirm that you checked"),
                              ({"learnings": [{**learning, "sources": ["specs/missing.md"]}], "verified": True},
                               "Source not found"),
                              ({}, "Nothing to save")):
            with self.subTest(message=message):
                with self.assertRaisesRegex(sessions.SessionError, message):
                    store.save_memory(sid, data)
        self.assertEqual([], [record for record in store.brain_info(sid)["records"] if record["type"] == "decision"])
        task = store.brain_info(sid)["task"]
        completed = store.brain_action(sid, {"action": "complete", "revision": task["revision"],
                                             "outcome": "Verified", "verification": ["Fixture"]})
        self.assertTrue(completed["ok"], completed)
        with self.assertRaisesRegex(sessions.SessionError, "linked task is finished"):
            store.save_memory(sid, {"progress": "More"})

    def automatic(self, store, **changes):
        """A session with the composer's default: project memory on, nothing to review."""
        return {**self.options(store, **changes), "brain": {"bank": "memory-bank", "auto": True}}

    def memory_events(self, store, sid):
        return [event for event in store.events(sid) if event["kind"] == "memory"]

    def test_unattended_memory_reaches_the_launch_without_a_step_from_a_person(self):
        from harness import memory_draft
        store = self.manager()
        governed = self.project / "project-brain/control/retrieval-manifests"
        before = sorted(governed.glob("*.json"))
        sid = store.create(self.automatic(store))["id"]
        session = self.wait_status(store, sid, "completed")
        brain = session["brain"]
        # The first message names the task and is the first query.
        self.assertFalse(brain["review"])
        self.assertRegex(brain["task_id"], r"^harness/check-the-cobalt-allocation-rule-[0-9a-f]{6}$")
        self.assertEqual(("Check the cobalt allocation rule", "Check the cobalt allocation rule"),
                         (brain["goal"], brain["query"]))
        self.assertEqual((False, brain["task"]["id"]), (brain["create"], brain["record_id"]))
        self.assertNotIn("Context prepared", json.dumps(store.events(sid)))
        received = json.loads(self.calls[0]["receipt"].read_text())
        # The runtime's rendered capsule, not the JSON: working state first, then
        # the passage that answers the message.
        self.assertTrue(brain["capsule_text"].startswith("working: "), brain["capsule_text"])
        self.assertIn(sessions.BRAIN_CONTEXT_HEADER + brain["capsule_text"] + "\n\n", received["prompt"])
        self.assertNotIn('"categories"', received["prompt"])
        self.assertIn("requires one owner.", received["prompt"])
        self.assertIn(memory_draft.instruction(), received["prompt"])
        # The project's own read hook stands down for a turn whose capsule is already in the prompt.
        self.assertEqual((brain["task_id"], "1"), (received["task_id"], received["delivered"]))
        # Usage › Context counts that text, not the JSON it replaced: the parts add up to what was inserted, and the
        # spec the message retrieved - its item line and the excerpt under it - is rules and docs.
        inserted = sessions.BRAIN_CONTEXT_HEADER + brain["capsule_text"] + "\n\n"
        part = store.results.history(sid)["launches"][0]["context"]["ledger"]["capsule"]
        self.assertEqual((len(inserted), len(inserted)), (part["inserted"], sum(part["kinds"].values())))
        self.assertEqual(context_usage.capsule_text_parts(inserted), part["kinds"])
        lines = inserted.split("\n")
        spec = next(index for index, line in enumerate(lines) if line.startswith("- memory specs/authority.md — "))
        self.assertTrue(lines[spec + 1].startswith("  "), lines[spec + 1])
        self.assertEqual(len(lines[spec]) + len(lines[spec + 1]) + 2, part["kinds"]["rules"])
        self.assertLess(part["inserted"], sum(sessions.capsule_parts(brain["capsule"]).values()))
        # A turn's own retrieval stays in ignored local state, out of the project's history.
        self.assertRegex(brain["capsule"]["manifest"], r"^memory-bank/local/retrieval-manifests/")
        self.assertEqual(before, sorted(governed.glob("*.json")))
        memory = self.memory_events(store, sid)
        self.assertEqual([True, True, False, False], [event["ok"] for event in memory])
        self.assertIn(f"Project memory for this turn: task {brain['task_id']}, ", memory[0]["text"])
        self.assertEqual(['started', 'failed'], [event['recovery'] for event in memory if 'recovery' in event])
        self.assertEqual(memory_draft.summary("missing"), memory[-1]["text"])
        self.assertEqual(2, len(self.calls))  # one bounded recovery; no third launch
        with self.assertRaisesRegex(sessions.SessionError, "for each message by itself"):
            store.prepare_context(sid, "cobalt allocation")

    def test_a_cursor_turn_gets_the_checkpoint_its_headless_mode_skips(self):
        # cursor-agent --print fires no stop hook, so nothing else would record
        # the turn the agent just worked.
        self.fake.write_text(FAKE_CURSOR)
        report = self.project / "memory-bank/local/last-turn-report.json"
        with patch.object(sessions.providers, "discover_providers", return_value=[
                {"id": "cursor", "name": "Offline fixture", "available": True, "executable": str(self.fake)}]):
            store = self.manager()
        sid = store.create(self.automatic(store, provider="cursor"))["id"]
        session = self.wait_status(store, sid, "completed")
        self.assertEqual(session["brain"]["task_id"], json.loads(report.read_text(encoding="utf-8"))["task_id"])
        # When the hook did fire this run, its checkpoint stands.
        written = report.stat().st_mtime
        context = store._task_context(wait=5)
        self.assertIsNone(context.checkpoint(session, since=written))
        self.assertEqual(written, report.stat().st_mtime)
        self.assertEqual(session["brain"]["task_id"], context.checkpoint(session, since=written + 1)["task_id"])

    def test_a_codex_turn_leaves_the_checkpoint_to_its_stop_hook(self):
        report = self.project / "memory-bank/local/last-turn-report.json"
        store = self.manager()
        sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, "completed")
        self.assertFalse(report.exists())

    def test_an_unattended_run_saves_its_draft_and_each_follow_up_retrieves_for_its_message(self):
        from harness import memory_draft
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager()
        sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, "completed")
        info = store.brain_info(sid)
        self.assertEqual(("The cobalt rule is checked at allocation.", ["Cover the release path."]),
                         (info["task"]["progress"], info["task"]["next_steps"]))
        findings = [record for record in info["records"] if record["type"] == "finding"]
        self.assertEqual([("Cobalt allocation needs one owner", "resolved", "verified")],
                         [(record["title"], record["status"], record["authority"]) for record in findings])
        # The ledger says the attestation was the agent's, not a person's.
        self.assertEqual([("observed", "verified"), ("open", "resolved")],
                         [(item["from"], item["to"]) for item in findings[0]["transitions"]][-2:])
        # The verification itself carries the machine-readable attestation.
        self.assertEqual([memory_draft.AUTOMATIC_REASON + " [attestation:agent]", memory_draft.AUTOMATIC_REASON],
                         [item["reason"] for item in findings[0]["transitions"][-2:]])
        saved = self.memory_events(store, sid)[-1]
        self.assertTrue(saved["ok"], saved)
        self.assertIn("Saved to project memory: the task's progress and next steps; "
                      "finding “Cobalt allocation needs one owner”.", saved["text"])
        self.assertRegex(saved["text"], r"Promoted to the Memory Bank as MEM-")
        # What the run's own tools read of the delivered memory, beside what it was handed. This Codex run ran
        # no command, so it could read nothing: its zero is observed, not unknown.
        self.assertEqual((1, 0, "observed"), (saved["delivered_sources"], saved["opened_delivered_sources"],
                                              saved["opened_observation"]))
        self.assertIn("Memory use: 1 delivered, 0 opened by the agent. Neither proves the claim was used.", saved["text"])
        chunk = next((self.project / "memory-bank/chunks").glob("MEM-*-*.md"), None)
        self.assertIsNotNone(chunk)
        self.assertIn("auto-promoted", chunk.read_text(encoding="utf-8"))

        store.send(sid, "Now check who owns the release path")
        self.wait_status(store, sid, "completed")
        brain = store.get(sid)["brain"]
        self.assertEqual("Now check who owns the release path", brain["query"])
        self.assertEqual(2, len(self.calls))
        self.assertEqual("native-context-fixture", self.calls[1]["session_id"])
        self.assertIn(brain["capsule_text"], self.calls[1]["prompt"])
        # The agent restated its learning; this session already saved it.
        again = self.memory_events(store, sid)[-1]
        self.assertTrue(again["ok"], again)
        self.assertIn("1 learning(s) were already saved from this session.", again["text"])
        self.assertEqual(1, len([record for record in store.brain_info(sid)["records"] if record["type"] == "finding"]))

    def test_opened_memory_is_the_runs_successful_reads_or_says_it_cannot_see_them(self):
        def run(provider, fake, environment=None):
            # Each run in a project of its own, so one run's saved progress is not the next one's memory.
            project, number = self.root / f"project-{provider}-{len(self.calls)}", len(self.calls)
            project.mkdir()
            install_knowledge_fixture(project)
            self.fake.write_text(fake)
            with patch.object(sessions.providers, "discover_providers", return_value=[
                    {"id": provider, "name": "Offline fixture", "available": True, "executable": str(self.fake)}]), \
                    patch.dict(os.environ, environment or {}):
                store = sessions.Sessions(self.root / f"state-{number}", [project], timeout=10)
                self.managers.append(store)
                sid = store.create(self.automatic(store, provider=provider))["id"]
                self.wait_status(store, sid, "completed")
            receipt = store.results.history(sid)["launches"][0]["receipt"]
            return self.memory_events(store, sid)[-1], receipt

        # The delivered spec is read after forty-one other files, past the receipt's list of opened paths.
        saved, receipt = run("claude", FAKE_READS)
        self.assertEqual((42, 40), (receipt["opened"], len(receipt["opened_paths"])))
        self.assertNotIn("specs/authority.md", receipt["opened_paths"])
        self.assertEqual((1, 1, "observed"), (saved["delivered_sources"], saved["opened_delivered_sources"],
                                              saved["opened_observation"]))
        self.assertIn("Memory use: 1 delivered, 1 opened by the agent, 1 reported used.", saved["text"])
        # A read that failed returned nothing, though the run view lists the attempt.
        saved, receipt = run("claude", FAKE_READS, {"FIXTURE_READ_FAILS": "1"})
        self.assertEqual((["specs/authority.md"], 0, "observed"), (receipt["opened_paths"], saved["opened_delivered_sources"],
                                                                   saved["opened_observation"]))
        # Codex read the spec with sed: its shell is not inspected, so the count is unknown rather than zero.
        saved, receipt = run("codex", FAKE_SHELL)
        self.assertEqual((1, None, "unknown"), (saved["delivered_sources"], saved["opened_delivered_sources"],
                                                saved["opened_observation"]))
        self.assertEqual({"delivered": 1, "opened": 0, "observation": "unknown", "unseen": ["shell"]}, receipt["memory"])
        self.assertIn("Memory use: 1 delivered, opening unknown (no read reported; shell commands not inspected), "
                      "1 reported used.", saved["text"])

    def test_memory_that_cannot_be_retrieved_costs_the_turn_its_memory_never_the_turn(self):
        store = self.manager()

        def unavailable(context, *arguments):
            raise sessions.SessionError("Capsule unavailable: fixture outage")

        with patch.object(TaskContext, "_retrieve", unavailable):
            sid = store.create(self.automatic(store))["id"]
            session = self.wait_status(store, sid, "completed")
        received = json.loads(self.calls[0]["receipt"].read_text())
        self.assertNotIn(sessions.BRAIN_CONTEXT_HEADER, received["prompt"])
        # Without a capsule of its own the turn leaves the project's hook to deliver one.
        self.assertEqual((session["brain"]["task_id"], None), (received["task_id"], received["delivered"]))
        self.assertIsNone(session["brain"]["capsule"])
        notice = self.memory_events(store, sid)[0]
        self.assertEqual((False, "Project memory was not retrieved for this turn: Capsule unavailable: fixture outage"),
                         (notice["ok"], notice["text"]))

    def test_memory_waits_for_another_knowledge_operation_instead_of_skipping_the_turn(self):
        store = self.manager()
        knowledge = self.knowledge(store)
        store.knowledge = knowledge
        knowledge.lock.acquire()
        try:
            sid = store.create(self.automatic(store))["id"]
            time.sleep(.5)
            self.assertEqual([], self.calls)
        finally:
            knowledge.lock.release()
        self.wait_status(store, sid, "completed")
        self.assertTrue(self.memory_events(store, sid)[0]["ok"])

    def test_missing_draft_recovers_once_without_repeating_the_request(self):
        self.assert_recovery("no draft")

    def test_reviewed_session_does_not_recover_a_missing_draft(self):
        from harness import memory_recovery
        store = self.manager()
        with patch.object(memory_recovery, 'recover', side_effect=AssertionError('Recovery for reviewed context')):
            sid = self.linked_run(store)
        self.assertEqual(1, len(self.calls))
        self.assertFalse(any('recovery' in e for e in self.memory_events(store, sid)))

    def test_unreadable_draft_recovers_once_without_repeating_the_request(self):
        self.assert_recovery("```memory-draft\n{oops\n```")

    def assert_recovery(self, initial):
        self.fake.write_text(FAKE_DRAFT.replace(
            'print(json.dumps({"type": "thread.started"',
            'if not os.environ.get("CONTEXT_MEMORY_RECOVERY"):\n    text = ' + repr(initial) + '\nprint(json.dumps({"type": "thread.started"'))
        store = self.manager()
        before = len(self.calls)
        sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, "completed")
        calls = self.calls[before:]
        self.assertEqual(2, len(calls))
        self.assertEqual("native-context-fixture", calls[1]["session_id"])
        self.assertEqual("plan", calls[1]["mode"])
        self.assertFalse(calls[1]["agents_enabled"])
        self.assertIn("Do not repeat the user task", calls[1]["prompt"])
        self.assertEqual(1, len([e for e in store.events(sid) if e['kind'] == 'user']))
        self.assertEqual("Check the cobalt allocation rule", store.get(sid)['brain']['query'])
        findings = [r for r in store.brain_info(sid)['records'] if r['type'] == 'finding']
        self.assertEqual(1, len(findings))
        self.assertEqual(['started', 'recovered'], [e['recovery'] for e in self.memory_events(store, sid) if 'recovery' in e])

    def test_valid_empty_draft_is_not_recovered_and_failed_primary_is_not_saved(self):
        empty = "```memory-draft\n" + json.dumps({'progress': '', 'next_steps': [], 'learnings': []}) + "\n```"
        self.fake.write_text(FAKE_NATIVE.replace('"RAW ASSISTANT FIXTURE OUTPUT"', repr(empty)))
        store = self.manager(); before = len(self.calls)
        sid = store.create(self.automatic(store))['id']; self.wait_status(store, sid, 'completed')
        self.assertEqual(1, len(self.calls) - before)
        self.assertFalse(any('recovery' in e for e in self.memory_events(store, sid)))
        self.fake.write_text(FAKE_NATIVE + "\nsys.exit(1)\n")
        before = len(self.calls)
        sid = store.create(self.automatic(store))['id']; self.wait_status(store, sid, 'failed')
        self.assertEqual(1, len(self.calls) - before)
        self.assertFalse(any('recovery' in e for e in self.memory_events(store, sid)))

    def test_recovery_cancellation_preserves_cancelled_status_and_saves_nothing(self):
        from harness import memory_recovery
        original = memory_recovery.recover
        def cancel_between_phases(store, sid, *args, **kwargs):
            store.cancel(sid)
            return original(store, sid, *args, **kwargs)
        store = self.manager()
        with patch.object(memory_recovery, 'recover', cancel_between_phases):
            sid = store.create(self.automatic(store))['id']
            self.wait_status(store, sid, 'cancelled')
        self.assertEqual(1, len(self.calls))
        self.assertEqual(['started', 'cancelled'], [e['recovery'] for e in self.memory_events(store, sid) if 'recovery' in e])
        self.assertFalse(any(r['type'] == 'finding' for r in store.brain_info(sid)['records']))
        self.assertEqual('cancelled', [e['outcome'] for e in store.events(sid) if 'outcome' in e][-1])

    def test_recovery_cancellation_while_child_is_running_saves_nothing(self):
        self.fake.write_text(FAKE_NATIVE.replace('print(json.dumps({"type": "thread.started"',
            'if os.environ.get("CONTEXT_MEMORY_RECOVERY"):\n    import time; time.sleep(5)\nprint(json.dumps({"type": "thread.started"'))
        store = self.manager(); sid = store.create(self.automatic(store))['id']
        deadline = time.monotonic() + 10
        while len(self.calls) < 2 and time.monotonic() < deadline: time.sleep(.01)
        self.assertEqual(2, len(self.calls))
        store.cancel(sid); self.wait_status(store, sid, 'cancelled')
        self.assertFalse(any(r['type'] == 'finding' for r in store.brain_info(sid)['records']))
        self.assertEqual('cancelled', self.memory_events(store, sid)[-1]['recovery'])

    def test_recovery_without_native_id_or_measurable_budget_is_skipped(self):
        self.fake.write_text(FAKE_NATIVE.replace('print(json.dumps({"type": "thread.started", "thread_id": "native-context-fixture"}), flush=True)', ''))
        store = self.manager(); sid = store.create(self.automatic(store))['id']; self.wait_status(store, sid, 'completed')
        self.assertEqual(1, len(self.calls))
        self.assertIn('skipped', [e.get('recovery') for e in self.memory_events(store, sid)])
        self.fake.write_text(FAKE_NATIVE)
        before = len(self.calls)
        sid = store.create(self.automatic(store, budgets={'tokens': 1000}))['id']; self.wait_status(store, sid, 'completed')
        self.assertEqual(1, len(self.calls) - before)
        self.assertIn('skipped', [e.get('recovery') for e in self.memory_events(store, sid)])

    def test_cancellation_after_recovered_reply_before_save_starts_no_write(self):
        self.fake.write_text(FAKE_DRAFT.replace(
            'print(json.dumps({"type": "thread.started"',
            'if not os.environ.get("CONTEXT_MEMORY_RECOVERY"):\n    text = "No draft"\nprint(json.dumps({"type": "thread.started"'))
        original = TaskContext.save_memory
        before = []
        def cancel_before_save(context, session, *args, **kwargs):
            before.append(context.sessions.brain_info(session['id'])['task'])
            context.sessions.cancel(session['id'])
            return original(context, session, *args, **kwargs)
        store = self.manager()
        with patch.object(TaskContext, 'save_memory', cancel_before_save):
            sid = store.create(self.automatic(store))['id']; self.wait_status(store, sid, 'cancelled')
        self.assertEqual(2, len(self.calls))
        self.assertEqual(1, len(before))
        self.assertEqual(before[0], store.brain_info(sid)['task'])
        self.assertFalse(any(r['type'] == 'finding' for r in store.brain_info(sid)['records']))
        self.assertEqual('cancelled', [e['outcome'] for e in store.events(sid) if 'outcome' in e][-1])

    def test_claude_recovery_usage_and_prompt_counts_share_original_launch(self):
        self.fake.write_text('''import json, os, sys
recovery = bool(os.environ.get("CONTEXT_MEMORY_RECOVERY"))
print(json.dumps({"type":"system","subtype":"init","session_id":"native-context-fixture","claude_code_version":"2.1.277"}))
reply = "Done without draft" if not recovery else "```memory-draft\\n" + json.dumps({"progress":"Cobalt checked", "next_steps":[], "learnings":[]}) + "\\n```"
print(json.dumps({"type":"result","subtype":"success","is_error":False,"result":reply,"session_id":"native-context-fixture","total_cost_usd":1.2 if recovery else 1.0,"usage":{"input_tokens":7 if recovery else 10,"output_tokens":2 if recovery else 3}}))
''')
        with patch.object(sessions.providers, 'discover_providers', return_value=[
                {'id':'claude', 'name':'Fixture', 'available':True, 'executable':str(self.fake)}]):
            store = self.manager()
            sid = store.create(self.automatic(store, provider='claude'))['id']; self.wait_status(store, sid, 'completed')
        self.assertEqual(2, len(self.calls))
        usage = store.get(sid)['budget_usage']
        self.assertEqual(22, usage['tokens'])
        self.assertAlmostEqual(1.2, usage['cost_usd'])
        self.assertTrue(any(e.get('memory_recovery') for e in store.events(sid) if e['kind']=='usage'))
        row = store.db.execute('SELECT context FROM launches WHERE session_id=?', (sid,)).fetchone()
        context = json.loads(row['context'])
        from harness import memory_recovery
        self.assertEqual(len(memory_recovery.PROMPT), context['recovery']['characters'])
        self.assertGreaterEqual(context['ledger']['instructions'], len(memory_recovery.PROMPT))

    def test_codex_recovery_calls_and_compaction_reach_the_launch_fill(self):
        # The fill settled before the recovery ran: the recovery's prompt reached the ledger, but its compaction and
        # call did not reach the fill, which stayed at the launch's 240,000 and two calls, with no divider.
        self.fake.write_text(FAKE_ROLLOUT)
        with patch.dict(os.environ, {"CODEX_HOME": str(self.root / "codex-home")}):
            store = self.manager()
            sid = store.create(self.automatic(store))["id"]
            self.wait_status(store, sid, "completed")
        self.assertEqual(2, len(self.calls))
        self.assertEqual(["started", "recovered"], [e["recovery"] for e in self.memory_events(store, sid) if "recovery" in e])
        context = store.results.history(sid)["launches"][0]["context"]
        from harness import memory_recovery
        self.assertEqual(len(memory_recovery.PROMPT), context["recovery"]["characters"])
        fill = context["fill"]
        self.assertEqual((12000, 24000, 240000, 3, 258400),
                         (fill["start"], fill["end"], fill["peak"], fill["calls"], fill["window"]))
        self.assertEqual([{"pre": 240000, "post": 24000, "call": 2}], fill["compactions"])
        self.assertEqual([{"pre": 240000, "post": 24000}], [event["compaction"] for event in store.events(sid) if "compaction" in event])

    def test_automatic_query_is_sanitized_before_goal_and_memory_state(self):
        from harness.task_context import message_query
        prompt = "Check CMS768 cobalt allocation for person@example.test; call +370 600 12345"
        options = TaskContext.validate_options({"bank": "memory-bank", "auto": True}, prompt)
        self.assertIn("CMS768 cobalt allocation", options["query"])
        for key in ("query", "goal", "task_id"):
            self.assertNotIn("person@example.test", options[key])
            self.assertNotIn("12345", options[key])
        for private_only in ("person@example.test", "Customer email: person@example.test",
                             "Call +370 600 12345", "Patient name: Alice Smith",
                             "Client address: Main Street 7, Vilnius", "Client address: Main Street 7\nVilnius",
                             "Patient name: Alice; Smith; Fix CMS768",
                             "Client address: Building A; Vilnius; Fix CMS768"):
            self.assertEqual("", message_query(private_only))
        # A quoted secret goes whole from the query and the goal stored in Git.
        quoted = "Fix CMS768 deploy password='alpha beta gamma' after migration"
        options = TaskContext.validate_options({"bank": "memory-bank", "auto": True}, quoted)
        for key in ("query", "goal", "task_id"):
            self.assertNotIn("beta gamma", options[key])
            self.assertNotIn("alpha", options[key])
        self.assertIn("CMS768 deploy", options["query"])
        # A secret is cut out, not a reason to drop the turn's memory; a pasted
        # transcript line loses its role prefix and keeps its words.
        self.assertEqual("", message_query("ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ123456"))
        self.assertEqual("copied request", message_query("User: copied request"))
        # The shared adapter has one implementation for installed hooks and Harness.
        runtime = Path(__file__).resolve().parents[1] / "PHP Core/memory-bank/scripts/automatic_query.py"
        from harness import automatic_query
        self.assertEqual(runtime.read_bytes(), Path(automatic_query.__file__).read_bytes())

    def test_a_slack_token_and_a_customer_number_stay_out_of_the_goal_and_the_records(self):
        from harness.task_context import message_query
        # Built here, so no literal token sits in the repository.
        token = "xox" + "b-" + "123456789012-1234567890123-" + "AbCdEfGhIjKlMnOpQrStUvWx"
        prompt = f"Fix CMS768 checkout timeout for customer ID 10492; notify with {token}"
        options = TaskContext.validate_options({"bank": "memory-bank", "auto": True}, prompt)
        for key in ("query", "goal", "task_id"):
            self.assertNotIn(token, options[key])
            self.assertNotIn("10492", options[key])
        self.assertIn("CMS768 checkout timeout for customer ID", options["query"])
        self.assertEqual("", message_query("Call customer #10492"))
        store = self.manager()
        sid = store.create(self.automatic(store, prompt=prompt))["id"]
        completed = self.wait_status(store, sid, "completed")
        self.assertTrue(completed["brain"]["task"]["id"])
        stored = self.records_text(self.project / "project-brain") + "".join(
            path.read_text(encoding="utf-8") for path in self.project.glob("**/*retrieval-manifests/*.json"))
        self.assertIn("CMS768", stored)
        self.assertNotIn(token, stored)
        self.assertNotIn("10492", stored)

    def test_pii_only_turn_skips_memory_and_does_not_create_a_task_or_manifest(self):
        store = self.manager()
        for private_only in ("person@example.test", "Customer email: person@example.test", "Call +370 600 12345",
                             "Patient name: Alice Smith", "Client address: Main Street 7, Vilnius"):
            sid = store.create(self.automatic(store, prompt=private_only))["id"]
            completed = self.wait_status(store, sid, "completed")
            self.assertEqual("", completed["brain"]["query"])
            self.assertIsNone(completed["brain"].get("capsule"))
            self.assertFalse(list(self.project.glob("**/*retrieval-manifests/*.json")))
            self.assertFalse(list(self.project.glob("project-brain/dynamic/tasks/*.md")))
            self.assertTrue(any("no safe technical content" in event.get("text", "")
                                for event in store.events(sid) if event["kind"] == "memory"))
            self.assertNotIn("person@example.test", json.dumps(completed["brain"]))

    def test_options_name_an_automatic_task_from_the_first_message(self):
        validate = TaskContext.validate_options
        options = validate({"bank": "memory-bank", "auto": True}, " \n Fix the\tlogin   redirect\nmore detail")
        self.assertEqual(("Fix the login redirect", " \n Fix the\tlogin   redirect\nmore detail".strip(), True, False),
                         (options["goal"], options["query"], options["create"], options["review"]))
        self.assertRegex(options["task_id"], r"^harness/fix-the-login-redirect-[0-9a-f]{6}$")
        self.assertNotEqual(options["task_id"], validate({"bank": "memory-bank", "auto": True}, "Fix the login redirect")["task_id"])
        self.assertTrue(validate({"bank": "memory-bank", "auto": True}, "Пофиксить вход")["task_id"].startswith("harness/task-"))
        self.assertEqual(200, len(validate({"bank": "memory-bank", "auto": True}, "x" * 500)["goal"]))
        for data, prompt in (({"bank": "memory-bank", "auto": True, "task_id": "T-1"}, "Go"),
                             ({"bank": "memory-bank", "auto": True, "create": True}, "Go"),
                             ({"bank": "memory-bank", "auto": True}, None),
                             ({"bank": "memory-bank", "auto": "yes"}, "Go"),
                             ({"bank": "memory-bank", "auto": True, "review": True}, "Go")):
            with self.subTest(data=data), self.assertRaises(sessions.SessionError):
                validate(data, prompt)
        # Review keeps its explicit query; an unattended session asks with each message.
        self.assertEqual("cobalt", validate({"bank": "memory-bank", "task_id": "T-1", "query": "cobalt", "review": True},
                                            "Check everything")["query"])
        self.assertEqual("Check everything", validate({"bank": "memory-bank", "task_id": "T-1"}, "Check everything")["query"])
        # Sessions linked before memory ran unattended were linked for review.
        self.assertTrue(sessions.reviewed({"task_id": "T-1"}))
        self.assertFalse(sessions.reviewed({"task_id": "T-1", "review": False}))

    def test_capsule_meter_measures_the_stored_capsule_and_the_inserted_prompt(self):
        store = self.manager()
        sid = store.create(self.options(store))["id"]
        waiting = self.wait_status(store, sid, "awaiting_context")
        capsule, meter = waiting["brain"]["capsule"], waiting["capsule_meter"]
        # The copied runtime's own measure (serialize_capsule in memory-bank/scripts/context.py).
        self.assertEqual(len(json.dumps(capsule, ensure_ascii=False, separators=(",", ":"))), meter["characters"])
        self.assertEqual(meter["characters"], sum(meter["kinds"].values()))
        self.assertLessEqual(meter["characters"], meter["limit"])
        self.assertEqual(sum(len(capsule[layer]) for layer in ("procedural", "semantic", "episodic")),
                         sum(meter["items"].values()))
        store.run_context(sid, waiting["brain"]["context_id"])
        self.wait_status(store, sid, "completed")
        inserted = sessions.BRAIN_CONTEXT_HEADER + json.dumps(capsule, ensure_ascii=False) + "\n\n"
        self.assertIn(inserted, self.calls[0]["prompt"])
        self.assertEqual(len(inserted), meter["prompt_characters"])

    def test_reviewed_context_survives_restart_and_each_followup_requires_a_new_receipt(self):
        store = self.manager()
        sid = store.create(self.options(store))["id"]
        waiting = self.wait_status(store, sid, "awaiting_context")
        self.assertEqual(self.calls, [])
        brain = waiting["brain"]
        self.assertFalse(brain["approved"])
        self.assertEqual(brain["capsule"]["task_uuid"], brain["task"]["id"])
        self.assertIn("specs/authority.md", json.dumps(brain["capsule"]))
        with self.assertRaises(sessions.SessionError):
            store.run_context(sid, "00000000-0000-4000-8000-000000000000")
        self.assertEqual(store.get(sid), waiting)
        self.assertEqual(self.calls, [])
        store.close()
        self.managers.remove(store)
        restarted = self.manager()
        self.assertEqual(restarted.get(sid)["brain"], brain)
        restarted.run_context(sid, brain["context_id"])
        completed = self.wait_status(restarted, sid, "completed")
        self.assertEqual(len(self.calls), 1)
        received = json.loads(self.calls[0]["receipt"].read_text())
        self.assertEqual(received["prompt"], self.calls[0]["prompt"])
        self.assertIn(json.dumps(brain["capsule"], ensure_ascii=False), received["prompt"])
        self.assertIn("rule requires one owner.", received["prompt"])
        self.assertIn("Check the cobalt allocation rule", received["prompt"])
        task_path = self.project / "project-brain/dynamic/tasks" / (brain["task"]["id"] + ".md")
        self.assertNotEqual(metadata(task_path)["status"], "completed")
        self.assertNotIn("RAW ASSISTANT FIXTURE OUTPUT", self.records_text(self.project / "project-brain"))
        updated = restarted.brain_action(sid, {"action": "brain-update", "revision": metadata(task_path)["revision"],
                                               "progress": "USER REVIEWED PROGRESS"})
        self.assertTrue(updated["ok"], updated)
        self.assertEqual(updated["result"]["id"], brain["task"]["id"])
        self.assertIn("USER REVIEWED PROGRESS", task_path.read_text())
        restarted.send(sid, "Verify the same rule once more")
        next_context = self.wait_status(restarted, sid, "awaiting_context")
        self.assertEqual(len(self.calls), 1)
        self.assertNotEqual(next_context["brain"]["context_id"], brain["context_id"])
        self.assertFalse(next_context["brain"]["approved"])
        with self.assertRaises(sessions.SessionError):
            restarted.run_context(sid, brain["context_id"])
        restarted.run_context(sid, next_context["brain"]["context_id"])
        self.wait_status(restarted, sid, "completed")
        self.assertEqual(len(self.calls), 2)
        self.assertEqual(self.calls[1]["session_id"], completed["native_session_id"])
        self.assertIn("Verify the same rule once more", self.calls[1]["prompt"])
        result = restarted.brain_action(sid, {"action": "complete", "revision": metadata(task_path)["revision"],
                                               "outcome": "EXPLICIT REVIEWED OUTCOME", "verification": ["Offline fixture verified"],
                                               "sources": ["specs/authority.md"]})
        self.assertTrue(result["ok"], result)
        self.assertEqual(metadata(task_path)["status"], "completed")
        text = self.records_text(self.project / "project-brain")
        self.assertIn("EXPLICIT REVIEWED OUTCOME", text)
        self.assertNotIn("RAW ASSISTANT FIXTURE OUTPUT", text)

    def test_changed_task_revision_or_same_stat_source_blocks_provider_until_context_is_reviewed_again(self):
        store = self.manager()
        knowledge = self.knowledge(store)
        for index, change in enumerate(("revision", "source")):
            with self.subTest(change=change):
                options = self.options(store)
                options["brain"]["task_id"] = "TASK-FRESH-" + str(index)
                sid = store.create(options)["id"]
                waiting = self.wait_status(store, sid, "awaiting_context")
                task = waiting["brain"]["task"]
                count = len(self.calls)
                if change == "revision":
                    response = knowledge.run(next(iter(store.projects)), {"action": "brain-update",
                        "record_id": task["id"], "revision": task["revision"], "progress": "Independent task update"})
                    self.assertTrue(response["ok"], response)
                else:
                    source = self.project / "specs/authority.md"
                    previous = source.stat()
                    source.write_text(source.read_text().replace("one owner", "two owner"))
                    os.utime(source, ns=(previous.st_atime_ns, previous.st_mtime_ns))
                    self.assertEqual(source.stat().st_size, previous.st_size)
                    self.assertEqual(source.stat().st_mtime_ns, previous.st_mtime_ns)
                store.run_context(sid, waiting["brain"]["context_id"])
                invalidated = self.wait_status(store, sid, "awaiting_context")
                self.assertFalse(invalidated["brain"]["context_id"])
                self.assertFalse(invalidated["brain"]["approved"])
                self.assertEqual(len(self.calls), count)
                store.prepare_context(sid, "cobalt allocation")
                refreshed = self.wait_status(store, sid, "awaiting_context")
                self.assertNotEqual(refreshed["brain"]["context_id"], waiting["brain"]["context_id"])
                store.run_context(sid, refreshed["brain"]["context_id"])
                self.wait_status(store, sid, "completed")
                self.assertEqual(len(self.calls), count + 1)

    def test_worktree_task_retrieval_and_explicit_brain_result_stay_in_actual_execution_root(self):
        self.git("symbolic-ref", "HEAD", "refs/heads/main")
        self.git("add", ".")
        self.git("commit", "--quiet", "-m", "Commit context fixture")
        source = self.project / "specs/authority.md"
        source.write_text("# Dirty primary\n\nPRIVATE UNCOMMITTED CONTEXT\n")
        primary_state = self.git("status", "--porcelain")
        store = self.manager()
        sid = store.create(self.options(store, workspace="worktree"))["id"]
        waiting = self.wait_status(store, sid, "awaiting_context")
        worktree = Path(waiting["project_path"])
        task_id = waiting["brain"]["task"]["id"]
        self.assertTrue((worktree / "memory-bank/local/context.db").is_file())
        self.assertFalse((self.project / "memory-bank/local/context.db").exists())
        self.assertTrue((worktree / "project-brain/dynamic/tasks" / f"{task_id}.md").is_file())
        self.assertFalse((self.project / "project-brain/dynamic/tasks" / f"{task_id}.md").exists())
        self.assertNotIn("PRIVATE UNCOMMITTED CONTEXT", json.dumps(waiting["brain"]["capsule"]))
        store.run_context(sid, waiting["brain"]["context_id"])
        self.wait_status(store, sid, "completed")
        self.assertEqual(self.calls[0]["project"], str(worktree))
        self.assertEqual(json.loads(self.calls[0]["receipt"].read_text())["cwd"], str(worktree))
        self.assertIn("one owner", self.calls[0]["prompt"])
        created = store.brain_action(sid, {"action": "brain-create", "record_type": "finding",
                                           "external_id": "F-WORKTREE", "title": "Reviewed worktree result",
                                           "sources": ["specs/authority.md"]})
        self.assertTrue(created["ok"], created)
        relative = Path("project-brain/dynamic/findings") / f"{created['result']['id']}.md"
        self.assertTrue((worktree / relative).is_file())
        self.assertFalse((self.project / relative).exists())
        self.assertEqual(self.git("status", "--porcelain"), primary_state)

    def test_client_cannot_supply_a_capsule_or_approve_another_sessions_context(self):
        store = self.manager()
        valid = self.options(store)
        for change in ({"capsule": {"semantic": "forged"}}, {"context_id": "forged"}, {"approved": True},
                       {"create": "yes"}, {"task_id": "../escape"}, {"record_id": "invalid"}):
            with self.subTest(change=change), self.assertRaises(sessions.SessionError):
                store.create({**valid, "brain": {**valid["brain"], **change}})
        self.assertEqual(store.list(), [])
        self.assertEqual(self.calls, [])

        first = store.create(valid)["id"]
        one = self.wait_status(store, first, "awaiting_context")
        options = self.options(store)
        options["brain"]["task_id"] = "TASK-OTHER"
        second = store.create(options)["id"]
        two = self.wait_status(store, second, "awaiting_context")
        with self.assertRaises(sessions.SessionError):
            store.run_context(first, two["brain"]["context_id"])
        with self.assertRaises(sessions.SessionError):
            store.brain_action(first, {"action": "complete", "revision": one["brain"]["task"]["revision"],
                                       "outcome": "Premature completion"})
        self.assertEqual(store.get(first), one)
        self.assertEqual(store.get(second), two)
        self.assertEqual(self.calls, [])

    def test_cancelled_prepare_preserves_created_identity_and_queued_retry_query(self):
        store = self.manager()
        original_call, original_retrieve = TaskContext._call, TaskContext._retrieve
        for index, old_error in enumerate((None, sessions.SessionError, RuntimeError)):
            with self.subTest(old_error=old_error):
                started, release = threading.Event(), threading.Event()
                created, retrievals = [], []

                def hold_created_task(context, session, options, workspace, action, **fields):
                    result = original_call(context, session, options, workspace, action, **fields)
                    if action == 'start':
                        created.append(result['task_uuid'])
                        started.set()
                        if not release.wait(5):
                            raise RuntimeError('The fixture did not release preparation.')
                    return result

                def retrieve(context, *arguments):
                    retrievals.append(arguments[1]['query'])
                    if old_error is not None and len(retrievals) == 1:
                        raise old_error('Cancelled preparation fixture failure.')
                    return original_retrieve(context, *arguments)

                options = self.options(store)
                options['brain']['task_id'] = 'TASK-CANCEL-' + str(index)
                with patch.object(TaskContext, '_call', hold_created_task), patch.object(TaskContext, '_retrieve', retrieve):
                    sid = store.create(options)['id']
                    try:
                        self.assertTrue(started.wait(5), store.events(sid))
                        self.assertEqual(store.cancel(sid)['status'], 'cancelled')
                        queued = store.prepare_context(sid, 'cobalt allocation updated query')
                        self.assertEqual(queued['status'], 'queued')
                    finally:
                        release.set()
                    waiting = self.wait_status(store, sid, 'awaiting_context')
                self.assertEqual(len(created), 1)
                self.assertEqual(waiting['brain']['record_id'], created[0])
                self.assertFalse(waiting['brain']['create'])
                self.assertEqual(waiting['brain']['query'], 'cobalt allocation updated query')
                self.assertEqual(retrievals[-1], 'cobalt allocation updated query')
                self.assertFalse(waiting['brain']['approved'])
                self.assertNotIn('Cancelled preparation fixture failure.', json.dumps(store.events(sid)))
                self.assertEqual([event for event in store.events(sid) if event['kind'] == 'error'], [])
                self.assertEqual(self.calls, [])

    def test_cancelled_freshness_check_cannot_consume_queued_retry_context(self):
        store = self.manager()
        sid = store.create(self.options(store))['id']
        prepared = self.wait_status(store, sid, 'awaiting_context')
        checked, release = threading.Event(), threading.Event()
        original = TaskContext.ensure_fresh

        def hold_freshness(context, session):
            result = original(context, session)
            checked.set()
            if not release.wait(5):
                raise RuntimeError('The fixture did not release its freshness check.')
            return result

        with patch.object(TaskContext, 'ensure_fresh', hold_freshness):
            store.run_context(sid, prepared['brain']['context_id'])
            try:
                self.assertTrue(checked.wait(5), store.events(sid))
                self.assertEqual(store.cancel(sid)['status'], 'cancelled')
                store.prepare_context(sid, 'cobalt allocation revised after cancellation')
            finally:
                release.set()
            waiting = self.wait_status(store, sid, 'awaiting_context')
        self.assertEqual(waiting['brain']['query'], 'cobalt allocation revised after cancellation')
        self.assertEqual(waiting['brain']['record_id'], prepared['brain']['record_id'])
        self.assertFalse(waiting['brain']['approved'])
        self.assertNotEqual(waiting['brain']['context_id'], prepared['brain']['context_id'])
        self.assertEqual(self.calls, [])

    def test_explicit_rebind_recovers_committed_start_without_a_saved_uuid_after_restart(self):
        store = self.manager()
        original = TaskContext._call
        committed = []

        def lose_start_reply(context, session, options, workspace, action, **fields):
            result = original(context, session, options, workspace, action, **fields)
            if action == 'start':
                committed.append(result['task_uuid'])
                raise sessions.SessionError('Native start reply was interrupted; refresh before retry.')
            return result

        with patch.object(TaskContext, '_call', lose_start_reply):
            sid = store.create(self.options(store))['id']
            waiting = self.wait_status(store, sid, 'awaiting_context')
        self.assertEqual(len(committed), 1)
        self.assertTrue(waiting['brain']['create'])
        self.assertNotIn('record_id', waiting['brain'])
        record = self.project / 'project-brain/dynamic/tasks' / (committed[0] + '.md')
        original_record = record.read_bytes()
        store.close()
        self.managers.remove(store)
        restarted = self.manager()
        restarted.prepare_context(sid, 'cobalt allocation recovered query')
        not_adopted = self.wait_status(restarted, sid, 'awaiting_context')
        self.assertTrue(not_adopted['brain']['create'])
        self.assertFalse(not_adopted['brain'].get('context_id'))
        self.assertEqual(self.calls, [])
        rebound = restarted.brain_action(sid, {'action': 'rebind', 'record_id': committed[0]})
        self.assertTrue(rebound['ok'], rebound)
        recovered = restarted.get(sid)['brain']
        self.assertEqual(recovered['record_id'], committed[0])
        self.assertFalse(recovered['create'])
        self.assertFalse(recovered['approved'])
        self.assertIsNone(recovered['context_id'])
        self.assertIsNone(recovered['capsule'])
        self.assertEqual(recovered['query'], 'cobalt allocation recovered query')
        self.assertEqual(record.read_bytes(), original_record)
        with self.assertRaises(sessions.SessionError):
            restarted.brain_action(sid, {'action': 'rebind', 'record_id': '00000000-0000-4000-8000-000000000000'})
        restarted.prepare_context(sid, recovered['query'])
        prepared = self.wait_status(restarted, sid, 'awaiting_context')
        self.assertEqual(prepared['brain']['capsule']['task_uuid'], committed[0])
        self.assertEqual(len(list(record.parent.glob('*.md'))), 1)
        self.assertEqual(self.calls, [])



if __name__ == "__main__":
    unittest.main()
