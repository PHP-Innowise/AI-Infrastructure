"""Fresh exact-result replay controls across archival and citation changes."""
from __future__ import annotations

import contextlib
import copy
import importlib
import io
import json
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock


SCRIPTS = Path(__file__).resolve().parents[1] / "PHP Core/memory-bank/scripts"


class MemoryResultReplayTests(unittest.TestCase):
    def setUp(self):
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.brain = importlib.import_module("brain_runtime")
            self.results = importlib.import_module("memory_results")
            self.cli = importlib.import_module("context")
            self.mcp = importlib.import_module("mcp_server")
        finally:
            sys.path.pop(0)
        temporary = tempfile.TemporaryDirectory(prefix="harbor-replay-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "specs/harbor.md"
        self.source.parent.mkdir()
        self.source.write_text("# Harbor\n\nEach harbor transfer has one canonical owner.\n")
        (self.root / "memory-bank/chunks").mkdir(parents=True)
        self.config = self.root / "project-brain/config/runtime.json"
        self.config.parent.mkdir(parents=True)
        self.config.write_text(json.dumps({"mode": "governed", "automatic_promotion": False}))
        self.task = self.brain.create_task(
            self.root, "HARBOR-REPLAY", "Verify harbor transfers", [], [], owner="local"
        )
        self.data = {
            "progress": "Harbor ownership checked.", "next_steps": [], "verified": True,
            "learnings": [{"type": "finding", "title": "Harbor transfers have one owner",
                          "consequence": "A harbor transfer names one canonical owner.",
                          "sources": ["specs/harbor.md#owner"]}],
        }

    def record(self, *, task_id=None, result_id="harbor-run", data=None,
               revision=1, owner="local"):
        return self.results.record_result(
            self.root, task_id or self.task["external_id"], result_id, revision,
            self.data if data is None else data, owner=owner,
            reason="Harbor regression checked canonical sources", attestation="agent",
        )

    def current_task(self):
        return self.brain.find_record(self.root, self.task["id"], record_type="task",
                                      include_archive=True)[1]

    def attempt(self, result_id="harbor-run", data=None):
        result = self.results.normalize(self.root, self.data if data is None else data, check_files=False)
        return self.results._attempt(self.root, self.task["id"], result_id,
                                     self.results.request_digest(self.task["id"], result))

    def close(self, status="completed"):
        task = self.current_task()
        self.brain.update_record(
            self.root, task["id"], expected_revision=task["revision"],
            progress=None, next_steps=[], files=[], sources=[], actor="local",
            transition_to=status, reason="Harbor fixture finished",
        )
        self.brain.compact(self.root)

    def shared_images(self):
        folders = (self.root / "project-brain/dynamic", self.root / "project-brain/archive",
                   self.root / "project-brain/control/handoffs", self.root / "memory-bank/chunks")
        images = {str(p.relative_to(self.root)): p.read_bytes()
                  for folder in folders for p in folder.rglob("*.md")}
        for p in self.brain.index_paths(self.root):
            if p.is_file():
                images[str(p.relative_to(self.root))] = p.read_bytes()
        return images

    def memory(self):
        shutil.copytree(SCRIPTS, self.root / "memory-bank/scripts",
                        ignore=shutil.ignore_patterns("__pycache__"))
        spec = importlib.util.spec_from_file_location(
            "harbor_mcp_runtime", self.root / "memory-bank/scripts/mcp_server.py")
        installed = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(installed)
        with mock.patch.dict("os.environ", {"PROJECT_BRAIN_OWNER": "local"}):
            return installed.Memory(self.root)

    def enable_promotion(self):
        (self.root / "memory-bank/README.md").write_text("# Harbor Memory Bank\n")
        (self.root / "memory-bank/INDEX.md").write_text(
            "# Harbor Memory Index\n\n"
            "| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n"
            "| --- | --- | --- | --- | --- | --- | --- | --- |\n")
        self.config.write_text(json.dumps({"mode": "governed", "automatic_promotion": True}))

    def test_deleted_source_exact_replay_keeps_shared_images(self):
        self.record()
        self.source.unlink()
        before = self.shared_images()
        with mock.patch.object(self.results, "source_fingerprints",
                               side_effect=AssertionError("Replay fingerprinted missing source")):
            reply = self.record()
        self.assertTrue(reply["replayed"])
        self.assertEqual(["existing"], [r["state"] for r in reply["records"]])
        self.assertEqual(before, self.shared_images())

    def test_completed_archive_replays_by_alias_and_uuid(self):
        self.record()
        self.close()
        before = self.shared_images()
        for identifier in (self.task["external_id"], self.task["id"]):
            with self.subTest(identifier=identifier):
                reply = self.record(task_id=identifier)
                self.assertTrue(reply["replayed"])
                self.assertEqual(["archived"], [r["state"] for r in reply["records"]])
                self.assertEqual(before, self.shared_images())

    def test_cancelled_archive_and_deleted_source_replay(self):
        self.record()
        self.close("cancelled")
        self.source.unlink()
        before = self.shared_images()
        self.assertTrue(self.record()["replayed"])
        self.assertEqual(before, self.shared_images())

    def test_archived_replay_retries_actual_pending_promotion_once(self):
        self.record()
        self.close()
        before = {name: image for name, image in self.shared_images().items()
                  if name.startswith("project-brain/archive/")}
        self.enable_promotion()
        first = self.record()
        self.assertTrue(first["replayed"])
        self.assertEqual(1, len(first["promotion"]["promoted"]))
        self.assertEqual([], first["promotion"]["failed"])
        chunks = {p.name: p.read_bytes() for p in (self.root / "memory-bank/chunks").glob("MEM-*.md")}
        self.assertEqual(1, len(chunks))
        second = self.record()
        self.assertTrue(second["replayed"])
        self.assertEqual([], second["promotion"]["promoted"])
        self.assertEqual(chunks, {p.name: p.read_bytes() for p in (self.root / "memory-bank/chunks").glob("MEM-*.md")})
        self.assertEqual(before, {name: image for name, image in self.shared_images().items()
                                  if name.startswith("project-brain/archive/")})

    def test_deleted_citation_replay_does_not_promote_unverifiable_learning(self):
        self.record()
        self.close()
        self.source.unlink()
        self.enable_promotion()
        before = self.shared_images()
        reply = self.record()
        self.assertTrue(reply["replayed"])
        self.assertEqual([], reply["promotion"]["promoted"])
        self.assertEqual([], reply["promotion"]["failed"])
        self.assertEqual(1, len(reply["promotion"]["blocked"]))
        self.assertEqual([], list((self.root / "memory-bank/chunks").glob("MEM-*.md")))
        self.assertEqual(before, self.shared_images())

    def test_new_or_unreceipted_archived_results_are_refused(self):
        self.record()
        self.close()
        before = self.shared_images()
        with self.assertRaisesRegex(self.brain.BrainError, "active task"):
            self.record(result_id="new-harbor-run")
        self.results._receipt(self.root, self.task["id"], "harbor-run").unlink()
        # Compaction archived the task and took its save evidence with it.
        self.assertFalse(self.attempt().exists())
        self.assertFalse((self.root / "project-brain/control/result-attempts" / self.task["id"]).exists())
        with self.assertRaisesRegex(self.brain.BrainError, "active task"):
            self.record()
        self.assertEqual(before, self.shared_images())

    def test_changed_receipted_payload_is_refused_after_deletion(self):
        self.record()
        self.source.unlink()
        changed = copy.deepcopy(self.data)
        changed["learnings"][0]["consequence"] = "Harbor transfers require two owners."
        before = self.shared_images()
        with self.assertRaisesRegex(self.brain.BrainError, "different content"):
            self.record(data=changed)
        self.assertEqual(before, self.shared_images())

    def test_missing_both_proofs_and_corrupt_local_receipts_keep_live_source_checks(self):
        self.record()
        self.attempt().unlink()
        self.source.unlink()
        receipt = self.results._receipt(self.root, self.task["id"], "harbor-run")
        before = self.shared_images()
        for content in (None, "not JSON", "[]"):
            with self.subTest(content=content):
                if content is None:
                    receipt.unlink(missing_ok=True)
                else:
                    receipt.write_text(content)
                with self.assertRaises(self.brain.BrainError):
                    self.record(revision=self.current_task()["revision"])
                self.assertEqual(before, self.shared_images())

    def test_new_result_missing_source_writes_no_learning(self):
        self.source.unlink()
        before = self.shared_images()
        with self.assertRaises(self.brain.BrainError):
            self.record()
        self.assertEqual(before, self.shared_images())
        self.assertFalse(self.results._receipt(self.root, self.task["id"], "harbor-run").exists())

    def test_new_result_checks_every_source_before_any_learning_write(self):
        data = copy.deepcopy(self.data)
        data["learnings"].append({**data["learnings"][0], "title": "Harbor secondary rule",
                                   "sources": ["specs/absent-harbor.md"]})
        before = self.shared_images()
        with self.assertRaises(self.brain.BrainError):
            self.record(data=data)
        self.assertEqual(before, self.shared_images())

    def test_missing_archived_task_cannot_be_reconstructed_from_receipt(self):
        self.record()
        self.close()
        self.brain.find_record(self.root, self.task["id"], include_archive=True)[0].unlink()
        with self.assertRaisesRegex(self.brain.BrainError, "not found"):
            self.record(task_id=self.task["id"])

    def test_archive_path_denies_unreceipted_active_status(self):
        self.record()
        self.close()
        path, task, body = self.brain.find_record(self.root, self.task["id"], include_archive=True)
        task["status"] = "active"
        path.write_text(self.brain.render_markdown_record(task, body))
        before = self.shared_images()
        # The archived record no longer validates (archived yet "active"); it
        # is refused by name rather than read as a writable task.
        with self.assertRaisesRegex(self.brain.BrainError, "active task|Project Brain record invalid"):
            self.record(result_id="new-harbor-run")
        self.assertEqual(before, self.shared_images())

    def test_replay_does_not_follow_replaced_symlink_but_new_write_refuses_it(self):
        self.record()
        target = self.root / "other.md"
        target.write_text("Foreign source image")
        self.source.unlink()
        try:
            self.source.symlink_to(target)
        except OSError as error:
            self.skipTest(f"Symlink fixture unavailable: {type(error).__name__}")
        before = self.shared_images()
        self.assertTrue(self.record()["replayed"])
        with self.assertRaisesRegex(self.brain.BrainError, "Symlink"):
            self.record(result_id="new-harbor-run", revision=self.current_task()["revision"])
        self.assertEqual(before, self.shared_images())

    def test_static_source_policy_and_verified_flag_still_apply_to_replay(self):
        self.record()
        before = self.shared_images()
        for source in ("../foreign.md", ".env", "memory-bank/chunks/foreign.md", "C:/foreign.md"):
            changed = copy.deepcopy(self.data)
            changed["learnings"][0]["sources"] = [source]
            with self.subTest(source=source), self.assertRaises(self.brain.BrainError):
                self.record(data=changed)
        for changes in ({"verified": False}, {"progress": "stderr: pasted transcript"},
                        {"unknown_field": True}):
            with self.subTest(changes=changes), self.assertRaises(self.brain.BrainError):
                self.record(data={**self.data, **changes})
        self.assertEqual(before, self.shared_images())

    def test_archived_replay_rechecks_current_owner_and_privacy(self):
        self.record()
        self.close()
        before = self.shared_images()
        with self.assertRaisesRegex(self.brain.BrainError, "not authorized"):
            self.record(owner="foreign")
        self.config.write_text(json.dumps({"mode": "governed", "automatic_promotion": False,
                                           "allowed_privacy": ["public"]}))
        with self.assertRaisesRegex(self.brain.BrainError, "privacy"):
            self.record()
        self.assertEqual(before, self.shared_images())

    def test_active_reused_alias_cannot_consume_archive_receipt(self):
        self.record()
        self.close()
        replacement = self.brain.create_task(self.root, self.task["external_id"],
                                             "A new harbor task", [], [], owner="local")
        self.source.unlink()
        before = self.shared_images()
        with self.assertRaises(self.brain.BrainError):
            self.record(revision=replacement["revision"])
        self.assertTrue(self.record(task_id=self.task["id"])["replayed"])
        self.assertEqual(before, self.shared_images())

    def test_unauthorized_active_alias_never_falls_back_to_archive(self):
        self.record()
        self.close()
        self.brain.create_task(self.root, self.task["external_id"], "Foreign harbor task",
                               [], [], owner="foreign")
        with self.assertRaisesRegex(self.brain.BrainError, "not authorized"):
            self.record()

    def test_uuid_shaped_alias_collision_is_refused(self):
        self.record()
        self.close()
        self.brain.create_task(self.root, self.task["id"], "A colliding harbor alias",
                               [], [], owner="local")
        before = self.shared_images()
        with self.assertRaisesRegex(self.brain.BrainError, "ambiguous"):
            self.record(task_id=self.task["id"])
        self.assertEqual(before, self.shared_images())

    def test_multiple_archived_aliases_require_unambiguous_uuid(self):
        self.record()
        self.close()
        replacement = self.brain.create_task(self.root, self.task["external_id"],
                                             "Second harbor task", [], [], owner="local")
        self.brain.cancel_task(self.root, replacement["id"], actor="local")
        self.brain.compact(self.root)
        with self.assertRaisesRegex(self.brain.BrainError, "ambiguous"):
            self.record()
        self.assertTrue(self.record(task_id=self.task["id"])["replayed"])

    def test_delayed_archived_replay_preserves_person_correction(self):
        saved = self.record()
        current = self.brain.find_record(self.root, saved["records"][0]["id"])[1]
        self.brain.update_record(
            self.root, current["id"], expected_revision=current["revision"],
            progress="Harbor owners must be explicitly confirmed.", next_steps=[], files=[],
            sources=[], actor="local", reverify=True, attestation="person",
            reason="A person corrected the harbor rule",
        )
        self.close()
        self.source.unlink()
        before = self.shared_images()
        reply = self.record()
        self.assertEqual(["archived-differs"], [r["state"] for r in reply["records"]])
        self.assertEqual(before, self.shared_images())

    def test_receipted_removed_learning_stays_missing(self):
        saved = self.record()
        path = self.brain.find_record(self.root, saved["records"][0]["id"])[0]
        path.unlink()
        self.brain.rebuild_indexes(self.root)
        self.source.unlink()
        before = self.shared_images()
        reply = self.record()
        self.assertEqual(["missing"], [r["state"] for r in reply["records"]])
        self.assertEqual(before, self.shared_images())

    def test_cli_auto_revision_replays_archived_task_and_missing_source(self):
        self.record()
        self.close()
        self.source.unlink()
        request = self.root / "harbor-result.json"
        request.write_text(json.dumps(self.data))
        argv = ["context.py", "--root", str(self.root), "--owner", "local", "record-result",
                "--task-id", self.task["id"], "--result-id", "harbor-run", "--revision", "auto",
                "--input", str(request), "--json"]
        out, err = io.StringIO(), io.StringIO()
        before = self.shared_images()
        with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            self.assertEqual(0, self.cli.main(), err.getvalue())
        self.assertTrue(json.loads(out.getvalue())["replayed"])
        self.assertEqual(before, self.shared_images())

    def test_mcp_forwards_exact_archive_replay_but_refuses_new_content(self):
        self.record()
        self.close()
        self.source.unlink()
        memory = self.memory()
        payload = {"task_id": self.task["id"], "result_id": "harbor-run", "revision": 1,
                   **self.data}
        before = self.shared_images()
        self.assertTrue(memory.call("memory_record_result", payload)["replayed"])
        with self.assertRaisesRegex(self.brain.BrainError, "different content"):
            memory.call("memory_record_result", {**payload, "progress": "Different harbor result"})
        self.assertEqual(before, self.shared_images())

    def test_mcp_requires_numeric_observed_revision_and_preserves_stale_cas(self):
        memory = self.memory()
        payload = {"task_id": self.task["id"], "result_id": "harbor-run", "revision": 1,
                   **self.data}
        before = self.shared_images()
        for revision in ("auto", "1", True, 0, 1.0):
            with self.subTest(revision=revision), self.assertRaisesRegex(self.brain.BrainError, "positive integer"):
                memory.call("memory_record_result", {**payload, "revision": revision})
            self.assertEqual(before, self.shared_images())
        saved = memory.call("memory_record_result", payload)
        self.assertFalse(saved["replayed"])
        before = self.shared_images()
        with self.assertRaisesRegex(self.brain.BrainError, "Stale task revision"):
            memory.call("memory_record_result", {**payload, "result_id": "new-harbor-run"})
        self.assertEqual(before, self.shared_images())

    def test_matching_receipt_cannot_bypass_static_source_policy(self):
        data = copy.deepcopy(self.data)
        data["learnings"][0]["sources"] = ["../foreign.md"]
        normalized = {key: data[key] for key in ("progress", "next_steps", "learnings")}
        digest = self.results.request_digest(self.task["id"], normalized)
        receipt = self.results._receipt(self.root, self.task["id"], "harbor-run")
        receipt.parent.mkdir(parents=True, exist_ok=True)
        receipt.write_text(json.dumps({"digest": digest}))
        before = self.shared_images()
        with self.assertRaises(self.brain.BrainError):
            self.record(data=data)
        self.assertEqual(before, self.shared_images())

    def test_auto_revision_resolves_task_while_mutation_lock_is_held(self):
        find = self.results.find_record
        observed = []
        def guarded_find(*args, **kwargs):
            observed.append(True)
            self.assertGreater(self.brain._LOCK_STATE.depths.get(
                str((self.root / "project-brain/local/runtime.lock").resolve()), 0), 0)
            return find(*args, **kwargs)
        with mock.patch.object(self.results, "find_record", side_effect=guarded_find):
            reply = self.record(revision="auto")
        self.assertFalse(reply["replayed"])
        self.assertTrue(observed)


if __name__ == "__main__":
    unittest.main()
