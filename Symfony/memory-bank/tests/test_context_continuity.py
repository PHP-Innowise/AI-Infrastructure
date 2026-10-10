"""End-to-end tests for local chat snapshots and explicit chat merges."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "context_continuity.py"
EDITION = SCRIPT.parents[2]
sys.path.insert(0, str(SCRIPT.parent))
import context_continuity as runtime


def clean_environment(**extra: str) -> dict[str, str]:
    """The caller's environment without an attached layout or an opt-out."""
    environment = {
        key: value for key, value in os.environ.items()
        if not key.startswith(("ACCELERATOR_", "CONTEXT_CONTINUITY_", "GIT_"))
    }
    environment.update(extra)
    return environment


class ContextContinuityTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="continuity-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / "readme.txt").write_text("initial\n")
        subprocess.run(["git", "-C", str(self.root), "add", "readme.txt"], check=True)
        subprocess.run(["git", "-C", str(self.root), "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "initial"], check=True)
        self.targets = 0

    def invoke(self, host: str, event: str, payload: object | None = None, *, json_output: bool = False,
               root: Path | None = None, environment: dict[str, str] | None = None) -> subprocess.CompletedProcess[bytes]:
        if event == "restore" and payload is None:
            payload = {"session_id": "new-target", "conversation_id": "new-target"}
        args = [sys.executable, str(SCRIPT), "--root", str(root or self.root), "--host", host, "--event", event]
        if json_output:
            args.append("--json")
        return subprocess.run(args, input=(json.dumps(payload).encode("utf-8") if payload is not None else None),
                              capture_output=True, timeout=10, env=environment or clean_environment())

    def prepare(self, *sessions: str, host: str = "codex", environment: dict[str, str] | None = None,
                root: Path | None = None) -> subprocess.CompletedProcess[bytes]:
        args = [sys.executable, str(SCRIPT), "--root", str(root or self.root), "--host", host, "--event", "merge"]
        for session in sessions:
            args.extend(["--source-session", session])
        return subprocess.run(args, capture_output=True, timeout=10, env=environment or clean_environment())

    def merged(self, host: str, *sessions: str) -> bytes:
        """Merge the captured chats into a brand-new task and return what it receives.

        A merge takes two chats or more; a single source is merged with a
        filler chat so a test can read one projection through the real path.
        """
        if len(sessions) < 2:
            self.invoke("codex", "capture", {"session_id": "filler", "prompt": "Filler chat."})
            sessions = (*sessions, "codex:filler")
        prepared = self.prepare(*sessions, host=host)
        self.assertEqual(0, prepared.returncode, prepared.stderr)
        self.targets += 1
        target = f"merged-target-{self.targets}"
        return self.invoke(host, "restore", {"session_id": target, "conversation_id": target}).stdout

    # -- automatic capture, explicit delivery ---------------------------------

    def test_a_new_session_receives_nothing_without_a_prepared_merge(self) -> None:
        """Carrying a branch's work forward is the Task Capsule's job, not a replay of chats."""
        self.assertEqual(b"", self.invoke("claude", "restore", json_output=True).stdout)
        self.assertFalse((self.root / ".context-handoff").exists())
        self.invoke("claude", "capture", {"session_id": "one", "last_assistant_message": "First."})
        self.invoke("claude", "capture", {"session_id": "two", "last_assistant_message": "Second."})
        for host in ("codex", "claude", "cursor"):
            with self.subTest(host=host):
                self.assertEqual(b"", self.invoke(host, "restore", json_output=True).stdout)
        self.assertFalse((self.root / ".context-handoff/merges").exists())

    def test_capture_merges_across_clients_without_brain_or_sqlite(self) -> None:
        captured = self.invoke("codex", "capture", {"session_id": "a", "last_assistant_message": "Visible final answer."})
        self.assertEqual(0, captured.returncode)
        self.assertEqual(b"", captured.stdout)
        self.invoke("claude", "capture", {"session_id": "b", "prompt": "Second chat."})
        self.assertEqual(0, self.prepare("codex:a", "claude:b", host="cursor").returncode)
        restored = self.invoke("cursor", "restore", json_output=True)
        self.assertEqual(0, restored.returncode)
        envelope = json.loads(restored.stdout)
        self.assertIn("Visible final answer.", envelope["additional_context"])
        self.assertIn("Second chat.", envelope["additional_context"])
        self.assertFalse((self.root / "memory-bank").exists())
        self.assertFalse((self.root / "project-brain").exists())

    def test_restore_json_uses_each_host_native_session_start_envelope(self) -> None:
        self.invoke("codex", "capture", {"session_id": "a", "last_assistant_message": "Visible."})
        self.invoke("codex", "capture", {"session_id": "b", "last_assistant_message": "Also visible."})
        envelopes = {}
        for host in ("claude", "codex", "cursor"):
            self.assertEqual(0, self.prepare("codex:a", "codex:b", host=host).returncode)
            envelopes[host] = json.loads(self.invoke(host, "restore", {"session_id": f"{host}-new", "conversation_id": f"{host}-new"}, json_output=True).stdout)
        self.assertEqual("SessionStart", envelopes["claude"]["hookSpecificOutput"]["hookEventName"])
        self.assertEqual("SessionStart", envelopes["codex"]["hookSpecificOutput"]["hookEventName"])
        self.assertIn("Visible.", envelopes["claude"]["hookSpecificOutput"]["additionalContext"])
        self.assertIn("Visible.", envelopes["cursor"]["additional_context"])

    def test_one_wired_script_dispatches_on_the_payload_event_name(self) -> None:
        for host, prompt_event, answer_event, start_event, session_key in (
            ("claude", "UserPromptSubmit", "Stop", "SessionStart", "session_id"),
            ("codex", "UserPromptSubmit", "Stop", "SessionStart", "session_id"),
            ("cursor", "beforeSubmitPrompt", "afterAgentResponse", "sessionStart", "conversation_id"),
        ):
            with self.subTest(host=host):
                for chat in ("a", "b"):
                    asked = self.invoke(host, "hook", {"hook_event_name": prompt_event, session_key: f"{host}-{chat}", "prompt": f"{host} question {chat}"})
                    answer_field = "text" if host == "cursor" else "last_assistant_message"
                    answered = self.invoke(host, "hook", {"hook_event_name": answer_event, session_key: f"{host}-{chat}", answer_field: f"{host} answer {chat}"})
                    self.assertEqual((0, b"", 0, b""), (asked.returncode, asked.stdout, answered.returncode, answered.stdout))
                self.assertEqual(b"", self.invoke(host, "hook", {"hook_event_name": "PreToolUse", session_key: f"{host}-a", "prompt": "ignored"}).stdout)
                self.assertEqual(0, self.prepare(f"{host}:{host}-a", f"{host}:{host}-b", host=host).returncode)
                started = self.invoke(host, "hook", {"hook_event_name": start_event, session_key: f"{host}-new"})
                envelope = json.loads(started.stdout)
                text = envelope["additional_context"] if host == "cursor" else envelope["hookSpecificOutput"]["additionalContext"]
                for chat in ("a", "b"):
                    self.assertIn(f"{host} question {chat}", text)
                    self.assertIn(f"{host} answer {chat}", text)
        listing = json.loads(self.invoke("codex", "list").stdout)
        self.assertNotIn("ignored", json.dumps(listing))

    def test_disabled_environment_neither_captures_nor_restores(self) -> None:
        environment = clean_environment(CONTEXT_CONTINUITY_DISABLED="1")
        result = self.invoke("codex", "capture", {"session_id": "a", "last_assistant_message": "Visible."}, environment=environment)
        self.assertEqual(0, result.returncode)
        self.assertFalse((self.root / ".context-handoff").exists())

    def test_restore_only_opt_out_preserves_capture_and_the_pending_merge(self) -> None:
        environment = clean_environment(CONTEXT_CONTINUITY_RESTORE_DISABLED="1")
        for session in ("a", "b"):
            self.invoke("codex", "capture", {"session_id": session, "prompt": f"Progress {session}"}, environment=environment)
        self.assertEqual(2, len(list((self.root / ".context-handoff").glob("*.json"))))
        self.assertEqual(0, self.prepare("codex:a", "codex:b").returncode)
        restored = self.invoke("codex", "restore", environment=environment)
        self.assertEqual(b"", restored.stdout)
        self.assertEqual(1, len(list((self.root / ".context-handoff/merges").glob("*.pending.json"))))

    def test_attached_state_keeps_snapshots_out_of_the_project(self) -> None:
        state = Path(self.tmp.name + "-state")
        state.mkdir()
        self.addCleanup(shutil.rmtree, state, True)
        environment = clean_environment(
            ACCELERATOR_HOME=str(EDITION), ACCELERATOR_STATE_DIR=str(state), ACCELERATOR_PROJECT_DIR=str(self.root),
        )
        for session in ("a", "b"):
            result = self.invoke("claude", "capture", {"session_id": session, "prompt": f"Attached {session}"},
                                 root=state, environment=environment)
            self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse((self.root / ".context-handoff").exists())
        self.assertEqual(2, len(list((state / ".context-handoff").glob("*.json"))))
        self.assertEqual(0, self.prepare("claude:a", "claude:b", host="claude", root=state, environment=environment).returncode)
        restored = self.invoke("claude", "restore", root=state, environment=environment).stdout.decode()
        self.assertIn("Attached a", restored)
        self.assertIn(str(state / ".context-handoff" / "merges"), restored)
        self.assertFalse((self.root / ".context-handoff").exists())
        # Without --root the runtime finds the attached state, as context.py does.
        listed = subprocess.run([sys.executable, str(SCRIPT), "--host", "claude", "--event", "list"],
                                capture_output=True, timeout=10, env=environment, cwd=self.root)
        self.assertEqual(0, listed.returncode, listed.stderr)
        self.assertEqual({"claude:a", "claude:b"}, {item["session"] for item in json.loads(listed.stdout)})

    def test_symlinked_local_lock_is_refused_without_following_target(self) -> None:
        store = self.root / ".context-handoff"
        store.mkdir()
        target = self.root / "outside-lock"
        (store / ".lock").symlink_to(target)
        self.invoke("codex", "capture", {"session_id": "a", "last_assistant_message": "Visible."})
        self.assertFalse(target.exists())
        self.assertEqual([], list(store.glob("*.json")))

    def test_merged_sources_keep_separate_provenance(self) -> None:
        self.invoke("claude", "capture", {"session_id": "one", "last_assistant_message": "First."})
        self.invoke("claude", "capture", {"session_id": "two", "last_assistant_message": "Second."})
        self.assertEqual(2, len(list((self.root / ".context-handoff").glob("*.json"))))
        restored = self.merged("codex", "claude:one", "claude:two")
        self.assertIn(b"First.", restored)
        self.assertIn(b"Second.", restored)
        self.assertIn(b"Source 1", restored)
        self.assertIn(b"Source 2", restored)
        archive = json.loads(next((self.root / ".context-handoff/merges").glob("*.json")).read_text())
        self.assertEqual({"claude:one", "claude:two"}, {s["session"] for s in archive["sources"]})

    def test_selected_merge_all_hosts_retains_conflicting_decisions_and_progress(self) -> None:
        self.invoke("claude", "capture", {"session_id": "a", "prompt": "Decision: use SQL. Progress: schema done."})
        self.invoke("cursor", "capture", {"conversation_id": "b", "text": "Decision: use files. Progress: parser done."})
        self.invoke("codex", "capture", {"session_id": "excluded", "prompt": "Unselected chat."})
        for host in ("codex", "claude", "cursor"):
            with self.subTest(host=host):
                prepared = self.prepare("claude:a", "cursor:b", host=host)
                self.assertEqual(0, prepared.returncode, prepared.stderr)
                result = self.invoke(host, "restore", {"session_id": f"{host}-target", "conversation_id": f"{host}-target"}).stdout
                self.assertIn(b"use SQL", result)
                self.assertIn(b"use files", result)
                self.assertIn(b"schema done", result)
                self.assertIn(b"parser done", result)
                self.assertIn(b"Conflicts: not evaluated", result)
                self.assertNotIn(b"Unselected chat", result)
        self.assertEqual([], list((self.root / ".context-handoff/merges").glob("*.pending.json")))

    def test_frozen_sources_survive_changes_rotation_and_target_resume(self) -> None:
        self.invoke("codex", "capture", {"session_id": "a", "prompt": "Original A."})
        self.invoke("claude", "capture", {"session_id": "b", "prompt": "Original B."})
        self.assertEqual(0, self.prepare("codex:a", "claude:b", host="cursor").returncode)
        initial = self.invoke("cursor", "restore").stdout
        archive = next(path for path in (self.root / ".context-handoff/merges").glob("*.json"))
        frozen = archive.read_bytes()
        self.invoke("codex", "capture", {"session_id": "a", "prompt": "Changed A."})
        for number in range(10):
            self.invoke("codex", "capture", {"session_id": f"rotate-{number}", "prompt": "Different chat."})
        self.assertEqual(initial, self.invoke("cursor", "restore", {"conversation_id": "new-target", "source": "resume"}).stdout)
        self.invoke("cursor", "capture", {"conversation_id": "new-target", "text": "Merged task implementation complete."})
        resumed = self.invoke("cursor", "restore").stdout
        self.assertIn(b"Original A", resumed)
        self.assertIn(b"Original B", resumed)
        self.assertIn(b"Current task progress", resumed)
        self.assertIn(b"implementation complete", resumed)
        self.assertNotIn(b"Changed A", resumed)
        self.assertEqual(frozen, archive.read_bytes())

    def test_resume_and_missing_identity_do_not_consume_pending_merge(self) -> None:
        for session in ("a", "b"):
            self.invoke("codex", "capture", {"session_id": session, "prompt": session})
        self.assertEqual(0, self.prepare("codex:a", "codex:b").returncode)
        pending = next((self.root / ".context-handoff/merges").glob("*.pending.json"))
        self.assertEqual(b"", self.invoke("codex", "restore", {"session_id": "a", "source": "resume"}).stdout)
        self.assertEqual(b"", self.invoke("codex", "restore", {"session_id": "a", "source": "startup"}).stdout)
        self.assertTrue(pending.exists())
        self.assertEqual(b"", self.invoke("codex", "restore", {}).stdout)
        self.assertEqual(b"", self.invoke("codex", "restore", {"session_id": "unknown", "source": "compact"}).stdout)
        self.assertTrue(pending.exists())
        self.assertNotEqual(b"", self.invoke("codex", "restore").stdout)
        self.assertFalse(pending.exists())

    def test_a_resumed_source_chat_is_not_handed_its_own_history(self) -> None:
        self.invoke("codex", "capture", {"session_id": "a", "prompt": "Source progress"})
        self.assertIsNone(runtime.restore(self.root, "codex", {"session_id": "a", "source": "resume"}))
        self.assertIsNone(runtime.restore(self.root, "codex", {"session_id": "a"}))

    def test_invalid_selected_sources_do_not_publish_partial_merge(self) -> None:
        self.invoke("codex", "capture", {"session_id": "a", "prompt": "A"})
        for sessions in (("codex:a", "codex:a"), ("codex:a", "claude:missing"), ("codex:a",)):
            self.assertEqual(1, self.prepare(*sessions).returncode)
        self.assertFalse((self.root / ".context-handoff/merges").exists())

    def test_identical_selection_retry_succeeds_but_changed_selection_is_refused(self) -> None:
        for session in ("a", "b", "c"):
            self.invoke("codex", "capture", {"session_id": session, "prompt": session})
        first = self.prepare("codex:a", "codex:b")
        self.assertEqual(0, first.returncode)
        self.assertEqual(first.stdout, self.prepare("codex:a", "codex:b").stdout)
        self.assertEqual(1, self.prepare("codex:a", "codex:c").returncode)

    def test_archive_quota_preserves_existing_tasks_without_partial_publication(self) -> None:
        for session in ("a", "b"):
            self.invoke("codex", "capture", {"session_id": session, "prompt": f"Original {session}"})
        runtime.prepare_merge(self.root, "codex", ["codex:a", "codex:b"])
        directory = self.root / ".context-handoff/merges"
        archive = next(directory.glob("*.json"))
        original = archive.read_bytes()
        for limits in ({"MAX_MERGE_ARCHIVES": 1}, {"MAX_MERGE_STORE_BYTES": len(original)}, {"MAX_MERGE_BYTES": 1}):
            with patch.multiple(runtime, **limits), self.assertRaises(runtime.ContinuityError):
                runtime.prepare_merge(self.root, "claude", ["codex:a", "codex:b"])
            self.assertEqual([archive], list(directory.glob("*.json")))
            self.assertEqual(original, archive.read_bytes())
        self.assertIsNotNone(runtime.restore(self.root, "codex", {"session_id": "target"}))

    def test_unicode_previews_fit_byte_budget_for_every_client(self) -> None:
        sessions = [f"codex:unicode-{number}" for number in range(8)]
        for number in range(8):
            self.invoke("codex", "capture", {"session_id": f"unicode-{number}", "prompt": "😀漢字" * 5000})
        for host in ("codex", "claude", "cursor"):
            result = self.merged(host, *sessions).rstrip(b"\n")
            self.assertLessEqual(len(result), 6000)
            for number in range(1, 9):
                self.assertIn(f"Source {number} |".encode(), result)

    def test_pending_selection_is_claimed_once_under_concurrent_restore(self) -> None:
        for session in ("a", "b", "extra"):
            self.invoke("codex", "capture", {"session_id": session, "prompt": session})
        self.assertEqual(0, self.prepare("codex:a", "codex:b").returncode)
        with ThreadPoolExecutor(max_workers=4) as pool:
            results = list(pool.map(lambda n: self.invoke("codex", "restore", {"session_id": f"target-{n}"}), range(4)))
        self.assertTrue(all(r.returncode == 0 for r in results))
        self.assertEqual(1, sum(1 for r in results if r.stdout))
        bundles = [json.loads(p.read_text()) for p in (self.root / ".context-handoff/merges").glob("*.json")]
        self.assertEqual([2], [len(b["sources"]) for b in bundles])

    def test_all_source_previews_fit_budget_with_heavily_escaped_text(self) -> None:
        for number in range(8):
            self.invoke("codex", "capture", {"session_id": f"source-{number}", "prompt": f"Start {number}:" + "\u0001\n\"" * 10000 + f"End {number}"})
        result = self.merged("claude", *(f"codex:source-{number}" for number in range(8))).decode()
        self.assertLessEqual(len(result.rstrip("\n")), 6000)
        for number in range(1, 9):
            self.assertIn(f"Source {number} |", result)
        self.assertEqual(8, len(json.loads(next((self.root / ".context-handoff/merges").glob("*.json")).read_text())["sources"]))

    def test_corrupt_frozen_archive_and_symlink_directory_fail_closed(self) -> None:
        for session in ("a", "b"):
            self.invoke("codex", "capture", {"session_id": session, "prompt": session.upper()})
        self.assertEqual(0, self.prepare("codex:a", "codex:b", host="claude").returncode)
        self.assertNotEqual(b"", self.invoke("claude", "restore").stdout)
        archive = next((self.root / ".context-handoff/merges").glob("*.json"))
        archive.write_text("{}")
        self.assertEqual(b"", self.invoke("claude", "restore").stdout)
        shutil.rmtree(archive.parent)
        outside = self.root / "outside"
        outside.mkdir()
        archive.parent.symlink_to(outside, target_is_directory=True)
        self.assertEqual(b"", self.invoke("cursor", "restore").stdout)
        self.assertEqual([], list(outside.iterdir()))

    def test_list_is_read_only_and_returns_selectable_source_identities(self) -> None:
        self.assertEqual([], json.loads(self.invoke("claude", "list").stdout))
        self.assertFalse((self.root / ".context-handoff").exists())
        self.invoke("cursor", "capture", {"conversation_id": "a", "text": "Progress."})
        listing = json.loads(self.invoke("claude", "list").stdout)
        self.assertEqual("cursor:a", listing[0]["session"])

    def test_prompt_and_final_response_append_once_to_one_session(self) -> None:
        self.invoke("claude", "capture", {"session_id": "one", "prompt": "What changed?"})
        self.invoke("claude", "capture", {"session_id": "one", "last_assistant_message": "The handler changed."})
        self.invoke("claude", "capture", {"session_id": "one", "last_assistant_message": "The handler changed."})
        snapshot = json.loads(next((self.root / ".context-handoff").glob("*.json")).read_text(encoding="utf-8"))
        self.assertEqual(1, snapshot["content"].count("What changed?"))
        self.assertEqual(1, snapshot["content"].count("The handler changed."))
        self.assertIn("event", snapshot["kind"])

    def test_plain_export_preserves_exact_utf8_bytes_when_no_final_message_exists(self) -> None:
        export = self.root / "visible.md"
        content = "Visible: ž\r\nlast line".encode("utf-8")
        export.write_bytes(content)
        self.invoke("claude", "capture", {"session_id": "a", "transcript_path": str(export)})
        snapshot = json.loads(next((self.root / ".context-handoff").glob("*.json")).read_text(encoding="utf-8"))
        self.assertEqual("Visible: ž\r\nlast line", snapshot["content"])
        self.assertEqual("visible-export", snapshot["kind"])

    def test_known_jsonl_projects_only_visible_user_and_assistant_text(self) -> None:
        transcript = self.root / "client.jsonl"
        transcript.write_text("\n".join([
            json.dumps({"type": "response_item", "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "Question"}]}}),
            json.dumps({"type": "response_item", "payload": {"type": "message", "role": "assistant", "channel": "final", "content": [{"type": "output_text", "text": "Answer"}, {"type": "tool_use", "name": "hidden"}]}}),
            json.dumps({"type": "response_item", "payload": {"type": "message", "role": "assistant", "channel": "analysis", "content": [{"type": "output_text", "text": "private thought"}]}}),
            json.dumps({"type": "reasoning", "payload": {"message": {"role": "assistant", "content": [{"type": "output_text", "text": "nested private thought"}]}}}),
        ]) + "\n", encoding="utf-8")
        self.invoke("codex", "capture", {"session_id": "a", "transcript_path": str(transcript)})
        restored = self.merged("claude", "codex:a")
        self.assertIn(b"Question", restored)
        self.assertIn(b"Answer", restored)
        self.assertNotIn(b"tool payload", restored)
        self.assertNotIn(b"hidden", restored)
        self.assertNotIn(b"private thought", restored)
        self.assertIn(b"projected-jsonl", restored)

    def test_transcript_projection_keeps_latest_final_response(self) -> None:
        transcript = self.root / "visible.txt"
        transcript.write_text("Earlier visible history.", encoding="utf-8")
        self.invoke("claude", "capture", {"session_id": "a", "transcript_path": str(transcript), "last_assistant_message": "Latest final."})
        restored = self.merged("codex", "claude:a")
        self.assertIn(b"Earlier visible history.", restored)
        self.assertIn(b"Latest final.", restored)
        self.assertIn(b"visible-export+event", restored)

    def test_rejects_secret_and_branch_restore_isolated(self) -> None:
        secret = "se" + "cret" + "=value"
        self.invoke("cursor", "capture", {"conversation_id": "a", "text": secret})
        self.assertFalse((self.root / ".context-handoff").exists())
        self.invoke("cursor", "capture", {"conversation_id": "a", "text": "Visible."})
        self.invoke("cursor", "capture", {"conversation_id": "b", "text": "Also visible."})
        self.assertEqual(0, self.prepare("cursor:a", "cursor:b", host="cursor").returncode)
        subprocess.run(["git", "-C", str(self.root), "switch", "-qc", "other"], check=True)
        self.assertEqual(b"", self.invoke("cursor", "restore").stdout)
        self.assertEqual(1, self.prepare("cursor:a", "cursor:b", host="claude").returncode)

    def test_fine_grained_token_is_never_captured_or_merged(self) -> None:
        self.invoke("codex", "capture", {"session_id": "a", "prompt": "github_" + "pat_" + "A" * 60})
        self.assertFalse((self.root / ".context-handoff").exists())

    def test_unavailable_or_secret_transcript_falls_back_to_safe_current_event(self) -> None:
        transcript = self.root / "old.txt"
        transcript.write_text("api_key=" + "sensitive-old-value")
        for source in (transcript, self.root / "missing.jsonl"):
            result = self.invoke("codex", "capture", {"session_id": "safe", "transcript_path": str(source), "last_assistant_message": "Safe current result."})
            self.assertEqual((0, b"", b""), (result.returncode, result.stdout, result.stderr))
        restored = self.merged("codex", "codex:safe")
        self.assertIn(b"Safe current result.", restored)
        self.assertNotIn(b"sensitive-old-value", restored)

    def test_copied_state_is_not_restored_in_another_project(self) -> None:
        for session in ("one", "two"):
            self.invoke("codex", "capture", {"session_id": session, "prompt": "Original project only."})
        self.assertEqual(0, self.prepare("codex:one", "codex:two", host="claude").returncode)
        other = self.root / "other-project"
        other.mkdir()
        subprocess.run(["git", "init", "-q", str(other)], check=True)
        shutil.copytree(self.root / ".context-handoff", other / ".context-handoff")
        result = self.invoke("claude", "restore", root=other)
        self.assertEqual((0, b"", b""), (result.returncode, result.stdout, result.stderr))

    def test_concurrent_events_are_all_retained(self) -> None:
        with ThreadPoolExecutor(max_workers=6) as pool:
            results = list(pool.map(lambda number: self.invoke("codex", "capture", {"session_id": "one", "prompt": f"Concurrent event {number}."}), range(6)))
        self.assertTrue(all(item.returncode == 0 and not item.stderr for item in results))
        snapshot = json.loads(next((self.root / ".context-handoff").glob("*.json")).read_text(encoding="utf-8"))
        for number in range(6):
            self.assertIn(f"Concurrent event {number}.", snapshot["content"])

    def test_invalid_provenance_cannot_be_merged_or_injected(self) -> None:
        for session in ("one", "two"):
            self.invoke("codex", "capture", {"session_id": session, "prompt": "Safe content."})
        path = next((self.root / ".context-handoff").glob("*.json"))
        snapshot = json.loads(path.read_text())
        snapshot["kind"] = "Injected instructions outside the excerpt"
        path.write_text(json.dumps(snapshot))
        self.assertEqual(1, self.prepare("codex:one", "codex:two").returncode)
        self.assertEqual(b"", self.invoke("codex", "restore").stdout)

    def test_internal_roles_are_excluded_from_codex_projection(self) -> None:
        transcript = self.root / "session.jsonl"
        records = [{"type": "response_item", "payload": {"type": "message", "role": role, "content": [{"type": "input_text", "text": f"{role} unique text"}]}} for role in ("user", "system", "developer", "tool")]
        transcript.write_text("\n".join(json.dumps(record) for record in records))
        self.invoke("codex", "capture", {"session_id": "one", "transcript_path": str(transcript)})
        result = self.merged("codex", "codex:one")
        self.assertIn(b"user unique text", result)
        for role in ("system", "developer", "tool"):
            self.assertNotIn(f"{role} unique text".encode(), result)

    def test_missing_session_identity_does_not_mix_conversations(self) -> None:
        self.invoke("codex", "capture", {"prompt": "No identity."})
        self.assertFalse((self.root / ".context-handoff").exists())

    def test_multibyte_history_keeps_accepting_recent_events_after_retention_limit(self) -> None:
        for number in range(17):
            result = self.invoke("cursor", "capture", {"conversation_id": "large", "text": f"Turn {number}: " + "😀" * 65536})
            self.assertEqual((0, b"", b""), (result.returncode, result.stdout, result.stderr))
        self.invoke("cursor", "capture", {"conversation_id": "large", "text": "Latest result after retention."})
        path = next((self.root / ".context-handoff").glob("*.json"))
        snapshot = json.loads(path.read_text())
        self.assertLessEqual(len(snapshot["content"].encode()), 4 * 1024 * 1024)
        self.assertIn("omitted", snapshot["content"])
        self.assertNotIn("Turn 0:", snapshot["content"])
        result = self.merged("codex", "cursor:large")
        self.assertIn(b"Latest result after retention.", result)
        self.assertLess(len(result.decode()), 6600)


if __name__ == "__main__":
    unittest.main()
