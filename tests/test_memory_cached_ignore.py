"""New Kestrel public cached-ignore controls; real temp CLI/MCP plus SQLite paths."""
import hashlib
import importlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_cached_ignore_public.json').read_text(encoding='utf-8'))


class CachedIgnoreTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='kestrel-ignore-')
        self.addCleanup(temp.cleanup)
        self.base = Path(temp.name).resolve()
        self.root = self.base / 'project'
        self.root.mkdir()
        self.env = {k: os.environ[k] for k in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'LC_CTYPE',
                    'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT') if k in os.environ}
        self.env.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1',
                        XDG_CONFIG_HOME=str(self.base / 'xdg'), TMPDIR=str(self.base))
        patch = mock.patch.dict(os.environ, self.env, clear=True)
        patch.start()
        self.addCleanup(patch.stop)
        self.runtime = ROOT / 'PHP Core/memory-bank/scripts'
        names = [f.stem for f in self.runtime.glob('*.py')]
        previous = {n: sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:
                sys.modules.pop(n, None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0, str(self.runtime))
        try:
            self.c = importlib.import_module('context')
            self.r = importlib.import_module('context_retrieval')
            self.w = importlib.import_module('workspace_roots')
        finally:
            sys.path.remove(str(self.runtime))
        self.run_git('init', '-q', '-b', PUBLIC['task'])
        self.write(PUBLIC['source'], PUBLIC['source_text'])
        self.db = self.c.connect(self.root / 'memory-bank/local/context.db')
        self.addCleanup(self.db.close)
        self.r.ensure_metadata_tables(self.db)
        self.config = {'allowed_privacy': ['public', 'team'], 'owners': ['*'],
                       'allowed_authority': ['observed', 'verified']}
        self.observations = []
        self.capture_directory = getattr(self.__class__, 'capture_directory', None)

    def run_git(self, *args):
        return subprocess.run(['git', '-C', str(self.root), *args], env=self.env,
                              capture_output=True, timeout=8, check=True)

    def write(self, name, content):
        dest = self.root / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        with dest.open('w', encoding='utf-8', newline='\n') as stream:
            stream.write(content)
        return dest

    def observe(self, label, value):
        self.observations.append({'label': label, 'value': value})
        if self.capture_directory:
            dest = Path(self.capture_directory)
            dest.mkdir(parents=True, exist_ok=True)
            (dest / (self._testMethodName + '.json')).write_text(
                json.dumps(self.observations, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')

    def native(self):
        for rel in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                    'project-brain/schemas', 'project-brain/templates'):
            shutil.copytree(ROOT / 'PHP Core' / rel, self.root / rel,
                            ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        current = json.loads((self.root / 'project-brain/config/runtime.json').read_text(encoding='utf-8'))
        current.update(PUBLIC['configuration'])
        self.write('project-brain/config/runtime.json', json.dumps(current))
        self.cli = self.root / 'memory-bank/scripts/context.py'
        self.chunk()

    def chunk(self):
        metadata = {'id': 'MEM-0731', 'title': 'Kestrel amethyst latch', 'type': 'operations',
                    'status': 'active', 'scope': ['project'], 'tags': ['kestrel'],
                    'created': '2026-10-01', 'last_verified': '2026-10-01', 'review_after': '2099-01-01',
                    'sources': [PUBLIC['source']], 'supersedes': [], 'superseded_by': None}
        return self.write(PUBLIC['chunk'], '---\n' + json.dumps(metadata) + '\n---\n\n' + PUBLIC['source_text'])

    def call(self, *args):
        result = subprocess.run([sys.executable, '-B', str(self.cli), '--root', str(self.root), *args],
                                cwd=self.root, env=self.env, capture_output=True, timeout=25)
        self.observe(args[0], {'returncode': result.returncode, 'stdout': result.stdout.decode('utf-8'),
                             'stderr': result.stderr.decode('utf-8')})
        return result

    def row(self, name=PUBLIC['chunk'], sources=None, kind='memory', body=None):
        if body is None:
            body = PUBLIC['source_text']
        self.write(name, body)
        self.db.execute('INSERT INTO documents(path,layer,kind,title,summary,content) VALUES(?,?,?,?,?,?)',
                        (name, 'semantic', kind, 'Kestrel amethyst latch', '', body))
        metadata = self.r._legacy_metadata(name, kind, body)
        self.db.execute('INSERT INTO document_metadata VALUES(' + ','.join('?' for _ in metadata) + ')', metadata)
        for source in sources if sources is not None else [PUBLIC['source']]:
            self.db.execute('INSERT INTO document_links VALUES(?,?,?)', (name, source.split('#', 1)[0], 'source'))
        self.db.commit()
        return name

    def linked(self, reference=PUBLIC['source']):
        kept, excluded = self.r.linked_documents(self.db, self.root, self.config, reference)
        self.observe('linked', {'kept': [i['path'] for i in kept], 'excluded': excluded})
        return kept, excluded

    def test_cached_cli_search_withholds_after_ignore(self):
        self.native()
        # This case has one indexed ordinary doc, no chunk alias.
        (self.root / PUBLIC['chunk']).unlink()
        self.assertEqual(0, self.call('index', '--json').returncode)
        self.write('.gitignore', PUBLIC['ignore'])
        result = self.call('search', PUBLIC['query'], '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(PUBLIC['empty_search'], json.loads(result.stdout))
        self.assertEqual(b'', result.stderr)

    def test_cached_search_retains_eligible_runner_up(self):
        self.native()
        (self.root / PUBLIC['chunk']).unlink()
        self.write('docs/kestrel-public.md', PUBLIC['source_text'])
        self.assertEqual(0, self.call('index', '--json').returncode)
        self.write('.gitignore', PUBLIC['ignore'])
        result = self.call('search', PUBLIC['query'], '--limit', '1', '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(['docs/kestrel-public.md'], [d['path'] for d in json.loads(result.stdout)['documents']])

    def test_ignored_citation_does_not_withhold_the_citing_record(self):
        # The chunk is shared through the Memory Bank already; the file it
        # cites being ignored (vendor/, .env, a local note) hides nothing.
        self.row()
        self.write('.gitignore', PUBLIC['ignore'])
        kept, excluded = self.linked()
        self.assertEqual([PUBLIC['chunk']], [d['path'] for d in kept])
        self.assertEqual([], excluded)

    def test_ignored_accelerator_citation_keeps_the_record(self):
        self.write('specs/kestrel.md', PUBLIC['source_text'])
        self.row(sources=['specs/kestrel.md'])
        self.write('.gitignore', 'specs/\n')
        kept, excluded = self.linked('specs/kestrel.md')
        self.assertEqual([PUBLIC['chunk']], [d['path'] for d in kept])
        self.assertEqual([], excluded)

    def test_brain_own_path_ignore_keeps_authority(self):
        name = self.row('project-brain/records/kestrel.md', kind='spec')
        self.write('.gitignore', 'project-brain/\n')
        self.assertEqual([name], [d['path'] for d in self.linked()[0]])

    def test_url_citation_is_not_a_local_git_path(self):
        name = self.row(sources=['https://example.org/kestrel#latch'])
        self.write('.gitignore', '*\n')
        self.assertEqual([name], [d['path'] for d in self.linked('https://example.org/kestrel#latch')[0]])

    def test_direct_links_new_operation_does_not_reuse_positive(self):
        # A document's own path is re-observed by every operation.
        name = self.row('docs/kestrel-note.md', kind='doc')
        self.assertEqual([name], [d['path'] for d in self.linked()[0]])
        self.write('.gitignore', 'docs/kestrel-note.md\n')
        self.assertEqual([], self.linked()[0])

    def test_citations_add_no_git_probe(self):
        names = [self.row('memory-bank/chunks/MEM-%04d-kestrel.md' % (740+i)) for i in range(8)]
        self.write('.gitignore', PUBLIC['ignore'])
        original = subprocess.run
        probes = []
        def count(command, **kwargs):
            if 'check-ignore' in command:
                probes.append(kwargs.get('input'))
            return original(command, **kwargs)
        with mock.patch('subprocess.run', side_effect=count):
            kept, excluded = self.linked()
        self.observe('batches', [v.decode('utf-8') for v in probes])
        self.assertEqual(set(names), {d['path'] for d in kept})
        self.assertEqual([], excluded)
        # Chunks are accelerator documents and citations are not probed.
        self.assertEqual([], probes)

    def test_git_unknown_is_typed_not_success(self):
        self.row('docs/kestrel-note.md', kind='doc')
        original = subprocess.run
        def fail(command, **kwargs):
            if 'check-ignore' in command:
                raise FileNotFoundError('supplied missing Git executable')
            return original(command, **kwargs)
        with mock.patch('subprocess.run', side_effect=fail):
            with self.assertRaises(self.c.SourceGitError) as caught:
                self.linked()
        self.assertEqual(PUBLIC['git_error'], str(caught.exception))

    def task_native(self):
        self.native()
        result = self.call('start', '--task-id', PUBLIC['task'], '--goal', PUBLIC['query'],
                           '--source', PUBLIC['source'], '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        result = self.call('update', '--task-id', PUBLIC['task'], '--progress', PUBLIC['progress'],
                           '--revision', '1', '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(0, self.call('index', '--json').returncode)

    def test_native_working_projection_survives_an_ignored_source(self):
        self.task_native()
        self.write('.gitignore', PUBLIC['ignore'])
        result = self.call('retrieve', PUBLIC['query'], '--task-id', PUBLIC['task'], '--gate', 'off', '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(PUBLIC['progress'], value['working']['progress'])
        # The ignored file itself is not quoted.
        self.assertNotIn(PUBLIC['source'], [d['path'] for d in value['semantic']])
        record = self.call('brain-get', '--record-id', PUBLIC['task'], '--json')
        self.assertEqual(PUBLIC['progress'], json.loads(record.stdout)['progress'])

    def test_real_mcp_recall_keeps_working_and_cited_chunk(self):
        self.task_native()
        self.write('.gitignore', PUBLIC['ignore'])
        request = {'jsonrpc': '2.0', 'id': 731, 'method': 'tools/call', 'params': {
            'name': 'memory_retrieve', 'arguments': {'task_id': PUBLIC['task'], 'query': PUBLIC['query']}}}
        result = subprocess.run([sys.executable, '-B', str(self.root / 'memory-bank/scripts/mcp_server.py'),
                                  '--root', str(self.root)], input=('\n'.join(json.dumps(v) for v in [
                                    {'jsonrpc': '2.0', 'id': 730, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18'}},
                                    {'jsonrpc': '2.0', 'method': 'notifications/initialized'}, request])+'\n').encode('utf-8'),
                                env=self.env, capture_output=True, timeout=35)
        self.observe('mcp', {'returncode': result.returncode, 'stdout': result.stdout.decode('utf-8'),
                             'stderr': result.stderr.decode('utf-8')})
        self.assertEqual(0, result.returncode, result.stderr)
        response = [json.loads(line) for line in result.stdout.splitlines() if json.loads(line).get('id') == 731][0]['result']
        self.assertFalse(response['isError'])
        capsule = response['structuredContent']['capsule']
        self.assertEqual(PUBLIC['progress'], capsule['working']['progress'])
        self.assertNotIn(PUBLIC['source'], [d['path'] for d in capsule['semantic']])

    def test_packing_ignore_change_prunes_the_ignored_document(self):
        self.task_native()
        # Drop own task to isolate selected semantic knowledge from working.
        database = self.c.connect(self.root / 'memory-bank/local/context.db')
        self.addCleanup(database.close)
        def pack(result):
            self.write('.gitignore', PUBLIC['ignore'])
            return result
        result = self.r.retrieve(database, self.root, PUBLIC['query'], None, limit=5,
                                 manifest_scope='local', gate_mode='off', pack=pack)
        self.observe('pack', result)
        # Ignored after selection: the document itself leaves every view.
        for key in ('selected', 'semantic', 'procedural', 'episodic'):
            self.assertNotIn(PUBLIC['source'], [d.get('path') for d in result[key]])
        for items in result['categories'].values():
            self.assertNotIn(PUBLIC['source'], [d.get('path') for d in items])

    def test_normal_check_ignore_tracking_semantics_not_no_index(self):
        original = subprocess.run
        commands = []
        def record(command, **kwargs):
            commands.append(command)
            return original(command, **kwargs)
        with mock.patch('subprocess.run', side_effect=record):
            self.c._source_git_ignored(self.root, [PUBLIC['source']], 2)
        query = [c for c in commands if 'check-ignore' in c]
        self.assertEqual(1, len(query))
        self.assertNotIn('--no-index', query[0])
        self.assertIn('--stdin', query[0])
        self.assertIn('-z', query[0])


CachedIgnoreTests.capture_directory = os.environ.get('KESTREL_OBSERVATION_DIR')
if __name__ == '__main__':
    unittest.main()
