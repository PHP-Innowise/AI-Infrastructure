"""NEW Kestrel rendered-image controls with actual native policy refresh."""
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
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
P = json.loads((ROOT/'tests/fixtures/memory_rendered_image_public.json').read_text(encoding='utf-8'))


class RenderedImageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='kestrel-rendered-')
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
        subprocess.run(['git', '-C', str(self.root), 'init', '-q', '-b', 'kestrel-render'],
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
        self.write(P['source'], '<?php // Kestrel public source\n')
        self.raw = P['header']+P['approved']+'\n\n'+P['block']+P['tail']
        self.masked = P['header']+P['approved']+'\n\n[redacted: '+P['label']+']'+P['tail']
        self.write(P['target'], self.raw)
        self.write(P['safe'], P['safe_raw'])
        for i in range(18):
            self.write('specs/kestrel-neutral-%d.md' % i, '# Neutral %d\n\nPublic inventory only.\n' % i)
        patch = mock.patch.dict(self.v.SECRET_PATTERNS, {P['label']: re.compile(P['pattern'])})
        patch.start()
        self.addCleanup(patch.stop)
        self.db = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(self.db.close)
        self.c.index_repository(self.db, self.root)
        self.assertEqual(self.masked, self.db.execute('SELECT content FROM documents WHERE path=?', (P['target'],)).fetchone()[0])
        self.images = {n: ((self.root/n).read_bytes(), (self.root/n).stat().st_mtime_ns)
                       for n in (P['target'], P['safe'], P['source'])}
        self.receipts = []
        original = self.r._commit_ordinary_receipts
        def record(*args, **kwargs):
            self.receipts.append(copy.deepcopy(kwargs))
            return original(*args, **kwargs)
        patch = mock.patch.object(self.r, '_commit_ordinary_receipts', side_effect=record)
        patch.start()
        self.addCleanup(patch.stop)

    def write(self, name, text):
        source_file = self.root/name
        source_file.parent.mkdir(parents=True, exist_ok=True)
        with source_file.open('w', encoding='utf-8', newline='\n') as stream:
            stream.write(text)

    def older_refresh(self):
        pattern = self.v.SECRET_PATTERNS.pop(P['label'])
        try:
            self.c.index_repository(self.db, self.root, incremental=True)
        finally:
            self.v.SECRET_PATTERNS[P['label']] = pattern
        self.assertEqual(self.raw, self.db.execute('SELECT content FROM documents WHERE path=?', (P['target'],)).fetchone()[0])
        self.unchanged()

    def unchanged(self):
        for name, image in self.images.items():
            self.assertEqual(image, ((self.root/name).read_bytes(), (self.root/name).stat().st_mtime_ns))
        self.assertEqual(hashlib.sha256(self.raw.encode('utf-8')).hexdigest(), self.db.execute(
            'SELECT source_hash FROM document_metadata WHERE path=?', (P['target'],)).fetchone()[0])

    def assemble(self, *, mode='governed', query=None, session=None, render=True):
        return self.c.assemble_capsule(self.db, self.root, mode=mode, query=query or P['query'],
            task_id='kestrel-render', limit=10, ephemeral=True, refresh_index=False,
            allow_unprovisioned=True, gate_mode='off', render=render, session_id=session)

    def assert_packet(self, capsule):
        expected = {P['target'], P['safe']}
        self.assertEqual(expected, {row['path'] for row in capsule['semantic']})
        self.assertEqual(expected, {row['path'] for row in capsule['selected']})
        self.assertEqual([P['safe'], P['target']], [row['path'] for row in capsule['selected']])
        for row in capsule['selected']:
            self.assertEqual({'path', 'layer', 'kind', 'title', 'snippet', 'category',
                              'estimated_tokens', 'record_id', 'conflicts'}, set(row))
            self.assertEqual(('semantic', 'spec', 'evidence', None, []),
                             (row['layer'], row['kind'], row['category'], row['record_id'], row['conflicts']))
        self.assertEqual(expected, {row['path'] for group in capsule['categories'].values() for row in group})
        manifest = json.loads((self.root/capsule['manifest']).read_text(encoding='utf-8'))
        self.assertEqual(expected, {row['path'] for row in manifest['selected']})
        for row in manifest['selected']:
            self.assertEqual(hashlib.sha256(self.images[row['path']][0]).hexdigest(), row['source_hash'])
        self.assertEqual(capsule['token_estimates'], manifest['token_estimates'])
        self.assertEqual(capsule['gate'], manifest['gate'])
        identities = sorted((name, hashlib.sha256(self.images[name][0]).hexdigest()) for name in expected)
        signature = hashlib.sha256('\n'.join(name+'\0'+digest for name, digest in identities).encode('utf-8')).hexdigest()
        self.assertEqual(signature, self.receipts[-1]['baseline']['signature']['paths'])
        wire = self.c.serialize_capsule(capsule)
        self.assertNotIn(P['value'], wire)
        self.assertNotIn('_indexed', wire)
        self.assertNotIn('_images', wire)
        self.assertNotIn(self.masked, wire)
        self.unchanged()

    def assert_excerpts(self, capsule):
        excerpts = self.c.capsule_excerpts(self.db, capsule)
        self.assertEqual({P['target']: P['expected_excerpt'], P['safe']: P['safe_excerpt']},
                         {name: str(text) for name, text in excerpts.items()})
        self.assertNotIn(P['value'], '\n'.join(self.c.render_capsule_lines(capsule, excerpts)))
        return excerpts

    def test_native_older_policy_refresh_during_pack_keeps_admitted_render(self):
        original = self.c.enforce_governed_capsule_contract
        def replace(value):
            self.older_refresh()
            return original(value)
        with mock.patch.object(self.c, 'enforce_governed_capsule_contract', side_effect=replace):
            capsule = self.assemble()
        self.assert_excerpts(capsule)
        self.assert_packet(capsule)

    def test_native_older_policy_refresh_after_return_keeps_final_print(self):
        capsule = self.assemble()
        self.older_refresh()
        excerpts = self.assert_excerpts(capsule)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.c.print_capsule(capsule, excerpts, repository=self.root)
        self.assertIn(P['expected_excerpt'], output.getvalue())
        self.assertNotIn(P['value'], output.getvalue())
        self.assert_packet(capsule)

    def test_window_and_section_receipt_share_admitted_image(self):
        original = self.c.describe_excerpt_windows
        proposals = []
        def replace(connection, capsule):
            self.older_refresh()
            result = original(connection, capsule)
            proposals.append(result)
            return result
        with mock.patch.object(self.c, 'describe_excerpt_windows', side_effect=replace):
            capsule = self.assemble(session=P['session'])
        expected_window = {'sha256': hashlib.sha256(P['expected_excerpt'].encode('utf-8')).hexdigest(),
                           'bytes': len(P['expected_excerpt'].encode('utf-8')), 'state': 'excerpt'}
        self.assertEqual(expected_window, proposals[0][P['target']])
        expected_revision = hashlib.sha256((P['title']+'\n'+P['approved']+'\n[redacted: '+P['label']+']').encode('utf-8')).hexdigest()[:16]
        planned = self.receipts[-1]['session']
        self.assertEqual([expected_revision, 1], planned['delivered'][P['target']+'#kestrel-indigo-latch'])
        self.assert_excerpts(capsule)
        self.assert_packet(capsule)

    def test_lightweight_budget_preserves_captured_search_image(self):
        capsule = self.assemble(mode='lightweight')
        self.older_refresh()
        self.assert_excerpts(capsule)
        self.unchanged()

    def test_healthy_rich_excerpt_keeps_fts_stemming_and_heading(self):
        capsule = self.assemble(query=P['healthy_query'])
        connect = self.r.sqlite3.connect
        with mock.patch.object(self.r.sqlite3, 'connect', wraps=connect) as opened:
            excerpts = self.c.capsule_excerpts(self.db, capsule)
        self.assertEqual(P['healthy_excerpt'], str(excerpts[P['target']]))
        self.assertEqual(P['healthy_heading'], excerpts[P['target']].heading)
        self.assertEqual(0, opened.call_count)

    def test_mismatch_marks_held_body_in_isolated_fts(self):
        capsule = self.assemble(query=P['healthy_query'])
        self.older_refresh()
        connect = self.r.sqlite3.connect
        with mock.patch.object(self.r.sqlite3, 'connect', wraps=connect) as opened:
            excerpts = self.c.capsule_excerpts(self.db, capsule)
        self.assertEqual(P['healthy_excerpt'], str(excerpts[P['target']]))
        self.assertEqual(P['healthy_heading'], excerpts[P['target']].heading)
        self.assertEqual(1, opened.call_count)
        self.assertEqual(':memory:', opened.call_args[0][0])

    def test_json_detachment_never_rehydrates_a_live_body(self):
        capsule = self.assemble()
        detached = json.loads(self.c.serialize_capsule(capsule))
        self.older_refresh()
        with mock.patch.object(self.c, 'marked_document', side_effect=AssertionError('detached live-body read')):
            excerpts = self.c.capsule_excerpts(self.db, detached)
        self.assertNotIn(P['value'], '\n'.join(str(text) for text in excerpts.values()))
        self.assertTrue(set(excerpts) <= {P['target'], P['safe']})

    def test_captured_missing_path_never_falls_back_to_database(self):
        capsule = self.assemble()
        empty = self.r._with_index_images(dict(capsule), {})
        self.older_refresh()
        with mock.patch.object(self.c, 'marked_document', side_effect=AssertionError('missing captured body')):
            self.assertEqual({}, self.c.capsule_excerpts(self.db, empty))

    def test_public_key_cannot_impersonate_private_capture(self):
        capsule = self.assemble()
        detached = dict(capsule)
        detached['_index_images'] = {P['target']: (P['title'], P['query'], self.raw)}
        with mock.patch.object(self.c, 'marked_document', side_effect=AssertionError('public capture spoof')):
            excerpts = self.c.capsule_excerpts(self.db, detached)
        self.assertNotIn(P['value'], '\n'.join(str(text) for text in excerpts.values()))

    def test_linked_task_renderer_uses_held_body(self):
        capsule = self.assemble()
        item = copy.deepcopy(capsule['semantic'][0])
        item.update(path=P['target'], kind='brain-task', selection='prompt-link', via=P['source'])
        packet = {'query': P['query'], 'semantic': [item], 'episodic': []}
        held = self.r._with_index_images(packet, {P['target']: (P['title'], P['query'], P['linked_body'])})
        self.older_refresh()
        excerpts = self.c.capsule_excerpts(self.db, held)
        self.assertEqual(P['linked_excerpt'], str(excerpts[P['target']]))
        self.assertEqual('Files', excerpts[P['target']].heading)

    def test_local_manifest_failure_return_retains_capture(self):
        with mock.patch.object(self.r, 'atomic_json', side_effect=OSError('Kestrel supplied publication failure')):
            capsule = self.assemble()
        self.assertIsNone(capsule['manifest'])
        self.assertEqual('unconfirmed', capsule['manifest_publication'])
        self.older_refresh()
        self.assert_excerpts(capsule)

    def test_fallback_sqlite_failure_uses_held_plain_body(self):
        capsule = self.assemble()
        self.older_refresh()
        with mock.patch.object(self.r.sqlite3, 'connect', side_effect=self.r.sqlite3.OperationalError('Kestrel supplied open failure')):
            self.assert_excerpts(capsule)

    def test_isolated_connection_closes_on_cancellation(self):
        capsule = self.assemble()
        self.older_refresh()
        local = mock.Mock()
        local.execute.side_effect = KeyboardInterrupt('Kestrel supplied cancellation')
        with mock.patch.object(self.r.sqlite3, 'connect', return_value=local):
            with self.assertRaises(KeyboardInterrupt):
                self.c.capsule_excerpts(self.db, capsule)
        local.close.assert_called_once_with()

    def test_shared_search_sink_keeps_first_successful_public_row_image(self):
        images = {}
        first = self.c.search_documents(self.db, P['query'], 10, 'semantic',
            relevant_only=True, repository=self.root, _image_sink=images)
        self.assertEqual((P['title'], P['query'], self.masked), images[P['target']])
        pattern = self.v.SECRET_PATTERNS.pop(P['label'])
        previous_label = 'public-kestrel-previous-label'
        self.v.SECRET_PATTERNS[previous_label] = pattern
        try:
            self.c.index_repository(self.db, self.root, incremental=True)
        finally:
            self.v.SECRET_PATTERNS.pop(previous_label)
            self.v.SECRET_PATTERNS[P['label']] = pattern
        expected_b = self.masked.replace('[redacted: '+P['label']+']', '[redacted: '+previous_label+']')
        self.assertEqual(expected_b, self.db.execute('SELECT content FROM documents WHERE path=?', (P['target'],)).fetchone()[0])
        second = self.c.search_documents(self.db, P['query'], 10, 'semantic',
            relevant_only=True, repository=self.root, _image_sink=images)
        self.assertEqual({P['target'], P['safe']}, {row['path'] for row in first})
        self.assertEqual({P['target'], P['safe']}, {row['path'] for row in second})
        public = self.c.deduplicate_context_items(first+second)
        self.assertEqual(first, public)
        self.assertEqual((P['title'], P['query'], self.masked), images[P['target']])
        capsule = self.r._with_index_images({'query': P['query'], 'semantic': public, 'episodic': []}, images)
        excerpts = self.c.capsule_excerpts(self.db, capsule)
        self.assertEqual(P['expected_excerpt'], str(excerpts[P['target']]))
        self.unchanged()

    def test_private_capture_is_immutable_and_deepcopy_retains_it(self):
        capsule = self.assemble()
        with self.assertRaises(AttributeError):
            capsule._admitted_images = ()
        cloned = copy.deepcopy(capsule)
        self.older_refresh()
        self.assert_excerpts(cloned)
        self.assertEqual(json.loads(self.c.serialize_capsule(capsule)), json.loads(self.c.serialize_capsule(cloned)))

    def test_final_holder_prunes_packed_out_paths(self):
        original = self.c.enforce_governed_capsule_contract
        def drop(value):
            result = original(value)
            result['semantic'] = [row for row in result['semantic'] if row['path'] != P['target']]
            self.c.synchronize_capsule_views(result)
            return result
        with mock.patch.object(self.c, 'enforce_governed_capsule_contract', side_effect=drop):
            capsule = self.assemble()
        self.assertEqual({P['safe']}, set(self.r._capsule_index_images(capsule)))
        self.assertEqual({P['safe']}, {row['path'] for row in capsule['selected']})
        self.older_refresh()
        self.assertEqual({P['safe']: P['safe_excerpt']},
                         {name: str(text) for name, text in self.c.capsule_excerpts(self.db, capsule).items()})


if __name__ == '__main__':
    unittest.main()
