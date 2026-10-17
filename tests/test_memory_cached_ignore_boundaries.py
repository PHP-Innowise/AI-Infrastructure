"""Fresh Skylark boundary oracles, prospectively sealed before correction."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import unittest
from unittest import mock
import tests.test_memory_cached_ignore as controls

SKYLARK = dict(controls.PUBLIC, query='SkylarkSignal842 indigo gate', marker='SkylarkSignal842',
    source='docs/skylark-ledger.md', source_text='# Skylark ledger\n\nSkylarkSignal842 uses the indigo gate.\n',
    progress='SkylarkProgress924 checked the indigo gate.', task='skylark/local-842',
    ignore='docs/skylark-ledger.md\n', chunk='memory-bank/chunks/MEM-0731-skylark-gate.md')


class SkylarkTests(controls.CachedIgnoreTests):
    def setUp(self):
        old = controls.PUBLIC
        controls.PUBLIC = SKYLARK
        self.addCleanup(setattr, controls, 'PUBLIC', old)
        super().setUp()

    def row(self, name=None, sources=None, kind='memory', body=None):
        return super().row(name or SKYLARK['chunk'], sources, kind, body)

    def test_cached_raw_search_withholds_only_the_ignored_document(self):
        self.native()
        self.assertEqual(0, self.call('index', '--json').returncode)
        self.write('.gitignore', SKYLARK['ignore'])
        result = self.call('search', SKYLARK['query'], '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        paths = [d['path'] for d in json.loads(result.stdout)['documents']]
        # The chunk citing it stays; the ignored file is not served from cache.
        self.assertIn(SKYLARK['chunk'], paths)
        self.assertNotIn(SKYLARK['source'], paths)

    def test_global_excludes_are_rechecked_on_cached_search(self):
        self.native()
        (self.root / SKYLARK['chunk']).unlink()
        self.write('docs/skylark-public.md', SKYLARK['source_text'])
        self.assertEqual(0, self.call('index', '--json').returncode)
        excludes = self.base / 'global-ignore'
        excludes.write_text(SKYLARK['ignore'], encoding='utf-8')
        config = Path(self.env['XDG_CONFIG_HOME']) / 'git/config'
        config.parent.mkdir(parents=True)
        # Forward slashes: a backslash starts an escape in a Git config value.
        config.write_text('[core]\n\texcludesFile = ' + excludes.as_posix() + '\n', encoding='utf-8')
        result = self.call('search', SKYLARK['query'], '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(['docs/skylark-public.md'], [d['path'] for d in json.loads(result.stdout)['documents']])

    def test_unadmitted_outside_link_hint_does_not_break_baseline(self):
        outside = self.base / 'outside.md'
        outside.write_text(SKYLARK['source_text'], encoding='utf-8')
        linked = self.root / 'docs/unindexed-skylark.md'
        self.symlink_or_skip(linked, outside)
        result = self.r._source_hint_candidates(self.db, self.root, self.config,
            [{'path': linked.relative_to(self.root).as_posix(), 'source_sha256': hashlib.sha256(outside.read_bytes()).hexdigest()}], set())
        self.assertEqual([], result)

    def test_admitted_eight_hints_use_one_own_path_batch(self):
        references = []
        for number in range(8):
            name = 'docs/skylark-hint-%s.md' % number
            self.row(name, sources=[], kind='spec')
            references.append({'path': name, 'source_sha256': hashlib.sha256((self.root/name).read_bytes()).hexdigest()})
        original = subprocess.run
        probes = []
        def count(command, **kwargs):
            if 'check-ignore' in command:
                probes.append(kwargs.get('input'))
            return original(command, **kwargs)
        with mock.patch('subprocess.run', side_effect=count):
            kept = self.r._source_hint_candidates(self.db, self.root, self.config, references, set())
        self.observe('hint-batches', {'kept': [i['path'] for i in kept], 'payloads': [p.decode('utf-8') for p in probes]})
        self.assertEqual([v['path'] for v in references], [i['path'] for i in kept])
        self.assertEqual([b''.join((v['path']+'\0').encode('utf-8') for v in references)], probes)

    def test_packing_policy_change_keeps_working_projection(self):
        self.task_native()
        connection = self.c.connect(self.root / 'memory-bank/local/context.db')
        self.addCleanup(connection.close)
        def pack(result):
            self.assertIsNotNone(result['working'])
            self.write('.gitignore', SKYLARK['ignore'])
            return result
        result = self.r.retrieve(connection, self.root, SKYLARK['query'], SKYLARK['task'],
            limit=5, gate_mode='off', manifest_scope='local', pack=pack)
        self.observe('working-post-pack', result)
        self.assertEqual(SKYLARK['progress'], result['working']['progress'])
        self.assertNotIn(SKYLARK['source'], [d['path'] for d in result['selected']])

    def test_cached_cli_unknown_git_is_fixed_failure(self):
        self.native()
        self.assertEqual(0, self.call('index', '--json').returncode)
        empty = self.base / 'empty-bin'
        empty.mkdir()
        self.env['PATH'] = str(empty)
        result = self.call('search', SKYLARK['query'], '--json')
        self.assertEqual(SKYLARK['unknown_cli'], {'returncode': result.returncode,
            'stdout': result.stdout.decode('utf-8'), 'stderr': result.stderr.decode('utf-8')})

    def test_shared_error_identity_preserves_context_hierarchy(self):
        self.assertIs(self.c.ContextError, self.w.ContextError)
        self.assertIs(self.c.SourceGitError, self.w.SourceGitError)
        self.assertTrue(issubclass(self.w.SourceGitError, self.c.ContextError))

    def test_attached_external_state_and_tooling_are_not_project_paths(self):
        state = self.base / 'attached-state'
        state.mkdir()
        tooling = self.base / 'tooling'
        tooling.mkdir()
        self.write('.gitignore', SKYLARK['ignore'])
        layout = self.w.Roots(tooling, self.root, state)
        with mock.patch.object(self.w, 'roots', return_value=layout):
            self.assertIsNone(self.w.project_source_key(state, 'project-brain/dynamic/skylark.md'))
            self.assertIsNone(self.w.project_source_key(state, str(tooling/'AGENTS.md')))
            self.assertEqual({SKYLARK['source']}, self.w.ignored_source_keys(state,
                [SKYLARK['source'], 'project-brain/dynamic/skylark.md', str(tooling/'AGENTS.md')], strict=True))


    def test_ignored_alias_keeps_lexical_git_name(self):
        alias = 'docs/skylark-alias.md'
        self.write(alias, SKYLARK['source_text'])
        (self.root/alias).unlink()
        self.symlink_or_skip(self.root/alias, self.root/SKYLARK['source'])
        self.write('.gitignore', alias+'\n')
        self.assertEqual({alias}, self.w.ignored_source_keys(self.root, [alias]))

    def test_ignored_cited_alias_does_not_withhold_the_citing_chunk(self):
        alias = 'docs/skylark-alias.md'
        self.symlink_or_skip(self.root/alias, self.root/SKYLARK['source'])
        self.row(sources=[alias])
        self.write('.gitignore', alias+'\n')
        self.assertEqual([SKYLARK['chunk']], [d['path'] for d in self.linked(alias)[0]])

    def lightweight_sources(self, count=5):
        refs = []
        for number in range(count):
            name = 'docs/skylark-source-%s.md' % number
            self.write(name, SKYLARK['source_text'])
            refs.append(name)
        self.c.start_working_task(self.db, SKYLARK['task'], SKYLARK['query'], [], refs)
        self.c.update_working_task(self.db, SKYLARK['task'], SKYLARK['progress'], [], [], [])
        return refs

    def test_lightweight_ignored_source_keeps_working_state(self):
        refs = self.lightweight_sources()
        self.write('.gitignore', refs[-1]+'\n')
        value = self.c.build_context_packet(self.db, self.root, 'skylark', SKYLARK['task'], 3,
                                            include_retrieval=False)
        self.assertEqual(SKYLARK['progress'], value['working']['progress'])
        self.assertEqual(SKYLARK['progress'], self.c.find_working_task(self.db, SKYLARK['task'])['progress'])

    def test_read_only_lightweight_keeps_working_with_an_ignored_source(self):
        import contextlib,io,types
        refs = self.lightweight_sources(1)
        self.write('.gitignore', refs[0]+'\n')
        args = types.SimpleNamespace(query='skylark', task_id=SKYLARK['task'], json=True)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.c.degraded_refresh(self.db, self.root, args, 'lightweight', 'ordinary')
        value = json.loads(output.getvalue())
        self.observe('read-only', value)
        self.assertEqual(SKYLARK['progress'], value['capsule']['working']['progress'])

    def test_automatic_hook_keeps_working_with_an_ignored_source(self):
        self.task_native()
        self.write('.gitignore', SKYLARK['ignore'])
        result = self.call('hook-context', '--task-id', SKYLARK['task'], '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(SKYLARK['progress'], value['working']['progress'])

    def test_ignore_change_after_pack_clipping_keeps_working(self):
        self.task_native()
        extra = []
        for number in range(5):
            name = 'docs/skylark-extra-%s.md' % number
            self.write(name, SKYLARK['source_text'])
            extra += ['--source', name]
        result = self.call('update', '--task-id', SKYLARK['task'], '--revision', '2', *extra, '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        connection = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(connection.close)
        def pack(value):
            value['working']['sources'] = value['working']['sources'][:4]
            self.write('.gitignore', 'docs/skylark-extra-4.md\n')
            return value
        value = self.r.retrieve(connection, self.root, SKYLARK['query'], SKYLARK['task'],
            limit=5, gate_mode='off', manifest_scope='local', pack=pack)
        self.assertEqual(SKYLARK['progress'], value['working']['progress'])

    def test_task_derived_pack_query_survives_an_ignore_change(self):
        self.task_native()
        connection = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(connection.close)
        def pack(value):
            self.write('.gitignore', SKYLARK['ignore'])
            return value
        value = self.r.retrieve(connection, self.root, SKYLARK['query'], SKYLARK['task'],
            limit=5, gate_mode='off', query_source='task', manifest_scope='local', pack=pack)
        self.assertEqual(SKYLARK['query'], value['query'])
        self.assertEqual(SKYLARK['progress'], value['working']['progress'])
        self.assertNotIn(SKYLARK['source'], [d['path'] for d in value['selected']])


    def symlink_or_skip(self, link, target):
        try:
            link.symlink_to(target)
        except OSError as error:
            if os.name == 'nt' and getattr(error, 'winerror', None) == 1314:
                self.skipTest('Windows symlink privilege unavailable; only this case is unrun')
            raise

    def test_lightweight_hook_keeps_task_with_an_ignored_source(self):
        self.native()
        refs = self.lightweight_sources(1)
        self.write('.gitignore', refs[0]+'\n')
        result = self.call('--mode', 'lightweight', 'hook-context', '--task-id', SKYLARK['task'], '--json')
        self.assertEqual(0, result.returncode, result.stderr)
        value = json.loads(result.stdout)
        self.assertEqual(SKYLARK['progress'], value['working']['progress'])


    def test_ignored_source_keeps_the_task_history_query(self):
        self.task_native()
        self.write('.gitignore', SKYLARK['ignore'])
        connection = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(connection.close)
        observed = []
        original = self.r.history_query
        def history(conn, text):
            observed.append(text)
            return original(conn, text)
        with mock.patch.object(self.r, 'history_query', side_effect=history):
            result = self.r.retrieve(connection, self.root, SKYLARK['query'], SKYLARK['task'],
                limit=5, gate_mode='off', query_source='task', history_text=SKYLARK['query'], manifest_scope='local')
        self.assertEqual([SKYLARK['query']], observed)
        self.assertEqual(SKYLARK['query'], result['query'])

    def test_episode_only_pack_survives_an_ignore_change(self):
        self.task_native()
        connection = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(connection.close)
        episode_query = 'SkylarkEpisode695 ultraviolet orb'
        self.c.insert_episode(connection, episode_query, 'SkylarkEpisode695 confirmed ultraviolet orb', [], [], [])
        connection.commit()
        def pack(value):
            self.assertEqual([], value['selected'])
            self.assertEqual(1, len(value['episodic']))
            self.write('.gitignore', SKYLARK['ignore'])
            return value
        result = self.r.retrieve(connection, self.root, episode_query, SKYLARK['task'],
            limit=5, gate_mode='off', query_source='task', history_text=episode_query,
            manifest_scope='local', pack=pack)
        self.observe('episode-only-after-pack', result)
        self.assertEqual(episode_query, result['query'])
        self.assertEqual(1, len(result['episodic']))
        manifest = json.loads((self.root/result['manifest']).read_text(encoding='utf-8'))
        self.assertEqual(1, manifest['local_episode_count'])

    def test_warning_only_assembly_keeps_working_with_an_ignored_source(self):
        self.task_native()
        self.write('.gitignore', SKYLARK['ignore'])
        connection = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(connection.close)
        result = self.c.assemble_capsule(connection, self.root, mode='governed', query=SKYLARK['query'],
            task_id=SKYLARK['task'], limit=3, ephemeral=True, refresh_index=False, query_source='task',
            index_warnings=['Supplied ordinary refresh warning'])
        self.assertEqual(SKYLARK['progress'], result['working']['progress'])

    def test_lightweight_assembly_keeps_working_with_an_ignored_source(self):
        self.native()
        refs = self.lightweight_sources(1)
        self.write('.gitignore', refs[0]+'\n')
        result = self.c.assemble_capsule(self.db, self.root, mode='lightweight', query=SKYLARK['query'],
            task_id=SKYLARK['task'], limit=3, ephemeral=True, refresh_index=False, query_source='task')
        self.assertEqual(SKYLARK['progress'], result['working']['progress'])


# Inherit setup/helpers only; the independent Kestrel controls are never replayed.
for _name in dir(controls.CachedIgnoreTests):
    if _name.startswith('test_'):
        setattr(SkylarkTests, _name, None)
if __name__ == '__main__':
    unittest.main()
