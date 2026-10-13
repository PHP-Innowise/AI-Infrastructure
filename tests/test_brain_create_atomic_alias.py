"""Active alias creation serializes lookup and canonical writes together."""

from __future__ import annotations

import contextlib
import importlib
import json
import multiprocessing
import os
import queue
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = Path(os.environ.get(
    "BRAIN_ALIAS_RUNTIME_SCRIPTS", str(ROOT / "PHP Core/memory-bank/scripts")
))


def load_runtime(scripts: Path):
    sys.path.insert(0, str(scripts))
    try:
        return importlib.import_module("brain_runtime")
    finally:
        sys.path.pop(0)


def create(brain, repository: Path, alias: str, kind: str = "finding", sources=None):
    return brain.create_record(
        repository, kind, alias, "Cobalt storage rule", [],
        ["specs/authority.md"] if sources is None else sources, owner="alice",
    )


def process_creator(scripts: str, repository: str, messages) -> None:
    """Signal before the real file lock; never intercept lookup or writes."""
    brain = load_runtime(Path(scripts))
    real_lock = brain.mutation_lock

    @contextlib.contextmanager
    def observed_lock(root):
        messages.put(("entered", None))
        with real_lock(root):
            yield

    brain.mutation_lock = observed_lock
    try:
        record = create(brain, Path(repository), "ATOMIC-ALIAS")
    except brain.BrainError as error:
        messages.put(("rejected", str(error)))
    except Exception as error:
        messages.put(("unexpected", type(error).__name__))
    else:
        messages.put(("created", record["id"]))


