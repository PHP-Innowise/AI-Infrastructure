"""Fresh Kingfisher policy upgrades and an independent public SQLite oracle."""
import contextlib
import copy
import hashlib
import importlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
P = json.loads((ROOT/'tests/fixtures/memory_secret_policy_delivery_public.json').read_text(encoding='utf-8'))


class SecretPolicyDeliveryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='kingfisher-policy-')
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.root = self.base/'project'
        self.root.mkdir()
        self.env = {k: os.environ[k] for k in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'LC_CTYPE',
                    'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT') if k in os.environ}
        self.env.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1',
                        TMPDIR=str(self.base), XDG_CONFIG_HOME=str(self.base/'xdg'))
        patch = mock.patch.dict(os.environ, self.env, clear=True)
        patch.start()
        self.addCleanup(patch.stop)
        subprocess.run(['git', '-C', str(self.root), 'init', '-q', '-b', 'kingfisher-policy'],
                       env=self.env, check=True, capture_output=True, timeout=8)
        runtime = ROOT/'PHP Core/memory-bank/scripts'
        names = [f.stem for f in runtime.glob('*.py')]
        old = {n: sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:
                sys.modules.pop(n, None)
            sys.modules.update(old)
        self.addCleanup(restore)
        sys.path.insert(0, str(runtime))
        try:
            self.c = importlib.import_module('context')
            self.r = importlib.import_module('context_retrieval')
            self.v = importlib.import_module('validate')
        finally:
            sys.path.remove(str(runtime))
        for rel in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                    'project-brain/schemas', 'project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel, self.root/rel,
                            ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
        cfg = json.loads((self.root/'project-brain/config/runtime.json').read_text(encoding='utf-8'))
        cfg.update(automatic_promotion=False, automatic_compaction=False)
        self.write('project-brain/config/runtime.json', json.dumps(cfg))
        self.write(P['source'], '<?php // Kingfisher public source\n')
        patch = mock.patch.dict(self.v.SECRET_PATTERNS,
                                {P['old_label']: re.compile(re.escape(P['old_marker']))})
        patch.start()
        self.addCleanup(patch.stop)
        self.db = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(self.db.close)
        self.rows = {}
        self.receipts = []
        self.beforeorder = []
        self.scores = {}

    def write(self, name, text):
        source_file = self.root/name
        source_file.parent.mkdir(parents=True, exist_ok=True)
        with source_file.open('w', encoding='utf-8', newline='\n') as stream:
            stream.write(text)
        return source_file

    def upgrade(self):
        self.v.SECRET_PATTERNS[P['new_label']] = re.compile(re.escape(P['new_marker']))

    def index(self, variant='body', strict=None):
        variant_row = P['variants'][variant]
        self.target = P['path_target'] if variant == 'path' else P['target']
        self.rows[self.target] = {'title': variant_row['title'], 'summary': variant_row['summary'],
                                  'raw': variant_row['text'], 'indexed': variant_row['text']}
        for key, row in P['survivors'].items():
            self.rows[P[key]] = row
        for name, row in self.rows.items():
            self.write(name, row['raw'])
        self.neutral = []
        for i in range(14):
            name = 'specs/neutral-%d.md' % i
            text = '# Neutral %d\n\nUnrelated public inventory %d.\n' % (i, i)
            self.write(name, text)
            self.neutral.append((name, 'semantic', 'spec', 'Neutral %d' % i,
                                 'Unrelated public inventory %d.' % i, text))
        self.strict_file = None
        if strict == 'memory':
            self.strict_file = 'memory-bank/chunks/MEM-0963-kingfisher-policy.md'
            metadata = {'id': 'MEM-0963', 'title': P['query'], 'type': 'constraint', 'status': 'active',
                        'scope': ['public'], 'tags': ['kingfisher'], 'created': '2026-10-01',
                        'last_verified': '2026-10-01', 'review_after': '2031-10-01',
                        'sources': [P['source']], 'supersedes': [], 'superseded_by': None}
            self.write(self.strict_file, '---\n'+json.dumps(metadata)+'\n---\n# '+P['query']+'\n\n'+P['new_marker']+'\n')
        elif strict == 'brain':
            cli = self.root/'memory-bank/scripts/context.py'
            for args in [
                ['start', '--task-id', 'kingfisher-policy', '--goal', P['query'], '--source', P['source'], '--json'],
                ['update', '--task-id', 'kingfisher-policy', '--revision', '1', '--progress', P['new_marker'], '--json'],
            ]:
                result = subprocess.run([sys.executable, '-B', str(cli), *args], cwd=self.root,
                                        env=self.env, capture_output=True, timeout=15)
                self.assertEqual(0, result.returncode, result.stderr)
            self.strict_file = self.r.find_task(self.root, 'kingfisher-policy')[0].relative_to(self.root).as_posix()
        self.c.index_repository(self.db, self.root)
        for name, row in self.rows.items():
            actual = self.db.execute('SELECT title,summary,content FROM documents WHERE path=?', (name,)).fetchone()
            self.assertEqual((row['title'], row['summary'], row['indexed']), tuple(actual))
        names = [*self.rows, *([self.strict_file] if self.strict_file else [])]
        self.images = {name: ((self.root/name).read_bytes(), (self.root/name).stat().st_mtime_ns) for name in names}
        self.domain = self.domain_image()

    def domain_image(self):
        value = {table: [tuple(row) for row in self.db.execute('SELECT * FROM '+table+' ORDER BY 1')]
                 for table in ('documents', 'document_metadata', 'document_links', 'document_source_state')}
        value['index_fingerprints'] = [tuple(row) for row in self.db.execute(
            'SELECT key,value FROM index_state WHERE key IN (?,?,?,?,?) ORDER BY key',
            ('config-fingerprint', 'skill-tree-fingerprint', 'secret-policy-fingerprint', 'skill-parity-drift', 'index-generation'))]
        return value

    def unchanged(self, domain=True):
        for name, image in self.images.items():
            self.assertEqual(image, ((self.root/name).read_bytes(), (self.root/name).stat().st_mtime_ns))
        if domain:
            self.assertEqual(self.domain, self.domain_image())

    def shadow(self, query, *, upgraded, limit=10):
        # Complete public FTS projection from literal expected index rows.
        # Implementation output and exported/gold rows never seed this oracle.
        db = sqlite3.connect(':memory:')
        self.addCleanup(db.close)
        db.row_factory = sqlite3.Row
        db.execute("CREATE VIRTUAL TABLE oracle USING fts5(path UNINDEXED,layer UNINDEXED,kind UNINDEXED,title,summary,content,tokenize='porter unicode61')")
        rows = [(name, 'semantic', 'spec', row['title'], row['summary'], row['indexed'])
                for name, row in self.rows.items()] + self.neutral
        db.executemany('INSERT INTO oracle VALUES(?,?,?,?,?,?)', rows)
        expression = ' OR '.join('"'+token+'"' for token in re.findall(r'\w+', query))
        result = [dict(row) for row in db.execute(
            "SELECT path,layer,kind,title,snippet(oracle,5,'[',']',' … ',18) AS snippet "
            'FROM oracle WHERE oracle MATCH ? ORDER BY bm25(oracle),path', (expression,))]
        if upgraded:
            result = [row for row in result if row['path'] != self.target]
        return result[:limit]

    def raw(self, variant, query=None, limit=10):
        query = query or P['query']
        self.index(variant)
        expected_before = self.shadow(query, upgraded=False, limit=10)
        self.assertEqual(expected_before, self.c.search_documents(self.db, query, 10, repository=self.root))
        if variant == 'beyond':
            self.assertNotIn(P['new_marker'], next(row['snippet'] for row in expected_before if row['path'] == P['target']))
        self.upgrade()
        expected = self.shadow(query, upgraded=True, limit=limit)
        full_index_state = [tuple(row) for row in self.db.execute('SELECT * FROM index_state ORDER BY key')]
        args = ['context.py', 'search', query, '--layer', 'semantic', '--limit', str(limit), '--json']
        output = io.StringIO()
        with mock.patch.object(self.c, 'default_root', return_value=self.root), mock.patch.object(sys, 'argv', args), \
                mock.patch.object(self.c, 'index_repository', side_effect=AssertionError('No reindex during search')), \
                contextlib.redirect_stdout(output):
            self.assertEqual(0, self.c.main())
        self.assertEqual(json.dumps({'query': query, 'documents': expected, 'episodes': []}, ensure_ascii=False)+'\n', output.getvalue())
        for row in json.loads(output.getvalue())['documents']:
            self.assertEqual(set(P['oracles']['raw_public_keys']), set(row))
        self.unchanged()
        self.assertEqual(full_index_state, [tuple(row) for row in self.db.execute('SELECT * FROM index_state ORDER BY key')])

    def retrieve(self, pack=None):
        original = self.r._commit_ordinary_receipts
        def capture(*args, **kwargs):
            self.receipts.append(copy.deepcopy(kwargs))
            return original(*args, **kwargs)
        candidates = self.r._candidates
        ranked = []
        def ranked_candidates(*args, **kwargs):
            value = candidates(*args, **kwargs)
            ranked.extend(value[0])
            return value
        def packing(value):
            self.beforeorder = [row['path'] for row in value['selected']]
            self.scores = {row['path']: float(row['adjusted_score']) for row in ranked}
            return pack(value) if pack else value
        with mock.patch.object(self.r, '_commit_ordinary_receipts', side_effect=capture), \
                mock.patch.object(self.r, '_candidates', side_effect=ranked_candidates):
            return self.r.retrieve(self.db, self.root, P['query'], None, limit=10, gate_mode='off',
                                   manifest_scope='local', pack=packing)

    def governed(self, variant, during=False, replace=False):
        self.index(variant)
        if not during:
            self.upgrade()
        def pack(value):
            self.assertEqual(set(P['oracles']['before_pack_members']), {row['path'] for row in value['selected']})
            self.upgrade()
            if replace:
                self.db.execute('UPDATE documents SET content=replace(content,?,?) WHERE path=?',
                                (P['new_marker'], '[redacted: public-new-marker]', P['target']))
                self.db.commit()
            return value
        result = self.retrieve(pack if during else None)
        expected = set(P['oracles']['post_upgrade_members'])
        self.assertEqual(expected, {row['path'] for row in result['selected']})
        self.assertEqual(expected, {row['path'] for key in ('procedural', 'semantic', 'episodic') for row in result[key] if 'path' in row})
        self.assertEqual(expected, {row['path'] for group in result['categories'].values() for row in group})
        manifest = json.loads((self.root/result['manifest']).read_text(encoding='utf-8'))
        self.assertEqual(expected, {row['path'] for row in manifest['selected']})
        self.assertIn({'path': P['target'], 'reason': 'secret-policy'}, manifest['excluded'])
        self.assertNotIn(P['new_marker'], json.dumps(result))
        self.assertEqual([name for name in self.beforeorder if name in expected], [row['path'] for row in result['selected']])
        for row in manifest['selected']:
            self.assertEqual(round(self.scores[row['path']], 6), row['score'])
            self.assertEqual(hashlib.sha256(self.images[row['path']][0]).hexdigest(), row['source_hash'])
        identities = sorted((row['path'], row['source_hash']) for row in manifest['selected'])
        digest = hashlib.sha256('\n'.join(name+'\0'+value for name, value in identities).encode('utf-8')).hexdigest()
        self.assertEqual(digest, self.receipts[-1]['baseline']['signature']['paths'])
        self.assertEqual(result['gate'], manifest['gate'])
        self.assertEqual(result['token_estimates'], manifest['token_estimates'])
        self.assertEqual(sum(row['estimated_tokens'] for row in result['selected']), result['token_estimates']['total'])
        self.unchanged(not replace)

    def strict_profile(self, kind):
        self.index(strict=kind)
        self.db.execute('UPDATE documents SET content=replace(content,?,?) WHERE path=?',
                        (P['new_marker'], '[redacted: public-new-marker]', self.strict_file))
        self.db.commit()
        self.upgrade()
        candidates, _ = self.r._candidates(self.db, P['query'], 100)
        self.assertIn(self.strict_file, {row['path'] for row in candidates})
        kept, excluded = self.r._runtime_filter(self.root, candidates, self.r.load_config(self.root), connection=self.db)
        self.assertNotIn(self.strict_file, {row['path'] for row in kept})
        self.assertIn({'path': self.strict_file, 'reason': 'secret-policy'}, excluded)
        self.unchanged(False)

    def test_raw_title(self): self.raw('title')
    def test_raw_path_only(self): self.raw('path')
    def test_raw_summary(self): self.raw('summary')
    def test_raw_body(self): self.raw('body')
    def test_raw_beyond_excerpt(self): self.raw('beyond')
    def test_raw_highlight_fracture(self): self.raw('body', 'kingfisher')
    def test_raw_filters_before_limit(self): self.raw('title', limit=1)
    def test_governed_initial_title(self): self.governed('title')
    def test_governed_initial_body(self): self.governed('body')
    def test_governed_pack_title(self): self.governed('title', True)
    def test_governed_pack_body(self): self.governed('body', True)
    def test_governed_pack_cache_replaced(self): self.governed('body', True, True)
    def test_strict_memory_source_never_masked_into_authority(self): self.strict_profile('memory')
    def test_strict_brain_source_never_masked_into_authority(self): self.strict_profile('brain')

    def test_masker_refusal_is_fixed_withholding(self):
        self.index()
        with mock.patch.object(self.v, 'mask_secrets', side_effect=self.v.ValidationError('Kingfisher supplied private diagnostic')):
            self.assertEqual([], self.c.search_documents(self.db, P['query'], 10, repository=self.root))
        self.unchanged()

    def test_policy_cache_is_not_a_persistent_permit(self):
        self.index()
        candidates = [{'path': name, 'kind': 'spec'} for name in self.rows]
        policy = self.r._CurrentSourcePolicy(self.db, self.root)
        policy.prime(candidates)
        self.assertIsNone(policy.reason(candidates[0]))
        self.upgrade()
        self.assertEqual('secret-policy', policy.reason(candidates[0]))
        self.assertIsNone(policy.reason(candidates[1]))
        self.unchanged()

    def test_source_hint_never_hydrates_oversize_cached_image(self):
        self.index()
        oversized = P['hint_oversize_codepoint'] * P['hint_oversize_repetitions']
        self.assertLess(len(oversized), self.r.SOURCE_HINT_FILE_BYTES)
        self.assertGreater(len(oversized.encode('utf-8')), self.r.SOURCE_HINT_FILE_BYTES)
        self.db.execute('UPDATE documents SET content=? WHERE path=?', (oversized, P['target']))
        self.db.commit()
        self.domain = self.domain_image()
        references = [{'path': name, 'source_sha256': hashlib.sha256(self.images[name][0]).hexdigest()}
                      for name in (P['target'], P['template'])]
        # Current small source passes native path/type/size/raw-SHA checks.
        self.assertEqual(self.images[P['target']][0].decode('utf-8'), self.r._source_hint_read(
            self.root, P['target'], references[0]['source_sha256']))
        images = []
        native = self.db
        class ObservedConnection:
            def execute(inner, sql, parameters=()):
                cursor = native.execute(sql, parameters)
                if re.match(r'SELECT\s+path,title,summary,content\s+FROM\s+documents', sql):
                    rows = cursor.fetchall()
                    images.append((sql, list(parameters), [row['path'] for row in rows],
                                   sum(len(row['content'].encode('utf-8')) for row in rows)))
                    return rows
                return cursor
        kept = self.r._source_hint_candidates(ObservedConnection(), self.root,
                                             self.r.load_config(self.root), references, set())
        self.assertEqual([P['template']], [row['path'] for row in kept])
        self.assertEqual(1, len(images))
        sql, parameters, names, hydrated_bytes = images[0]
        self.assertIn('length(CAST(content AS BLOB))<=?', sql)
        self.assertEqual(self.r.SOURCE_HINT_FILE_BYTES, parameters[-1])
        self.assertEqual([P['template']], names)
        self.assertEqual(len(P['survivors']['template']['indexed'].encode('utf-8')), hydrated_bytes)
        self.unchanged()


if __name__ == '__main__':
    unittest.main()
