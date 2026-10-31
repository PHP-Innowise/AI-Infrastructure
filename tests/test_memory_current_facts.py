"""Known-answer current-fact projections, without models or a second authority."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import memory_current_facts as facts


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


if __name__ == '__main__': unittest.main()
