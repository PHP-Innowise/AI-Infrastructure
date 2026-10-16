"""A large Brain is read once per command, and what is remembered stays true.

One record-result walked every record thirty times and a prompt twice; the
runtime now keeps each record's image for the rest of the process, keyed by
the file's identity, and lists a directory again when a writer changed it
while it was being read.
"""
from __future__ import annotations
import contextlib
import importlib
import io
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / 'PHP Core/memory-bank/scripts'


class BrainScaleTests(unittest.TestCase):
    def setUp(self):
        clean = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        environment = mock.patch.dict(os.environ, clean, clear=True)
        environment.start(); self.addCleanup(environment.stop)
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.brain = importlib.import_module('brain_runtime')
            self.results = importlib.import_module('memory_results')
            self.validate = importlib.import_module('validate')
        finally:
            sys.path.pop(0)
        folder = tempfile.TemporaryDirectory(prefix='heron-scale-')
        self.addCleanup(folder.cleanup)
        self.root = Path(folder.name)
        (self.root / 'specs').mkdir()
        (self.root / 'specs/heron.md').write_text('# Heron\n\nHerons wade.\n', encoding='utf-8')
        config = self.root / 'project-brain/config/runtime.json'; config.parent.mkdir(parents=True)
        config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': False}), encoding='utf-8')
        self.records = [self.brain.create_record(self.root, 'finding', f'HERON-{n}', f'Heron finding {n}', [],
                                                 ['specs/heron.md'], owner='local', goal=f'Heron goal {n}.')
                        for n in range(3)]

    def titles(self):
        return sorted(record['title'] for _, record, _ in self.brain.iter_records(self.root))

    def test_an_unchanged_brain_is_read_once_per_process(self):
        self.titles()
        with mock.patch.object(self.brain, 'read_record_text', side_effect=AssertionError('read again')):
            for _ in range(3):
                self.assertEqual(['Heron finding 0', 'Heron finding 1', 'Heron finding 2'], self.titles())

    def next_process(self):
        """What a new process starts with: nothing remembered in memory."""
        self.brain._RECORD_IMAGE_CACHE.clear()
        self.brain._VALID_RECORD_IMAGES.clear()
        self.brain._PERSISTED_STATE.update(root=None, dirty=False, used=set(), held=False)
        self.addCleanup(self.brain._PERSISTED_STATE.update, held=False)

    def test_a_run_that_writes_nothing_leaves_no_cache(self):
        # A read-only or degraded refresh promises no filesystem writes.
        self.next_process()
        (self.root / 'project-brain/local').mkdir(exist_ok=True)
        self.brain.hold_persisted_images()
        with mock.patch.object(self.brain, '_PERSISTED_SETTLE_NS', 0):
            self.assertEqual(3, len(self.titles()))
        self.assertFalse((self.root / 'project-brain/local' / self.brain._PERSISTED_IMAGES).exists())

    def test_a_new_process_reads_no_unchanged_record(self):
        # Every prompt is a new process; each read, parsed and validated every
        # record again - most of a prompt on a large Brain.
        self.next_process()
        with mock.patch.object(self.brain, '_PERSISTED_SETTLE_NS', 0):
            self.titles()
            self.next_process()
            with mock.patch.object(self.brain, 'read_record_text', side_effect=AssertionError('read again')), \
                 mock.patch.object(self.brain, 'validate_record', side_effect=AssertionError('validated again')):
                self.assertEqual(['Heron finding 0', 'Heron finding 1', 'Heron finding 2'], self.titles())
            # A record changed since is read again, and only that one.
            changed = self.records[1]
            path = self.brain.dynamic_path(self.root, changed)
            path.write_text(path.read_text(encoding='utf-8').replace('Heron finding 1', 'Heron finding one'),
                            encoding='utf-8')
            self.next_process()
            real = self.brain.read_record_text
            with mock.patch.object(self.brain, 'read_record_text', side_effect=real) as read:
                self.assertEqual(['Heron finding 0', 'Heron finding 2', 'Heron finding one'], self.titles())
            self.assertEqual([path], [call.args[0] for call in read.call_args_list])

    def test_a_cache_that_does_not_fit_is_a_miss(self):
        self.next_process()
        with mock.patch.object(self.brain, '_PERSISTED_SETTLE_NS', 0):
            self.titles()
            cache = self.root / 'project-brain/local' / self.brain._PERSISTED_IMAGES
            self.assertTrue(cache.is_file())
            for damage in (b'\x00garbage', None):
                self.next_process()
                if damage is None:
                    # Written by another validator, or another Python.
                    patcher = mock.patch.object(self.brain, '_persisted_version', return_value=('other',))
                else:
                    cache.write_bytes(damage)
                    patcher = contextlib.nullcontext()
                real = self.brain.read_record_text
                with patcher, mock.patch.object(self.brain, 'read_record_text', side_effect=real) as read:
                    self.assertEqual(3, len(self.titles()))
                self.assertEqual(3, read.call_count)

    def test_a_record_changed_just_now_is_not_kept(self):
        # Its times could repeat for a second edit on a coarse clock.
        self.next_process()
        self.titles()
        self.next_process()
        real = self.brain.read_record_text
        with mock.patch.object(self.brain, 'read_record_text', side_effect=real) as read:
            self.titles()
        self.assertEqual(3, read.call_count)

    def test_another_projects_records_stay_out_of_this_cache(self):
        other = Path(tempfile.mkdtemp(prefix='heron-other-'))
        self.addCleanup(shutil.rmtree, other, True)
        (other / 'specs').mkdir()
        (other / 'specs/heron.md').write_text('# Heron\n\nHerons wade.\n', encoding='utf-8')
        config = other / 'project-brain/config/runtime.json'; config.parent.mkdir(parents=True)
        config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': False}), encoding='utf-8')
        (other / 'project-brain/local').mkdir()
        self.brain.create_record(other, 'finding', 'EGRET-1', 'Egret finding', [], ['specs/heron.md'],
                                 owner='local', goal='Egret goal.')
        self.next_process()
        with mock.patch.object(self.brain, '_PERSISTED_SETTLE_NS', 0):
            list(self.brain.iter_records(other))
            self.titles()
        version, entries = self.brain.marshal.loads(
            (self.root / 'project-brain/local' / self.brain._PERSISTED_IMAGES).read_bytes())
        self.assertTrue(entries)
        self.assertTrue(all(key.startswith(str(self.root / 'project-brain') + os.sep) for key in entries))

    def test_every_pass_hands_out_its_own_copy(self):
        _, record, _ = next(self.brain.iter_records(self.root))
        record['title'] = 'Edited in memory only'
        record['transitions'].append({'edited': True})
        _, again, _ = next(self.brain.iter_records(self.root))
        self.assertNotEqual('Edited in memory only', again['title'])
        self.assertNotIn({'edited': True}, again['transitions'])

    def test_a_record_written_since_is_read_again(self):
        self.titles()
        record = self.records[0]
        self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
                                 progress='Herons wade at dawn.', next_steps=[], files=[], sources=[],
                                 actor='local', reason='Heron progress')
        self.assertEqual('Herons wade at dawn.', self.brain.find_record(self.root, record['id'])[1]['progress'])

    def test_an_edit_in_place_of_the_same_size_is_read_again(self):
        self.titles()
        path, record, _ = self.brain.find_record(self.root, self.records[1]['id'])
        text = path.read_text(encoding='utf-8')
        edited = text.replace('Heron finding 1', 'Heron finding X')
        self.assertEqual(len(text), len(edited))
        before = path.stat()
        with open(path, 'r+', encoding='utf-8') as stream:  # same inode, same size
            stream.write(edited)
        os.utime(path, ns=(before.st_atime_ns, before.st_mtime_ns + 1_000_000))
        self.assertIn('Heron finding X', self.titles())

    def test_a_directory_changed_while_listed_is_listed_again(self):
        # A writer replaces a record while a reader lists the directory: the
        # reader used to fail with "Project Brain inventory unavailable".
        real_scandir = os.scandir
        record = self.records[2]
        replaced = []
        def scandir(directory):
            if not replaced and str(directory).endswith('findings'):
                replaced.append(True)
                self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
                                         progress='Replaced mid-listing.', next_steps=[], files=[], sources=[],
                                         actor='local', reason='Heron concurrent write')
            return real_scandir(directory)
        with mock.patch.object(self.brain.os, 'scandir', scandir):
            listed = {item['id']: item for _, item, _ in self.brain.iter_records(self.root)}
        self.assertEqual(replaced, [True])
        self.assertEqual('Replaced mid-listing.', listed[record['id']]['progress'])

    def test_a_writers_temporary_file_gone_before_it_is_looked_at_is_not_an_error(self):
        findings = self.root / 'project-brain/dynamic/findings'
        temporary = findings / '.heron.md.tmp1234'
        temporary.write_text('partial', encoding='utf-8')
        real_lstat = os.lstat
        def lstat(path, *args, **kwargs):
            if str(path) == str(temporary) and temporary.exists():
                temporary.unlink()
            return real_lstat(path, *args, **kwargs)
        with mock.patch.object(self.brain.os, 'lstat', lstat):
            self.assertEqual(3, len(self.titles()))

    def test_a_directory_that_never_settles_is_still_unavailable(self):
        real_scandir = os.scandir
        def scandir(directory):
            if str(directory).endswith('findings'):
                (Path(directory) / f'.churn-{len(os.listdir(directory))}').write_text('x', encoding='utf-8')
            return real_scandir(directory)
        with mock.patch.object(self.brain.os, 'scandir', scandir), mock.patch.object(self.brain.time, 'sleep'):
            with self.assertRaisesRegex(self.brain.BrainError, 'inventory unavailable'):
                self.titles()

    def test_a_cloud_placeholder_record_is_read_and_a_junction_is_not(self):
        # Windows marks OneDrive placeholders and deduplicated files as reparse
        # points too; only one that redirects the path is refused.
        real_lstat = os.lstat
        def lstat_tagged(tag):
            def lstat(path, *args, **kwargs):
                image = real_lstat(path, *args, **kwargs)
                if str(path).endswith('.md'):
                    fields = {name: getattr(image, name) for name in dir(image) if name.startswith('st_')}
                    fields.update(st_file_attributes=0x400, st_reparse_tag=tag)
                    return types.SimpleNamespace(**fields)
                return image
            return lstat
        with mock.patch.object(self.brain.os, 'lstat', lstat_tagged(0x9000601A)):
            self.assertEqual(3, len(self.titles()))
        with mock.patch.object(self.brain.os, 'lstat', lstat_tagged(0xA0000003)):
            with self.assertRaisesRegex(self.brain.BrainError, 'inventory unavailable'):
                self.titles()

    def test_the_save_frontier_follows_every_record_change(self):
        first = self.results._domain_image(self.root)
        self.assertEqual(first, self.results._domain_image(self.root))
        record = self.records[0]
        self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
                                 progress='Frontier moved.', next_steps=[], files=[], sources=[],
                                 actor='local', reason='Heron frontier')
        self.assertNotEqual(first, self.results._domain_image(self.root))

    def test_a_chunk_that_gains_a_secret_is_caught_on_the_next_check(self):
        chunk = self.root / 'MEM-heron.md'
        chunk.write_text('# Heron\n\nNothing secret here.\n', encoding='utf-8')
        self.validate.validate_secret_patterns(chunk)
        before = chunk.stat()
        chunk.write_text('# Heron\n\nkey: AKIAABCDEFGHIJKLMNOP\n', encoding='utf-8')
        os.utime(chunk, ns=(before.st_atime_ns, before.st_mtime_ns + 1_000_000))
        with self.assertRaises(self.validate.ValidationError):
            self.validate.validate_secret_patterns(chunk)


