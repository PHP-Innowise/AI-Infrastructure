"""Offline session-to-Brain context binding with the copied project runtime."""

import copy
import json
import os
from pathlib import Path
import subprocess
import sqlite3
import sys
import tempfile
import threading
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import context_usage, sessions
from harness.knowledge import KnowledgeManager
from harness.task_context import TaskContext
from memory_diagnosis import unavailable_funnel_diagnosis
from tests.test_harness_knowledge import install_knowledge_fixture, metadata, source_link_fixture


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


def diagnosis_example(count=1):
    value = unavailable_funnel_diagnosis()
    value['observation_state'] = 'observed'
    value['loss_observations']['budget'] = {'count': count, 'state': 'exact'}
    value['next_action'] = 'inspect-observed-loss' if count else 'unknown'
    return value


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

    def test_a_capsule_with_a_changelog_and_an_event_is_valid(self):
        task = {'external_id': 'TASK-H', 'id': 'uuid-h', 'revision': 2}
        capsule = {'task_id': 'TASK-H', 'task_uuid': 'uuid-h', 'task_revision': 2,
                   'working': {'task_id': 'TASK-H'}, 'procedural': [], 'semantic': [],
                   'episodic': [{'path': 'project-brain/dynamic/events/x.md'}, {'path': 'CHANGELOG.md'}]}
        TaskContext._validate_capsule(capsule, task)
        capsule['episodic'].append({'path': 'docs/third.md'})
        with self.assertRaises(sessions.SessionError):
            TaskContext._validate_capsule(capsule, task)

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

    def cost_receipt(self, store, sid):
        with store.lock:
            row = store.db.execute('SELECT receipt FROM launches WHERE session_id=? ORDER BY rowid DESC LIMIT 1', (sid,)).fetchone()
        return json.loads(row['receipt'])

    def test_full_cost_final_receipt_covers_prompt_runtime_save_and_raw_tool_body(self):
        script = FAKE_DRAFT.replace('{"type": "turn.completed"}',
            '{"type": "turn.completed", "usage": {"input_tokens": 123, "output_tokens": 17, "cached_input_tokens": 21}}')
        script = script.replace('draft = {', 'print(json.dumps({"type": "item.completed", "item": {"id": "countable1", "type": "command_execution", "status": "completed", "exit_code": 0, "aggregated_output": "é\\n🐍"}}), flush=True)\ndraft = {')
        self.fake.write_text(script)
        store = self.manager(); sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, 'completed')
        receipt = self.cost_receipt(store, sid); observed = receipt['full_cost']
        self.assertIn('memory', receipt)
        self.assertEqual(('final', 'completed'), (observed['state'], observed['outcome']))
        self.assertEqual(140, observed['provider']['provider_tokens'])
        self.assertEqual(7, observed['provider']['executions'][0]['tool_component_bytes'])
        self.assertNotIn('é\n🐍', json.dumps(observed, ensure_ascii=False))
        prompt = json.loads(self.calls[-1]['receipt'].read_text())['prompt']
        self.assertEqual(len(prompt), observed['constructed_payloads']['main_prompt']['characters'])
        self.assertEqual(len(prompt.encode()), observed['constructed_payloads']['main_prompt']['bytes'])
        self.assertLess(observed['constructed_payloads']['capsule_body']['bytes'], observed['constructed_payloads']['capsule_envelope']['bytes'])
        self.assertGreater(observed['stage_seconds']['recall'], 0)
        self.assertGreater(observed['stage_seconds']['save'], 0)
        self.assertGreaterEqual(observed['lifecycle_seconds'], sum(observed['stage_seconds'].values()))
        writes = [item for item in observed['local_operations'] if item['operation'] == 'record-result']
        self.assertEqual(1, len(writes))
        self.assertEqual({'created': 1}, writes[0]['record_states'])
        self.assertGreater(writes[0]['runtime_io'][0]['request_file_bytes'], 0)
        self.assertGreater(writes[0]['runtime_io'][0]['returned_stdout_bytes'], 0)
        self.assertTrue(writes[0]['runtime_io'][0]['stdout_eof'])
        self.assertEqual(1, len(self.findings(store, sid)))
        self.assertEqual('status', store.events(sid)[-1]['kind'])
        self.assertEqual('completed', store.events(sid)[-1]['outcome'])

    def test_full_cost_recovery_observes_one_terminal_per_actual_fake_process(self):
        script = FAKE_DRAFT.replace('text = "Checked the rule.', 'recovering = bool(os.environ.get("CONTEXT_MEMORY_RECOVERY"))\ntext = "Checked the rule.')
        script = script.replace('print(json.dumps({"type": "thread.started"', 'if not recovering: text = "Checked without a draft."\nprint(json.dumps({"type": "thread.started"')
        script = script.replace('{"type": "turn.completed"}', '{"type": "turn.completed", "usage": {"input_tokens": 3 if recovering else 10, "output_tokens": 1 if recovering else 2, "cached_input_tokens": 0}}')
        self.fake.write_text(script)
        store = self.manager(); sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, 'completed')
        observed = self.cost_receipt(store, sid)['full_cost']
        self.assertEqual(2, len(self.calls))
        self.assertEqual(16, observed['provider']['provider_tokens'])
        self.assertEqual([1, 1], [row['terminal_count'] for row in observed['provider']['executions']])
        self.assertEqual(['main', 'recovery'], [row['phase'] for row in observed['provider']['executions']])
        self.assertTrue(observed['processes']['recovery']['started'])
        self.assertGreater(observed['constructed_payloads']['recovery_prompt']['bytes'], 0)
        self.assertEqual(1, len(self.findings(store, sid)))
        self.assertEqual(16, store.get(sid)['budget_usage']['tokens'])

    def test_full_cost_factory_failure_preserves_automatic_memory_work(self):
        from harness import cost_observation
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager()
        with patch.object(cost_observation, 'LaunchObservation', side_effect=RuntimeError('PRIVATE_FACTORY_SENTINEL')):
            sid = store.create(self.automatic(store))['id']
            self.wait_status(store, sid, 'completed')
        observed = self.cost_receipt(store, sid)['full_cost']
        self.assertEqual('unavailable', observed['state'])
        self.assertEqual(1, len(self.findings(store, sid)))
        self.assertNotIn('PRIVATE_FACTORY_SENTINEL', json.dumps(store.events(sid)))

    def test_full_cost_raw_callback_failure_preserves_work_and_marks_unknown(self):
        from harness import cost_observation
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager()
        with patch.object(cost_observation.LaunchObservation, 'raw', side_effect=RuntimeError('PRIVATE_RAW_SENTINEL')):
            sid = store.create(self.automatic(store))['id']
            self.wait_status(store, sid, 'completed')
        observed = self.cost_receipt(store, sid)['full_cost']
        self.assertTrue(observed['observer_incomplete'])
        self.assertIsNone(observed['provider']['provider_tokens'])
        self.assertEqual(1, len(self.findings(store, sid)))
        self.assertNotIn('PRIVATE_RAW_SENTINEL', json.dumps(observed))

    def test_full_cost_review_preparation_has_nonlaunch_receipt_without_provider(self):
        store = self.manager(); sid = store.create(self.options(store))['id']
        self.wait_status(store, sid, 'awaiting_context')
        self.assertEqual([], self.calls)
        with store.lock:
            self.assertEqual(0, store.db.execute('SELECT count(*) FROM launches WHERE session_id=?', (sid,)).fetchone()[0])
        records = [event for event in store.events(sid) if event['kind'] == 'cost_observation']
        self.assertEqual(1, len(records))
        self.assertEqual('awaiting_context', records[0]['full_cost']['outcome'])
        self.assertEqual([], records[0]['full_cost']['provider']['executions'])

    def test_full_cost_lock_retry_is_counted_with_one_returned_runtime_call(self):
        from harness import cost_observation
        from harness.knowledge import KnowledgeBusy
        store = self.manager()
        manager = self.knowledge(store)
        observed = cost_observation.LaunchObservation('codex')
        context = TaskContext(manager, wait=1, observation=observed)
        original = manager.run
        attempts = []
        def busy_once(*args, **kwargs):
            attempts.append(True)
            if len(attempts) == 1:
                raise KnowledgeBusy()
            return original(*args, **kwargs)
        with patch.object(manager, 'run', busy_once):
            response = context._knowledge('run', next(iter(store.projects)), {'action': 'index'})
        self.assertTrue(response['ok'])
        final = observed.finish('completed')
        self.assertEqual(2, len(attempts))
        self.assertEqual(1, len(final['local_operations']))
        operation = final['local_operations'][0]
        self.assertEqual(1, operation['lock_retries'])
        self.assertEqual(1, len(operation['runtime_io']))
        self.assertTrue(operation['runtime_io'][0]['returned'])
        self.assertNotIn('stale_revision_retries', final['notes'])

    def test_full_cost_cursor_checkpoint_is_timed_and_runtime_observed(self):
        self.fake.write_text(FAKE_CURSOR)
        store = self.manager()
        store.providers['cursor'] = {**store.providers['codex'], 'id': 'cursor'}
        sid = store.create(self.automatic(store, provider='cursor'))['id']
        self.wait_status(store, sid, 'completed')
        observed = self.cost_receipt(store, sid)['full_cost']
        self.assertIsNone(observed['provider']['provider_tokens'])
        self.assertGreater(observed['stage_seconds']['checkpoint'], 0)
        checkpoints = [row for row in observed['local_operations'] if row['operation'] == 'checkpoint']
        self.assertEqual(1, len(checkpoints))
        self.assertEqual('returned', checkpoints[0]['outcome'])
        self.assertTrue(checkpoints[0]['runtime_io'][0]['returned'])
        self.assertTrue((self.project / 'memory-bank/local/last-turn-report.json').exists())

    def test_full_cost_stopping_during_history_matches_terminal_status(self):
        from harness import memory_use
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager()
        def stop_after_memory(*args):
            store.stopping.set()
        with patch.object(memory_use, 'fold_project', stop_after_memory):
            sid = store.create(self.automatic(store))['id']
            self.wait_status(store, sid, 'interrupted')
        observed = self.cost_receipt(store, sid)['full_cost']
        self.assertEqual('interrupted', observed['outcome'])
        self.assertGreater(observed['stage_seconds']['history'], 0)
        self.assertEqual('interrupted', store.events(sid)[-1]['outcome'])
        findings = list((self.project / 'project-brain/dynamic/findings').glob('*.md'))
        self.assertEqual(1, len(findings))
        self.assertEqual(self.LEARNING['title'], metadata(findings[0])['title'])

    def test_full_cost_cancel_resend_keeps_old_receipt_and_blocks_old_memory_write(self):
        script = FAKE_DRAFT.replace('text = "Checked the rule.',
            'new = "NEW_BOUNDARY" in json.loads(receipt.read_text())["prompt"]\n'
            'draft["progress"] = "NEW progress" if new else "OLD progress"\n'
            'draft["learnings"][0]["title"] = "NEW claim" if new else "OLD claim"\n'
            'text = "Checked the rule.')
        script = script.replace('{"type": "turn.completed"}',
            '{"type": "turn.completed", "usage": {"input_tokens": 28 if new else 12, "output_tokens": 2, "cached_input_tokens": 0}}')
        self.fake.write_text(script)
        store = self.manager()
        entered, release = threading.Event(), threading.Event()
        original = TaskContext.save_memory
        calls = []
        def hold_first_save(context, *args, **kwargs):
            calls.append(kwargs.get('generation'))
            if len(calls) == 1:
                entered.set()
                if not release.wait(5):
                    raise RuntimeError('Save barrier not released')
            return original(context, *args, **kwargs)
        with patch.object(TaskContext, 'save_memory', hold_first_save):
            sid = store.create(self.automatic(store))['id']
            try:
                self.assertTrue(entered.wait(5))
                with store.lock:
                    old_generation = store.generations[sid]
                    self.assertNotIn(sid, store.active)
                    before = store.db.execute('SELECT receipt FROM launches WHERE id=?', (old_generation,)).fetchone()
                    self.assertNotIn('full_cost', json.loads(before['receipt']))
                self.assertEqual('cancelled', store.cancel(sid)['status'])
                queued = store.send(sid, 'NEW_BOUNDARY Check the cobalt release path')
                self.assertEqual('queued', queued['status'])
                with store.lock:
                    new_generation = store.generations[sid]
                    self.assertNotIn(sid, store.cancelled)
                self.assertNotEqual(old_generation, new_generation)
            finally:
                release.set()
            completed = self.wait_status(store, sid, 'completed')
        with store.lock:
            rows = {row['id']: row for row in store.db.execute('SELECT id,status,receipt FROM launches WHERE session_id=?', (sid,))}
        self.assertEqual({old_generation, new_generation}, set(rows))
        old_receipt = json.loads(rows[old_generation]['receipt'])
        old = old_receipt['full_cost']
        new = json.loads(rows[new_generation]['receipt'])['full_cost']
        self.assertIn('memory', old_receipt)
        self.assertEqual(json.loads(before['receipt']), {key: value for key, value in old_receipt.items() if key != 'full_cost'})
        self.assertEqual('superseded', old['outcome'])
        self.assertEqual('interrupted', rows[old_generation]['status'])
        self.assertEqual(14, old['provider']['provider_tokens'])
        self.assertEqual('completed', new['outcome'])
        self.assertEqual(30, new['provider']['provider_tokens'])
        self.assertEqual(30, completed['budget_usage']['tokens'])
        self.assertEqual('NEW_BOUNDARY Check the cobalt release path', completed['brain']['query'])
        self.assertEqual('completed', rows[new_generation]['status'])
        self.assertEqual(2, len(self.calls))
        self.assertEqual('NEW progress', store.brain_info(sid)['task']['progress'])
        self.assertEqual(['NEW claim'], [item['title'] for item in store.brain_info(sid)['records'] if item['type'] == 'finding'])
        old_writes = [row for row in old['local_operations'] if row['operation'] == 'record-result']
        self.assertEqual(1, len(old_writes))
        self.assertFalse(old_writes[0]['runtime_io'][0]['started'])
        self.assertGreater(old['stage_seconds']['save'], 0)

    def test_one_automatic_draft_preserves_distinct_source_identities(self):
        store = self.manager(); sid = self.linked_run(store)
        (self.project / "specs/other.md").write_text("# Other cobalt authority\n")
        learnings = [self.LEARNING, {**self.LEARNING, "sources": ["specs/other.md"]},
                     {**self.LEARNING, "sources": ["specs/authority.md#L1"]}]
        result = store._task_context().save_memory(store.get(sid), {"learnings": learnings}, automatic=True)
        self.assertTrue(result["ok"], result)
        self.assertEqual([], result["skipped"])
        self.assertEqual(3, len(result["saved"]["records"]))
        self.assertEqual(3, len({record["id"] for record in result["saved"]["records"]}))
        self.assertEqual(3, len(self.findings(store, sid)))

    def test_all_conflicting_automatic_learnings_write_and_promote_nothing(self):
        from harness import memory_draft
        store = self.manager(); sid = self.linked_run(store)
        context = store._task_context()
        before_task = store.brain_info(sid)["task"]
        before_chunks = sorted((self.project / "memory-bank/chunks").glob("*.md"))
        draft = {"learnings": [self.LEARNING, self.LEARNING,
                               {**self.LEARNING, "consequence": "Every cobalt allocation names two owners."}]}
        for save in (context.save_memory, context._save_memory_chain):
            with self.subTest(save=save.__name__):
                with patch.object(context, "_call", wraps=context._call) as writer:
                    result = save(store.get(sid), draft, automatic=True)
                writer.assert_not_called()
                self.assertTrue(result["ok"], result)
                self.assertEqual({"task": None, "records": [], "promotion": None}, result["saved"])
                self.assertEqual(["conflict"] * 3, [item["reason"] for item in result["skipped"]])
        self.assertEqual(before_task["revision"], store.brain_info(sid)["task"]["revision"])
        self.assertEqual([], self.findings(store, sid))
        self.assertEqual(before_chunks, sorted((self.project / "memory-bank/chunks").glob("*.md")))
        self.assertIn("Left out 3 conflicting learning(s)", memory_draft.summary("drafted", result))

    def test_completed_automatic_run_saves_progress_and_independent_claim_only(self):
        independent = {**self.LEARNING, "title": "Amber boundary is explicit",
                       "consequence": "Amber stays within the checked boundary."}
        draft = {"progress": "Conflicting cobalt claims need verification.", "next_steps": ["Check cobalt authority"],
                 "learnings": [self.LEARNING, {**self.LEARNING, "consequence": "Cobalt has two owners."}, independent]}
        self.fake.write_text(FAKE_DRAFT.replace('text = "Checked the rule.',
            'draft = json.loads(' + repr(json.dumps(draft)) + ')\ntext = "Checked the rule.'))
        store = self.manager(); sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, "completed")
        info = store.brain_info(sid)
        self.assertEqual(draft["progress"], info["task"]["progress"])
        self.assertEqual(draft["next_steps"], info["task"]["next_steps"])
        self.assertEqual([], self.findings(store, sid))
        findings = [record for record in info["records"] if record["type"] == "finding"]
        self.assertEqual([independent["title"]], [record["title"] for record in findings])
        self.assertEqual(independent["consequence"], findings[0]["progress"])
        notice = self.memory_events(store, sid)[-1]["text"]
        self.assertIn("Left out 2 conflicting learning(s)", notice)
        self.assertNotIn(self.LEARNING["consequence"], notice)
        self.assertNotIn("Cobalt has two owners.", notice)
        chunks = list((self.project / "memory-bank/chunks").glob("*.md"))
        self.assertEqual(1, len(chunks))
        self.assertIn(independent["consequence"], chunks[0].read_text())
        self.assertNotIn("Cobalt has two owners.", chunks[0].read_text())

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

    def test_automatic_followup_corrects_a_remembered_learning_and_repeat_is_a_noop(self):
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager()
        sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, "completed")
        initial = self.findings(store, sid)[0]
        brain = store.get(sid)["brain"]
        store._save_brain(sid, {**brain, "remembered": [["finding", "cobalt allocation needs one owner"]]})
        consequence = "Every cobalt allocation names two accountable owners."
        (self.project / "specs/authority.md").write_text("# Cobalt authority\n\n" + consequence + "\n")
        self.fake.write_text(FAKE_DRAFT.replace(self.LEARNING["consequence"], consequence))
        store.send(sid, "Check the updated cobalt authority")
        self.wait_status(store, sid, "completed")
        findings = self.findings(store, sid)
        self.assertEqual(1, len(findings))
        corrected = findings[0]
        self.assertEqual(initial["id"], corrected["id"])
        self.assertEqual(consequence, corrected["progress"])
        self.assertGreater(corrected["revision"], initial["revision"])
        chunks = list((self.project / "memory-bank/chunks").glob("*.md"))
        active = [path for path in chunks if metadata(path)["status"] == "active"]
        self.assertEqual(1, len(active))
        self.assertIn(consequence, active[0].read_text())
        self.assertTrue(any(metadata(path)["status"] == "superseded" for path in chunks))
        task_revision = store.brain_info(sid)["task"]["revision"]
        store.send(sid, "Recheck the unchanged cobalt authority")
        self.wait_status(store, sid, "completed")
        self.assertEqual(corrected["revision"], self.findings(store, sid)[0]["revision"])
        self.assertEqual(task_revision, store.brain_info(sid)["task"]["revision"])
        self.assertEqual(len(chunks), len(list((self.project / "memory-bank/chunks").glob("*.md"))))
        self.assertNotIn("already saved from this session", self.memory_events(store, sid)[-1]["text"])
        self.assertIn(consequence, self.calls[-1]["prompt"])
        self.assertNotIn(self.LEARNING["consequence"], self.calls[-1]["prompt"])

    def test_automatic_followup_same_title_with_new_sources_reaches_the_writer(self):
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager()
        sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, "completed")
        initial = self.findings(store, sid)[0]
        (self.project / "specs/new-authority.md").write_text("# Cobalt authority\n\n" + self.LEARNING["consequence"] + "\n")
        self.fake.write_text(FAKE_DRAFT.replace("specs/authority.md", "specs/new-authority.md"))
        store.send(sid, "Check cobalt authority in specs/new-authority.md")
        self.wait_status(store, sid, "completed")
        findings = self.findings(store, sid)
        self.assertEqual(2, len(findings))
        self.assertEqual(2, len({record["id"] for record in findings}))
        self.assertEqual({("specs/authority.md",), ("specs/new-authority.md",)},
                         {tuple(record["sources"]) for record in findings})
        self.assertIn(initial["id"], {record["id"] for record in findings})

    def test_automatic_repeat_finishes_promotion_after_it_was_deferred(self):
        configuration = self.project / "project-brain/config/runtime.json"
        config = json.loads(configuration.read_text())
        config["automatic_promotion"] = False
        configuration.write_text(json.dumps(config))
        self.fake.write_text(FAKE_DRAFT)
        store = self.manager()
        sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, "completed")
        finding = self.findings(store, sid)[0]
        self.assertEqual([], list((self.project / "memory-bank/chunks").glob("*.md")))
        config["automatic_promotion"] = True
        configuration.write_text(json.dumps(config))
        store.send(sid, "Recheck cobalt allocation after promotion resumes")
        self.wait_status(store, sid, "completed")
        self.assertEqual(finding["revision"], self.findings(store, sid)[0]["revision"])
        chunks = list((self.project / "memory-bank/chunks").glob("*.md"))
        self.assertEqual(1, len(chunks))
        self.assertIn(self.LEARNING["consequence"], chunks[0].read_text())
        self.assertIn("Promoted to the Memory Bank", self.memory_events(store, sid)[-1]["text"])

    def test_automatic_correction_preserves_a_person_attested_claim(self):
        store = self.manager()
        sid = store.create(self.automatic(store))["id"]
        self.wait_status(store, sid, "completed")
        saved = store.save_memory(sid, {"learnings": [self.LEARNING], "verified": True})
        self.assertTrue(saved["ok"], saved)
        finding = self.findings(store, sid)[0]
        self.fake.write_text(FAKE_DRAFT.replace(self.LEARNING["consequence"], "Every cobalt allocation needs two owners."))
        store.send(sid, "Check a revised cobalt allocation claim")
        self.wait_status(store, sid, "completed")
        current = self.findings(store, sid)[0]
        self.assertEqual((finding["id"], finding["revision"], finding["progress"]),
                         (current["id"], current["revision"], current["progress"]))
        notice = self.memory_events(store, sid)[-1]
        self.assertTrue(notice["ok"], notice)
        self.assertIn("different consequence than this run", notice["text"])

    def test_legacy_chain_keeps_corrections_without_claiming_idempotence(self):
        from harness import knowledge
        store = self.manager()
        sid = self.linked_run(store)
        context = store._task_context()
        original = context._call
        def older_runtime(session, options, workspace, action, **fields):
            if action == "record-result":
                raise sessions.SessionError(knowledge.OLDER_RUNTIME)
            return original(session, options, workspace, action, **fields)
        changed = {**self.LEARNING, "consequence": "Every cobalt allocation names two accountable owners."}
        with patch.object(context, "_call", side_effect=older_runtime):
            first = context.save_memory(store.get(sid), {"learnings": [self.LEARNING]}, automatic=True)
            second = context.save_memory(store.get(sid), {"learnings": [changed]}, automatic=True,
                                         known=[["finding", "cobalt allocation needs one owner"]])
        self.assertTrue(first["ok"] and second["ok"], (first, second))
        findings = self.findings(store, sid)
        self.assertEqual(2, len(findings))
        self.assertEqual({self.LEARNING["consequence"], changed["consequence"]},
                         {record["progress"] for record in findings})

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

    def test_automatic_launch_receives_source_linked_answers_without_settings(self):
        query, paths = source_link_fixture(self.project)
        store = self.manager()
        sid = store.create(self.automatic(store, prompt=query))['id']
        session = self.wait_status(store, sid, 'completed')
        self.assertFalse(session['brain']['review'])
        received = json.loads(self.calls[0]['receipt'].read_text())
        self.assertIn('Floating point never touches', received['prompt'])
        self.assertIn('A refund can never exceed', received['prompt'])
        selected = {item['path']: item for item in session['brain']['capsule']['semantic']}
        self.assertTrue(paths <= set(selected))
        self.assertTrue(all(selected[path]['selection'] == 'source-link' for path in paths))
        self.assertNotIn('awaiting_context', {event.get('status') for event in store.events(sid)})

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
        # The runtime recognizes the saved learning, rather than a title cache suppressing it.
        again = self.memory_events(store, sid)[-1]
        self.assertTrue(again["ok"], again)
        self.assertIn("Already in project memory: finding “Cobalt allocation needs one owner”.", again["text"])
        repeated = [record for record in store.brain_info(sid)["records"] if record["type"] == "finding"]
        self.assertEqual(1, len(repeated))
        self.assertEqual([(record["id"], record["title"], record["status"], record["authority"]) for record in findings],
                         [(record["id"], record["title"], record["status"], record["authority"]) for record in repeated])

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
        preview = sessions.BRAIN_CONTEXT_HEADER + json.dumps(capsule, ensure_ascii=False) + "\n\n"
        self.assertEqual(len(preview), meter["prompt_characters"])
        inserted = sessions.BRAIN_CONTEXT_HEADER + json.dumps(capsule, ensure_ascii=False, separators=(",", ":")) + "\n\n"
        self.assertIn(inserted, self.calls[0]["prompt"])
        self.assertEqual(capsule, json.JSONDecoder().raw_decode(
            self.calls[0]["prompt"].split(sessions.BRAIN_CONTEXT_HEADER, 1)[1])[0])
        self.assertEqual(len(inserted), waiting["capsule_delivery_meter"]["prompt_characters"])
        ledger = store.results.history(sid)["launches"][-1]["context"]["ledger"]
        self.assertEqual(len(inserted), ledger["capsule"]["inserted"])
        self.assertEqual(len(inserted), sum(ledger["capsule"]["kinds"].values()))
        self.assertEqual(len(self.calls[0]["prompt"]), ledger["total"])

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
        envelope = sessions.BRAIN_CONTEXT_HEADER + json.dumps(brain["capsule"], ensure_ascii=False, separators=(",", ":")) + "\n\n"
        self.assertIn(envelope, received["prompt"])
        self.assertEqual(brain["capsule"], json.JSONDecoder().raw_decode(
            received["prompt"].split(sessions.BRAIN_CONTEXT_HEADER, 1)[1])[0])
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

    def test_funnel_diagnosis_automatic_prepare_is_metadata_and_survives_restart(self):
        expected = diagnosis_example()
        original = KnowledgeManager.run

        def diagnosed(manager, key, data, **options):
            response = original(manager, key, data, **options)
            if data.get('action') == 'refresh' and response.get('ok'):
                response = {**response, 'result': {**response['result'], 'funnel_diagnosis': expected}}
            return response

        store = self.manager()
        with patch.object(KnowledgeManager, 'run', diagnosed):
            sid = store.create(self.automatic(store))['id']
            completed = self.wait_status(store, sid, 'completed')
        brain = completed['brain']
        self.assertEqual(brain['funnel_diagnosis'], expected)
        self.assertNotIn('funnel_diagnosis', brain['capsule'])
        self.assertNotIn('funnel_diagnosis', brain['capsule_text'])
        self.assertNotIn('funnel_diagnosis', brain['source_hashes'])
        ordinary = copy.deepcopy(completed)
        ordinary['brain'].pop('funnel_diagnosis')
        self.assertEqual(store._prompt(completed, 'Check the latch'), store._prompt(ordinary, 'Check the latch'))
        self.assertEqual(TaskContext._material(brain['capsule']), TaskContext._material(ordinary['brain']['capsule']))
        self.assertEqual(sessions.memory_notice(brain), sessions.memory_notice(ordinary['brain']))
        self.assertEqual(completed['capsule_meter'], sessions.capsule_meter(brain['capsule'], brain['capsule_text']))
        received = json.loads(self.calls[0]['receipt'].read_text())
        self.assertNotIn('funnel_diagnosis', received['prompt'])
        returned = store.get(sid)['brain']
        returned['funnel_diagnosis']['loss_observations']['budget']['count'] = 99
        self.assertEqual(store.get(sid)['brain']['funnel_diagnosis'], expected)
        store._save_brain(sid, brain)
        stored = json.loads(store.db.execute('SELECT brain FROM sessions WHERE id=?', (sid,)).fetchone()[0])
        self.assertEqual(stored['funnel_diagnosis'], expected)
        store.close()
        self.managers.remove(store)
        restarted = self.manager()
        self.assertEqual(restarted.get(sid)['brain'], brain)

    def test_funnel_diagnosis_get_rejects_only_child_without_rewriting_record(self):
        store = self.manager()
        sid = store.create(self.options(store))['id']
        waiting = self.wait_status(store, sid, 'awaiting_context')
        brain = waiting['brain']
        forged = {**brain, 'funnel_diagnosis': {'private_body': 'invented rejected metadata'}}
        encoded = json.dumps(forged)
        with store.lock:
            store.db.execute('UPDATE sessions SET brain=? WHERE id=?', (encoded, sid))
            store.db.commit()
        self.assertEqual(store.get(sid)['brain'], brain)
        self.assertEqual(store.db.execute('SELECT brain FROM sessions WHERE id=?', (sid,)).fetchone()[0], encoded)
        store._save_brain(sid, {**brain, 'funnel_diagnosis': diagnosis_example()})
        with patch.object(sessions, 'normalize_funnel_diagnosis', side_effect=RuntimeError('metadata-only fixture failure')):
            self.assertEqual(store.get(sid)['brain'], brain)
            store._save_brain(sid, {**brain, 'funnel_diagnosis': diagnosis_example()})
        self.assertEqual(store.get(sid)['brain'], brain)
        self.assertEqual(self.calls, [])

    def test_funnel_diagnosis_prepare_replaces_previous_child_with_current_metadata_only(self):
        store = self.manager()
        sid = store.create(self.options(store))['id']
        waiting = self.wait_status(store, sid, 'awaiting_context')
        brain = {**waiting['brain'], 'review': False}
        original = KnowledgeManager.run
        for child, failed in ((None, False), ({'schema_version': 2}, False),
                              (unavailable_funnel_diagnosis(), False), (diagnosis_example(), True)):
            with self.subTest(child=child, normalizer_failure=failed):
                store._save_brain(sid, {**brain, 'funnel_diagnosis': diagnosis_example(5)})
                def refresh(manager, key, data, **options):
                    response = original(manager, key, data, **options)
                    if data.get('action') == 'refresh' and response.get('ok'):
                        result = dict(response['result'])
                        result.pop('funnel_diagnosis', None)
                        if child is not None:
                            result['funnel_diagnosis'] = child
                        response = {**response, 'result': result}
                    return response
                context = store._task_context()
                with patch.object(KnowledgeManager, 'run', refresh):
                    if failed:
                        with patch('harness.task_context.normalize_funnel_diagnosis', side_effect=RuntimeError('metadata-only failure')):
                            prepared = context.prepare(store.get(sid))
                    else:
                        prepared = context.prepare(store.get(sid))
                store._save_brain(sid, prepared)
                saved = store.get(sid)['brain']
                self.assertIsInstance(saved['capsule'], dict)
                self.assertEqual(saved['record_id'], brain['record_id'])
                self.assertEqual(saved['source_hashes'], brain['source_hashes'])
                self.assertNotIn('funnel_diagnosis', saved['capsule'])
                if child == unavailable_funnel_diagnosis():
                    self.assertEqual(saved['funnel_diagnosis'], child)
                else:
                    self.assertNotIn('funnel_diagnosis', saved)
        self.assertEqual(self.calls, [])

    def test_funnel_diagnosis_query_send_rebind_and_freshness_invalidate(self):
        store = self.manager()
        sid = store.create(self.options(store))['id']
        waiting = self.wait_status(store, sid, 'awaiting_context')
        brain = waiting['brain']
        child = diagnosis_example()
        store._save_brain(sid, {**brain, 'funnel_diagnosis': child})
        with patch.object(store, '_queue_context'):
            queued = store.prepare_context(sid, 'cobalt latch revised query')
        self.assertNotIn('funnel_diagnosis', queued['brain'])
        self.assertEqual(queued['brain']['query'], 'cobalt latch revised query')
        store._save_brain(sid, {**brain, 'funnel_diagnosis': child})
        rebound = store.brain_action(sid, {'action': 'rebind', 'record_id': brain['record_id']})
        self.assertTrue(rebound['ok'], rebound)
        recovered = store.get(sid)['brain']
        self.assertNotIn('funnel_diagnosis', recovered)
        self.assertEqual(recovered['record_id'], brain['record_id'])
        self.assertEqual(recovered['query'], brain['query'])
        store._save_brain(sid, {**brain, 'funnel_diagnosis': child})
        with patch.object(TaskContext, 'ensure_fresh', side_effect=sessions.SessionError('invented freshness change')):
            store.run_context(sid, brain['context_id'])
            invalidated = self.wait_status(store, sid, 'awaiting_context')
        self.assertNotIn('funnel_diagnosis', invalidated['brain'])
        self.assertIsNone(invalidated['brain']['context_id'])
        self.assertEqual(self.calls, [])
        with patch.object(store.jobs, 'put_nowait'):
            store._status(sid, 'completed')
            store.db.execute('UPDATE sessions SET native_session_id=? WHERE id=?', ('offline-resume-id', sid))
            store.db.commit()
            store._save_brain(sid, {**brain, 'review': False, 'funnel_diagnosis': child})
            sent = store.send(sid, 'cobalt latch next safe query')
        self.assertNotIn('funnel_diagnosis', sent['brain'])
        self.assertEqual(sent['brain']['query'], 'cobalt latch next safe query')
        self.assertIsNone(sent['brain']['context_id'])


    def test_ordinary_receipt_prepare_get_save_restart_and_content_exclusion(self):
        store = self.manager()
        sid = store.create(self.automatic(store))['id']
        completed = self.wait_status(store, sid, 'completed')
        brain = completed['brain']
        expected = {'schema_version': 1, 'observation_state': 'observed', 'plans_present': True, 'returned_status': 'committed'}
        self.assertEqual(expected, brain['ordinary_receipt'])
        ordinary = copy.deepcopy(completed)
        ordinary['brain'].pop('ordinary_receipt')
        self.assertEqual(store._prompt(completed, 'Check the latch'), store._prompt(ordinary, 'Check the latch'))
        self.assertEqual(TaskContext._material(brain['capsule']), TaskContext._material(ordinary['brain']['capsule']))
        self.assertEqual(sessions.memory_notice(brain), sessions.memory_notice(ordinary['brain']))
        self.assertEqual(completed['capsule_meter'], sessions.capsule_meter(brain['capsule'], brain['capsule_text']))
        for value in (brain['capsule'], brain['capsule_text'], brain['source_hashes'], json.loads(self.calls[0]['receipt'].read_text())['prompt']):
            self.assertNotIn('ordinary_receipt', value)
        returned = store.get(sid)['brain']
        returned['ordinary_receipt']['returned_status'] = 'skipped'
        self.assertEqual(expected, store.get(sid)['brain']['ordinary_receipt'])
        store._save_brain(sid, brain)
        store.close()
        self.managers.remove(store)
        restarted = self.manager()
        self.assertEqual(brain, restarted.get(sid)['brain'])


