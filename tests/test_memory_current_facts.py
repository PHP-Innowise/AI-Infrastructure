"""Known-answer current-fact projections, without models or a second authority."""
import hashlib
import json
import subprocess
from datetime import datetime, timedelta, timezone
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import memory_current_facts as facts
from memory_pilot_support import canonical_module


class CurrentFactsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'facts.json'
        self.rows = [
            {'value': 'MySQL', 'recorded': '2026-01-01T00:00:00Z', 'lineage': 'database', 'revision': 1},
            {'value': 'PostgreSQL', 'recorded': '2026-02-01T00:00:00Z', 'lineage': 'database', 'revision': 2},
        ]
        self.save()

    def save(self):
        self.source.write_text(json.dumps(self.rows), encoding='utf-8')

    def selectors(self):
        return [{'id': f'v{i}', 'namespace': 'shop', 'subject': 'api', 'predicate': 'database',
                 'qualifiers': {'environment': 'production'}, 'source': 'facts.json',
                 'pointer': f'/{i}/value', 'recorded_at_pointer': f'/{i}/recorded',
                 'lineage_pointer': f'/{i}/lineage', 'revision_pointer': f'/{i}/revision'}
                for i in range(len(self.rows))]

    def project(self, selectors=None, moment='2026-10-31T00:00:00Z'):
        candidates = facts.collect_candidates(self.root, selectors or self.selectors(), as_of=moment)
        return facts.resolve_current(candidates, namespace='shop', as_of=moment)

    def test_latest_revision_only_inside_one_lineage_and_history_survives(self):
        group = self.project()['facts'][0]
        self.assertEqual('current', group['state'])
        self.assertEqual('PostgreSQL', group['claim'])
        self.assertEqual(['v0', 'v1'], [row['id'] for row in group['history']])
        self.rows[0]['lineage'] = 'different'; self.rows[0]['revision'] = 99; self.save()
        self.assertEqual('ambiguous', self.project()['facts'][0]['state'])

    def test_explicit_conflict_is_not_erased_by_newer_revision(self):
        self.rows[1]['conflicts'] = ['v0']; self.save()
        selectors = self.selectors(); selectors[1]['conflicts_pointer'] = '/1/conflicts'
        self.assertEqual('ambiguous', self.project(selectors)['facts'][0]['state'])

    def test_unselected_conflict_remains_visible(self):
        self.rows[0]['conflicts'] = ['unselected']; self.save()
        selectors = self.selectors()[:1]; selectors[0]['conflicts_pointer'] = '/0/conflicts'
        self.assertEqual('ambiguous', self.project(selectors)['facts'][0]['state'])

    def test_resolver_rechecks_time_and_null_end_is_open(self):
        candidates = facts.collect_candidates(self.root, self.selectors(), as_of='2027-01-01T00:00:00Z')
        result = facts.resolve_current(candidates, namespace='shop', as_of='2026-01-15T00:00:00Z')
        self.assertEqual('MySQL', result['facts'][0]['claim'])
        self.rows[1]['end'] = None; self.save()
        selectors = self.selectors(); selectors[1]['valid_to_pointer'] = '/1/end'
        self.assertEqual('PostgreSQL', self.project(selectors)['facts'][0]['claim'])

    def test_scopes_do_not_merge_and_order_does_not_choose_a_winner(self):
        selectors = self.selectors(); selectors[1]['qualifiers'] = {'environment': 'staging'}
        result = self.project(selectors)
        self.assertEqual(2, len(result['facts']))
        self.assertEqual(result, self.project(list(reversed(selectors))))

    def test_future_recorded_time_cannot_be_backdated_into_snapshot(self):
        self.rows[1]['recorded'] = '2027-01-01T00:00:00Z'; self.save()
        self.assertEqual('MySQL', self.project()['facts'][0]['claim'])
        self.assertEqual('future', self.project()['facts'][0]['history'][1]['reason'])

    def test_scalar_false_zero_null_empty_and_pointer_escapes(self):
        for value in (False, 0, None, ''):
            self.rows = [{'value': value, 'recorded': '2026-01-01T01:00:00+01:00',
                          'lineage': 'one', 'revision': 1}]; self.save()
            self.assertEqual(json.dumps(value), json.dumps(self.project()['facts'][0]['claim']))
        self.assertEqual('value', facts.json_pointer({'a/b': {'~key': 'value'}}, '/a~1b/~0key'))
        for pointer in ('/~2', '/01', '/-'):
            with self.assertRaises(ValueError): facts.json_pointer(['a'], pointer)

    def test_changed_missing_and_private_sources_never_become_current(self):
        selectors = self.selectors()[:1]
        selectors[0]['expected_sha256'] = '0' * 64
        self.assertEqual('stale', self.project(selectors)['facts'][0]['state'])
        for name in ('.env', '../outside.json', 'missing.json'):
            selectors[0]['source'] = name
            result = self.project(selectors)
            self.assertNotEqual('current', result['facts'][0]['state'])
            self.assertNotIn(name, json.dumps(result))

    def test_retiming_never_clears_a_known_source_digest_mismatch(self):
        for recorded,end,collection,resolution in [
            ('2027-01-01T00:00:00Z',None,'2026-10-31T00:00:00Z','2027-02-01T00:00:00Z'),
            ('2026-01-01T00:00:00Z','2026-06-01T00:00:00Z','2026-10-31T00:00:00Z','2026-02-01T00:00:00Z')]:
            self.rows=[{'value':'untrusted','recorded':recorded,'end':end}];self.save()
            selectors=self.selectors();selectors[0].update(expected_sha256='0'*64,valid_to_pointer='/0/end')
            selectors[0].pop('lineage_pointer');selectors[0].pop('revision_pointer')
            candidates=facts.collect_candidates(self.root,selectors,as_of=collection)
            result=facts.resolve_current(candidates,namespace='shop',as_of=resolution)
            with self.subTest(recorded=recorded):
                self.assertEqual('stale',candidates[0]['reason'])
                self.assertNotEqual('current',result['facts'][0]['state'])

    def test_symlink_and_secret_values_are_refused_without_echoing(self):
        secret = 'sk_live_' + 'A' * 24
        self.rows[1]['value'] = secret; self.save()
        result = self.project()
        self.assertNotIn(secret, json.dumps(result))
        self.assertNotEqual('current', result['facts'][0]['state'])
        self.source.unlink()
        outside = self.root.parent / (self.root.name + '-outside.json')
        outside.write_text('[]'); self.addCleanup(outside.unlink)
        self.source.symlink_to(outside)
        self.assertNotEqual('current', self.project()['facts'][0]['state'])

    def test_rebuild_does_not_change_source_and_naive_time_is_refused(self):
        before = hashlib.sha256(self.source.read_bytes()).hexdigest()
        self.assertEqual(self.project(), self.project())
        self.assertEqual(before, hashlib.sha256(self.source.read_bytes()).hexdigest())
        with self.assertRaises(ValueError): self.project(moment='2026-10-31')

    def test_unrepresentable_utc_query_is_a_controlled_cli_refusal(self):
        selectors = self.root / 'selectors.json'
        selectors.write_text('[]')
        for moment in ('0001-01-01T00:00:00+01:00', '9999-12-31T23:59:59-01:00'):
            with self.subTest(moment=moment):
                with self.assertRaisesRegex(ValueError, 'supported UTC range'):
                    facts.instant(moment)
                result = subprocess.run([sys.executable, '-B', str(Path(facts.__file__)),
                    '--root', str(self.root), '--selectors', str(selectors),
                    '--as-of', moment], capture_output=True, text=True)
                self.assertEqual(2, result.returncode, result.stderr)
                self.assertNotIn('Traceback', result.stderr)
                self.assertEqual('current facts: invalid or unsafe input\n', result.stderr)
                self.assertEqual('', result.stdout)


class FrontmatterFactsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'specs/database.md'
        self.source.parent.mkdir()
        self.source.write_text('Production uses PostgreSQL.\n')
        (self.root / 'memory-bank/chunks').mkdir(parents=True)
        self.brain = canonical_module('brain_runtime')
        self.moment = (datetime.now(timezone.utc) + timedelta(days=2)).isoformat()

    def record(self, *, alias='DATABASE-1', privacy='public', attestation='agent', kind='finding'):
        with mock.patch.object(self.brain, 'utc_now', return_value='2026-10-01T00:00:00Z'):
            record = self.brain.create_record(self.root, kind, alias, 'Database choice', [],
                ['specs/database.md'], owner='local', privacy=privacy,
                goal='Check production database choice.')
        with mock.patch.object(self.brain, 'utc_now', return_value='2026-10-02T00:00:00Z'):
            return self.brain.update_record(self.root, record['id'], expected_revision=record['revision'],
                progress='PostgreSQL', next_steps=[], files=[], sources=[], actor='local',
                transition_to='accepted' if kind == 'decision' else 'resolved',
                authority='verified', attestation=attestation)

    def selector(self, record, *, kind='brain-frontmatter', pointer='/progress'):
        path = self.brain.dynamic_path(self.root, record).relative_to(self.root).as_posix()
        return {'id': record['id'], 'namespace': 'shop', 'subject': 'api', 'predicate': 'database',
            'qualifiers': {'environment': 'production'}, 'source_kind': kind,
            'source': path, 'pointer': pointer}

    def projection(self, selectors, moment=None):
        moment = moment or self.moment
        rows = facts.collect_candidates(self.root, selectors, as_of=moment)
        return facts.resolve_current(rows, namespace='shop', as_of=moment)

    def canonical_selector(self, value='MariaDB', *, name='canonical'):
        (self.root/(name+'.json')).write_text(json.dumps({
            'value': value, 'recorded': '2026-10-03T00:00:00Z'}))
        return {'id': name, 'namespace': 'shop', 'subject': 'api', 'predicate': 'database',
                'qualifiers': {'environment': 'production'}, 'source': name+'.json',
                'pointer': '/value', 'recorded_at_pointer': '/recorded'}

    def test_canonical_json_outranks_eligible_brain_summary_and_preserves_history(self):
        record = self.record(); weaker = self.selector(record)
        for value in ('MariaDB', 'PostgreSQL'):
            canonical = self.canonical_selector(value)
            selectors = [weaker, canonical]
            result = self.projection(selectors)
            group = result['facts'][0]
            self.assertEqual(('current', value, ['canonical']),
                             (group['state'], group['claim'], group['current_ids']))
            self.assertEqual(2, len(group['history']))
            self.assertEqual(result, self.projection(list(reversed(selectors))))

    def test_canonical_precedence_preserves_conflicts_stale_and_unknown_blockers(self):
        record = self.record(); weaker = self.selector(record)
        canonical = self.canonical_selector()
        path = self.root/weaker['source']; original = path.read_bytes()
        self.rewrite(path, lambda md: md.update(conflicts=['48d5c1da-8f09-4ac1-bf6b-2e6850d34527']))
        self.assertEqual('ambiguous', self.projection([weaker, canonical])['facts'][0]['state'])
        path.write_bytes(original)
        self.source.write_text('Changed canonical evidence.\n')
        self.assertEqual('stale', self.projection([weaker, canonical])['facts'][0]['state'])
        self.source.write_text('Production uses PostgreSQL.\n')
        unsupported = dict(weaker, pointer='/missing')
        self.assertEqual('unclassified', self.projection([unsupported, canonical])['facts'][0]['state'])
        other = self.canonical_selector('SQLite', name='other')
        self.assertEqual('ambiguous', self.projection([weaker, canonical, other])['facts'][0]['state'])

    def test_canonical_precedence_does_not_merge_bank_scope_or_future_claims(self):
        record = self.record(); weaker = self.selector(record)
        _, bank = self.promoted(record)
        canonical = self.canonical_selector()
        groups = self.projection([weaker, bank, canonical])['facts']
        self.assertEqual(2, len(groups))
        direct = next(group for group in groups if not group.get('canonical_scope'))
        self.assertEqual(('current', 'MariaDB'), (direct['state'], direct['claim']))
        self.assertEqual(['application'], next(group['canonical_scope'] for group in groups
                                              if group.get('canonical_scope')))
        (self.root/'canonical.json').write_text(json.dumps({'value': 'MariaDB',
            'recorded': (datetime.fromisoformat(self.moment)+timedelta(days=1)).isoformat()}))
        group = self.projection([weaker, canonical])['facts'][0]
        self.assertEqual(('current', 'PostgreSQL', [record['id']]),
                         (group['state'], group['claim'], group['current_ids']))

    def rewrite(self, path, mutate):
        text = path.read_text()
        header, body = text[4:].split('\n---\n', 1)
        metadata = json.loads(header)
        mutate(metadata)
        path.write_text('---\n' + json.dumps(metadata) + '\n---\n' + body)

    def promoted(self, record):
        proposal = self.brain.create_promotion(self.root, [record['id']], 'Database choice',
            'Production uses PostgreSQL.', proposer='local')
        self.brain.review_promotion(self.root, proposal['id'], reviewer='reviewer', approve=True)
        self.brain.apply_promotion(self.root, proposal['id'])
        chunk = next((self.root / 'memory-bank/chunks').glob('*.md'))
        selector = self.selector(record, kind='bank-frontmatter', pointer='/body')
        selector.update(id='bank', source=chunk.relative_to(self.root).as_posix())
        return chunk, selector

    def test_native_verified_records_keep_exact_claim_body_and_attestation(self):
        for attestation, kind in [('agent', 'finding'), ('person', 'decision')]:
            record = self.record(alias=attestation, attestation=attestation, kind=kind)
            selector = self.selector(record)
            result = self.projection([selector])
            group = result['facts'][0]
            self.assertEqual(('current', 'PostgreSQL'), (group['state'], group['claim']))
            self.assertEqual(('verified', attestation),
                (group['history'][0]['authority'], group['history'][0]['attestation']))
            selector['pointer'] = '/body'
            text = (self.root / selector['source']).read_text()
            self.assertEqual(text[4:].split('\n---\n', 1)[1], self.projection([selector])['facts'][0]['claim'])
            self.assertTrue(result['derived']); self.assertFalse(result['runtime_integrated'])

    def test_provenance_privacy_lifecycle_and_unknown_attestation_fail_closed(self):
        for privacy in ('restricted', 'private'):
            record = self.record(alias=privacy, privacy=privacy)
            result = self.projection([self.selector(record)])
            self.assertNotEqual('current', result['facts'][0]['state'])
            self.assertNotIn('PostgreSQL', json.dumps(result))
        for key, value in [('authority', 'observed'), ('transitions', None), ('superseded_by', '48d5c1da-8f09-4ac1-bf6b-2e6850d34527')]:
            record = self.record(alias=key)
            path = self.brain.dynamic_path(self.root, record)
            if key == 'transitions':
                self.rewrite(path, lambda md: md['transitions'][-2].update(reason='Unknown verification'))
            else:
                self.rewrite(path, lambda md: md.update({key: value}))
            self.assertNotEqual('current', self.projection([self.selector(record)])['facts'][0]['state'])

    def test_project_admission_applies_to_brain_and_bank_origins(self):
        record = self.record(privacy='team')
        _, bank = self.promoted(record)
        selectors = [self.selector(record), bank]
        config = self.root / 'project-brain/config/runtime.json'
        config.parent.mkdir(parents=True, exist_ok=True)
        for value in ({'allowed_privacy': ['public']}, {'allowed_authority': ['observed']},
                      {'allowed_privacy': 'team'}, {'allowed_authority': None}):
            config.write_text(json.dumps(value))
            for selector in selectors:
                with self.subTest(config=value, kind=selector['source_kind']):
                    result = self.projection([selector])
                    self.assertEqual('unclassified', result['facts'][0]['state'])
                    self.assertNotIn('PostgreSQL', json.dumps(result))
        config.write_text(json.dumps({'allowed_privacy': ['team'], 'allowed_authority': ['verified']}))
        for selector in selectors:
            self.assertEqual('current', self.projection([selector])['facts'][0]['state'])

    def test_invalid_or_linked_admission_config_is_not_read_or_ignored(self):
        record = self.record()
        selector = self.selector(record)
        config = self.root / 'project-brain/config/runtime.json'
        config.parent.mkdir(parents=True, exist_ok=True)
        for text in ('{', '{"allowed_privacy":[],"allowed_privacy":["public"]}'):
            config.write_text(text)
            self.assertEqual('unclassified', self.projection([selector])['facts'][0]['state'])
        config.unlink()
        outside = self.root / 'outside.json'
        outside.write_text('{"allowed_privacy":["public"]}')
        config.symlink_to(outside)
        self.assertEqual('unclassified', self.projection([selector])['facts'][0]['state'])
        config.unlink()
        self.assertEqual('current', self.projection([selector])['facts'][0]['state'])

    def test_original_sources_are_complete_crlf_neutral_and_not_refreshed(self):
        record = self.record()
        selector = self.selector(record)
        self.source.write_bytes(b'Production uses PostgreSQL.\r\n')
        self.assertEqual('current', self.projection([selector])['facts'][0]['state'])
        self.source.write_text('Production uses MariaDB.\n')
        candidates = facts.collect_candidates(self.root, [selector], as_of=self.moment)
        self.assertEqual('stale', candidates[0]['reason'])
        self.assertNotEqual('current', facts.resolve_current(candidates, namespace='shop',
            as_of='2028-01-01T00:00:00Z')['facts'][0]['state'])
        self.source.unlink()
        self.assertNotEqual('current', self.projection([selector])['facts'][0]['state'])
        self.source.write_text('Production uses PostgreSQL.\n')
        self.rewrite(self.root / selector['source'], lambda md: md.update(source_fingerprints=[]))
        self.assertNotEqual('current', self.projection([selector])['facts'][0]['state'])

    def test_conflicts_and_scope_survive_without_caller_pointers(self):
        record = self.record()
        selector = self.selector(record)
        conflict = '48d5c1da-8f09-4ac1-bf6b-2e6850d34527'
        self.rewrite(self.root / selector['source'], lambda md: md.update(conflicts=[conflict]))
        result = self.projection([selector])
        self.assertEqual('ambiguous', result['facts'][0]['state'])
        self.assertEqual([conflict], result['facts'][0]['history'][0]['conflicts'])
        other = dict(selector, id='staging', subject='worker', qualifiers={'environment': 'staging'})
        result = self.projection([selector, other])
        self.assertEqual(2, len(result['facts']))
        self.assertEqual(result, self.projection([other, selector]))

    def test_canonical_time_cannot_be_overridden_or_backdated(self):
        record = self.record()
        selector = self.selector(record)
        self.assertNotEqual('current', self.projection([selector], '2026-10-01T12:00:00Z')['facts'][0]['state'])
        for key in ('recorded_at_pointer', 'valid_from_pointer', 'valid_to_pointer', 'lineage_pointer',
                    'revision_pointer', 'conflicts_pointer', 'authority_pointer', 'attestation_pointer'):
            with self.subTest(key=key), self.assertRaises(ValueError):
                facts.collect_candidates(self.root, [dict(selector, **{key: '/created_at'})], as_of=self.moment)

    def test_promoted_bank_keeps_original_evidence_and_conservative_day_precision(self):
        record = self.record(attestation='person')
        chunk, selector = self.promoted(record)
        group = self.projection([selector])['facts'][0]
        self.assertEqual('current', group['state'])
        self.assertIn('Production uses PostgreSQL.', group['claim'])
        self.assertEqual('person', group['history'][0]['attestation'])
        self.assertEqual('day', group['history'][0]['recorded_at_precision'])
        created = json.loads(chunk.read_text()[4:].split('\n---\n', 1)[0])['created']
        self.assertNotEqual('current', self.projection([selector], created + 'T12:00:00Z')['facts'][0]['state'])
        self.source.write_text('Production uses MariaDB.\n')
        self.assertEqual('stale', self.projection([selector])['facts'][0]['state'])

    def test_bank_without_matching_verified_origins_does_not_infer_person_from_tags(self):
        record = self.record()
        chunk, selector = self.promoted(record)
        self.rewrite(chunk, lambda md: md.update(sources=['specs/database.md'],
            source_digests=[self.brain.fingerprint(self.root, 'specs/database.md')], tags=[]))
        self.assertNotEqual('current', self.projection([selector])['facts'][0]['state'])

    def test_bank_cannot_launder_changed_source_by_repinning_only_the_chunk(self):
        record = self.record()
        chunk, selector = self.promoted(record)
        self.source.write_text('Production uses MariaDB.\n')
        self.rewrite(chunk, lambda md: md.update(source_digests=[
            self.brain.fingerprint(self.root, source) for source in md['sources']]))
        result = self.projection([selector])
        self.assertNotEqual('current', result['facts'][0]['state'])
        self.assertNotIn('Production uses PostgreSQL.', json.dumps(result))

    def test_bank_origin_conflicts_scopes_and_inclusive_expiry_are_preserved(self):
        record = self.record()
        chunk, selector = self.promoted(record)
        conflict = '48d5c1da-8f09-4ac1-bf6b-2e6850d34527'
        self.rewrite(self.root / self.selector(record)['source'], lambda md: md.update(conflicts=[conflict]))
        self.rewrite(chunk, lambda md: md.update(source_digests=[
            self.brain.fingerprint(self.root, source) for source in md['sources']]))
        group = self.projection([selector])['facts'][0]
        self.assertEqual('ambiguous', group['state'])
        self.assertEqual([conflict], group['history'][0]['conflicts'])
        self.rewrite(self.root / self.selector(record)['source'], lambda md: md.update(conflicts=[]))
        self.rewrite(chunk, lambda md: md.update(source_digests=[
            self.brain.fingerprint(self.root, source) for source in md['sources']]))
        copy = chunk.with_name(chunk.name.replace('database-choice', 'other-scope'))
        copy.write_bytes(chunk.read_bytes())
        self.rewrite(copy, lambda md: md.update(scope=['worker']))
        other = dict(selector, id='worker', source=copy.relative_to(self.root).as_posix())
        result = self.projection([selector, other])
        self.assertEqual(2, len(result['facts']))
        self.assertEqual([['application'], ['worker']], [group['canonical_scope'] for group in result['facts']])
        self.assertEqual(result, self.projection([other, selector]))
        expiry = (datetime.now(timezone.utc) + timedelta(days=4)).date().isoformat()
        self.rewrite(chunk, lambda md: md.update(valid_to=expiry))
        self.assertEqual('current', self.projection([selector], expiry + 'T23:59:59Z')['facts'][0]['state'])
        tomorrow = (datetime.fromisoformat(expiry) + timedelta(days=1)).isoformat() + 'Z'
        self.assertNotEqual('current', self.projection([selector], tomorrow)['facts'][0]['state'])

    def test_limits_and_secret_or_oversized_body_claims_fail_closed(self):
        record = self.record(privacy='team')
        selector = self.selector(record, pointer='/body')
        self.assertEqual('current', self.projection([selector])['facts'][0]['state'])
        path = self.root / selector['source']
        text = path.read_text()
        header, _ = text[4:].split('\n---\n', 1)
        for body in ('x' * 2001, 'sk_live_' + 'A' * 24):
            path.write_text('---\n' + header + '\n---\n' + body)
            result = self.projection([selector])
            self.assertNotEqual('current', result['facts'][0]['state'])
            self.assertNotIn(body, json.dumps(result))
        path.write_text(text)
        with mock.patch.dict(facts.ADAPTER_LIMITS, max_unique_reads=1):
            self.assertNotEqual('current', self.projection([selector])['facts'][0]['state'])
        with mock.patch.object(facts, 'MAX_TREE', 16):
            self.assertNotEqual('current', self.projection([selector])['facts'][0]['state'])

    def test_explicit_json_kind_keeps_legacy_projection_and_cli_contract(self):
        source = self.root / 'facts.json'
        source.write_text('{"claim":"PostgreSQL","at":"2026-10-02T00:00:00Z"}')
        selector = {'id': 'json', 'namespace': 'shop', 'subject': 'api', 'predicate': 'database',
            'source': 'facts.json', 'pointer': '/claim', 'recorded_at_pointer': '/at'}
        legacy = self.projection([selector])
        self.assertEqual(legacy, self.projection([dict(selector, source_kind='json')]))
        self.assertNotIn('adapter_limits', legacy)
        record = self.record()
        selectors = self.root / 'selectors.json'
        selectors.write_text(json.dumps([self.selector(record)]))
        result = subprocess.run([sys.executable, '-B', str(Path(facts.__file__)), '--root', str(self.root),
            '--selectors', str(selectors), '--as-of', self.moment, '--json'], capture_output=True, text=True)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual('PostgreSQL', json.loads(result.stdout)['projections'][0]['facts'][0]['claim'])
        self.assertEqual('', result.stderr)

    def test_rebuild_is_read_only_and_private_paths_symlinks_duplicate_headers_refused(self):
        record = self.record()
        selector = self.selector(record)
        before = {p.relative_to(self.root).as_posix(): p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        self.projection([selector]); self.projection([selector])
        self.assertEqual(before, {p.relative_to(self.root).as_posix(): p.read_bytes() for p in self.root.rglob('*') if p.is_file()})
        for source in ('project-brain/local/private.md', 'project-brain/control/handoffs/task.md',
                       'project-brain/archive/findings/' + record['id'] + '.md'):
            result = self.projection([dict(selector, source=source)])
            self.assertNotEqual('current', result['facts'][0]['state'])
            self.assertNotIn(source, json.dumps(result))
        original = self.source.read_bytes(); self.source.unlink()
        external = self.root / 'outside.md'; external.write_bytes(original)
        self.source.symlink_to(external)
        self.assertNotEqual('current', self.projection([selector])['facts'][0]['state'])
        self.source.unlink(); self.source.write_bytes(original)
        path = self.root / selector['source']
        text = path.read_text()
        path.write_text(text.replace('"authority": "verified"', '"authority": "observed", "authority": "verified"'))
        self.assertNotEqual('current', self.projection([selector])['facts'][0]['state'])


if __name__ == '__main__': unittest.main()