class CreateAliasAtomicityTest(unittest.TestCase):
    def setUp(self) -> None:
        self.brain = load_runtime(SCRIPTS)
        self.temporary = tempfile.TemporaryDirectory(prefix="brain-alias-atomic-")
        self.addCleanup(self.temporary.cleanup)
        self.repository = Path(self.temporary.name)
        source = self.repository / "specs/authority.md"
        source.parent.mkdir()
        source.write_text("# Authority\n\nCobalt storage rule.\n", encoding="utf-8")

    def records(self):
        return list(self.brain.iter_records(self.repository))

    def canonical_bytes(self):
        return {
            str(member.relative_to(self.repository)): member.read_bytes()
            for member in (self.repository / "project-brain").rglob("*")
            if member.is_file() and "local" not in member.parts
        }

    def assert_index_ids(self, expected):
        active, _ = self.brain.index_paths(self.repository)
        self.assertEqual(set(expected), {item["id"] for item in json.loads(active.read_text())})

    def test_two_processes_same_alias_have_one_canonical_record(self):
        context = multiprocessing.get_context("spawn")
        messages = context.Queue()
        processes = [context.Process(
            target=process_creator, args=(str(SCRIPTS), str(self.repository), messages),
        ) for _ in range(2)]
        try:
            with self.brain.mutation_lock(self.repository):
                for process in processes:
                    process.start()
                # Before the fix both lookups have completed. After the fix
                # neither lookup has run: the real lock is still held here.
                for _ in processes:
                    self.assertEqual(("entered", None), messages.get(timeout=15))
            outcomes = [messages.get(timeout=15) for _ in processes]
            for process in processes:
                process.join(timeout=15)
                self.assertFalse(process.is_alive())
                self.assertEqual(0, process.exitcode)
        finally:
            # Release the parent lock before joining or terminating workers.
            for process in processes:
                if process.pid is not None:
                    if process.is_alive():
                        process.terminate()
                    process.join(timeout=5)
                    if process.is_alive():
                        process.kill()
                        process.join(timeout=5)
                    self.assertFalse(process.is_alive())
                    process.close()
            messages.close()
            messages.join_thread()
        self.assertEqual(["created", "rejected"], sorted(item[0] for item in outcomes))
        rejection = next(value for kind, value in outcomes if kind == "rejected")
        self.assertEqual("Project Brain record already exists: ATOMIC-ALIAS", rejection)
        records = self.records()
        self.assertEqual(1, len(records))
        _, record, body = records[0]
        self.assertEqual("ATOMIC-ALIAS", record["external_id"])
        self.assertEqual(1, record["revision"])
        self.assertIn("Cobalt storage rule", body)
        self.assert_index_ids([record["id"]])

    def test_concurrent_different_aliases_keep_both_records(self):
        brain = self.brain
        real_lock = brain.mutation_lock
        entered = [threading.Event(), threading.Event()]
        outcomes = queue.Queue()
        worker_number = threading.local()

        @contextlib.contextmanager
        def observed_lock(root):
            entered[worker_number.value].set()
            with real_lock(root):
                yield

        def worker(number):
            worker_number.value = number
            try:
                outcomes.put(create(brain, self.repository, "INDEPENDENT-" + str(number)))
            except Exception as error:
                outcomes.put(error)

        workers = [threading.Thread(target=worker, args=(number,)) for number in range(2)]
        with mock.patch.object(brain, "mutation_lock", observed_lock):
            try:
                with real_lock(self.repository):
                    for thread in workers:
                        thread.start()
                    for event in entered:
                        self.assertTrue(event.wait(timeout=10))
            finally:
                for thread in workers:
                    if thread.ident is not None:
                        thread.join(timeout=10)
                        self.assertFalse(thread.is_alive())
        records = [outcomes.get(timeout=1) for _ in workers]
        self.assertTrue(all(isinstance(record, dict) for record in records), records)
        self.assertEqual(2, len(self.records()))
        self.assert_index_ids(record["id"] for record in records)

    def test_active_namespace_is_cross_type_and_precedes_source_validation(self):
        record = create(self.brain, self.repository, "SHARED-ALIAS")
        before = self.canonical_bytes()
        with self.assertRaisesRegex(self.brain.BrainError, "^Project Brain record already exists: SHARED-ALIAS$"):
            create(self.brain, self.repository, "SHARED-ALIAS", kind="decision", sources=["missing.md"])
        self.assertEqual(before, self.canonical_bytes())
        self.assert_index_ids([record["id"]])

    def test_archived_only_alias_can_be_reused_without_rewriting_archive(self):
        record = create(self.brain, self.repository, "REUSABLE-ALIAS")
        active, _, _ = self.brain.find_record(self.repository, record["id"])
        archived = self.repository / "project-brain/archive/finding" / active.name
        archived.parent.mkdir(parents=True)
        active.rename(archived)  # Physical archive fixture; not a compaction test.
        self.brain.rebuild_indexes(self.repository)
        archived_bytes = archived.read_bytes()
        replacement = create(self.brain, self.repository, "REUSABLE-ALIAS")
        self.assertNotEqual(record["id"], replacement["id"])
        self.assertEqual(archived_bytes, archived.read_bytes())
        self.assertEqual(replacement["id"], self.brain.find_record(self.repository, "REUSABLE-ALIAS")[1]["id"])
        with self.assertRaisesRegex(self.brain.BrainError, "identifier is ambiguous"):
            self.brain.find_record(self.repository, "REUSABLE-ALIAS", include_archive=True)
        self.assert_index_ids([replacement["id"]])

    def test_existing_ambiguity_is_preserved_without_writes(self):
        create(self.brain, self.repository, "AMBIGUOUS-ALIAS")
        second = create(self.brain, self.repository, "SECOND-ALIAS")
        location, metadata, body = self.brain.find_record(self.repository, second["id"])
        metadata["external_id"] = "AMBIGUOUS-ALIAS"
        location.write_text(self.brain.render_markdown_record(metadata, body), encoding="utf-8")
        before = self.canonical_bytes()
        with self.assertRaisesRegex(self.brain.BrainError, "identifier is ambiguous"):
            create(self.brain, self.repository, "AMBIGUOUS-ALIAS", sources=["missing.md"])
        self.assertEqual(before, self.canonical_bytes())
        self.assertEqual(2, len(self.records()))

    def test_failed_write_rolls_back_and_releases_alias_for_next_attempt(self):
        with mock.patch.object(self.brain, "rebuild_indexes", side_effect=OSError("fixture index failure")):
            with self.assertRaisesRegex(OSError, "fixture index failure"):
                create(self.brain, self.repository, "RETRY-ALIAS")
        self.assertEqual([], self.records())
        record = create(self.brain, self.repository, "RETRY-ALIAS")
        self.assertEqual(1, len(self.records()))
        self.assert_index_ids([record["id"]])

    def test_existing_outer_mutation_lock_remains_reentrant(self):
        with self.brain.mutation_lock(self.repository):
            record = create(self.brain, self.repository, "NESTED-ALIAS")
        self.assertEqual(record["id"], self.brain.find_record(self.repository, "NESTED-ALIAS")[1]["id"])
        self.assert_index_ids([record["id"]])


if __name__ == "__main__":
    unittest.main()