class BrainDiagnosisTests(unittest.TestCase):
    def setUp(self):
        self.store = sessions.Sessions.__new__(sessions.Sessions)
        self.store.lock = threading.RLock()
        self.store.db = sqlite3.connect(':memory:')
        self.addCleanup(self.store.db.close)
        self.store.db.execute('CREATE TABLE sessions (id TEXT PRIMARY KEY, brain TEXT, updated_at TEXT, fleet_action TEXT)')
        self.store.db.execute("INSERT INTO sessions(id,brain) VALUES ('fixture','{}')")
        self.brain = {'bank': 'memory-bank', 'task_id': 'TASK-JUNIPER', 'query': 'Juniper cable latch',
                      'create': False, 'record_id': 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
                      'context_id': 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'capsule': {'working': {}},
                      'capsule_text': 'working: Juniper', 'source_hashes': {'specs/latch.md': 'f' * 64},
                      'approved': False, 'remembered': [{'title': 'Preserved bookkeeping'}]}

    def saved(self):
        return json.loads(self.store.db.execute("SELECT brain FROM sessions WHERE id='fixture'").fetchone()[0])

    def test_funnel_diagnosis_save_rebuilds_and_keeps_unavailable_without_mutating_input(self):
        for child in (diagnosis_example(), unavailable_funnel_diagnosis()):
            with self.subTest(state=child['observation_state']):
                value = {**self.brain, 'funnel_diagnosis': child}
                original = copy.deepcopy(value)
                self.store._save_brain('fixture', value)
                self.assertEqual(value, original)
                self.assertEqual(self.saved(), original)
                child['loss_observations']['budget']['count'] = 77
                self.assertEqual(self.saved(), original)

    def test_funnel_diagnosis_save_invalidates_only_child_at_all_unusable_context_shapes(self):
        for change in ({'context_id': None}, {'context_id': ''}, {'context_id': True},
                       {'context_id': 'unsupported'}, {'capsule': None}, {'capsule': []}):
            with self.subTest(change=change):
                ordinary = {**self.brain, **change}
                value = {**ordinary, 'funnel_diagnosis': diagnosis_example()}
                self.store._save_brain('fixture', value)
                self.assertEqual(self.saved(), ordinary)
                self.assertIn('funnel_diagnosis', value)
        missing = {key: value for key, value in self.brain.items() if key != 'context_id'}
        self.store._save_brain('fixture', {**missing, 'funnel_diagnosis': diagnosis_example()})
        self.assertEqual(self.saved(), missing)

    def test_funnel_diagnosis_save_rejection_and_regular_failure_preserve_ordinary_write(self):
        for child in (None, {'schema_version': 2}, {**diagnosis_example(), 'raw_body': 'invented rejected text'}):
            self.store._save_brain('fixture', {**self.brain, 'funnel_diagnosis': child})
            self.assertEqual(self.saved(), self.brain)
        with patch.object(sessions, 'normalize_funnel_diagnosis', side_effect=RuntimeError('metadata-only fixture failure')):
            self.store._save_brain('fixture', {**self.brain, 'funnel_diagnosis': diagnosis_example()})
        self.assertEqual(self.saved(), self.brain)
        self.store._save_brain('fixture', None)
        self.assertIsNone(self.saved())

    def test_funnel_diagnosis_normalizer_cancellation_propagates_before_store_write(self):
        self.store._save_brain('fixture', self.brain)
        for error in (KeyboardInterrupt(), SystemExit(23)):
            with patch.object(sessions, 'normalize_funnel_diagnosis', side_effect=error):
                with self.assertRaises(type(error)):
                    self.store._save_brain('fixture', {**self.brain, 'funnel_diagnosis': diagnosis_example()})
            self.assertEqual(self.saved(), self.brain)

    def retrieve_context(self):
        knowledge = SimpleNamespace(sessions=self.store, layout=lambda *_: {'attached': None, 'folder': Path('.')})
        context = TaskContext(knowledge)
        task = {'id': self.brain['record_id'], 'external_id': 'TASK-JUNIPER', 'revision': 1, 'status': 'in-progress'}
        capsule = {'task_id': task['external_id'], 'task_uuid': task['id'], 'task_revision': 1,
                   'working': {'task_id': task['external_id']}, 'procedural': [], 'semantic': [], 'episodic': []}
        return context, task, capsule

    def test_funnel_diagnosis_retrieve_keeps_three_tuple_and_best_effort_optional_sink(self):
        context, task, capsule = self.retrieve_context()
        options = {'bank': 'memory-bank', 'task_id': task['external_id'], 'query': 'Juniper cable', 'review': False}
        session = {'project_id': 'project', 'provider': 'codex'}
        for child, retained in ((None, False), ({'schema_version': 2}, False),
                                (diagnosis_example(), True), (unavailable_funnel_diagnosis(), True)):
            with self.subTest(child=child):
                result = {'capsule': capsule, 'capsule_text': 'working: Juniper'}
                if child is not None:
                    result['funnel_diagnosis'] = child
                sink = {}
                with patch.object(context, '_knowledge', return_value={'ok': True, 'result': result}), \
                        patch.object(context, '_source_hashes', return_value={}):
                    returned = context._retrieve(session, options, Path('.'), task, {'root': '.'}, True, sink)
                    legacy = context._retrieve(session, options, Path('.'), task, {'root': '.'}, True)
                self.assertEqual(returned, (capsule, {}, 'working: Juniper'))
                self.assertEqual(legacy, returned)
                self.assertEqual(sink, {'funnel_diagnosis': child} if retained else {})
        with patch.object(context, '_knowledge', return_value={'ok': True, 'result': {'capsule': capsule,
                          'capsule_text': 'working: Juniper', 'funnel_diagnosis': diagnosis_example()}}), \
                patch.object(context, '_source_hashes', return_value={}), \
                patch('harness.task_context.normalize_funnel_diagnosis', side_effect=RuntimeError('metadata-only failure')):
            sink = {}
            self.assertEqual(context._retrieve(session, options, Path('.'), task, {'root': '.'}, True, sink),
                             (capsule, {}, 'working: Juniper'))
            self.assertEqual(sink, {})

    def test_funnel_diagnosis_reviewed_retrieve_and_freshness_do_not_replace_metadata(self):
        context, task, capsule = self.retrieve_context()
        session = {'project_id': 'project', 'provider': 'codex', 'brain': {**self.brain, 'task': task,
                   'capsule': capsule, 'source_hashes': {}, 'funnel_diagnosis': diagnosis_example(3)}}
        options = {'bank': 'memory-bank', 'task_id': task['external_id'], 'query': 'Juniper cable', 'review': True}
        with patch.object(context, '_knowledge', return_value={'ok': True, 'result': capsule}), \
                patch.object(context, '_source_hashes', return_value={}):
            sink = {}
            returned = context._retrieve(session, options, Path('.'), task, {'root': '.'}, False, sink)
        self.assertEqual(returned, (capsule, {}, None))
        self.assertEqual(sink, {})
        original = copy.deepcopy(session['brain'])
        automatic = {**options, 'review': False}
        def response(operation, *args, **kwargs):
            if operation == 'inspect':
                return {'task': task}
            return {'ok': True, 'result': {'capsule': capsule, 'capsule_text': 'working: Juniper',
                    'funnel_diagnosis': diagnosis_example(9)}}
        with patch.object(context, '_bound', return_value=(session['brain'], automatic, Path('.'), {'root': '.'})), \
                patch.object(context, '_knowledge', side_effect=response), patch.object(context, '_source_hashes', return_value={}):
            self.assertTrue(context.ensure_fresh(session))
        self.assertEqual(session['brain'], original)
        self.assertNotIn('funnel_diagnosis', TaskContext._material(capsule))
        from harness.task_context import FIELDS, KEPT
        self.assertNotIn('funnel_diagnosis', FIELDS)
        self.assertNotIn('funnel_diagnosis', KEPT)
        with self.assertRaises(sessions.SessionError):
            TaskContext.validate_options({**options, 'funnel_diagnosis': diagnosis_example()})

    def test_funnel_diagnosis_obsolete_recall_success_or_failure_preserves_newer_brain(self):
        # Challenge the completed-result guard, not an unsupported parallel native preparation.
        self.store.accelerators = SimpleNamespace(keep_current=lambda *_: None)
        self.store.cancelled = set()
        self.store.stopping = threading.Event()
        self.store.generations = {'fixture': 'old-generation'}
        self.store.get = lambda *_: {'id': 'fixture', 'brain': self.saved()}
        self.store._event = lambda *_: self.fail('An obsolete recall emitted an event.')
        old = {'id': 'fixture', 'project_id': 'project', 'brain': self.brain}
        newer = {**self.brain, 'context_id': 'cccccccc-cccc-4ccc-8ccc-cccccccccccc',
                 'query': 'Juniper newer query', 'funnel_diagnosis': diagnosis_example(8)}
        for fail in (False, True):
            with self.subTest(fail=fail):
                self.store.generations['fixture'] = 'old-generation'
                def prepare(_):
                    self.store._save_brain('fixture', newer)
                    self.store.generations['fixture'] = 'new-generation'
                    if fail:
                        raise sessions.SessionError('obsolete fixture failure')
                    return {**self.brain, 'funnel_diagnosis': diagnosis_example(2)}
                returned = self.store._recall('fixture', old, 'old-generation', SimpleNamespace(prepare=prepare))
                self.assertIsNone(returned)
                self.assertEqual(self.saved(), newer)

    def test_funnel_diagnosis_empty_query_and_fleet_resume_invalidate_centrally(self):
        self.store.accelerators = SimpleNamespace(keep_current=lambda *_: None)
        self.store.cancelled = set()
        self.store.stopping = threading.Event()
        self.store.generations = {'fixture': 'generation'}
        self.store._event = lambda *_: None
        self.store.get = lambda *_: {'id': 'fixture', 'brain': self.saved(), 'status': 'cancelled',
                                    'fleet': {'dry_run': True}, 'project_id': 'project'}
        self.store._save_brain('fixture', {**self.brain, 'query': '', 'funnel_diagnosis': diagnosis_example()})
        recalled = self.store._recall('fixture', self.store.get('fixture'), 'generation', None)
        self.assertNotIn('funnel_diagnosis', recalled['brain'])
        self.assertIsNone(recalled['brain']['capsule'])
        self.store._save_brain('fixture', {**self.brain, 'funnel_diagnosis': diagnosis_example()})
        self.store._workspace = lambda *_: Path('.')
        self.store.admit = lambda: None
        self.store.jobs = SimpleNamespace(full=lambda: False, put_nowait=lambda *_: None)
        self.store._status = lambda *_: None
        with patch.object(sessions, 'fleet_runtime', return_value={'available': True}):
            resumed = self.store._continue_fleet('fixture', 'resume')
        self.assertNotIn('funnel_diagnosis', resumed['brain'])
        self.assertIsNone(resumed['brain']['context_id'])


    @staticmethod
    def receipt():
        return {'schema_version': 1, 'observation_state': 'observed', 'plans_present': True, 'returned_status': 'committed'}

    def test_brain_receipt_only_and_two_siblings_are_independently_owned(self):
        for values in ({'ordinary_receipt': self.receipt()},
                       {'ordinary_receipt': self.receipt(), 'funnel_diagnosis': {'raw': 'PRIVATE'}},
                       {'ordinary_receipt': {'raw': 'PRIVATE'}, 'funnel_diagnosis': diagnosis_example()},
                       {'ordinary_receipt': self.receipt(), 'funnel_diagnosis': diagnosis_example()}):
            original = {**self.brain, **values}
            before = copy.deepcopy(original)
            self.store._save_brain('fixture', original)
            saved = self.saved()
            self.assertEqual(before, original)
            self.assertEqual('schema_version' in values['ordinary_receipt'], 'ordinary_receipt' in saved)
            if 'funnel_diagnosis' in values:
                self.assertEqual('schema_version' in values['funnel_diagnosis'], 'funnel_diagnosis' in saved)
            for change in ({'context_id': None}, {'capsule': None}):
                self.store._save_brain('fixture', {**original, **change})
                self.assertNotIn('ordinary_receipt', self.saved())
                self.assertNotIn('funnel_diagnosis', self.saved())
        value = {**self.brain, 'ordinary_receipt': self.receipt(), 'funnel_diagnosis': diagnosis_example()}
        for normalizer, retained in (('normalize_ordinary_receipt', 'funnel_diagnosis'), ('normalize_funnel_diagnosis', 'ordinary_receipt')):
            with patch.object(sessions, normalizer, side_effect=RuntimeError('metadata-only failure')):
                self.store._save_brain('fixture', value)
            self.assertIn(retained, self.saved())
        self.store._save_brain('fixture', self.brain)
        for error in (KeyboardInterrupt(), SystemExit(4)):
            with patch.object(sessions, 'normalize_ordinary_receipt', side_effect=error), self.assertRaises(type(error)) as caught:
                self.store._save_brain('fixture', value)
            self.assertIs(error, caught.exception)
            self.assertEqual(self.brain, self.saved())

    def test_receipt_retrieve_keeps_tuple_and_historical_freshness(self):
        context, task, capsule = self.retrieve_context()
        options = {'bank': 'memory-bank', 'task_id': task['external_id'], 'query': 'Juniper cable', 'review': False}
        session = {'project_id': 'project', 'provider': 'codex'}
        result = {'capsule': capsule, 'capsule_text': 'working: Juniper',
                  'ordinary_receipt': self.receipt(), 'funnel_diagnosis': diagnosis_example()}
        class Sink(dict):
            def __setitem__(self, key, value): raise AssertionError('sink hook')
        with patch.object(context, '_knowledge', return_value={'ok': True, 'result': result}), patch.object(context, '_source_hashes', return_value={}):
            sink = {}
            returned = context._retrieve(session, options, Path('.'), task, {'root': '.'}, True, sink)
            self.assertEqual((capsule, {}, 'working: Juniper'), returned)
            self.assertEqual({key: result[key] for key in ('ordinary_receipt', 'funnel_diagnosis')}, sink)
            self.assertEqual(returned, context._retrieve(session, options, Path('.'), task, {'root': '.'}, True, Sink()))
            self.assertEqual(returned, context._retrieve(session, options, Path('.'), task, {'root': '.'}, True))
        session['brain'] = {**self.brain, 'task': task, 'capsule': capsule, 'source_hashes': {},
                            'ordinary_receipt': self.receipt()}
        before = copy.deepcopy(session['brain'])
        def response(operation, *args, **kwargs):
            return {'task': task} if operation == 'inspect' else {'ok': True, 'result': result}
        with patch.object(context, '_bound', return_value=(session['brain'], options, Path('.'), {'root': '.'})), \
                patch.object(context, '_knowledge', side_effect=response), patch.object(context, '_source_hashes', return_value={}):
            self.assertTrue(context.ensure_fresh(session))
        self.assertEqual(before, session['brain'])
        from harness.task_context import FIELDS, KEPT
        self.assertNotIn('ordinary_receipt', FIELDS)
        self.assertNotIn('ordinary_receipt', KEPT)
        with self.assertRaises(sessions.SessionError):
            TaskContext.validate_options({**options, 'ordinary_receipt': self.receipt()})
        with patch.object(context, '_knowledge', return_value={'ok': True, 'result': capsule}), patch.object(context, '_source_hashes', return_value={}):
            sink = {}
            self.assertEqual((capsule, {}, None), context._retrieve(session, {**options, 'review': True}, Path('.'), task, {'root': '.'}, False, sink))
        self.assertEqual({}, sink)

    def test_receipt_lifecycle_invalidation_retention_and_generation_rules(self):
        value = {**self.brain, 'ordinary_receipt': self.receipt(), 'funnel_diagnosis': diagnosis_example()}
        for change in ({'approved': True}, {'remembered': [{'title': 'new bookkeeping'}]}, {'approved': False}):
            self.store._save_brain('fixture', {**value, **change})
            self.assertEqual(self.receipt(), self.saved()['ordinary_receipt'])
        self.store.accelerators = SimpleNamespace(keep_current=lambda *_: None)
        self.store.cancelled = set()
        self.store.stopping = threading.Event()
        self.store.generations = {'fixture': 'old-generation'}
        self.store.get = lambda *_: {'id': 'fixture', 'brain': self.saved()}
        self.store._event = lambda *_: self.fail('obsolete recall emitted')
        newer = {**value, 'context_id': 'cccccccc-cccc-4ccc-8ccc-cccccccccccc', 'query': 'new query',
                 'ordinary_receipt': {**self.receipt(), 'returned_status': 'deferred'}}
        for failed in (False, True):
            self.store.generations['fixture'] = 'old-generation'
            def prepare(_):
                self.store._save_brain('fixture', newer)
                self.store.generations['fixture'] = 'new-generation'
                if failed: raise sessions.SessionError('obsolete failure')
                return value
            self.assertIsNone(self.store._recall('fixture', {'brain': value}, 'old-generation', SimpleNamespace(prepare=prepare)))
            self.assertEqual(newer, self.saved())
        self.store._event = lambda *_: None
        self.store.generations['fixture'] = 'generation'
        self.store._save_brain('fixture', {**value, 'query': ''})
        recalled = self.store._recall('fixture', self.store.get('fixture'), 'generation', None)
        self.assertNotIn('ordinary_receipt', recalled['brain'])
        self.assertNotIn('funnel_diagnosis', recalled['brain'])
        self.assertIsNone(recalled['brain']['capsule'])
        self.store._save_brain('fixture', self.brain)
        self.assertNotIn('ordinary_receipt', self.saved())

    def test_optional_receipt_normalizer_capability_and_cancellation(self):
        # Fresh-process import controls distinguish absent capability from broken dependencies.
        script = """
import builtins, sys
from types import SimpleNamespace
original = builtins.__import__
mode = sys.argv[1]
def imported(name, *args, **kwargs):
    if name == 'memory_diagnosis':
        if mode == 'older':
            return SimpleNamespace(normalize_funnel_diagnosis=lambda value: value)
        missing = 'memory_diagnosis' if mode == 'absent' else 'invented_dependency'
        raise ModuleNotFoundError('invented import failure', name=missing)
    return original(name, *args, **kwargs)
builtins.__import__ = imported
try:
    from harness import sessions
except ModuleNotFoundError as error:
    assert mode == 'dependency' and error.name == 'invented_dependency'
else:
    assert mode != 'dependency'
    value = {'fixture': 1}
    assert sessions.normalize_funnel_diagnosis(value) == (value if mode == 'older' else None)
    assert sessions.normalize_ordinary_receipt(value) is None
"""
        for mode in ('absent', 'older', 'dependency'):
            environment = {**os.environ, 'PYTHONPATH': str(Path(__file__).resolve().parents[1] / 'harness/src')}
            imported = subprocess.run([sys.executable, '-c', script, mode], env=environment,
                                      text=True, capture_output=True, timeout=15)
            self.assertEqual(0, imported.returncode, imported.stderr)
        with patch.object(sessions, '_memory_diagnosis', None):
            self.assertIsNone(sessions.normalize_ordinary_receipt(self.receipt()))
        with patch.object(sessions, '_memory_diagnosis', SimpleNamespace(normalize_funnel_diagnosis=lambda value: value)):
            self.assertIsNone(sessions.normalize_ordinary_receipt(self.receipt()))
            self.assertEqual(diagnosis_example(), sessions.normalize_funnel_diagnosis(diagnosis_example()))
        context, task, capsule = self.retrieve_context()
        options = {'bank': 'memory-bank', 'task_id': task['external_id'], 'query': 'Juniper cable', 'review': False}
        result = {'capsule': capsule, 'capsule_text': 'working: Juniper',
                  'ordinary_receipt': self.receipt(), 'funnel_diagnosis': diagnosis_example()}
        for normalizer, retained in (('normalize_ordinary_receipt', 'funnel_diagnosis'), ('normalize_funnel_diagnosis', 'ordinary_receipt')):
            sink = {}
            with patch.object(context, '_knowledge', return_value={'ok': True, 'result': result}), \
                    patch.object(context, '_source_hashes', return_value={}), \
                    patch('harness.task_context.' + normalizer, side_effect=RuntimeError('metadata-only')):
                context._retrieve({'project_id': 'project'}, options, Path('.'), task, {'root': '.'}, True, sink)
            self.assertIn(retained, sink)
        for error in (KeyboardInterrupt(), SystemExit(8)):
            with patch.object(context, '_knowledge', return_value={'ok': True, 'result': result}), \
                    patch.object(context, '_source_hashes', return_value={}), \
                    patch('harness.task_context.normalize_ordinary_receipt', side_effect=error), self.assertRaises(type(error)) as caught:
                context._retrieve({'project_id': 'project'}, options, Path('.'), task, {'root': '.'}, True, {})
            self.assertIs(error, caught.exception)



if __name__ == "__main__":
    unittest.main()
