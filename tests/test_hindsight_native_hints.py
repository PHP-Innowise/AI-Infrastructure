"""Fresh public source hints through copied native FTS5 and Harness callers.

No provider CLI, live service, credentials, historical gold or closed fixture.
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
import context_retrieval as retrieval
import brain_runtime as brain
from harness import sessions, cost_observation
from harness.knowledge import KnowledgeManager, KnowledgeBusy
from harness.task_context import TaskContext
from harness import hindsight_recall as remote

PUBLIC = json.loads((ROOT / 'tests/fixtures/hindsight_native_public.json').read_text())


def install(project):
    for relative in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/templates',
                     'project-brain/config', 'project-brain/schemas', 'project-brain/indexes'):
        shutil.copytree(EDITION / relative, project / relative,
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    for relative in ('memory-bank/README.md', 'memory-bank/INDEX.md', 'project-brain/README.md', 'project-brain/PROTOCOL.md'):
        shutil.copyfile(EDITION / relative, project / relative)
    (project / 'memory-bank/chunks').mkdir()
    (project / 'specs').mkdir()
    for source in PUBLIC['sources']:
        (project / source['path']).write_bytes(source['text'].encode())


def hints(project, *names):
    return {'schema_version': 1, 'references': [
        {'path': name, 'source_sha256': hashlib.sha256((project / name).read_bytes()).hexdigest()} for name in names]}


def manifest(project, result):
    return json.loads((project / result['manifest']).read_text())


class NativeHints(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.project = Path(temporary.name) / 'project'
        self.project.mkdir()
        install(self.project)
        self.db = native.connect(self.project / 'memory-bank/local/context.db')
        self.addCleanup(self.db.close)
        self.warm()
        self.route, self.raven, self.crlf = [source['path'] for source in PUBLIC['sources']]

    def warm(self):
        _, _, warnings = native.refresh_layers(self.db, self.project)
        self.assertEqual([], warnings)

    def retrieve(self, payload=None, query=None, **options):
        return retrieval.retrieve(self.db, self.project, query or PUBLIC['unmatched_query'], None,
                                  limit=3, gate_mode=options.pop('gate_mode', 'off'), source_hints=payload, **options)

    def test_exact_source_rescue_keeps_lexical_no_match_and_manifest(self):
        baseline = self.retrieve()
        self.assertNotIn(self.route, {item['path'] for item in baseline['selected']})
        result = self.retrieve(hints(self.project, self.route))
        item = next(item for item in result['semantic'] if item['path'] == self.route)
        self.assertEqual('source-hint', item['selection'])
        self.assertNotIn('match', item)
        self.assertIn(PUBLIC['sources'][0]['expected'], item['snippet'])
        saved = manifest(self.project, result)
        self.assertEqual(baseline['no_match'], result['no_match'])
        self.assertEqual((0, None), (saved['selected'][0]['score'], saved['selected'][0]['rank']))
        self.assertEqual({'profile': 'source-hints-v1', 'state': 'ready', 'supplied': 1,
                          'admitted': 1, 'delivered': 1}, saved['source_hints'])

    def test_existing_lexical_path_and_order_are_not_relabelled(self):
        baseline = self.retrieve(query=PUBLIC['query'])
        result = self.retrieve(hints(self.project, self.route), query=PUBLIC['query'])
        self.assertEqual(baseline['selected'], result['selected'])
        self.assertTrue(any(item['path'] == self.route for item in result['selected']))
        self.assertFalse(any(item.get('selection') == 'source-hint' for item in result['selected']))

    def test_crlf_uses_raw_and_normalized_hashes_separately(self):
        payload = hints(self.project, self.crlf)
        result = self.retrieve(payload)
        item = manifest(self.project, result)['selected'][0]
        self.assertEqual(self.crlf, item['path'])
        normalized = PUBLIC['sources'][2]['text'].replace('\r\n', '\n')
        self.assertEqual(hashlib.sha256(normalized.encode()).hexdigest(), item['source_hash'])
        self.assertNotEqual(payload['references'][0]['source_sha256'], item['source_hash'])

    def test_changed_deleted_unknown_and_unindexed_sources_abstain(self):
        old = hints(self.project, self.raven)
        (self.project / self.raven).write_bytes(PUBLIC['sources'][1]['corrected'].encode())
        self.assertEqual([], self.retrieve(old)['selected'])
        (self.project / self.raven).unlink()
        self.assertEqual([], self.retrieve(old)['selected'])
        (self.project / 'specs/not-indexed.md').write_text('Fresh independent source.')
        self.assertEqual([], self.retrieve(hints(self.project, 'specs/not-indexed.md'))['selected'])
        old['references'][0]['path'] = 'specs/unknown.md'
        self.assertEqual([], self.retrieve(old)['selected'])

    def test_bounded_shape_and_unsafe_names_do_not_read_hints(self):
        good = hints(self.project, self.route)
        malformed = [True, {'schema_version': True, 'references': []},
                     {'schema_version': 1, 'references': good['references'] * 9},
                     json.dumps(good)[:-1] + ',"schema_version":1}', 'x' * 8193]
        for name in ('../outside.md', '/outside.md', 'a//b.md', 'a/./b.md', 'C:/file.md',
                     'a\\b.md', 'specs/a.md#x', '.env', '.git/config', 'specs/\u0085bad.md'):
            malformed.append({'schema_version': 1, 'references': [{'path': name, 'source_sha256': 'a' * 64}]})
        for payload in malformed:
            with self.subTest(payload=str(payload)[:40]), patch.object(retrieval, '_source_hint_read', side_effect=AssertionError('unsafe hint read')):
                self.assertEqual([], self.retrieve(payload)['selected'])

    def test_no_follow_single_link_and_file_bounds(self):
        payload = hints(self.project, self.route)
        node = self.project / self.route
        saved = node.read_bytes()
        outside = self.project.parent / 'foreign.md'
        outside.write_bytes(saved)
        node.unlink(); node.symlink_to(outside)
        self.assertEqual([], self.retrieve(payload)['selected'])
        node.unlink(); os.link(outside, node)
        self.assertEqual([], self.retrieve(payload)['selected'])
        node.unlink(); node.write_bytes(saved + b'x' * 131072)
        self.assertEqual([], self.retrieve(payload)['selected'])

    def test_parent_directory_replacement_rejects_detached_image(self):
        payload = hints(self.project, self.route)
        read = os.read
        replaced = []
        def swap(fd, size):
            chunk = read(fd, size)
            if not replaced:
                replaced.append(True)
                (self.project / 'specs').rename(self.project / 'detached')
                (self.project / 'specs').mkdir()
                (self.project / self.route).write_text('Changed current parent/source.')
            return chunk
        with patch.object(retrieval.os, 'read', side_effect=swap):
            self.assertIsNone(retrieval._source_hint_read(self.project, self.route, payload['references'][0]['source_sha256']))

    def test_privacy_owner_authority_terminal_and_conflicts(self):
        payload = hints(self.project, self.route)
        original = dict(self.db.execute('SELECT * FROM document_metadata WHERE path=?', (self.route,)).fetchone())
        cells = [dict(privacy='private'), dict(privacy='restricted', owner='foreign'), dict(authority='invented'),
                 dict(kind='brain-task', lifecycle='completed'), dict(conflicts=json.dumps(['foreign-record'])),
                 dict(record_id='hint-record', incoming=True)]
        config = brain.load_config(self.project)
        config['owners'] = ['fixture-owner']
        for changes in cells:
            with self.subTest(changes=changes):
                for key, value in original.items():
                    if key != 'path':
                        self.db.execute(f'UPDATE document_metadata SET {key}=? WHERE path=?', (value, self.route))
                self.db.execute('UPDATE documents SET kind=? WHERE path=?', ('spec', self.route))
                for key, value in changes.items():
                    if key == 'kind':
                        self.db.execute('UPDATE documents SET kind=? WHERE path=?', (value, self.route))
                    elif key != 'incoming':
                        self.db.execute(f'UPDATE document_metadata SET {key}=? WHERE path=?', (value, self.route))
                if changes.get('incoming'):
                    self.db.execute('UPDATE document_metadata SET conflicts=? WHERE path=?', (json.dumps(['hint-record']), self.raven))
                self.db.commit()
                with patch.object(retrieval, 'load_config', return_value=config):
                    self.assertEqual([], self.retrieve(payload)['selected'])

    def test_own_task_and_host_loaded_source_excluded(self):
        task = brain.create_record(self.project, 'task', 'fixture/kestrel-task', 'Public dispatch work', [],
                                   [self.route], owner='agent', goal='Dispatch owner check')
        self.warm()
        task_file, _, _ = brain.find_task(self.project, task['id'])
        task_name = task_file.relative_to(self.project).as_posix()
        result = retrieval.retrieve(self.db, self.project, PUBLIC['unmatched_query'], task['id'], limit=3,
                                    gate_mode='off', source_hints=hints(self.project, task_name))
        self.assertEqual([], result['selected'])
        self.assertTrue(any(item['reason'] == 'working-task' for item in manifest(self.project, result)['excluded']))
        with patch.object(retrieval, 'host_loaded_paths', return_value={self.route}):
            result = self.retrieve(hints(self.project, self.route))
        self.assertEqual([], result['selected'])
        self.assertTrue(any(item['reason'] == 'host-loaded' for item in manifest(self.project, result)['excluded']))

    def test_final_custody_prunes_once_and_does_not_record_raced_hint(self):
        payload = hints(self.project, self.route, self.raven)
        original = (self.project / self.route).read_bytes()
        calls = []
        def pack(result):
            calls.append(1)
            (self.project / self.route).write_text('Changed during final pack.')
            return native.enforce_governed_capsule_contract(result)
        result = self.retrieve(payload, pack=pack, session_id='new-hint-race')
        self.assertEqual(1, len(calls))
        for key in ('semantic', 'selected'):
            self.assertEqual([self.raven], [item['path'] for item in result[key]])
        saved = manifest(self.project, result)
        self.assertEqual([self.raven], [item['path'] for item in saved['selected']])
        self.assertEqual(1, saved['source_hints']['delivered'])
        deliveries = retrieval._load_session_deliveries(self.db)['new-hint-race']
        self.assertNotIn(self.route, json.dumps(deliveries))
        (self.project / self.route).write_bytes(original)
        restored = self.retrieve(hints(self.project, self.route), session_id='new-hint-race')
        self.assertEqual([self.route], [item['path'] for item in restored['selected']])

    def test_pack_omission_and_novelty_only_record_delivered_sources(self):
        payload = hints(self.project, self.route)
        def omit(result):
            result.update(semantic=[], selected=[], categories={})
            return result
        result = self.retrieve(payload, pack=omit, session_id='new-hint-pack')
        self.assertEqual([], manifest(self.project, result)['selected'])
        self.assertEqual(0, manifest(self.project, result)['source_hints']['delivered'])
        first = self.retrieve(payload, session_id='new-hint-pack')
        second = self.retrieve(payload, session_id='new-hint-pack')
        self.assertEqual([self.route], [item['path'] for item in first['selected']])
        self.assertEqual([], second['selected'])
        self.assertEqual(1, second['repeated'])

    def test_old_schema_disables_hints_and_invalid_schema_precedes_receipts(self):
        node = self.project / 'project-brain/schemas/retrieval-manifest.schema.json'
        schema = json.loads(node.read_text())
        schema['properties']['selected']['items']['properties']['selection']['enum'].remove('source-hint')
        node.write_text(json.dumps(schema))
        self.assertEqual([], self.retrieve(hints(self.project, self.route))['selected'])
        schema['properties']['selected']['items']['properties']['selection']['enum'].append('source-hint')
        schema['properties']['source_hints']['properties']['admitted']['maximum'] = 0
        node.write_text(json.dumps(schema))
        self.assertEqual([], self.retrieve(hints(self.project, self.route))['selected'])
        schema['properties']['source_hints']['properties']['admitted']['maximum'] = 8
        schema['properties']['query']['const'] = 'different-fixed-public-query'
        node.write_text(json.dumps(schema))
        before = retrieval._load_session_deliveries(self.db)
        with self.assertRaises(brain.BrainError):
            self.retrieve(hints(self.project, self.route), session_id='schema-reject')
        self.assertEqual(before, retrieval._load_session_deliveries(self.db))

    def test_native_compaction_treats_hint_as_first_related_removal(self):
        result = self.retrieve(hints(self.project, self.raven), query=PUBLIC['query'])
        self.assertEqual([self.route, self.raven], [item['path'] for item in result['semantic']])
        result['last_turn'] = 'Previous public dispatch check: ownership unchanged.'
        # A fixed synthetic cap between the complete result and its local-only view.
        without = copy.deepcopy(result)
        without['semantic'] = without['semantic'][:1]
        native.synchronize_capsule_views(without)
        without = native.enforce_governed_capsule_contract(without)
        ceiling = native.capsule_character_count(without) + 1
        with patch.object(native, 'CAPSULE_CHARACTER_LIMIT', ceiling):
            packed = native.enforce_governed_capsule_contract(result)
        self.assertEqual([self.route], [item['path'] for item in packed['semantic']])
        self.assertEqual(result['last_turn'], packed['last_turn'])

    def test_render_omitted_hint_does_not_poison_next_gate(self):
        payload = hints(self.project, self.route)
        def omit(result):
            result.update(semantic=[], selected=[], categories={})
            return result
        first = self.retrieve(payload, pack=omit, gate_mode='enforce')
        self.assertEqual([], first['selected'])
        later = self.retrieve(payload, gate_mode='enforce')
        self.assertEqual([self.route], [item['path'] for item in later['selected']])
        signature = retrieval._load_last_retrievals(self.db)
        self.assertTrue(signature)

    def test_local_episode_budget_and_slot_survive_optional_hints(self):
        native.insert_episode(self.db, 'Obsidian semaphore leasing', 'Lease acquired.', [], [], [])
        self.db.commit()
        baseline = self.retrieve()
        self.assertTrue(baseline['episodic'])
        self.db.execute('UPDATE documents SET layer=?, kind=? WHERE path=?', ('episodic', retrieval.HISTORY_KIND, self.raven))
        self.db.commit()
        event_attempt = self.retrieve(hints(self.project, self.raven))
        self.assertEqual(baseline['episodic'], event_attempt['episodic'])
        # Synthetic episode estimate saturates the unchanged 8000-token budget;
        # the production guard and its manifest schema remain fully enforced.
        episode_image = retrieval._serialized_episode(baseline['episodic'][0])
        estimate = retrieval._estimate_tokens
        def saturated(text):
            return 7999 if text == episode_image else estimate(text)
        with patch.object(retrieval, '_estimate_tokens', side_effect=saturated):
            result = self.retrieve(hints(self.project, self.route))
        self.assertEqual(baseline['episodic'], result['episodic'])
        self.assertEqual([], result['semantic'])

    def test_hint_chunk_does_not_evict_lexical_brain_source(self):
        record = brain.create_record(self.project, 'finding', 'public/kestrel-finding', PUBLIC['query'], [],
                                     [self.route], owner='agent')
        self.warm()
        record_file, _, _ = brain.find_record(self.project, record['id'])
        name = record_file.relative_to(self.project).as_posix()
        baseline = self.retrieve(query=PUBLIC['query'])
        self.assertIn(name, {item['path'] for item in baseline['selected']})
        self.db.execute('UPDATE documents SET kind=? WHERE path=?', ('memory', self.raven))
        self.db.execute('UPDATE document_metadata SET category=?, source_fingerprints=? WHERE path=?',
                        ('durable', json.dumps([brain.fingerprint(self.project, name)]), self.raven))
        self.db.commit()
        result = self.retrieve(hints(self.project, self.raven), query=PUBLIC['query'])
        self.assertIn(name, {item['path'] for item in result['selected']})
        self.assertFalse(any(item['path'] == name and item['reason'] == 'promoted-to-chunk'
                             for item in manifest(self.project, result)['excluded']))

    def test_actual_render_ceiling_drops_hint_before_local_excerpt(self):
        baseline = self.retrieve(query=PUBLIC['query'])
        candidate = self.retrieve(hints(self.project, self.raven), query=PUBLIC['query'])
        baseline.update(omitted={}, warnings=[], last_turn='Previous dispatch ownership check.')
        candidate.update(omitted={}, warnings=[], last_turn=baseline['last_turn'])
        expected = native.render_capsule_lines(baseline, native.capsule_excerpts(self.db, baseline))
        # Fixed room for the compaction marker, never a changed answer/source.
        ceiling = sum(len(line) + 1 for line in expected) + len(native.COMPACTION_MARKER) + 1
        with patch.object(native, 'RENDERED_CAPSULE_LIMIT', ceiling):
            native.shown_in_render(self.db, candidate)
            lines = native.render_capsule_lines(candidate, native.capsule_excerpts(self.db, candidate))
        self.assertEqual([self.route], [item['path'] for item in candidate['semantic']])
        self.assertIn(PUBLIC['sources'][0]['expected'], '\n'.join(lines))
        self.assertIn(baseline['last_turn'], '\n'.join(lines))


class ReferenceBoundary(unittest.TestCase):
    def setUp(self):
        self.scope = remote.Scope('11111111-1111-4111-8111-111111111111', '22222222-2222-4222-8222-222222222222')
        self.binding = remote.SourceBinding.from_bytes(self.scope, PUBLIC['sources'][0]['path'], PUBLIC['sources'][0]['text'].encode())
        self.catalogue = remote.Catalogue(self.scope, (self.binding,))
        self.calls = []
        outer = self
        class Client:
            def post(self, scope, request, metrics):
                outer.calls.append((scope, request))
                metrics.update(request_body_bytes=123, response_body_bytes=234, response_eof=True)
                return json.dumps({'results': [{'document_id': outer.binding.document_id, 'type': 'world',
                    'text': PUBLIC['raw_remote_prose'], 'metadata': {'harness_scope': scope.key,
                    'source_sha256': outer.binding.source_sha256}}]}).encode()
        self.client = Client()

    def test_reference_only_result_and_operator_scope_route(self):
        provider = remote.source_hint_provider(self.client, self.catalogue, project_id='project-a',
                                               bank='memory-bank', workspace='/fixture/project-a')
        common = dict(active=lambda: True, project_id='project-a', bank='memory-bank', workspace='/fixture/project-a')
        outcome = provider(PUBLIC['query'], **common)
        self.assertEqual((self.binding,), outcome.references)
        self.assertEqual({'schema_version': 1, 'references': [{'path': self.binding.path,
                          'source_sha256': self.binding.source_sha256}]}, outcome.native_payload())
        self.assertNotIn(PUBLIC['raw_remote_prose'], json.dumps(outcome.diagnostics))
        for changes in ({'project_id': 'project-b'}, {'bank': 'other-bank'}, {'workspace': '/fixture/project-b'}):
            self.assertEqual((), provider('foreign secret-free request', **{**common, **changes}).references)
        self.assertEqual(1, len(self.calls))

    def test_privacy_and_cancel_are_checked_before_transport(self):
        for query, active in [('API_KEY=public-fixture-secret-value', lambda: True), (PUBLIC['query'], lambda: False)]:
            outcome = remote.recall_references(query, self.client, self.catalogue, active=active)
            self.assertEqual((), outcome.references)
        self.assertEqual([], self.calls)

    def test_hostile_reason_and_diagnostics_do_not_claim_tokens(self):
        error = remote.RemoteFailure('timeout'); error.reason = 'credential/private/secret-prose'
        with patch.object(self.client, 'post', side_effect=error):
            result = remote.recall_references(PUBLIC['query'], self.client, self.catalogue)
        self.assertEqual('unavailable', result.diagnostics['outcome'])
        observed = cost_observation.LaunchObservation('codex')
        observed.source_hints(provider_calls=1, outcome='resolved', seconds=.1, references=1,
            transport={'attempts': True, 'request_body_bytes': -1, 'response_body_bytes': 999999,
                       'response_eof': 'secret', 'remote_usd': 0, 'provider_tokens': 0, 'body': 'secret'})
        row = observed.snapshot()['remote_operations'][0]
        self.assertEqual('incomplete', row['metadata_state'])
        self.assertIsNone(row['transport_method_calls']); self.assertIsNone(row['remote_usd'])
        self.assertIsNone(row['provider_tokens']); self.assertFalse(row['total_cost_complete'])
        self.assertNotIn('secret', json.dumps(row))
        self.assertFalse(observed.incomplete)
        final = observed.finish('failed')
        observed.source_hints(provider_calls=1, outcome='resolved', seconds=0)
        self.assertEqual(final, observed.snapshot())

    def test_mixed_results_scope_hash_type_duplicate_and_eight_cap(self):
        bindings = tuple(remote.SourceBinding.from_bytes(self.scope, f'specs/public-ref-{number}.md',
                         f'Public dispatch reference {number}.\n'.encode()) for number in range(9))
        catalogue = remote.Catalogue(self.scope, bindings)
        def row(binding):
            return {'document_id': binding.document_id, 'type': 'world', 'text': PUBLIC['raw_remote_prose'],
                    'metadata': {'harness_scope': self.scope.key, 'source_sha256': binding.source_sha256}}
        unknown = row(bindings[0]); unknown['document_id'] = 'unknown-public-id'
        foreign = row(bindings[0]); foreign['metadata']['harness_scope'] = 'foreign-scope'
        changed = row(bindings[1]); changed['metadata']['source_sha256'] = '0' * 64
        wrong_type = row(bindings[2]); wrong_type['type'] = 'opinion'
        raw = json.dumps({'results': [unknown, foreign, changed, wrong_type, *[row(item) for item in bindings], row(bindings[0])]}).encode()
        with patch.object(self.client, 'post', return_value=raw):
            result = remote.recall_references(PUBLIC['query'], self.client, catalogue)
        self.assertEqual(bindings[:8], result.references)
        self.assertEqual(5, result.diagnostics['rejected'])
        self.assertEqual(8, len(result.native_payload()['references']))

    def test_unknown_remote_cost_does_not_null_known_native_tokens(self):
        observed = cost_observation.LaunchObservation('codex')
        observed.new_phase('main')
        observed.process('main', 'started')
        observed.raw('main', {'type': 'thread.started', 'thread_id': 'new-public-token-fixture'})
        observed.raw('main', {'type': 'turn.completed', 'usage': {'input_tokens': 111, 'output_tokens': 7, 'cached_input_tokens': 11}})
        before = observed.snapshot()['provider']
        self.assertEqual(118, before['provider_tokens'])
        observed.source_hints(provider_calls=1, outcome='unavailable', seconds=.1, transport={})
        after = observed.snapshot()['provider']
        self.assertEqual(before['provider_tokens'], after['provider_tokens'])
        self.assertFalse(observed.snapshot()['remote_operations'][0]['total_cost_complete'])


class HarnessHints(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / 'project'; self.project.mkdir(); install(self.project)
        for target, name, value in ((sessions.providers, 'discover_providers', [{'id': 'codex', 'name': 'No model fixture', 'available': True, 'executable': 'fixture'}]),
                                    (sessions.providers, 'model_options', {'models': [], 'efforts': []}),
                                    (sessions.Sessions, '_worker', None)):
            patched = patch.object(target, name, return_value=value); patched.start(); self.addCleanup(patched.stop)
        denial = patch.object(sessions.providers, 'build_command', side_effect=AssertionError('No native model'))
        denial.start(); self.addCleanup(denial.stop)
        self.store = sessions.Sessions(self.root / 'state', [self.project]); self.addCleanup(self.store.close)
        self.manager = KnowledgeManager(self.store); self.addCleanup(self.manager.close)
        self.project_id = next(iter(self.store.projects))
        self.sid = self.store.create({'project_id': self.project_id, 'provider': 'codex', 'mode': 'plan',
            'prompt': PUBLIC['unmatched_query'], 'brain': {'bank': 'memory-bank', 'task_id': PUBLIC['task_alias'],
            'query': PUBLIC['unmatched_query'], 'create': True, 'goal': PUBLIC['goal'], 'review': False}})['id']
        self.session = self.store.get(self.sid)
        self.observed = cost_observation.LaunchObservation('codex')
        scope = remote.Scope('33333333-3333-4333-8333-333333333333', '44444444-4444-4444-8444-444444444444')
        self.binding = remote.SourceBinding.from_bytes(scope, PUBLIC['sources'][0]['path'], PUBLIC['sources'][0]['text'].encode())
        self.calls = []
        def provider(query, **options):
            self.calls.append((query, options['project_id'], options['bank'], options['workspace']))
            self.assertTrue(options['active']())
            return remote.ReferenceOutcome((self.binding,), {'attempts': 1, 'request_body_bytes': 123,
                        'response_body_bytes': 234, 'response_eof': True})
        self.manager.source_hint_provider = provider
        self.context = TaskContext(self.manager, observation=self.observed)

    def test_real_prepare_refresh_render_manifest_and_prompt(self):
        commands = []
        execute = self.manager._execute
        def traced(command, *args, **kwargs):
            commands.append(command)
            return execute(command, *args, **kwargs)
        with patch.object(self.manager, '_execute', side_effect=traced):
            result = self.context.prepare(self.session)
        route = self.binding.path
        self.assertEqual([route], [item['path'] for item in result['capsule']['semantic']])
        self.assertIn(PUBLIC['sources'][0]['expected'], result['capsule_text'])
        self.assertEqual([route], list(result['source_hashes']))
        self.assertEqual(1, len(self.calls))
        self.assertEqual((PUBLIC['unmatched_query'], self.project_id, 'memory-bank', str(self.project)), self.calls[0])
        refresh = [command for command in commands if 'refresh' in command]
        self.assertEqual(1, len(refresh))
        self.assertEqual(1, sum(argument.startswith('--source-hints-json=') for argument in refresh[0]))
        envelope = sessions.capsule_envelope(result['capsule'], result['capsule_text'])
        self.assertEqual(sessions.BRAIN_CONTEXT_HEADER + result['capsule_text'] + '\n\n', envelope)
        prepared_session = {**self.session, 'brain': result}
        prompt = self.store._prompt(prepared_session, PUBLIC['unmatched_query'], delivery=True)
        self.assertIn(envelope, prompt)
        self.assertNotIn(PUBLIC['raw_remote_prose'], prompt)
        saved = manifest(self.project, result['capsule'])
        self.assertEqual([route], [item['path'] for item in saved['selected']])
        self.assertEqual(1, self.observed.snapshot()['remote_operations'][0]['provider_calls'])

    def test_callback_once_with_busy_retry_and_local_fallback_on_error(self):
        run = self.manager.run; busy = [True]
        def contended(project_id, data, **kwargs):
            if data['action'] == 'refresh' and busy:
                busy.pop(); raise KnowledgeBusy('fixture lock')
            return run(project_id, data, **kwargs)
        self.context.wait = 1
        with patch.object(self.manager, 'run', side_effect=contended), patch('harness.task_context.time.sleep'):
            self.context.prepare(self.session)
        self.assertEqual(1, len(self.calls))
        self.manager.source_hint_provider = lambda *args, **kwargs: (_ for _ in ()).throw(ValueError('private callback error'))
        result = self.context.prepare(self.store.get(self.sid))
        self.assertEqual([], result['capsule']['semantic'])
        self.assertNotIn('private callback error', json.dumps(self.observed.snapshot()))

    def test_remote_record_survives_cli_or_capsule_refusal(self):
        self.context.prepare(self.session)
        for response in ({'ok': False, 'error': 'fixed local failure'}, {'ok': True, 'result': {'capsule': {}}}):
            with patch.object(self.manager, 'run', return_value=response):
                with self.assertRaises(sessions.SessionError):
                    self.context._retrieve(self.session, {'query': PUBLIC['unmatched_query'], 'review': False,
                        'bank': 'memory-bank', 'task_id': PUBLIC['task_alias']}, self.project, {},
                        self.manager.info(self.project_id, 'memory-bank'), True)
        self.assertEqual(3, len(self.observed.snapshot()['remote_operations']))

    def test_cancellation_after_capability_and_after_callback_prevents_launch(self):
        cancelled = []
        def guard():
            if cancelled:
                raise sessions.SessionError('fixture stale')
        self.context.guard = guard
        info = self.manager.info(self.project_id, 'memory-bank')
        options = {'query': PUBLIC['unmatched_query'], 'review': False, 'bank': 'memory-bank', 'task_id': PUBLIC['task_alias']}
        def capability(*args):
            cancelled.append(True); return True
        with patch.object(self.manager, '_supports_source_hints', side_effect=capability), patch.object(self.manager, 'run') as run:
            with self.assertRaises(sessions.SessionError):
                self.context._retrieve(self.session, options, self.project, {}, info, True)
            run.assert_not_called()
        self.assertEqual([], self.calls)
        cancelled.clear()
        def provider(*args, **kwargs):
            cancelled.append(True)
            return remote.ReferenceOutcome((self.binding,), {'attempts': 1, 'request_body_bytes': 5,
                                            'response_body_bytes': 7, 'response_eof': True})
        self.manager.source_hint_provider = provider
        with patch.object(self.manager, 'run') as run:
            with self.assertRaises(sessions.SessionError):
                self.context._retrieve(self.session, options, self.project, {}, info, True)
            run.assert_not_called()
        row = self.observed.snapshot()['remote_operations'][-1]
        self.assertEqual('cancelled', row['outcome']); self.assertEqual(7, row['response_body_bytes'])

    def test_old_runtime_and_public_fields_cannot_trigger_hints(self):
        node = self.project / 'memory-bank/scripts/context.py'
        node.write_text(node.read_text().replace('SOURCE_HINT_PROFILE = "source-hints-v1"', 'OLD_RUNTIME = True'))
        result = self.context.prepare(self.session)
        self.assertEqual([], result['capsule']['semantic']); self.assertEqual([], self.calls)
        with self.assertRaises(sessions.SessionError):
            self.manager.run(self.project_id, {'action': 'refresh', 'query': PUBLIC['query'], 'source_hints': hints(self.project, self.binding.path)})

    def test_malformed_private_hint_cannot_break_single_local_refresh(self):
        for name in ('specs/bad\x00.md', '../foreign.md', '.env', 'C:/file.md', 'specs/a.md#section'):
            payload = {'schema_version': 1, 'references': [{'path': name, 'source_sha256': 'a' * 64}]}
            self.assertIsNone(self.manager._source_hints_json(payload))
        payload['references'][0]['path'] = 'specs/bad\x00.md'
        response = self.manager.run(self.project_id, {'action': 'refresh', 'query': PUBLIC['unmatched_query'],
                                    'task_id': PUBLIC['task_alias']}, _ephemeral=True, _host='codex', _source_hints=payload)
        self.assertTrue(response['ok'], response)


if __name__ == '__main__':
    unittest.main()
