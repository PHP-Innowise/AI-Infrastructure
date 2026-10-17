"""Fresh snapshot, attestation and failed-refresh regressions for memory."""
from __future__ import annotations

import copy
import contextlib
import importlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / "PHP Core/memory-bank/scripts"


class MemoryRuntimeReliabilityTests(unittest.TestCase):
    def setUp(self):
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.brain = importlib.import_module("brain_runtime")
            self.context = importlib.import_module("context")
            self.retrieval = importlib.import_module("context_retrieval")
        finally:
            sys.path.pop(0)
        temporary = tempfile.TemporaryDirectory(prefix="memory-reliability-")
        self.addCleanup(temporary.cleanup)
        self.repository = Path(temporary.name)
        source = self.repository / "specs/lantern.md"
        source.parent.mkdir()
        source.write_text("# Lantern\n\nLantern routing requires an immutable parcel.\n", encoding="utf-8")
        (self.repository / "memory-bank/chunks").mkdir(parents=True)

    def test_invalid_utf8_source_is_excluded_and_old_index_row_removed(self):
        public = json.loads((ROOT / 'tests/fixtures/memory_regression_remediation_public.json').read_text())['utf8']
        good, bad = self.repository / public['valid_path'], self.repository / public['bad_path']
        good.parent.mkdir(parents=True, exist_ok=True)
        bad.parent.mkdir(parents=True, exist_ok=True)
        good.write_text(public['valid_text'], encoding='utf-8')
        bad.write_text(public['before_text'], encoding='utf-8')
        connection = self.context.connect(self.repository / 'memory-bank/local/context.db')
        self.addCleanup(connection.close)
        self.context.index_repository(connection, self.repository)
        self.assertEqual(1, connection.execute('SELECT COUNT(*) FROM documents WHERE path=?', (public['bad_path'],)).fetchone()[0])
        bad.write_bytes(bytes.fromhex(public['invalid_cp1251_hex']))
        for _ in range(2):
            layers, result, warnings = self.context.refresh_layers(connection, self.repository)
            self.assertEqual([], warnings)
            self.assertTrue(all(state == 'updated' for state in layers.values()))
            self.assertIn(public['expected_excluded'], result['excluded'])
            self.assertEqual(public['expected_invalid_index_rows'], connection.execute(
                'SELECT COUNT(*) FROM documents WHERE path=?', (public['bad_path'],)).fetchone()[0])
            self.assertEqual(public['expected_valid_index_rows'], connection.execute(
                'SELECT COUNT(*) FROM documents WHERE path=?', (public['valid_path'],)).fetchone()[0])
        self.assertTrue(any(row['path'] == public['valid_path'] for row in
                            self.context.search_documents(connection, 'Compass', 5)))

    def create(self, kind="finding", alias="LANTERN-RECORD"):
        return self.brain.create_record(
            self.repository, kind, alias, "Lantern routing boundary", [],
            ["specs/lantern.md"], owner="alice", privacy="public",
            goal="Verify the immutable parcel boundary.",
        )

    def test_explicit_attestation_wins_over_markers_in_free_form_reasons(self):
        for explicit, embedded in (("person", "agent"), ("agent", "person")):
            with self.subTest(explicit=explicit):
                record = self.create(alias="LANTERN-" + explicit)
                verified = self.brain.update_record(
                    self.repository, record["id"], expected_revision=record["revision"],
                    progress=None, next_steps=[], files=[], sources=[], actor="alice",
                    authority="verified", attestation=explicit,
                    reason="Literal example [attestation:" + embedded + "]",
                )
                self.assertEqual(explicit, self.brain.record_attestation(verified))
                revised = self.brain.update_record(
                    self.repository, record["id"], expected_revision=verified["revision"],
                    progress="The parcel boundary was checked again.", next_steps=[],
                    files=[], sources=[], actor="alice", reverify=True, attestation=embedded,
                    reason="Previous example [attestation:" + explicit + "]",
                )
                self.assertEqual(embedded, self.brain.record_attestation(revised))

    def documents(self):
        return self.retrieval._brain_documents(self.repository, self.brain.load_config(self.repository))

    def test_record_hash_and_eligibility_use_the_same_snapshot(self):
        record = self.create()
        path, original, body = self.brain.find_record(self.repository, record["id"])
        text = path.read_text(encoding="utf-8")
        replacement = copy.deepcopy(original)
        replacement["privacy"] = "private"
        replacement["revision"] += 1

        def replace_after_read(repository, **_):
            # The file changes after the iterator read it. The index keeps the
            # image it read whole - eligibility, content and hash alike - and
            # never pairs one image's privacy with another's bytes.
            path.write_text(self.brain.render_markdown_record(replacement, body), encoding="utf-8")
            yield path, original, body, text

        with mock.patch.object(self.retrieval.brain_runtime, "iter_record_images", side_effect=replace_after_read):
            documents, metadata, excluded, links, _ = self.documents()
        self.assertEqual([original["title"]], [document[3] for document in documents])
        self.assertEqual((original["privacy"], self.retrieval._content_hash(text)), (metadata[0][2], metadata[0][6]))
        # The next pass reads the private image and leaves it out.
        documents, metadata, excluded, links, _ = self.documents()
        self.assertEqual([], documents)
        self.assertEqual([], metadata)
        self.assertEqual([], links)
        self.assertIn({"path": path.relative_to(self.repository).as_posix(), "reason": "private"}, excluded)

    def test_allowed_replacement_body_and_hash_share_one_image(self):
        record = self.create()
        path, original, body = self.brain.find_record(self.repository, record['id'])
        text = path.read_text(encoding='utf-8')
        replacement = copy.deepcopy(original)
        replacement.update(title='Lantern revised parcel boundary', revision=original['revision'] + 1)
        replacement_body = '# Lantern revised parcel boundary\n\nThe immutable parcel uses a new boundary.\n'
        image = self.brain.render_markdown_record(replacement, replacement_body)
        def replace_after_read(repository, **_):
            path.write_text(image, encoding='utf-8')
            yield path, original, body, text
        with mock.patch.object(self.retrieval.brain_runtime, 'iter_record_images', side_effect=replace_after_read):
            documents, metadata, _, _, _ = self.documents()
        self.assertEqual((original['title'], body), (documents[0][3], documents[0][5]))
        self.assertEqual(self.retrieval._content_hash(text), metadata[0][6])
        # Title, body and hash move together to the replacement on the next pass.
        documents, metadata, _, _, _ = self.documents()
        self.assertEqual((replacement['title'], replacement_body), (documents[0][3], documents[0][5]))
        self.assertEqual(self.retrieval._content_hash(image), metadata[0][6])

    def test_unreadable_handoff_is_excluded_without_failing_other_records(self):
        self.create()
        folder = self.repository / 'project-brain/control/handoffs'
        folder.mkdir(parents=True, exist_ok=True)
        handoff = folder / 'unreadable.md'
        handoff.write_text('Public placeholder', encoding='utf-8')
        original = Path.read_text
        def read(path, *args, **kwargs):
            if path == handoff:
                raise OSError('Fixture read failed')
            return original(path, *args, **kwargs)
        with mock.patch.object(Path, 'read_text', read):
            documents, _, excluded, _, _ = self.retrieval._brain_documents(
                self.repository, self.brain.load_config(self.repository))
        self.assertEqual(1, len(documents))
        self.assertIn({'path': 'project-brain/control/handoffs/unreadable.md',
                       'reason': 'task-filter-or-invalid'}, excluded)

    def test_handoff_parse_and_hash_share_the_captured_text(self):
        task = self.create(kind='task', alias='LANTERN-HANDOFF')
        path = self.brain.handoff_path(self.repository, task['id'])
        original = path.read_text(encoding='utf-8')
        _, body = self.brain.parse_markdown_record(path)
        parse = self.retrieval.parse_markdown_record
        def replace_after_parse(member, **kwargs):
            result = parse(member, **kwargs)
            if member == path:
                member.write_text(original + '\nNew lantern handoff image.\n', encoding='utf-8')
            return result
        with mock.patch.object(self.retrieval, 'parse_markdown_record', side_effect=replace_after_parse):
            documents, metadata, _, _, _ = self.retrieval._brain_documents(
                self.repository, self.brain.load_config(self.repository))
        relative = path.relative_to(self.repository).as_posix()
        self.assertEqual(body, next(row[5] for row in documents if row[0] == relative))
        self.assertEqual(self.retrieval._content_hash(original),
                         next(row[6] for row in metadata if row[0] == relative))
        self.assertNotEqual(self.retrieval._content_hash(original),
                            self.retrieval._content_hash(path.read_text(encoding='utf-8')))

    def test_failed_governed_refresh_does_not_retrieve_or_record_delivery(self):
        connection = self.context.connect(self.repository / "memory-bank/local/context.db")
        self.addCleanup(connection.close)
        warning = "Index refresh failed: unrelated public source is not valid UTF-8"
        with mock.patch.object(self.context, "refresh_layers", return_value=({}, {}, [warning])), \
                mock.patch.object(self.context, "governed_binding", side_effect=self.context.ContextError("Working task not found")), \
                mock.patch.object(self.context, "retrieve", side_effect=AssertionError("retrieval after failed refresh")), \
                mock.patch.object(self.context, "prompt_path_seeds", side_effect=AssertionError("seeds after failed refresh")):
            capsule = self.context.assemble_capsule(
                connection, self.repository, mode="governed", query="lantern immutable parcel",
                task_id="LANTERN-WARMING", limit=5, ephemeral=True, allow_unprovisioned=True,
                automatic_links=True, prompt="Inspect specs/lantern.md", render=True,
            )
        for layer in ("procedural", "semantic", "episodic"):
            self.assertEqual([], capsule[layer])
        self.assertEqual([warning], capsule["warnings"])
        self.assertIsNone(capsule.get("manifest"))
        self.assertEqual("warming", capsule["kind"])
        self.assertFalse(list(self.repository.glob("**/*manifest*.json")))

    def test_failed_refresh_with_query_passes_warnings_without_a_second_index(self):
        connection = self.context.connect(self.repository / "memory-bank/local/context.db")
        self.addCleanup(connection.close)
        warning = "Index refresh failed: public metadata unavailable"
        with mock.patch.object(self.context, "refresh_layers", side_effect=AssertionError("duplicate refresh")), \
                mock.patch.object(self.context, "governed_binding", side_effect=self.context.ContextError("Working task not found")), \
                mock.patch.object(self.context, "retrieve", side_effect=AssertionError("stale retrieval")):
            capsule = self.context.assemble_capsule(
                connection, self.repository, mode="governed", query="lantern boundary",
                task_id="LANTERN-WARMING", limit=5, ephemeral=True, refresh_index=False,
                index_warnings=[warning], allow_unprovisioned=True,
            )
        self.assertEqual([warning], capsule["warnings"])
        self.assertEqual([], capsule["semantic"])
        self.assertEqual([], capsule["no_match"])

    def test_failed_refresh_keeps_fresh_authoritative_working_state(self):
        task = self.create(kind="task", alias="LANTERN-ACTIVE")
        connection = self.context.connect(self.repository / "memory-bank/local/context.db")
        self.addCleanup(connection.close)
        with mock.patch.object(self.context, "refresh_layers", return_value=({}, {}, ["Index unavailable"])), \
                mock.patch.object(self.context, "governed_binding", return_value={"task_uuid": task["id"]}), \
                mock.patch.object(self.context, "retrieve", side_effect=AssertionError("stale retrieval")):
            capsule = self.context.assemble_capsule(
                connection, self.repository, mode="governed", query="lantern boundary",
                task_id=task["external_id"], limit=5, ephemeral=True,
            )
        self.assertEqual(task["external_id"], capsule["working"]["task_id"])
        self.assertEqual(task["goal"], capsule["working"]["goal"])
        self.assertEqual(task["id"], capsule["task_uuid"])
        self.assertEqual(task["revision"], capsule["task_revision"])
        self.assertEqual([], capsule["semantic"])

    def test_empty_failed_capsule_render_does_not_query_old_frequency_state(self):
        with mock.patch.object(self.context, "excerpt_weights", side_effect=AssertionError("stale frequency read")):
            self.assertEqual({}, self.context.capsule_excerpts(None, {
                "query": "lantern boundary", "semantic": [], "episodic": [],
            }))

    def test_failed_cli_refresh_does_not_distill_against_the_old_index(self):
        task = self.create(kind='task', alias='LANTERN-CLI')
        arguments = ['context.py', '--root', str(self.repository), 'refresh', '--query',
                     'lantern parcel boundary', '--task-id', task['external_id'], '--json']
        stdout, stderr = io.StringIO(), io.StringIO()
        with mock.patch.object(sys, 'argv', arguments), \
                mock.patch.object(self.context, 'refresh_layers', return_value=(
                    {'procedural': 'failed', 'semantic': 'failed', 'episodic': 'failed'}, {}, ['Index refresh failed'])), \
                mock.patch.object(self.context, 'governed_binding', return_value={'task_uuid': task['id']}), \
                mock.patch.object(self.context, 'distill_capsule_query', side_effect=AssertionError('stale distillation')), \
                mock.patch.object(self.context, 'retrieve', side_effect=AssertionError('stale retrieval')), \
                contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            self.assertEqual(1, self.context.main())
        result = json.loads(stdout.getvalue())
        self.assertEqual(task['external_id'], result['capsule']['working']['task_id'])
        self.assertEqual([], result['capsule']['semantic'])
        self.assertIsNone(result['capsule']['manifest'])
        self.assertNotIn('Traceback', stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
