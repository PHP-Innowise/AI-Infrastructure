"""Knowledge UI contracts exercised against an owned copy of the stdlib runtime."""

from datetime import date, timedelta
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import knowledge, sessions
from harness.knowledge import KnowledgeManager


EDITION = Path(__file__).resolve().parents[1] / "PHP Core"


def install_knowledge_fixture(project):
    """Copy executable/runtime policy inputs, never a user's local index or records."""
    for relative in ("memory-bank/scripts", "memory-bank/templates", "project-brain/templates",
                     "project-brain/config", "project-brain/schemas", "project-brain/indexes"):
        shutil.copytree(EDITION / relative, project / relative,
                        ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
    for relative in ("memory-bank/README.md", "memory-bank/INDEX.md", "project-brain/README.md",
                     "project-brain/PROTOCOL.md"):
        shutil.copyfile(EDITION / relative, project / relative)
    (project / "memory-bank/chunks").mkdir()
    (project / "specs").mkdir()
    (project / "specs/authority.md").write_text(
        "# Cobalt authority\n\nThe cobalt allocation rule requires one owner.\n", encoding="utf-8")
    subprocess.run(["git", "init", "--quiet", str(project)], check=True, capture_output=True)


def metadata(path):
    return json.loads(path.read_text(encoding="utf-8")[4:].split("\n---\n", 1)[0])


class KnowledgeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        discovery = patch.object(sessions.providers, "discover_providers", return_value=[])
        discovery.start()
        self.addCleanup(discovery.stop)
        worker = patch.object(sessions.Sessions, "_worker", return_value=None)
        worker.start()
        self.addCleanup(worker.stop)
        self.store = sessions.Sessions(self.root / "state", [self.project])
        self.addCleanup(self.store.close)
        self.manager = KnowledgeManager(self.store)
        self.addCleanup(self.manager.close)
        self.project_id = next(iter(self.store.projects))
        native = patch.object(sessions.providers, "build_command",
                              side_effect=AssertionError("Knowledge must never call a model CLI"))
        self.native = native.start()
        self.addCleanup(native.stop)

    def install(self):
        install_knowledge_fixture(self.project)

    def run_action(self, action, **fields):
        response = self.manager.run(self.project_id, {"action": action, **fields})
        self.assertTrue(response["ok"], response)
        self.assertEqual(response["action"], action)
        return response["result"]

    @staticmethod
    def snapshot(root):
        return {str(path.relative_to(root)): (path.stat().st_mtime_ns, path.read_bytes())
                for path in root.rglob("*") if path.is_file() and not path.is_symlink()}

    def chunk(self, identifier="MEM-20260101-aaaaaaaa", slug="cobalt-rule", **changes):
        today = date.today()
        fields = {"id": identifier, "title": "Cobalt allocation rule", "type": "convention",
                  "status": "active", "scope": ["application"], "tags": ["allocation"],
                  "created": "2020-01-01", "last_verified": today.isoformat(),
                  "review_after": (today + timedelta(days=365)).isoformat(),
                  "sources": ["specs/authority.md"], "supersedes": [], "superseded_by": None}
        fields.update(changes)
        path = self.project / "memory-bank/chunks" / f"{identifier}-{slug}.md"
        path.write_text("---\n" + json.dumps(fields, indent=2) +
                        "\n---\n\n# Cobalt rule\n\nThe cobalt allocation rule requires one owner.\n",
                        encoding="utf-8")
        return path

    def test_absent_and_installed_roots_browse_without_mutation(self):
        empty = self.manager.info(self.project_id)
        self.assertEqual(empty["banks"], [])
        self.assertIsNone(empty["bank_id"])
        self.assertFalse(empty["runtime_available"])
        self.assertEqual(self.manager.brain(self.project_id)["entries"], [])
        self.assertEqual(list(self.project.iterdir()), [])
        with self.assertRaises(sessions.SessionError):
            self.manager.run(self.project_id, {"action": "start", "task_id": "TASK-1", "goal": "A task"})
        self.assertEqual(list(self.project.iterdir()), [])
        self.install()
        before = self.snapshot(self.project)
        info = self.manager.info(self.project_id)
        self.assertEqual(info["bank_id"], "memory-bank")
        self.assertEqual(info["root"], "")
        self.assertEqual(info["mode"], "governed")
        self.assertTrue(info["runtime_available"])
        self.assertTrue(info["brain_available"])
        self.manager.brain(self.project_id)
        self.assertEqual(self.snapshot(self.project), before)
        self.assertFalse((self.project / "memory-bank/local/context.db").exists())

    def test_real_maintenance_search_and_memory_lifecycle_preserve_evidence(self):
        self.install()
        chunk = self.chunk()
        self.run_action("reindex-bank")
        self.run_action("index")
        self.assertTrue((self.project / "memory-bank/local/context.db").is_file())
        self.run_action("status")
        self.run_action("validate")
        self.run_action("bank-audit")
        found = self.run_action("search", query="cobalt allocation", layer="semantic")
        self.assertIn("memory-bank/chunks/", json.dumps(found))
        self.assertIn("Cobalt", json.dumps(found))
        before_body = chunk.read_text(encoding="utf-8").split("\n---\n", 1)[1]
        review_after = (date.today() + timedelta(days=30)).isoformat()
        self.run_action("bank-reverify", memory_id="MEM-20260101-aaaaaaaa", review_after=review_after)
        verified = metadata(chunk)
        self.assertEqual(verified["review_after"], review_after)
        digest = verified["source_digests"][0]
        self.assertEqual(digest["path"], "specs/authority.md")
        self.assertEqual(digest["sha256"], hashlib.sha256((self.project / digest["path"]).read_bytes()).hexdigest())
        yesterday = (date.today() - timedelta(days=1)).isoformat()
        retired = self.run_action("bank-retire", memory_id="MEM-20260101-aaaaaaaa",
                                  valid_to=yesterday, reason="Replaced allocation process")
        self.assertEqual(retired["status"], "archived")
        self.assertEqual(metadata(chunk)["valid_to"], yesterday)
        self.assertEqual(chunk.read_text(encoding="utf-8").split("\n---\n", 1)[1], before_body)
        self.assertIn("archived", (self.project / "memory-bank/INDEX.md").read_text())
        self.run_action("compact")
        self.native.assert_not_called()

    def test_task_lifecycle_uses_numeric_cas_and_reads_leave_authority_unchanged(self):
        self.install()
        task = self.run_action("start", task_id="TASK-COBALT", goal="Apply the cobalt authority rule",
                               files=["specs/authority.md"], sources=["specs/authority.md"])
        record_id = task["task_uuid"]
        path = self.project / "project-brain/dynamic/tasks" / f"{record_id}.md"
        self.assertEqual(metadata(path)["revision"], 1)
        listing = self.manager.brain(self.project_id)
        entry = next(item for item in listing["entries"] if item.get("id") == record_id and item.get("type") == "task")
        before = self.snapshot(self.project)
        detail = self.manager.brain(self.project_id, path=entry["path"])
        self.assertEqual(detail["metadata"]["revision"], 1)
        self.assertIn("Apply the cobalt authority rule", detail["content"])
        self.assertEqual(self.snapshot(self.project), before)
        changed = self.run_action("brain-update", record_id=record_id, revision=1,
                                  progress="Verified the canonical source", next_steps=["Finish verification"],
                                  phase="implementation")
        self.assertEqual(changed["revision"], 2)
        before = self.snapshot(self.project / "project-brain")
        stale = self.manager.run(self.project_id, {"action": "brain-update", "record_id": record_id,
                                                  "revision": 1, "progress": "Stale change"})
        self.assertFalse(stale["ok"])
        self.assertIn("revision", stale["error"].lower())
        self.assertEqual(self.snapshot(self.project / "project-brain"), before)
        self.run_action("complete", task_id="TASK-COBALT", revision=2,
                        outcome="The cobalt authority rule is verified", verification=["Offline fixture check"],
                        sources=["specs/authority.md"])
        self.assertEqual(metadata(path)["status"], "completed")
        self.assertGreater(metadata(path)["revision"], 2)
        self.native.assert_not_called()

    def test_selected_nested_bank_executes_and_reads_in_its_own_edition_root(self):
        self.install()
        edition = self.project / "PHP Core"
        edition.mkdir()
        install_knowledge_fixture(edition)
        before = self.snapshot(self.project / "memory-bank")
        info = self.manager.info(self.project_id, "PHP Core/memory-bank")
        self.assertEqual(info["root"], "PHP Core")
        self.assertEqual({bank["id"] for bank in info["banks"]}, {"memory-bank", "PHP Core/memory-bank"})
        response = self.manager.run(self.project_id, {"bank": "PHP Core/memory-bank", "action": "start",
                                                      "task_id": "TASK-NESTED", "goal": "Verify the nested edition",
                                                      "sources": ["specs/authority.md"]})
        self.assertTrue(response["ok"], response)
        record_id = response["result"]["task_uuid"]
        relative = f"dynamic/tasks/{record_id}.md"
        self.assertTrue((edition / "project-brain" / relative).is_file())
        self.assertFalse((self.project / "project-brain" / relative).exists())
        detail = self.manager.brain(self.project_id, "PHP Core/memory-bank", relative)
        self.assertEqual(detail["metadata"]["external_id"], "TASK-NESTED")
        with self.assertRaises(sessions.SessionError):
            self.manager.brain(self.project_id, "memory-bank", relative)
        self.assertEqual(self.snapshot(self.project / "memory-bank"), before)

    def test_retrieve_refresh_and_rebind_use_the_real_task_and_fresh_canonical_source(self):
        self.install()
        started = self.run_action("start", task_id="TASK-RETRIEVAL", goal="Verify cobalt allocation",
                                  sources=["specs/authority.md"])
        capsule = self.run_action("retrieve", task_id="TASK-RETRIEVAL", query="cobalt allocation",
                                  paths=["specs/authority.md"])
        self.assertEqual(capsule["task_uuid"], started["task_uuid"])
        self.assertIn("one owner", json.dumps(capsule))
        self.assertTrue((self.project / capsule["manifest"]).is_file())
        source = self.project / "specs/authority.md"
        source.write_text(source.read_text().replace("one owner", "two owners"))
        refreshed = self.run_action("refresh", task_id="TASK-RETRIEVAL", query="cobalt allocation")
        self.assertIn("two owners", json.dumps(refreshed))
        mismatch = self.manager.run(self.project_id, {"action": "rebind", "task_id": "TASK-OTHER",
                                                     "record_id": started["task_uuid"]})
        self.assertFalse(mismatch["ok"])
        record = self.project / "project-brain/dynamic/tasks" / f"{started['task_uuid']}.md"
        before = record.read_bytes()
        (self.project / "memory-bank/local/context.db").unlink()
        rebound = self.run_action("rebind", task_id="TASK-RETRIEVAL", record_id=started["task_uuid"])
        self.assertIn(started["task_uuid"], json.dumps(rebound))
        self.assertFalse(rebound["already_bound"])
        self.assertEqual(record.read_bytes(), before)
        retried = self.run_action("retrieve", task_id="TASK-RETRIEVAL", query="cobalt allocation")
        self.assertEqual(retried["task_uuid"], started["task_uuid"])
        self.native.assert_not_called()

    def test_durable_promotion_requires_eligible_sources_independent_review_and_pinned_revision(self):
        self.install()

        def decision(identifier, authority="verified", privacy="team"):
            record = self.run_action("brain-create", record_type="decision", external_id=identifier,
                                     title="Cobalt allocation decision", authority=authority, privacy=privacy,
                                     sources=["specs/authority.md"])
            return self.run_action("brain-update", record_id=record["id"], revision=record["revision"],
                                   transition="accepted", progress="Allocate exactly one owner to prevent duplicate work")

        task = self.run_action("start", task_id="TASK-NOT-DURABLE", goal="Session bookkeeping")
        unverified = decision("D-UNVERIFIED", authority="observed")
        private = decision("D-PRIVATE", privacy="private")
        for record_id in (task["task_uuid"], unverified["id"], private["id"]):
            with self.subTest(ineligible_record=record_id):
                rejected = self.manager.run(self.project_id, {"action": "promote-propose", "source_ids": [record_id],
                    "title": "Rejected proposal", "content": "A consequence worth remembering"})
                self.assertFalse(rejected["ok"], rejected)
        self.assertEqual(list((self.project / "project-brain/control/promotions").glob("*.json")), [])
        record = decision("D-ELIGIBLE")
        source = self.project / "specs/authority.md"
        original = source.read_bytes()
        source.write_bytes(original + b"\nChanged canonical source\n")
        stale = self.manager.run(self.project_id, {"action": "promote-propose", "source_ids": [record["id"]],
            "title": "Stale proposal", "content": "A consequence worth remembering"})
        self.assertFalse(stale["ok"], stale)
        source.write_bytes(original)
        proposal = self.run_action("promote-propose", source_ids=[record["id"]], title="One allocation owner",
                                   content="REVIEWED DURABLE CONSEQUENCE: allocate exactly one owner")
        proposal_path = self.project / "project-brain/control/promotions" / f"{proposal['id']}.json"
        before = proposal_path.read_bytes()
        self_review = self.manager.run(self.project_id, {"action": "promote-review", "promotion_id": proposal["id"],
                                                        "reviewer": proposal["proposer"]})
        self.assertFalse(self_review["ok"])
        self.assertEqual(proposal_path.read_bytes(), before)
        self.run_action("promote-review", promotion_id=proposal["id"], reviewer="independent-fixture-reviewer")
        changed = self.run_action("brain-update", record_id=record["id"], revision=record["revision"],
                                  progress="Assign a single owner and verify the allocation boundary")
        denied = self.manager.run(self.project_id, {"action": "promote-apply", "promotion_id": proposal["id"]})
        self.assertFalse(denied["ok"], denied)
        self.assertEqual(list((self.project / "memory-bank/chunks").glob("*.md")), [])
        current = self.run_action("promote-propose", source_ids=[changed["id"]], title="Reviewed single owner",
                                  content="REVIEWED DURABLE CONSEQUENCE: verify one allocation owner")
        self.run_action("promote-review", promotion_id=current["id"], reviewer="independent-fixture-reviewer")
        applied = self.run_action("promote-apply", promotion_id=current["id"])
        self.assertEqual(applied["status"], "applied")
        chunks = list((self.project / "memory-bank/chunks").glob("*.md"))
        self.assertEqual(len(chunks), 1)
        self.assertEqual(metadata(chunks[0])["id"], applied["destination_memory_id"])
        self.assertIn("REVIEWED DURABLE CONSEQUENCE", chunks[0].read_text())
        self.assertIn("specs/authority.md", metadata(chunks[0])["sources"])
        self.assertIn(applied["destination_memory_id"], (self.project / "memory-bank/INDEX.md").read_text())
        self.native.assert_not_called()

    def test_export_filters_private_records_and_local_runtime_files(self):
        self.install()
        self.chunk()
        self.chunk("MEM-20260101-bbbbbbbb", "archived-rule", status="archived", valid_to="2020-02-01")
        public = self.run_action("brain-create", record_type="finding", external_id="F-PUBLIC",
                                 title="Shareable allocation finding", sources=["specs/authority.md"])
        private = self.run_action("brain-create", record_type="finding", external_id="F-PRIVATE",
                                  title="Confidential fixture finding", sources=["specs/authority.md"])
        private_path = self.project / "project-brain/dynamic/findings" / f"{private['id']}.md"
        text = private_path.read_text(encoding="utf-8")
        fields = metadata(private_path)
        fields["privacy"] = "private"
        private_path.write_text("---\n" + json.dumps(fields, indent=2) + "\n---\n" +
                                text.split("\n---\n", 1)[1] + "\nPRIVATE-FIXTURE-BODY\n", encoding="utf-8")
        (self.project / "memory-bank/local").mkdir(exist_ok=True)
        (self.project / "memory-bank/local/private-note.md").write_text("LOCAL-FIXTURE-BODY")
        self.run_action("reindex-bank")
        response = self.manager.run(self.project_id, {"action": "export"})
        self.assertTrue(response["ok"], response)
        self.assertNotIn("destination", response["result"])
        data = self.manager.download(response["download_id"])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            names = archive.namelist()
            contents = b"\n".join(archive.read(name) for name in names)
            self.assertIn(b"Shareable allocation finding", contents)
            self.assertNotIn(b"PRIVATE-FIXTURE-BODY", contents)
            self.assertNotIn(b"LOCAL-FIXTURE-BODY", contents)
            self.assertFalse(any("context.db" in name or "/local/" in name or "/scripts/" in name for name in names))
            self.assertFalse(any("bbbbbbbb" in name for name in names))
            manifest = json.loads(archive.read(next(name for name in names if name.endswith("MANIFEST.json"))))
            self.assertIn(public["id"], json.dumps(manifest["included"]))
            self.assertTrue(any("privacy" in item["reason"] for item in manifest["excluded"]))
        wider = self.manager.run(self.project_id, {"action": "export", "include_superseded": True})
        self.assertTrue(wider["ok"], wider)
        with zipfile.ZipFile(io.BytesIO(self.manager.download(wider["download_id"]))) as archive:
            self.assertTrue(any("bbbbbbbb" in name for name in archive.namelist()))
        with self.assertRaises(sessions.SessionError):
            self.manager.download("../unknown")

    def test_invalid_arguments_and_unsafe_tree_entries_fail_before_runtime_execution(self):
        self.install()
        valid_start = {"action": "start", "task_id": "TASK-1", "goal": "Verify the fixture"}
        invalid = [
            {"action": "unknown"}, {"action": "status", "extra": True},
            {**valid_start, "task_id": "../escape"}, {**valid_start, "files": ["../outside.md"]},
            {**valid_start, "sources": "specs/authority.md"},
            {"action": "brain-update", "record_id": "not-a-record", "revision": 1, "progress": "Update"},
            {"action": "complete", "task_id": "TASK-1", "revision": "auto", "outcome": "Done"},
            {"action": "complete", "task_id": "TASK-1", "revision": True, "outcome": "Done"},
            {"action": "bank-reverify", "memory_id": "../MEM-0001", "review_after": "2026-01-01"},
            {"action": "bank-retire", "memory_id": "MEM-0001", "valid_to": "2026-02-30"},
            {"action": "export", "include_archive": "yes"},
        ]
        before = self.snapshot(self.project)
        with patch.object(self.manager, "_execute", side_effect=AssertionError("Invalid input reached CLI")) as execute:
            for data in invalid:
                with self.subTest(data=data), self.assertRaises(sessions.SessionError):
                    self.manager.run(self.project_id, data)
            for bank in ("../memory-bank", "/memory-bank", "memory-bank\\chunks", "memory-bank\0"):
                with self.subTest(bank=bank), self.assertRaises(sessions.SessionError):
                    self.manager.info(self.project_id, bank)
            for path in ("../config/runtime.json", "/etc/passwd", "config/runtime.json", "scripts/validate.py",
                         "dynamic\\tasks\\record.md", "dynamic/tasks/record.md\0"):
                with self.subTest(path=path), self.assertRaises(sessions.SessionError):
                    self.manager.brain(self.project_id, path=path)
            for project_id in ("unknown", [], {}):
                with self.subTest(project_id=project_id), self.assertRaises(sessions.SessionError):
                    self.manager.info(project_id)
            execute.assert_not_called()
        self.assertEqual(self.snapshot(self.project), before)
        outside = self.root / "outside.md"
        outside.write_text("OUTSIDE-FIXTURE")
        unsafe = self.project / "memory-bank/chunks/unsafe.md"
        for kind in ("symlink", "hardlink"):
            with self.subTest(kind=kind):
                if kind == "symlink":
                    unsafe.symlink_to(outside)
                else:
                    os.link(outside, unsafe)
                try:
                    with patch.object(self.manager, "_execute", side_effect=AssertionError("Unsafe tree reached CLI")):
                        with self.assertRaises(sessions.SessionError):
                            self.manager.run(self.project_id, {"action": "index"})
                    self.assertEqual(outside.read_text(), "OUTSIDE-FIXTURE")
                finally:
                    unsafe.unlink()
        record = self.project / "project-brain/dynamic/findings/unsafe.md"
        record.parent.mkdir(parents=True)
        for kind in ("symlink", "hardlink"):
            with self.subTest(brain_record=kind):
                if kind == "symlink":
                    record.symlink_to(outside)
                else:
                    os.link(outside, record)
                try:
                    self.assertFalse(any(item["path"] == "dynamic/findings/unsafe.md"
                                         for item in self.manager.brain(self.project_id)["entries"]))
                    with self.assertRaises(sessions.SessionError):
                        self.manager.brain(self.project_id, path="dynamic/findings/unsafe.md")
                    self.assertEqual(outside.read_text(), "OUTSIDE-FIXTURE")
                finally:
                    record.unlink()

    @unittest.skipUnless(Path("/proc").is_dir(), "Owned process cleanup checks require procfs")
    def test_timeout_output_limit_and_close_stop_the_owned_runtime_process_tree(self):
        self.install()
        script = self.project / "memory-bank/scripts/context.py"
        for mode in ("timeout", "overflow", "close"):
            with self.subTest(mode=mode):
                marker = self.root / f"{mode}-processes.json"
                script.write_text(
                    "import json, os, pathlib, signal, subprocess, sys, time\n"
                    "signal.signal(signal.SIGTERM, signal.SIG_IGN)\n"
                    "child = subprocess.Popen([sys.executable, '-c', 'import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(120)'])\n"
                    f"pathlib.Path({str(marker)!r}).write_text(json.dumps([os.getpid(), child.pid]))\n"
                    "print('SENSITIVE-STDERR-FIXTURE', file=sys.stderr, flush=True)\n"
                    + ("print('x' * 65536, flush=True)\n" if mode == "overflow" else "")
                    + "time.sleep(120)\n", encoding="utf-8")
                manager = KnowledgeManager(self.store)
                self.addCleanup(manager.close)
                outcomes = []

                def run():
                    try:
                        outcomes.append(manager.run(self.project_id, {"action": "status"}))
                    except Exception as error:
                        outcomes.append(error)

                with patch.object(knowledge, "EXECUTION_TIMEOUT", 1), patch.object(
                        knowledge, "OUTPUT_LIMIT", 128 if mode == "overflow" else 4 * 1024 * 1024):
                    thread = threading.Thread(target=run, daemon=True)
                    thread.start()
                    deadline = time.monotonic() + 5
                    while not marker.exists() and time.monotonic() < deadline and thread.is_alive():
                        time.sleep(.01)
                    self.assertTrue(marker.exists(), outcomes)
                    if mode == "close":
                        manager.close()
                    thread.join(6)
                    self.assertFalse(thread.is_alive(), "Knowledge operation did not release its lock")
                self.assertEqual(len(outcomes), 1)
                self.assertNotIn("SENSITIVE-STDERR-FIXTURE", str(outcomes[0]))
                if mode != "close":
                    self.assertFalse(outcomes[0]["ok"])
                    self.assertIn("changes may have been applied", outcomes[0]["error"])
                else:
                    self.assertTrue(isinstance(outcomes[0], sessions.SessionError) or not outcomes[0]["ok"])
                self.assertIsNone(manager.process)
                for pid in json.loads(marker.read_text()):
                    deadline = time.monotonic() + 3
                    while time.monotonic() < deadline:
                        try:
                            stopped = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] == "Z"
                        except FileNotFoundError:
                            stopped = True
                        if stopped:
                            break
                        time.sleep(.02)
                    self.assertTrue(stopped, "Owned runtime descendant remained active")
                manager.close()


if __name__ == "__main__":
    unittest.main()