@unittest.skipUnless(shutil.which('git'), 'Git')
class BrainIndexStabilityTests(unittest.TestCase):
    """A prompt's index refresh rebuilds the Brain rows from the records; when
    they come out the same it no longer rewrites them, so the index
    generation - and the token frequencies cached against it - stand."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='heron-index-')
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name).resolve()
        self.root = self.base / 'project'
        self.root.mkdir()
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        self.env.update(HOME=str(self.base), XDG_CONFIG_HOME=str(self.base / 'xdg'), TMPDIR=str(self.base),
                        PYTHONDONTWRITEBYTECODE='1', GIT_OPTIONAL_LOCKS='0')
        for relative in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/schemas',
                         'project-brain/templates'):
            shutil.copytree(ROOT / 'PHP Core' / relative, self.root / relative,
                            ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        config = self.root / 'project-brain/config/runtime.json'; config.parent.mkdir(parents=True)
        config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': False}), encoding='utf-8')
        (self.root / 'specs').mkdir()
        (self.root / 'specs/heron.md').write_text('# Heron\n\nHerons wade.\n', encoding='utf-8')
        (self.root / 'specs/egret.md').write_text('# Egret\n\nEgrets stand.\n', encoding='utf-8')
        (self.root / '.gitignore').write_text('memory-bank/local/\nproject-brain/local/\n', encoding='utf-8')
        clean = mock.patch.dict(os.environ, self.env, clear=True)
        clean.start(); self.addCleanup(clean.stop)
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.brain = importlib.import_module('brain_runtime')
        finally:
            sys.path.pop(0)
        self.records = [self.brain.create_record(self.root, 'finding', f'HERON-{n}', f'Heron finding {n}', [],
                                                 ['specs/heron.md'], owner='local', goal=f'Heron goal {n}.')
                        for n in range(3)]
        for args in (('init', '-q', '-b', 'feature/heron'), ('add', '-A'),
                     ('-c', 'user.email=h@example.invalid', '-c', 'user.name=H', 'commit', '-q', '-m', 'fixture')):
            subprocess.run(['git', '-C', str(self.root), *args], env=self.env, check=True, capture_output=True)

    def index(self):
        result = subprocess.run([sys.executable, str(self.root / 'memory-bank/scripts/context.py'), 'index',
                                 '--incremental', '--json'], cwd=self.root, env=self.env,
                                capture_output=True, text=True, timeout=120)
        self.assertEqual(0, result.returncode, result.stderr)
        with contextlib.closing(sqlite3.connect(self.root / 'memory-bank/local/context.db')) as db:
            state = dict(db.execute('SELECT key, value FROM index_state'))
            progress = dict(db.execute("SELECT path, content FROM documents WHERE path LIKE 'project-brain/%'"))
        return json.loads(result.stdout), state['index-generation'], progress

    def test_an_unchanged_brain_leaves_the_index_and_its_generation_alone(self):
        self.index()
        reply, generation, rows = self.index()
        self.assertEqual(3, reply['brain'])
        for _ in range(2):
            again, same, unchanged = self.index()
            self.assertEqual((3, generation, rows), (again['brain'], same, unchanged))
        record = self.records[0]
        self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
                                 progress='Herons wade at dawn.', next_steps=[], files=[], sources=[],
                                 actor='local', reason='Heron progress')
        _, moved, rows = self.index()
        self.assertNotEqual(generation, moved)
        self.assertTrue(any('Herons wade at dawn.' in content for content in rows.values()))

    def test_a_record_whose_citation_is_gone_leaves_the_index(self):
        self.index(); _, generation, _ = self.index()
        (self.root / 'specs/heron.md').unlink()
        reply, moved, rows = self.index()
        self.assertEqual((0, {}), (reply['brain'], rows))
        self.assertNotEqual(generation, moved)

    def test_a_cited_file_is_asked_once_per_pass_and_again_in_the_next(self):
        sys.path.insert(0, str(SCRIPTS))
        try:
            retrieval = importlib.import_module('context_retrieval')
        finally:
            sys.path.pop(0)
        config = retrieval.load_config(self.root)
        asked = []
        real_contains = retrieval.workspace_roots.contains
        def contains(repository, path):
            asked.append(Path(path).name)
            return real_contains(repository, path)
        with mock.patch.object(self.brain.workspace_roots, 'contains', contains):
            documents = retrieval._brain_documents(self.root, config)[0]
            self.assertEqual(3, len(documents))
            self.assertEqual(['heron.md'], asked)
            (self.root / 'specs/heron.md').unlink()
            self.assertEqual([], retrieval._brain_documents(self.root, config)[0])
        # Outside a pass nothing is kept: every check asks again.
        self.assertIsNone(getattr(self.brain._CITED_FILES, 'memo', None))
        with self.assertRaises(self.brain.BrainError):
            self.brain._cited_file(self.root, 'specs/heron.md')


@unittest.skipUnless(shutil.which('git'), 'Git')
class MemoryBankScaleTests(unittest.TestCase):
    """Every prompt checks the Memory Bank's chunks again - their citations can
    go away while a chunk stays - and it also screened every unchanged chunk
    for secrets and resolved every citation again chunk by chunk: 1.5 s a
    prompt with 2,000 chunks."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='heron-bank-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        env = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        env.update(HOME=str(self.root), XDG_CONFIG_HOME=str(self.root / 'xdg'), GIT_OPTIONAL_LOCKS='0')
        patcher = mock.patch.dict(os.environ, env, clear=True)
        patcher.start(); self.addCleanup(patcher.stop)
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.context = importlib.import_module('context')
            self.validate = importlib.import_module('validate')
            self.roots = importlib.import_module('workspace_roots')
        finally:
            sys.path.pop(0)
        config = self.root / 'project-brain/config/runtime.json'; config.parent.mkdir(parents=True)
        config.write_text(json.dumps({'mode': 'governed', 'automatic_promotion': False}), encoding='utf-8')
        (self.root / 'specs').mkdir()
        (self.root / 'specs/heron.md').write_text('# Heron\n\nHerons wade at dawn.\n', encoding='utf-8')
        self.chunks = self.root / 'memory-bank/chunks'
        self.chunks.mkdir(parents=True)
        for n in range(3):
            meta = {'id': f'MEM-20261016-0000000{n}', 'title': f'Heron wading {n}', 'type': 'operations',
                    'status': 'active', 'scope': ['project'], 'tags': ['heron'], 'created': '2026-10-16',
                    'last_verified': '2026-10-16', 'review_after': '2099-01-01', 'sources': ['specs/heron.md'],
                    'supersedes': [], 'superseded_by': None}
            (self.chunks / f'MEM-20261016-0000000{n}-heron-{n}.md').write_text(
                '---\n' + json.dumps(meta, indent=2) + f'\n---\n\n# Heron wading {n}\n\nHerons wade at dawn.\n',
                encoding='utf-8')
        for args in (('init', '-q', '-b', 'main'), ('add', '-A'),
                     ('-c', 'user.email=h@example.invalid', '-c', 'user.name=H', 'commit', '-q', '-m', 'fixture')):
            subprocess.run(['git', '-C', str(self.root), *args], check=True, capture_output=True, timeout=60)
        self.connection = self.context.connect(self.root / 'memory-bank/local/context.db')
        self.addCleanup(self.connection.close)

    def index(self):
        with self.roots.source_frame_scope():
            self.context.index_repository(self.connection, self.root, incremental=True)
        return sorted(row[0] for row in self.connection.execute(
            "SELECT path FROM documents WHERE path LIKE 'memory-bank/chunks/%'"))

    def test_an_unchanged_chunk_is_not_screened_again(self):
        screened = []
        real = self.context.validate_secret_patterns
        def screen(path, *args, **kwargs):
            screened.append(Path(path).name)
            return real(path, *args, **kwargs)
        with mock.patch.object(self.context, 'validate_secret_patterns', screen):
            self.assertEqual(3, len(self.index()))
            self.assertEqual(3, len(screened))
            self.assertEqual(3, len(self.index()))
            self.assertEqual(3, len(screened))
            changed = next(self.chunks.glob('MEM-*-heron-1.md'))
            before = changed.stat()
            changed.write_text(changed.read_text(encoding='utf-8') + '\nHerons also wade at dusk.\n',
                               encoding='utf-8')
            os.utime(changed, ns=(before.st_atime_ns, before.st_mtime_ns + 1_000_000))
            self.assertEqual(3, len(self.index()))
            self.assertEqual(4, len(screened))
            self.assertEqual(changed.name, screened[-1])

    def test_a_citation_is_looked_at_once_a_pass_and_again_in_the_next(self):
        asked = []
        real = self.validate.workspace_roots.contains
        def contains(repository, path):
            asked.append(Path(path).name)
            return real(repository, path)
        with mock.patch.object(self.validate.workspace_roots, 'contains', contains):
            self.assertEqual(3, len(self.index()))
            self.assertEqual(1, asked.count('heron.md'))
            # A citation that goes away still takes its chunks out.
            (self.root / 'specs/heron.md').unlink()
            self.assertEqual([], self.index())
        self.assertIsNone(getattr(self.validate._SOURCE_FACTS, 'memo', None))


