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
from harness import sessions
from harness.knowledge import KnowledgeManager
from harness.task_context import TaskContext
from tests.test_harness_knowledge import install_knowledge_fixture, metadata


FAKE_NATIVE = r'''
import json, os, pathlib, sys
receipt = pathlib.Path(sys.argv[1])
receipt.write_text(json.dumps({"cwd": os.getcwd(), "prompt": sys.stdin.read(),
                              "task_id": os.environ.get("CONTEXT_TASK_ID")}))
print(json.dumps({"type": "thread.started", "thread_id": "native-context-fixture"}), flush=True)
print(json.dumps({"type": "item.completed", "item": {"type": "agent_message", "text": "RAW ASSISTANT FIXTURE OUTPUT"}}), flush=True)
print(json.dumps({"type": "turn.completed"}), flush=True)
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
                          "create": True, "goal": "Verify the cobalt allocation rule"}, **changes}

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
