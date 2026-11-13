"""NEW public trajectory contract through actual native FTS5/packing/rendering.

No upstream import, dataset/gold, model, image, credential or remote service.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
EDITION = ROOT / 'PHP Core'
sys.path.insert(0, str(ROOT / 'harness/src'))
sys.path.insert(0, str(EDITION / 'memory-bank/scripts'))
import context as native
from harness import longmemeval_projection as subject

PUBLIC = json.loads((ROOT / 'tests/fixtures/longmemeval_projection_public.json').read_text())
EXPECTED = json.loads((ROOT / 'tests/fixtures/longmemeval_projection_expected_neutral.json').read_text())


def install(project):
    for relative in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/templates',
                     'project-brain/config', 'project-brain/schemas', 'project-brain/indexes'):
        shutil.copytree(EDITION / relative, project / relative,
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    for relative in ('memory-bank/README.md', 'memory-bank/INDEX.md', 'project-brain/README.md', 'project-brain/PROTOCOL.md'):
        shutil.copyfile(EDITION / relative, project / relative)
    (project / 'memory-bank/chunks').mkdir()
    (project / 'specs').mkdir()


class ProjectionContract(unittest.TestCase):
    def test_complete_literal_documents_input_hashes_and_arrival_order(self):
        for trajectory, expected in zip(PUBLIC['trajectories'], EXPECTED['documents']):
            actual = subject.project_trajectory(trajectory)
            self.assertEqual(expected['path'], actual.relative)
            self.assertEqual(expected['text'].encode(), actual.body)
            self.assertEqual(expected['sha256'], actual.sha256)
            self.assertEqual(expected['input_sha256'], actual.input_sha256)
            self.assertEqual(expected['state_count'], actual.states)
        item = subject.project_trajectory(PUBLIC['trajectories'][0])
        self.assertLess(item.body.index(b'step 2'), item.body.index(b'step 7'))
        self.assertLess(item.body.index(b'step 7'), item.body.index(b'Arrival action: "open safety panel"'))

    def test_strict_json_transport_duplicate_nonfinite_and_deep_invalid(self):
        accepted = json.dumps(PUBLIC['trajectories'][0]).encode()
        self.assertEqual(PUBLIC['trajectories'][0], subject.decode_trajectory(accepted))
        for raw in (b'{"id":"a","id":"b"}', b'{"step":NaN}', b'\xff',
                    b'[' * 20 + b'0' + b']' * 20, b'"\\ud800"',
                    b' ' * (subject.INPUT_BYTES + 1)):
            with self.subTest(raw_bytes=len(raw)), self.assertRaises(subject.ProjectionError):
                subject.decode_trajectory(raw)

    def test_unsupported_eval_fields_and_aliases_are_rejected(self):
        for location, key, value in (('root', 'answer', 'GOLD_SENTINEL'),
                                     ('root', 'question_id', 'LABEL_SENTINEL'),
                                     ('state', 'evidence', 'GOLD_EVIDENCE_SENTINEL'),
                                     ('state', 'text', 'conflicting alias'),
                                     ('state', 'thoughts', None),
                                     ('state', 'state_index', 2)):
            item = copy.deepcopy(PUBLIC['trajectories'][0])
            (item if location == 'root' else item['states'][0])[key] = value
            with self.subTest(key=key), self.assertRaises(subject.ProjectionError):
                subject.project_trajectory(item)

    def test_types_controls_bounds_and_no_coercion(self):
        for key, value in (('step', True), ('step', -1), ('step', 2 ** 31),
                           ('action', 3), ('url', 'bad\x00url'),
                           ('accessibility_tree', '\udfff'),
                           ('accessibility_tree', 'x' * 65537)):
            item = copy.deepcopy(PUBLIC['trajectories'][0])
            item['states'][0][key] = value
            with self.subTest(key=key), self.assertRaises(subject.ProjectionError):
                subject.project_trajectory(item)
        for states in ([], [PUBLIC['trajectories'][0]['states'][0]] * 129):
            item = copy.deepcopy(PUBLIC['trajectories'][0])
            item['states'] = states
            with self.assertRaises(subject.ProjectionError):
                subject.project_trajectory(item)
        item = copy.deepcopy(PUBLIC['trajectories'][0])
        for state in item['states']:
            state['accessibility_tree'] = 'z' * 60000
        with self.assertRaises(subject.ProjectionError):
            subject.project_trajectory(item)

    def test_screenshot_reference_opaque_canonical_and_no_file_read(self):
        for reference in ('/etc/passwd', '../foreign.png', 'shots/../foreign.png',
                           'file:foreign.png', 'https://foreign.invalid/a',
                           'shots\\a.png', './a.png', 'shots//a.png', 'shots/a.png/'):
            item = copy.deepcopy(PUBLIC['trajectories'][0])
            item['states'][0]['screenshot'] = reference
            with self.subTest(reference=reference), self.assertRaises(subject.ProjectionError):
                subject.project_trajectory(item)
        with patch('builtins.open', side_effect=AssertionError('projection must not open image')):
            self.assertEqual(2, subject.project_trajectory(PUBLIC['trajectories'][0]).states)


class NativeProjection(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.project = self.base / 'public-project'
        self.project.mkdir()
        install(self.project)
        self.memory = subject.NativeProjectionMemory(self.project, native)
        self.addCleanup(self.memory.close)

    def insert_all(self):
        for trajectory in PUBLIC['trajectories']:
            self.memory.insert(trajectory)

    def test_native_fts5_refresh_manifest_and_exact_canonical_support(self):
        self.insert_all()
        for name in ('ferry', 'gear'):
            output = self.memory.query(PUBLIC['requests'][name])
            self.assertEqual(1, len(output))
            self.assertEqual('text', output[0]['type'])
            self.assertIn(EXPECTED['support'][name], output[0]['value'])
            self.assertIn(native.MEMORY_RENDER_HEADER, output[0]['value'])
            self.assertLessEqual(len(output[0]['value']) + 1, native.RENDERED_CAPSULE_LIMIT)
        database = native.connect(self.project / 'memory-bank/local/context.db')
        try:
            indexed = database.execute('SELECT path, content FROM documents WHERE path LIKE ?',
                                       ('specs/longmemeval/%',)).fetchall()
            self.assertEqual(3, len(indexed))
            self.assertEqual({row['path']: row['content'] for row in indexed},
                             {item['path']: item['text'] for item in EXPECTED['documents']})
        finally:
            database.close()
        manifests = list((self.project / 'memory-bank/local').rglob('*.json'))
        selected = [json.loads(node.read_text()).get('selected', []) for node in manifests]
        self.assertTrue(all(sum(item.get('path', '').startswith('specs/longmemeval/')
                                for item in entries) <= 3 for entries in selected))
        self.assertTrue(any(any(item.get('path', '').startswith('specs/longmemeval/')
                                for item in entries) for entries in selected))
        metadata = self.memory.post_query_hook(question_id='DO_NOT_RETAIN', answer='GOLD_NEVER')
        self.assertEqual('delivered', metadata['state'])
        self.assertIsNone(metadata['provider_tokens'])
        self.assertFalse(metadata['provider_tokens_complete'])
        self.assertNotIn('GOLD_NEVER', json.dumps(metadata))
        self.assertEqual(len(output[0]['value'].encode()), metadata['context_bytes'])

    def test_the_bridge_measures_the_full_capsule_whatever_the_project_ships(self):
        # The bridge measures retrieval; the project's automatic capsule may be
        # set to carry no document at all.
        config = self.project / 'project-brain/config/runtime.json'
        settings = json.loads(config.read_text())
        config.write_text(json.dumps({**settings, 'capsule_profile': 'navigation'}))
        self.insert_all()
        output = self.memory.query(PUBLIC['requests']['ferry'])
        self.assertEqual(1, len(output))
        self.assertIn(EXPECTED['support']['ferry'], output[0]['value'])

    def test_exact_replay_conflict_and_input_mutation_do_not_rewrite(self):
        item = copy.deepcopy(PUBLIC['trajectories'][0])
        self.memory.insert(item)
        source = self.project / EXPECTED['documents'][0]['path']
        before = source.stat().st_mtime_ns
        self.memory.insert(dict(reversed(list(item.items()))))
        self.assertEqual(before, source.stat().st_mtime_ns)
        item['states'][0]['accessibility_tree'] = 'conflicting replacement'
        with self.assertRaisesRegex(subject.ProjectionError, 'conflicting_identity'):
            self.memory.insert(item)
        self.assertEqual(EXPECTED['documents'][0]['text'].encode(), source.read_bytes())
        self.assertIn(EXPECTED['support']['ferry'], self.memory.query(PUBLIC['requests']['ferry'])[0]['value'])

    def test_gold_rejection_precedes_file_mutation_and_native_callback(self):
        item = copy.deepcopy(PUBLIC['trajectories'][0])
        item['answer'] = 'GOLD_NEW_LABEL'
        with patch.object(native, 'refresh_layers', side_effect=AssertionError('must not query')):
            with self.assertRaises(subject.ProjectionError):
                self.memory.insert(item)
        self.assertEqual([], list((self.project / 'specs/longmemeval').iterdir()))

    def test_no_match_and_sanitized_empty_have_no_fallback_evidence(self):
        self.insert_all()
        self.assertEqual(EXPECTED['no_match_context'], self.memory.query(PUBLIC['requests']['no_match']))
        self.assertEqual('no_evidence', self.memory.post_query_hook()['state'])
        with patch.object(native, 'connect', side_effect=AssertionError('empty must not connect')):
            self.assertEqual([], self.memory.query('alice@example.com'))
        self.assertEqual('sanitized_empty', self.memory.post_query_hook()['state'])

    def test_tamper_deletion_foreign_file_and_equal_byte_replacement_fence(self):
        for mutation in ('tamper', 'delete', 'foreign', 'replace'):
            with self.subTest(mutation=mutation):
                temporary = self.base / mutation
                temporary.mkdir()
                install(temporary)
                memory = subject.NativeProjectionMemory(temporary, native)
                try:
                    memory.insert(PUBLIC['trajectories'][0])
                    source = temporary / EXPECTED['documents'][0]['path']
                    if mutation == 'tamper':
                        source.write_text('foreign bytes')
                    elif mutation == 'delete':
                        source.unlink()
                    elif mutation == 'foreign':
                        (source.parent / 'foreign.md').write_text('outside corpus')
                    else:
                        sibling = source.with_suffix('.new')
                        sibling.write_bytes(source.read_bytes())
                        sibling.replace(source)
                    with patch.object(native, 'connect', side_effect=AssertionError('fence before index')):
                        with self.assertRaisesRegex(subject.ProjectionError, 'source_changed'):
                            memory.query(PUBLIC['requests']['ferry'])
                        with self.assertRaisesRegex(subject.ProjectionError, 'instance_unavailable'):
                            memory.insert(PUBLIC['trajectories'][1])
                finally:
                    memory.close()

    def test_symlink_hardlink_and_parent_replacement_never_follow(self):
        self.memory.insert(PUBLIC['trajectories'][0])
        source = self.project / EXPECTED['documents'][0]['path']
        backup = self.base / 'foreign-source.md'
        source.rename(backup)
        source.symlink_to(backup)
        with self.assertRaisesRegex(subject.ProjectionError, 'source_changed'):
            self.memory.query(PUBLIC['requests']['ferry'])
        source.unlink()
        source.write_bytes(backup.read_bytes())
        self.assertEqual(EXPECTED['documents'][0]['text'].encode(), backup.read_bytes())
        other = self.base / 'hard-project'
        other.mkdir()
        install(other)
        memory = subject.NativeProjectionMemory(other, native)
        try:
            memory.insert(PUBLIC['trajectories'][0])
            os.link(other / EXPECTED['documents'][0]['path'], self.base / 'hardlink.md')
            with self.assertRaisesRegex(subject.ProjectionError, 'source_changed'):
                memory.query(PUBLIC['requests']['ferry'])
        finally:
            memory.close()
        third = self.base / 'parent-project'
        third.mkdir()
        install(third)
        memory = subject.NativeProjectionMemory(third, native)
        try:
            (third / 'specs').rename(third / 'old-specs')
            (third / 'specs').mkdir()
            with self.assertRaisesRegex(subject.ProjectionError, 'source_changed'):
                memory.query(PUBLIC['requests']['ferry'])
        finally:
            memory.close()

    def test_partial_write_cancellation_fences_without_cleanup_adoption(self):
        real_write = os.write
        calls = []
        def interrupted(fd, raw):
            calls.append(len(raw))
            if len(calls) == 1:
                return real_write(fd, raw[:5])
            raise KeyboardInterrupt()
        with patch.object(subject.os, 'write', interrupted):
            with self.assertRaises(KeyboardInterrupt):
                self.memory.insert(PUBLIC['trajectories'][0])
        source = self.project / EXPECTED['documents'][0]['path']
        self.assertEqual(EXPECTED['documents'][0]['text'].encode()[:5], source.read_bytes())
        with self.assertRaisesRegex(subject.ProjectionError, 'instance_unavailable'):
            self.memory.query(PUBLIC['requests']['ferry'])
        with self.assertRaises(FileExistsError):
            subject.NativeProjectionMemory(self.project, native)

    def test_two_corpora_and_opaque_query_image_are_isolated(self):
        self.memory.insert(PUBLIC['trajectories'][0])
        other = self.base / 'independent-project'
        other.mkdir()
        install(other)
        memory = subject.NativeProjectionMemory(other, native)
        try:
            memory.insert(PUBLIC['trajectories'][1])
            self.assertEqual([], memory.query(PUBLIC['requests']['ferry']))
            output = memory.query(PUBLIC['requests']['gear'], query_image='/unread/opaque-query.png')
            self.assertIn(EXPECTED['support']['gear'], output[0]['value'])
            self.assertNotIn(EXPECTED['support']['ferry'], output[0]['value'])
            self.assertTrue(memory.post_query_hook()['query_image_ignored'])
            self.assertEqual(0, memory.post_query_hook()['images_ingested'])
        finally:
            memory.close()

    def test_refresh_failure_is_incomplete_not_successful_no_match(self):
        self.insert_all()
        with patch.object(native, 'refresh_layers', return_value=({}, {}, ['PRIVATE_ERROR_TEXT'])):
            self.assertEqual([], self.memory.query(PUBLIC['requests']['ferry']))
        metadata = self.memory.post_query_hook()
        self.assertEqual('refresh_failed', metadata['state'])
        self.assertNotIn('PRIVATE_ERROR_TEXT', json.dumps(metadata))

    def test_native_privacy_masking_unchanged_and_default_budgets(self):
        item = copy.deepcopy(PUBLIC['trajectories'][1])
        item['states'][0]['text'] += '\nContact owner alice@example.com for the exact private code.'
        self.memory.insert(item)
        output = self.memory.query(PUBLIC['requests']['gear'])
        self.assertNotIn('alice@example.com', json.dumps(output))
        self.assertIn(EXPECTED['support']['gear'], output[0]['value'])
        self.assertEqual('shadow', native.configured_retrieval_gate(self.project))
        self.assertEqual('governed', native.configured_mode(self.project))
        self.assertLessEqual(len(output[0]['value']) + 1, native.RENDERED_CAPSULE_LIMIT)

    def test_query_bound_close_and_schema_prerequisite_refuse(self):
        with self.assertRaises(subject.ProjectionError):
            self.memory.query('x' * (subject.QUERY_CHARS + 1))
        self.memory.close()
        self.memory.close()
        with self.assertRaisesRegex(subject.ProjectionError, 'instance_unavailable'):
            self.memory.query(PUBLIC['requests']['ferry'])
        other = self.base / 'unconfigured'
        other.mkdir()
        (other / 'specs').mkdir()
        with self.assertRaisesRegex(subject.ProjectionError, 'native_configuration'):
            subject.NativeProjectionMemory(other, native)
        self.assertFalse((other / 'specs/longmemeval').exists())

    def test_trajectory_private_id_never_becomes_native_title(self):
        item = copy.deepcopy(PUBLIC['trajectories'][1])
        item['id'] = 'alice@example.com'
        self.memory.insert(item)
        output = self.memory.query(PUBLIC['requests']['gear'])
        self.assertIn(EXPECTED['support']['gear'], output[0]['value'])
        self.assertNotIn('alice@example.com', json.dumps(output))
        self.assertNotIn('alice@example.com', json.dumps(self.memory.post_query_hook()))

    def test_fifo_replacement_refuses_before_native_and_fences(self):
        self.memory.insert(PUBLIC['trajectories'][0])
        source = self.project / EXPECTED['documents'][0]['path']
        source.unlink()
        os.mkfifo(source)
        # A FIFO with no writer makes a blocking O_RDONLY open hang. The custody
        # flags must admit nonblocking open, then reject the nonregular inode.
        real_open = os.open
        def guarded_open(name, flags, *args, **kwargs):
            if name == source.name:
                self.assertTrue(flags & os.O_NONBLOCK)
            return real_open(name, flags, *args, **kwargs)
        with patch.object(subject.os, 'open', guarded_open):
            with patch.object(native, 'connect', side_effect=AssertionError('no native call')):
                with self.assertRaisesRegex(subject.ProjectionError, 'source_changed'):
                    self.memory.query(PUBLIC['requests']['ferry'])
        with self.assertRaisesRegex(subject.ProjectionError, 'instance_unavailable'):
            self.memory.query(PUBLIC['requests']['ferry'])

    def test_access_time_change_does_not_invalidate_unchanged_source(self):
        self.memory.insert(PUBLIC['trajectories'][0])
        real_stat = os.fstat
        seen = []
        def observed_stat(fd):
            value = real_stat(fd)
            if value.st_size == len(EXPECTED['documents'][0]['text'].encode()):
                seen.append(fd)
                if len(seen) == 2:
                    fields = list(value)
                    fields[7] = value.st_atime + 10
                    # Named ns fields do not enter the generation tuple for atime.
                    return os.stat_result(fields, {'st_mtime_ns': value.st_mtime_ns,
                                                   'st_ctime_ns': value.st_ctime_ns})
            return value
        with patch.object(subject.os, 'fstat', observed_stat):
            self.memory._custody()
        self.assertEqual(2, len(seen))
        self.assertIn(EXPECTED['support']['ferry'], self.memory.query(PUBLIC['requests']['ferry'])[0]['value'])

    def test_foreign_directory_population_stops_at_first_unknown_entry(self):
        self.memory.insert(PUBLIC['trajectories'][0])
        class Entry:
            def __init__(self, name):
                self.name = name
        class Entries:
            def __enter__(self):
                return self
            def __exit__(self, *args):
                return False
            def __iter__(self):
                yield Entry(Path(EXPECTED['documents'][0]['path']).name)
                yield Entry('foreign.md')
                raise AssertionError('must not consume an unbounded foreign tail')
        with patch.object(subject.os, 'scandir', return_value=Entries()):
            with self.assertRaisesRegex(subject.ProjectionError, 'source_changed'):
                self.memory.query(PUBLIC['requests']['ferry'])
        self.assertEqual('failed', self.memory.post_query_hook()['state'])

    def test_every_source_scan_starts_from_a_rewound_directory(self):
        # btrfs refreshes a held directory's last index only on open and on a
        # seek: a scan that does not rewind first misses the file inserted
        # since the previous one and fences the instance with source_changed.
        calls = []
        real_lseek, real_scandir = subject.os.lseek, subject.os.scandir
        def lseek(fd, position, how):
            calls.append(('lseek', fd, position, how))
            return real_lseek(fd, position, how)
        def scandir(target):
            calls.append(('scandir', target))
            return real_scandir(target)
        with patch.object(subject.os, 'lseek', lseek), patch.object(subject.os, 'scandir', scandir):
            self.insert_all()
            self.memory.query(PUBLIC['requests']['ferry'])
        source = self.memory._source_fd
        scans = [index for index, call in enumerate(calls) if call == ('scandir', source)]
        self.assertGreater(len(scans), 1)
        for index in scans:
            self.assertEqual(('lseek', source, 0, os.SEEK_SET), calls[index - 1])

    def test_invalid_native_mode_and_gate_refuse_before_database_creation(self):
        self.insert_all()
        for name in ('PROJECT_BRAIN_MODE', 'CONTEXT_RETRIEVAL_GATE'):
            with self.subTest(name=name), patch.dict(os.environ, {name: 'not-supported'}):
                with patch.object(native, 'connect', side_effect=AssertionError('preflight first')):
                    with self.assertRaises(native.BrainError):
                        self.memory.query(PUBLIC['requests']['ferry'])
            self.assertFalse((self.project / 'memory-bank/local/context.db').exists())

    def test_corpus_query_and_context_admission_limits_keep_existing_bytes(self):
        with patch.object(subject, 'MAX_TRAJECTORIES', 1):
            self.memory.insert(PUBLIC['trajectories'][0])
            with self.assertRaisesRegex(subject.ProjectionError, 'corpus_bound'):
                self.memory.insert(PUBLIC['trajectories'][1])
        source = self.project / EXPECTED['documents'][0]['path']
        with patch.object(subject, 'TOTAL_BYTES', len(source.read_bytes())):
            with self.assertRaisesRegex(subject.ProjectionError, 'corpus_bound'):
                self.memory.insert(PUBLIC['trajectories'][1])
        self.assertEqual(EXPECTED['documents'][0]['text'].encode(), source.read_bytes())
        with patch.object(subject, 'MAX_QUERIES', 1):
            self.assertEqual([], self.memory.query(PUBLIC['requests']['no_match']))
            with patch.object(native, 'connect', side_effect=AssertionError('cap before connect')):
                with self.assertRaisesRegex(subject.ProjectionError, 'query_bound'):
                    self.memory.query(PUBLIC['requests']['ferry'])
        with patch.object(subject, 'CONTEXT_BYTES', 1):
            with self.assertRaisesRegex(subject.ProjectionError, 'context_bound'):
                self.memory.query(PUBLIC['requests']['ferry'])
        self.assertEqual('failed', self.memory.post_query_hook()['state'])
        self.assertEqual(0, self.memory.post_query_hook()['context_bytes'])

    def test_connection_cleanup_failure_does_not_claim_delivered_context(self):
        self.insert_all()
        real_connect = native.connect
        class Connection:
            def __init__(self, inner):
                self.inner = inner
            def __getattr__(self, name):
                return getattr(self.inner, name)
            def __enter__(self):
                return self.inner.__enter__()
            def __exit__(self, *args):
                return self.inner.__exit__(*args)
            def close(self):
                self.inner.close()
                raise RuntimeError('CLOSE_FAILURE_SENTINEL')
        with patch.object(native, 'connect', side_effect=lambda node: Connection(real_connect(node))):
            with self.assertRaisesRegex(RuntimeError, 'CLOSE_FAILURE_SENTINEL'):
                self.memory.query(PUBLIC['requests']['ferry'])
        metadata = self.memory.post_query_hook()
        self.assertEqual('cleanup_failed', metadata['state'])
        self.assertEqual((0, 0), (metadata['context_bytes'], metadata['context_chars']))
        self.assertIsNone(metadata['provider_tokens'])
        self.assertNotIn('CLOSE_FAILURE_SENTINEL', json.dumps(metadata))


if __name__ == '__main__':
    unittest.main()