@unittest.skipUnless(shutil.which('git'), 'Git')
class ConcurrentCheckpointTests(unittest.TestCase):
    """Another session's turn checkpoint can rewrite the task record while a
    prompt's capsule is assembled. The working state read before it is then
    left out, and the capsule said only "working: unavailable"."""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='heron-pair-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        env = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        env.update(HOME=str(self.root), XDG_CONFIG_HOME=str(self.root / 'xdg'), GIT_OPTIONAL_LOCKS='0')
        patcher = mock.patch.dict(os.environ, env, clear=True)
        patcher.start(); self.addCleanup(patcher.stop)
        for relative in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                         'project-brain/schemas', 'project-brain/templates'):
            shutil.copytree(SCRIPTS.parents[1] / relative, self.root / relative,
                            ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        (self.root / 'docs').mkdir()
        (self.root / 'docs/heron.md').write_text('# Heron pairing\n\nHerons pair at dawn by the reeds.\n',
                                                 encoding='utf-8')
        for args in (('init', '-q', '-b', 'feature/heron-pair'), ('add', '-A'),
                     ('-c', 'user.email=h@example.invalid', '-c', 'user.name=H', 'commit', '-q', '-m', 'fixture')):
            subprocess.run(['git', '-C', str(self.root), *args], check=True, capture_output=True, timeout=60)
        subprocess.run([sys.executable, str(self.root / 'memory-bank/scripts/context.py'), '--root', str(self.root),
                        'start', '--task-id', 'feature/heron-pair', '--goal', 'Heron pairing', '--source',
                        'docs/heron.md', '--json'], check=True, capture_output=True, timeout=120)
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.context = importlib.import_module('context')
            self.retrieval = importlib.import_module('context_retrieval')
        finally:
            sys.path.pop(0)

    def refresh(self):
        arguments = ['context.py', '--root', str(self.root), 'refresh', '--host', 'claude',
                     '--query', 'When do herons pair?', '--task-id', 'feature/heron-pair',
                     '--expected-branch', 'feature/heron-pair', '--ephemeral', '--sanitize']
        with mock.patch.object(sys, 'argv', arguments), mock.patch.object(self.context, 'utf8_output'), \
             mock.patch.object(sys, 'stdout', new_callable=io.StringIO) as output, \
             mock.patch.object(sys, 'stderr', new_callable=io.StringIO):
            self.context.main()
        return output.getvalue()

    def checkpointed(self, text):
        """The record as another session's checkpoint leaves it."""
        _, frontmatter, body = text.split('---\n', 2)
        record = json.loads(frontmatter)
        record['progress'] = 'Another session checkpointed.'
        return '---\n' + json.dumps(record, indent=2) + '\n---\n' + body

    def test_a_record_rewritten_during_the_read_gives_its_new_state(self):
        self.assertIn('working: feature/heron-pair', self.refresh())
        real = self.retrieval._read_final_working_record
        with mock.patch.object(self.retrieval, '_read_final_working_record',
                               lambda path: self.checkpointed(real(path))):
            capsule = self.refresh()
        self.assertIn('working: feature/heron-pair', capsule)
        self.assertIn('Another session checkpointed.', capsule)

    def test_a_record_rewritten_at_the_last_moment_is_named(self):
        # Between the second reading and the custody check before delivery.
        real = self.retrieval._read_final_working_record
        readings = []
        def late(path):
            readings.append(path)
            return real(path) if len(readings) == 1 else self.checkpointed(real(path))
        with mock.patch.object(self.retrieval, '_read_final_working_record', late):
            capsule = self.refresh()
        self.assertEqual(2, len(readings))
        record = next((self.root / 'project-brain/dynamic/tasks').glob('*.md')).relative_to(self.root).as_posix()
        self.assertIn("working: unavailable (the task's record changed while this turn read it; read "
                      + record + ')', capsule)


class SourceKeyTests(unittest.TestCase):
    """A document's key is cut from the path string where the path is spelled
    under its base; every other spelling gets pathlib's answer or refusal."""

    def test_the_key_is_the_one_pathlib_gives(self):
        sys.path.insert(0, str(SCRIPTS))
        try:
            roots = importlib.import_module('workspace_roots')
        finally:
            sys.path.pop(0)
        repository = Path('/srv/heron')
        bases = [repository, repository / 'docs', Path('//srv'), Path('/'), Path('/srv/hero')]
        names = ['a.md', 'docs/a.md', 'docs/../a.md', 'd\u00e9j\u00e0 vu.md', 'a b/c.md', '.hidden/x.md']
        for base in bases:
            for name in names:
                for path in (base / name, repository / name, Path('/srv/heronX') / name, base):
                    with self.subTest(base=str(base), path=str(path)):
                        try:
                            expected = path.relative_to(base).as_posix()
                        except ValueError:
                            with self.assertRaises(ValueError):
                                roots.key_for(repository, base, path)
                        else:
                            self.assertEqual(expected, roots.key_for(repository, base, path))


class LongDocumentTests(unittest.TestCase):
    """A document longer than MARKED_DOCUMENT_CHARACTERS is marked only in the
    stretch that answers, once per command. FTS5 marked all of it, a third of
    a second a megabyte, several times a prompt: a 20 MB reference under
    docs/ that a question matched held the prompt for minutes."""

    def setUp(self):
        clean = {k: v for k, v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        environment = mock.patch.dict(os.environ, clean, clear=True)
        environment.start(); self.addCleanup(environment.stop)
        sys.path.insert(0, str(SCRIPTS))
        try:
            self.context = importlib.import_module('context')
            self.retrieval = importlib.import_module('context_retrieval')
            self.roots = importlib.import_module('workspace_roots')
        finally:
            sys.path.pop(0)
        self.limit = self.retrieval.MARKED_DOCUMENT_CHARACTERS
        filler = '\n'.join(f'## Part {n}\n\n' + ' '.join(['ledger account report'] * 80) for n in range(400))
        self.answer = '## Heron lantern\n\nThe heron lantern keeper trims the wick at dusk.\n'
        self.text = filler[:len(filler) // 2] + '\n' + self.answer + filler[len(filler) // 2:]

    def test_the_stretch_is_the_section_where_the_words_meet(self):
        stretch = self.retrieval._answering_stretch(self.text, ['heron', 'lantern', 'keeper'])
        self.assertTrue(stretch.startswith('## Heron lantern'), stretch[:80])
        self.assertLessEqual(len(stretch), self.limit)

    def test_a_long_section_keeps_its_heading_and_its_densest_window(self):
        section = ('## Heron\n\n' + 'filler words here. ' * 20000 + 'The heron lantern keeper waits.\n'
                   + 'more filler. ' * 10000)
        stretch = self.retrieval._answering_stretch(section, ['heron', 'lantern', 'keeper'])
        self.assertTrue(stretch.startswith('## Heron\n'))
        self.assertIn('heron lantern keeper', stretch)
        self.assertLessEqual(len(stretch), self.limit)

    def test_a_long_document_is_never_marked_whole_and_once_per_command(self):
        folder = tempfile.TemporaryDirectory(prefix='heron-long-')
        self.addCleanup(folder.cleanup)
        connection = self.context.connect(Path(folder.name) / 'context.db')
        self.addCleanup(connection.close)
        rows = [('docs/long.md', 'semantic', 'doc', 'Long reference', 'Long reference', self.text)]
        rows += [(f'docs/other-{n}.md', 'semantic', 'doc', f'Other {n}', f'Other {n}', 'ledger account report')
                 for n in range(20)]
        with connection:
            connection.executemany('INSERT INTO documents(path, layer, kind, title, summary, content) '
                                   'VALUES (?, ?, ?, ?, ?, ?)', rows)
        marked = []
        real = self.retrieval._held_marked_document
        def held(image, tokens):
            marked.append(len(image[2]))
            return real(image, tokens)
        with mock.patch.object(self.retrieval, '_held_marked_document', held), self.roots.source_frame_scope():
            first = self.retrieval.marked_document(connection, 'docs/long.md', 'heron lantern keeper')
            again = self.retrieval.marked_document(connection, 'docs/long.md', 'heron lantern keeper')
        self.assertEqual(first, again)
        self.assertEqual(1, len(marked))
        self.assertLessEqual(marked[0], self.limit)
        self.assertIn(self.retrieval.EXCERPT_MARK_OPEN + 'heron' + self.retrieval.EXCERPT_MARK_CLOSE,
                      first.casefold())
        # Outside a command nothing is kept: a long-lived server asks again.
        with mock.patch.object(self.retrieval, '_held_marked_document', held):
            self.retrieval.marked_document(connection, 'docs/long.md', 'heron lantern keeper')
            self.retrieval.marked_document(connection, 'docs/long.md', 'heron lantern keeper')
        self.assertEqual(3, len(marked))


if __name__ == '__main__':
    unittest.main()
