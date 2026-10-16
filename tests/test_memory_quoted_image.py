"""NEW Raven exact quoted-row custody; no closed fixture or golden export."""
import contextlib
import copy
import hashlib
import importlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
P = json.loads((ROOT/'tests/fixtures/memory_quoted_image_public.json').read_text(encoding='utf-8'))


class QuotedImageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='raven-quoted-')
        self.addCleanup(temporary.cleanup)
        self.base = Path(temporary.name)
        self.root = self.base/'project'
        self.root.mkdir()
        env = {k: os.environ[k] for k in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'LC_CTYPE',
               'SystemRoot', 'WINDIR', 'COMSPEC', 'PATHEXT') if k in os.environ}
        env.update(GIT_OPTIONAL_LOCKS='0', PYTHONDONTWRITEBYTECODE='1',
                   TMPDIR=str(self.base), XDG_CONFIG_HOME=str(self.base/'xdg'))
        patch = mock.patch.dict(os.environ, env, clear=True)
        patch.start()
        self.addCleanup(patch.stop)
        subprocess.run(['git', '-C', str(self.root), 'init', '-q', '-b', 'raven-custody'],
                       env=env, check=True, capture_output=True, timeout=8)
        runtime = ROOT/'PHP Core/memory-bank/scripts'
        names = [p.stem for p in runtime.glob('*.py')]
        previous = {n: sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:
                sys.modules.pop(n, None)
            sys.modules.update(previous)
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
        self.write(P['source'], '<?php // Raven public spindle source\n')
        patch = mock.patch.dict(self.v.SECRET_PATTERNS,
                                {P['old_label']: re.compile(re.escape(P['old_marker']))})
        patch.start()
        self.addCleanup(patch.stop)
        self.db = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(self.db.close)

    def write(self, name, text):
        source_file = self.root/name
        source_file.parent.mkdir(parents=True, exist_ok=True)
        with source_file.open('w', encoding='utf-8', newline='\n') as stream:
            stream.write(text)
        return source_file

    def prepare(self, contextual=False):
        self.raw = P['header']+(P['block'] if contextual else P['clean_body'])+'\n'
        self.write(P['target'], self.raw)
        self.write(P['safe'], P['safe_raw'])
        self.write(P['masked'], P['masked_raw'])
        for i in range(18):
            self.write('specs/raven-neutral-%d.md' % i, '# Inventory %d\n\nUnrelated inventory item.\n' % i)
        self.c.index_repository(self.db, self.root)
        self.initial_images = {n: ((self.root/n).read_bytes(), (self.root/n).stat().st_mtime_ns)
                               for n in (P['target'], P['safe'], P['masked'], P['source'])}
        self.initial_metadata = [tuple(row) for row in self.db.execute('SELECT * FROM document_metadata ORDER BY path')]
        self.initial_hash = self.db.execute('SELECT source_hash FROM document_metadata WHERE path=?', (P['target'],)).fetchone()[0]
        self.assertEqual(hashlib.sha256(self.raw.encode('utf-8')).hexdigest(), self.initial_hash)

    def upgrade(self):
        self.v.SECRET_PATTERNS[P['label']] = re.compile(P['pattern'])

    def tuple_for(self, name):
        row = self.db.execute('SELECT title,summary,content FROM documents WHERE path=?', (name,)).fetchone()
        return tuple(row)

    def candidate(self, name=None):
        name = name or P['target']
        return dict(self.db.execute('SELECT d.rowid AS document_rowid,d.path,d.layer,d.kind,d.title,m.source_hash '
                                   'FROM documents d JOIN document_metadata m ON m.path=d.path WHERE d.path=?', (name,)).fetchone())

    def unchanged_sources(self):
        for name, image in self.initial_images.items():
            self.assertEqual(image, ((self.root/name).read_bytes(), (self.root/name).stat().st_mtime_ns))
        self.assertEqual(self.initial_hash, self.db.execute(
            'SELECT source_hash FROM document_metadata WHERE path=?', (P['target'],)).fetchone()[0])

    def test_native_refresh_between_quote_and_prime_withholds_old_excerpt(self):
        self.prepare(True)
        expected_a = (P['title'], P['summary'], self.raw)
        expected_b = (P['title'], P['summary'], P['header']+'[redacted: '+P['label']+']\n')
        self.assertEqual(expected_a, self.tuple_for(P['target']))
        self.upgrade()
        native = self.r._candidates
        commit = self.r._commit_ordinary_receipts
        captured = {}
        receipts = []
        def remask(*args, **kwargs):
            value = native(*args, **kwargs)
            captured['rows'] = copy.deepcopy(value[0])
            target = next(row for row in value[0] if row['path'] == P['target'])
            self.assertEqual(P['quoted'], target['snippet'])
            self.assertNotIn('RAVEN_BOUNDARY', target['snippet'])
            # Actual native policy-fingerprint refresh of unchanged sources.
            self.c.index_repository(self.db, self.root)
            self.assertEqual(expected_b, self.tuple_for(P['target']))
            return value
        def record(*args, **kwargs):
            receipts.append(copy.deepcopy(kwargs))
            return commit(*args, **kwargs)
        with mock.patch.object(self.r, '_candidates', side_effect=remask), \
                mock.patch.object(self.r, '_commit_ordinary_receipts', side_effect=record):
            result = self.r.retrieve(self.db, self.root, P['query'], None, limit=10,
                                     gate_mode='off', manifest_scope='local')
        expected = {P['safe'], P['masked']}
        self.assertEqual(expected, {row['path'] for row in result['selected']})
        self.assertNotIn(P['value'], json.dumps(result))
        self.assertEqual(expected, {row['path'] for layer in ('procedural', 'semantic', 'episodic') for row in result[layer] if 'path' in row})
        self.assertEqual(expected, {row['path'] for group in result['categories'].values() for row in group})
        self.assertEqual([row['path'] for row in captured['rows'] if row['path'] in expected],
                         [row['path'] for row in result['selected']])
        quotes = {P['safe']: P['safe_quote'], P['masked']: P['masked_quote']}
        for row in result['selected']:
            self.assertEqual(set(P['public_keys']), set(row))
            self.assertEqual(quotes[row['path']], row['snippet'])
            self.assertEqual('semantic', row['layer'])
            self.assertEqual('spec', row['kind'])
        manifest = json.loads((self.root/result['manifest']).read_text(encoding='utf-8'))
        self.assertEqual(expected, {row['path'] for row in manifest['selected']})
        self.assertIn({'path': P['target'], 'reason': 'secret-policy'}, manifest['excluded'])
        scores = {row['path']: row['adjusted_score'] for row in captured['rows']}
        for row in manifest['selected']:
            self.assertEqual(round(scores[row['path']], 6), row['score'])
            self.assertEqual(hashlib.sha256(self.initial_images[row['path']][0]).hexdigest(), row['source_hash'])
        identities = sorted((row['path'], row['source_hash']) for row in manifest['selected'])
        signature = hashlib.sha256('\n'.join(n+'\0'+h for n, h in identities).encode('utf-8')).hexdigest()
        self.assertEqual(signature, receipts[-1]['baseline']['signature']['paths'])
        self.assertEqual(result['gate'], manifest['gate'])
        self.assertEqual(result['token_estimates'], manifest['token_estimates'])
        self.assertEqual(sum(row['estimated_tokens'] for row in result['selected']), result['token_estimates']['total'])
        self.assertEqual(expected_a, next(row for row in captured['rows'] if row['path'] == P['target'])['_indexed_policy_image'])
        self.unchanged_sources()
        self.assertEqual(self.initial_metadata, [tuple(row) for row in self.db.execute('SELECT * FROM document_metadata ORDER BY path')])

    def quote(self, evidence=None, connection=None):
        item = self.candidate()
        self.r._quote_candidates(connection or self.db, [item], P['query'], evidence or ['raven', 'copper', 'spindle'])
        return item

    def test_highlight_holds_exact_unhighlighted_tuple(self):
        self.prepare(True)
        item = self.quote()
        self.assertEqual((P['title'], P['summary'], self.raw), item['_indexed_policy_image'])
        self.assertEqual(P['quoted'], item['snippet'])
        self.assertEqual(self.initial_hash, item['source_hash'])
        self.unchanged_sources()

    def test_plain_fallback_holds_exact_unhighlighted_tuple(self):
        self.prepare()
        item = self.quote(['unmatchable_raven_word'])
        self.assertEqual((P['title'], P['summary'], self.raw), item['_indexed_policy_image'])
        self.assertEqual(P['clean_body'], item['snippet'])
        self.unchanged_sources()

    def test_missing_quote_cannot_rebind_later_cached_row(self):
        self.prepare()
        native = self.db
        class MissingQuote:
            def __getattr__(inner, name):
                return getattr(native, name)
            def __enter__(inner):
                native.__enter__()
                return inner
            def __exit__(inner, *args):
                return native.__exit__(*args)
            def execute(inner, sql, parameters=()):
                if sql.startswith('SELECT rowid') or sql.startswith('SELECT d.rowid'):
                    return native.execute('SELECT rowid,content FROM documents WHERE 0')
                return native.execute(sql, parameters)
        item = self.quote(connection=MissingQuote())
        policy = self.r._CurrentSourcePolicy(self.db, self.root)
        policy.prime([item])
        self.assertEqual('stale', policy.reason(item))
        self.assertIn('_indexed_policy_image', item)
        self.assertIsNone(item['_indexed_policy_image'])
        self.unchanged_sources()

    def quote_changed_identity(self, field, replacement):
        self.prepare()
        item = self.candidate()
        self.db.execute('UPDATE documents SET '+field+'=? WHERE path=?', (replacement, P['target']))
        self.db.commit()
        self.r._quote_candidates(self.db, [item], P['query'], ['raven', 'copper', 'spindle'])
        self.assertIn('_indexed_policy_image', item)
        self.assertIsNone(item['_indexed_policy_image'])
        self.assertEqual(self.initial_hash, item['source_hash'])
        self.unchanged_sources()

    def test_changed_quote_title_is_not_silently_adopted(self): self.quote_changed_identity('title', 'Foreign Raven title')
    def test_reused_rowid_foreign_path_is_not_silently_adopted(self): self.quote_changed_identity('path', P['foreign_path'])
    def test_changed_quote_kind_is_not_silently_adopted(self): self.quote_changed_identity('kind', 'policy')
    def test_changed_quote_layer_is_not_silently_adopted(self): self.quote_changed_identity('layer', 'procedural')

    def test_conflict_row_retains_same_statement_body(self):
        self.prepare()
        self.db.execute('UPDATE document_metadata SET record_id=? WHERE path=?', (P['event_id'], P['target']))
        rows = self.r._conflict_candidates(self.db, {P['event_id']})
        self.assertEqual(1, len(rows))
        self.assertEqual((P['title'], P['summary'], self.raw), rows[0]['_indexed_policy_image'])
        self.assertEqual(P['clean_snippet'], rows[0]['snippet'])
        self.assertNotIn('content', rows[0])
        self.assertNotIn('summary', rows[0])

    def test_history_event_shared_quote_keeps_exact_body(self):
        self.prepare()
        self.db.execute('UPDATE documents SET kind=?,layer=? WHERE path=?', ('brain-event', 'episodic', P['target']))
        history = {'terms': ['raven', 'spindle'], 'weights': {'raven': 1.0, 'spindle': 1.0},
                   'minimum': 2, 'floor': 2.0, 'events': {'raven': {P['target']}, 'spindle': {P['target']}}, 'episodes': {}}
        events, episodes = self.r._history_candidates(self.db, history)
        self.assertEqual([], episodes)
        self.assertEqual([P['target']], [row['path'] for row in events])
        self.assertEqual((P['title'], P['summary'], self.raw), events[0]['_indexed_policy_image'])
        self.assertEqual(P['clean_body'], events[0]['snippet'])

    def link_fixture(self):
        self.db.execute('INSERT INTO document_links(path,ref_path,ref_kind) VALUES(?,?,?)',
                        (P['target'], P['source'], 'source'))
        self.db.commit()

    def test_explicit_path_quote_reuses_admitted_body(self):
        self.prepare()
        self.link_fixture()
        original = self.r.linked_documents
        def replace(*args, **kwargs):
            rows = original(*args, **kwargs)
            self.db.execute('UPDATE documents SET content=? WHERE path=?',
                            (P['header']+P['replacement_body']+'\n', P['target']))
            self.db.commit()
            return rows
        with mock.patch.object(self.r, 'linked_documents', side_effect=replace):
            rows, excluded = self.r._path_candidates(self.db, self.root, self.r.load_config(self.root), [P['source']], set())
        self.assertEqual([], excluded)
        self.assertEqual([P['target']], [row['path'] for row in rows])
        self.assertEqual(P['clean_snippet'], rows[0]['snippet'])
        self.assertEqual((P['title'], P['summary'], self.raw), rows[0]['_indexed_policy_image'])
        self.unchanged_sources()

    def hint_after_admission(self, oversized=False):
        self.prepare()
        reference = {'path': P['target'], 'source_sha256': self.initial_hash}
        original = self.r._runtime_filter
        def replace(*args, **kwargs):
            rows = original(*args, **kwargs)
            body = P['hint_multibyte']*P['hint_repetitions'] if oversized else P['header']+P['replacement_body']+'\n'
            self.db.execute('UPDATE documents SET content=? WHERE path=?', (body, P['target']))
            self.db.commit()
            return rows
        with mock.patch.object(self.r, '_runtime_filter', side_effect=replace):
            rows = self.r._source_hint_candidates(self.db, self.root, self.r.load_config(self.root), [reference], set())
        self.assertEqual([P['target']], [row['path'] for row in rows])
        self.assertEqual(P['clean_snippet'], rows[0]['snippet'])
        self.assertEqual((P['title'], P['summary'], self.raw), rows[0]['_indexed_policy_image'])
        self.unchanged_sources()

    def test_hint_quote_reuses_admitted_body(self): self.hint_after_admission()
    def test_hint_later_oversize_cache_cannot_replace_held_body(self): self.hint_after_admission(True)

    def test_automatic_link_holds_same_statement_image(self):
        self.prepare(True)
        self.link_fixture()
        self.db.execute('UPDATE document_metadata SET category=? WHERE path=?', ('durable', P['target']))
        self.db.commit()
        expected_a = self.tuple_for(P['target'])
        self.upgrade()
        original = self.db
        class ReplaceAfterRows:
            def execute(inner, sql, parameters=()):
                cursor = original.execute(sql, parameters)
                if 'd.content' in sql and 'd.path IN' in sql:
                    rows = cursor.fetchall()
                    original.execute('UPDATE documents SET content=? WHERE path=?',
                                     (P['header']+'[redacted: '+P['label']+']\n', P['target']))
                    original.commit()
                    return types.SimpleNamespace(fetchall=lambda: rows)
                return cursor
        rows, excluded = self.r._automatic_link_candidates(
            ReplaceAfterRows(), self.root, self.r.load_config(self.root), [(P['source'], 'prompt')], [],
            own_paths=set(), coverage={})
        self.assertEqual([], rows)
        self.assertEqual([], excluded)
        self.assertIn(P['value'], expected_a[2])
        self.unchanged_sources()

    def test_source_graph_quote_reuses_admitted_body(self):
        self.prepare()
        self.db.execute('UPDATE document_metadata SET category=? WHERE path=?', ('durable', P['target']))
        self.db.execute('INSERT INTO document_links(path,ref_path,ref_kind) VALUES(?,?,?)',
                        (P['target'], P['safe'], 'source'))
        self.db.commit()
        anchors, _ = self.r._candidates(self.db, P['query'], 100)
        anchor = next(row for row in anchors if row['path'] == P['safe'])
        original = self.r.linked_documents
        def replace(*args, **kwargs):
            rows = original(*args, **kwargs)
            self.db.execute('UPDATE documents SET content=? WHERE path=?',
                            (P['header']+P['replacement_body']+'\n', P['target']))
            self.db.commit()
            return rows
        with mock.patch.object(self.r, 'linked_documents', side_effect=replace), \
                mock.patch.object(self.r, '_shared_source_documents', return_value=([], 0)):
            rows, stats = self.r._source_link_candidates(self.db, self.root, self.r.load_config(self.root),
                                                        [anchor], {P['safe'], P['masked']})
        self.assertEqual([P['target']], [row['path'] for row in rows])
        self.assertEqual(P['clean_snippet'], rows[0]['snippet'])
        self.assertEqual((P['title'], P['summary'], self.raw), rows[0]['_indexed_policy_image'])
        self.assertEqual(1, stats['eligible'])
        self.unchanged_sources()

    def test_already_held_oversize_hint_is_refused_before_masking(self):
        self.prepare()
        oversized = P['hint_multibyte']*P['hint_repetitions']
        self.assertGreater(len(oversized.encode('utf-8')), self.r.SOURCE_HINT_FILE_BYTES)
        policy = self.r._CurrentSourcePolicy(self.db, self.root)
        item = {'path': P['target'], 'kind': 'spec', 'title': P['title'],
                '_indexed_policy_image': (P['title'], P['summary'], oversized)}
        policy.prime([item])
        trace = types.SimpleNamespace(_current_source_policy=policy, observe=lambda *a, **kw: None)
        original = self.v.mask_secrets
        checked = []
        def watch(text):
            checked.append(len(text.encode('utf-8')))
            return original(text)
        with mock.patch.object(self.v, 'mask_secrets', side_effect=watch):
            rows = self.r._source_hint_candidates(self.db, self.root, self.r.load_config(self.root),
                [{'path': P['target'], 'source_sha256': self.initial_hash}], set(), trace=trace)
        self.assertEqual([], rows)
        self.assertFalse(any(size > self.r.SOURCE_HINT_FILE_BYTES for size in checked))
        self.unchanged_sources()


if __name__ == '__main__':
    unittest.main()
