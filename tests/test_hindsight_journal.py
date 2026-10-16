"""New supplied public images and actual private SQLite ownership controls.

No service/native/model calls or actor/process crash claims. Full literal input
oracles were authored before the implementation, independently of its imports.
"""
from dataclasses import replace
import hashlib
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import hindsight_export as he, hindsight_journal as hj
from harness.hindsight_recall import Catalogue, Scope

FIXTURES = Path(__file__).parent / 'fixtures'
PUBLIC = json.loads((FIXTURES / 'hindsight_journal_public.json').read_text())
EXPECTED = json.loads((FIXTURES / 'hindsight_journal_expected.json').read_text())


def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode()


class JournalTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.workspace = self.root / 'public'
        self.workspace.mkdir()
        self.scope = Scope(PUBLIC['team_id'], PUBLIC['project_id'])
        self.dbfile = self.root / 'private.sqlite3'
        self.db = self.connect()
        self.journal = hj.create_new(self.db, self.scope, PUBLIC['journal_id'])

    def connect(self, *, node=None, autocommit=None):
        options = {'timeout': 0.01}
        if autocommit is not None:
            options['autocommit'] = autocommit
        db = sqlite3.connect(node or self.dbfile, **options)
        self.addCleanup(db.close)
        db.execute('PRAGMA journal_mode=WAL')
        db.execute('PRAGMA synchronous=FULL')
        return db

    def open(self, db=None):
        return hj.open_existing(db or self.connect(), self.scope, PUBLIC['journal_id'])

    def nonce(self, number):
        return str(uuid.uuid5(uuid.UUID(PUBLIC['journal_id']), 'fresh-control-' + str(number)))

    def select(self, number=1):
        values = PUBLIC['sources']
        if number == 2:
            values = [dict(values[0], text=values[0]['changed_text']), values[2]]
        elif number == 3:
            values = []
        return self.export(values)

    def export(self, values):
        approvals = []
        for item in values:
            raw = item['text'].encode()
            node = self.workspace / item['path']
            node.parent.mkdir(parents=True, exist_ok=True)
            node.write_bytes(raw)
            approvals.append(he.PublicApproval(self.scope, item['path'], hashlib.sha256(raw).hexdigest()))
        return he.build_public_export(self.workspace, self.scope, tuple(approvals))

    def prepare(self, number=1, *, journal=None, prior=None):
        return (journal or self.journal).prepare(self.select(number), PUBLIC['generations'][number-1],
                                               number - 1, previous_owned=prior)

    def denied(self, reason, function, *args, **kwargs):
        with self.assertRaises(hj.JournalError) as result:
            function(*args, **kwargs)
        self.assertEqual(reason, result.exception.reason)
        self.assertEqual(reason, str(result.exception))

    def observations(self, challenge, *, absent=()):
        all_images = {d['binding']['document_id']: d['observation'] for generation in EXPECTED['generations']
                      for d in generation['documents']}
        result = []
        for binding in challenge.expected:
            if binding.document_id in absent:
                result.append(hj.observe_document(binding, 404, b'', eof=True))
            else:
                result.append(hj.observe_document(binding, 200, encoded(all_images[binding.document_id]), eof=True))
        return tuple(result)

    def evaluate(self, *, absent=(), profile=hj.VISIBLE, nonce=100):
        challenge = self.journal.challenge(self.nonce(nonce))
        return self.journal.evaluate(challenge, self.observations(challenge, absent=absent), profile=profile)

    def test_independent_whole_manifests_frozen_requests_and_reopen(self):
        old = None
        for number in (1, 2, 3):
            selected = self.select(number)
            prepared = self.journal.prepare(selected, PUBLIC['generations'][number-1], number-1, previous_owned=old)
            oracle = EXPECTED['generations'][number-1]
            self.assertEqual(oracle['manifest'].encode(), prepared.manifest)
            self.assertEqual(oracle['manifest_sha256'], hashlib.sha256(prepared.manifest).hexdigest())
            fresh = self.open()
            self.assertEqual(prepared, fresh.current())
            for identity, document in zip(prepared.intent_ids, oracle['documents']):
                self.assertEqual(document['descriptor']['intent_id'], identity)
                self.assertEqual(document['body'].encode(), fresh.request_body(identity))
            old = selected.catalogue
        self.assertEqual(0, self.journal.diagnostics()['recorded_attempts'])
        self.assertEqual(0, self.journal.diagnostics()['executed_transport_calls'])

    def test_current_replay_cas_and_noncurrent_generation_reuse(self):
        selected = self.select()
        first = self.prepare()
        self.assertEqual(first, self.journal.prepare(selected, PUBLIC['generations'][0], 1))
        self.denied('conflict', self.journal.prepare, selected, PUBLIC['generations'][0], 0)
        self.denied('conflict', self.journal.prepare, self.select(2), PUBLIC['generations'][0], 1)
        self.prepare(2)
        self.denied('conflict', self.journal.prepare, selected, PUBLIC['generations'][0], 2)
        self.assertEqual(2, self.journal.current().revision)

    def test_revalidate_forged_export_before_writes(self):
        selected = self.select()
        object.__setattr__(selected, 'serialized', selected.serialized + b' ')
        self.denied('input', self.journal.prepare, selected, PUBLIC['generations'][0], 0)
        self.assertIsNone(self.journal.current())
        self.assertEqual(0, self.journal.diagnostics()['intents'])

    def test_reserved_ownership_survives_lost_ack_and_reopen(self):
        first = self.prepare()
        ticket = self.journal.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        fresh = self.open()
        self.assertEqual(1, fresh.diagnostics()['pending_attempts'])
        self.denied('admitted', fresh.reserve_attempt, first.intent_ids[0], PUBLIC['nonces'][1], expected_revision=1)
        self.denied('admitted', fresh.reserve_attempt, first.intent_ids[1], ticket.nonce, expected_revision=1)
        self.assertEqual(1, fresh.diagnostics()['recorded_attempts'])

    def test_two_connections_cas_and_attempt_single_winner(self):
        other = self.open()
        first = self.prepare()
        self.denied('conflict', other.prepare, self.select(2), PUBLIC['generations'][1], 0)
        self.journal.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        self.denied('admitted', other.reserve_attempt, first.intent_ids[0], PUBLIC['nonces'][1], expected_revision=1)
        self.assertEqual(first, other.current())

    def test_pending_old_version_blocks_new_and_late_ack_keeps_head(self):
        old_export = self.select()
        first = self.prepare()
        ticket = self.journal.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        second = self.prepare(2, prior=old_export.catalogue)
        self.denied('pending', self.journal.reserve_attempt, second.intent_ids[0], PUBLIC['nonces'][1], expected_revision=2)
        self.denied('settlement', self.journal.attest_terminal, ticket, 'settled_unknown', profile='timeout')
        self.assertEqual(1, self.journal.diagnostics()['pending_attempts'])
        self.journal.attest_terminal(ticket, 'settled_unknown', profile=hj.QUIESCENT)
        self.assertEqual(second, self.journal.current())
        new = self.journal.reserve_attempt(second.intent_ids[0], PUBLIC['nonces'][1], expected_revision=2)
        self.assertNotEqual(ticket.intent_id, new.intent_id)
        challenge = self.journal.challenge(self.nonce(100))
        self.assertIn(old_export.catalogue.sources[0], challenge.expected)
        self.assertEqual('incomplete', self.journal.evaluate(challenge, self.observations(challenge), profile=hj.VISIBLE)['state'])

    def test_late_completion_after_removal_never_recatalogues(self):
        first = self.prepare()
        ticket = self.journal.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        self.prepare(2)
        third = self.prepare(3)
        self.journal.attest_terminal(ticket, 'success', profile=hj.QUIESCENT)
        self.assertEqual((), self.open().current().intent_ids)
        self.assertEqual(third, self.journal.current())
        answer = self.evaluate()
        self.assertEqual('cleanup_required', answer['state'])
        self.assertEqual(1, answer['obsolete_present'])
        self.assertEqual(0, answer['authorized_mutations'])

    def test_exact_terminal_replay_forgery_and_no_readmission(self):
        first = self.prepare()
        ticket = self.journal.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        self.denied('ticket', self.journal.attest_terminal, replace(ticket, body_sha256='0'*64), 'success', profile=hj.QUIESCENT)
        self.journal.attest_terminal(ticket, 'definite_no_write', profile=hj.QUIESCENT)
        self.journal.attest_terminal(ticket, 'definite_no_write', profile=hj.QUIESCENT)
        self.denied('conflict', self.journal.attest_terminal, ticket, 'success', profile=hj.QUIESCENT)
        self.denied('admitted', self.journal.reserve_attempt, first.intent_ids[0], PUBLIC['nonces'][1], expected_revision=1)
        self.assertEqual(0, self.journal.diagnostics()['pending_attempts'])

    def test_supplied_correspondence_never_remote_noop_or_cost(self):
        self.prepare()
        result = self.evaluate()
        self.assertEqual('supplied_selection_current', result['state'])
        self.assertEqual(3, result['expected_count'])
        self.assertIsNone(result['actual_remote_noop'])
        self.assertIsNone(result['provider_tokens'])
        self.assertIsNone(result['remote_usd'])
        self.assertIsNone(result['physical_sqlite_io_bytes'])
        self.assertFalse(result['total_cost_complete'])
        self.assertEqual(0, result['authorized_mutations'])
        self.assertEqual(0, result['executed_transport_calls'])

    def test_scope_masked_404_requires_supplied_visibility(self):
        first = self.prepare()
        binding = self.select().catalogue.sources[0]
        projection = hj.observe_document(binding, 404, b'', eof=True)
        self.assertEqual('observed_not_found', projection.state)
        result = self.evaluate(absent=(binding.document_id,), profile=None)
        self.assertEqual('incomplete', result['state'])
        result = self.evaluate(absent=(binding.document_id,), nonce=101)
        self.assertEqual('missing_source_images', result['state'])
        self.assertEqual(1, result['missing_current'])
        ticket = self.journal.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        self.journal.attest_terminal(ticket, 'success', profile=hj.QUIESCENT)
        self.assertEqual('missing_source_images', self.evaluate(absent=(binding.document_id,), nonce=102)['state'])
        self.denied('admitted', self.journal.reserve_attempt, first.intent_ids[0], PUBLIC['nonces'][1], expected_revision=1)

    def test_old_present_cleanup_and_absent_old_exact_selection(self):
        old = self.select()
        self.prepare()
        self.prepare(2, prior=old.catalogue)
        self.assertEqual('cleanup_required', self.evaluate()['state'])
        removed = tuple(b.document_id for b in old.catalogue.sources[:2])
        result = self.evaluate(absent=removed, nonce=101)
        self.assertEqual('supplied_selection_current', result['state'])
        self.assertEqual(0, result['obsolete_present'])
        self.assertIsNone(result['actual_remote_noop'])

    def test_stale_head_frontier_replay_and_reopen_challenges(self):
        self.prepare()
        challenge = self.journal.challenge(self.nonce(100))
        supplied = self.observations(challenge)
        self.prepare(2)
        self.denied('challenge', self.journal.evaluate, challenge, supplied, profile=hj.VISIBLE)
        self.denied('challenge', self.journal.evaluate, challenge, supplied, profile=hj.VISIBLE)
        current = self.journal.current()
        fresh = self.journal.challenge(self.nonce(101))
        self.journal.reserve_attempt(current.intent_ids[0], PUBLIC['nonces'][0], expected_revision=2)
        self.denied('challenge', self.journal.evaluate, fresh, self.observations(fresh), profile=hj.VISIBLE)
        opened = self.open()
        self.denied('challenge', opened.evaluate, fresh, self.observations(fresh), profile=hj.VISIBLE)
        newly_issued = opened.challenge(fresh.nonce)
        self.assertNotEqual(fresh.instance_id, newly_issued.instance_id)

    def test_complete_duplicate_extra_and_forged_observation_sets(self):
        self.prepare()
        for number, change in enumerate((lambda x: x[:-1], lambda x: (x[0], x[0], x[2]),
                                         lambda x: x + (x[0],)), 100):
            challenge = self.journal.challenge(self.nonce(number))
            self.denied('observation', self.journal.evaluate, challenge, change(self.observations(challenge)), profile=hj.VISIBLE)
        challenge = self.journal.challenge(self.nonce(103))
        bad = list(self.observations(challenge))
        bad[0] = replace(bad[0], state='conflict')
        result = self.journal.evaluate(challenge, tuple(bad), profile=hj.VISIBLE)
        self.assertEqual(('incomplete', 1), (result['state'], result['conflict']))

    def test_projection_missing_nullable_mismatched_fields_and_raw_bytes(self):
        selected = self.select()
        for binding, document in zip(selected.catalogue.sources, EXPECTED['generations'][0]['documents']):
            image = document['observation']
            self.assertEqual('source_match', hj.observe_document(binding, 200, encoded(image), eof=True).state)
            for field in ('id', 'bank_id', 'original_text', 'tags', 'document_metadata', 'retain_params'):
                changed = dict(image, **{field: None})
                self.assertEqual('unknown', hj.observe_document(binding, 200, encoded(changed), eof=True).state)
            changed = dict(image, original_text=image['original_text'] + 'changed')
            self.assertEqual('conflict', hj.observe_document(binding, 200, encoded(changed), eof=True).state)
            changed = dict(image, tags=[])
            self.assertEqual('conflict', hj.observe_document(binding, 200, encoded(changed), eof=True).state)
            changed = dict(image, document_metadata=dict(image['document_metadata'], source_id='different'))
            self.assertEqual('conflict', hj.observe_document(binding, 200, encoded(changed), eof=True).state)
            changed = dict(image, retain_params={})
            self.assertEqual('unknown', hj.observe_document(binding, 200, encoded(changed), eof=True).state)

    def test_strict_json_duplicate_nonfinite_surrogate_depth_nodes_and_transport_bounds(self):
        binding = self.select().catalogue.sources[0]
        valid = encoded(EXPECTED['generations'][0]['documents'][0]['observation'])
        bodies = [valid[:-1]+b',"id":"duplicate"}', valid[:-1]+b',"extra":NaN}',
                  valid[:-1]+b',"extra":1e1000}', valid[:-1]+b',"extra":-1e1000}',
                  valid[:-1]+b',"\\ud800":0}', b'\xff', b'['*18+b'0'+b']'*18,
                  valid[:-1]+b',"extra":['+b'0,'*4096+b'0]}', b' '* (hj.MAX_RESPONSE+1)]
        for body in bodies:
            self.assertEqual('unknown', hj.observe_document(binding, 200, body, eof=True).state)
        for status, body, eof in ((True, valid, True), (200, valid, False), (500, valid, True),
                                  (404, valid, False), (200, valid.decode(), True)):
            self.assertEqual('unknown', hj.observe_document(binding, status, body, eof=eof).state)
        for invalid in (None, {}, 'not-a-binding'):
            self.denied('input', hj.observe_document, invalid, 200, valid, eof=True)

    def test_runtime_profiles_and_ambient_transaction_untouched(self):
        before = self.db.total_changes
        self.db.execute('BEGIN')
        self.denied('transaction', self.journal.current)
        self.assertTrue(self.db.in_transaction)
        self.assertEqual(before, self.db.total_changes)
        self.db.execute('ROLLBACK')
        for statement, undo in (('PRAGMA synchronous=OFF', 'PRAGMA synchronous=FULL'),
                                ('PRAGMA busy_timeout=1001', 'PRAGMA busy_timeout=10'),
                                ('PRAGMA ignore_check_constraints=1', 'PRAGMA ignore_check_constraints=0')):
            self.db.execute(statement)
            self.denied('profile', self.journal.current)
            self.db.execute(undo)
        self.db.row_factory = sqlite3.Row
        self.denied('profile', self.journal.current)
        self.db.row_factory = None
        self.db.text_factory = bytes
        self.denied('profile', self.journal.current)
        self.db.text_factory = str
        self.db.execute("ATTACH ':memory:' AS extra")
        self.denied('profile', self.journal.current)
        self.db.execute('DETACH extra')
        self.assertIsNone(self.journal.current())

    def test_supported_rollback_profile_memory_refusal_and_autocommit(self):
        db = self.connect(node=self.root/'rollback.sqlite3')
        db.execute('PRAGMA journal_mode=DELETE')
        db.execute('PRAGMA synchronous=EXTRA')
        journal = hj.create_new(db, self.scope, PUBLIC['journal_id'])
        self.assertIsNone(journal.current())
        memory = sqlite3.connect(':memory:')
        self.addCleanup(memory.close)
        self.denied('profile', hj.create_new, memory, self.scope, PUBLIC['journal_id'])
        if hasattr(sqlite3.Connection, 'autocommit'):
            direct = self.connect(node=self.root/'auto.sqlite3', autocommit=True)
            automatic = hj.create_new(direct, self.scope, PUBLIC['journal_id'])
            prepared = automatic.prepare(self.select(), PUBLIC['generations'][0], 0)
            self.assertFalse(direct.in_transaction)
            self.assertEqual(prepared, hj.open_existing(direct, self.scope, PUBLIC['journal_id']).current())
            direct.autocommit = False
            self.denied('transaction', automatic.current)
            self.assertTrue(direct.in_transaction)
            direct.autocommit = True

    def test_missing_foreign_replaced_identity_and_create_no_reset(self):
        empty = self.connect(node=self.root/'empty.sqlite3')
        self.denied('schema', hj.open_existing, empty, self.scope, PUBLIC['journal_id'])
        empty.execute('CREATE TABLE foreign_data (x TEXT)')
        self.denied('schema', hj.create_new, empty, self.scope, PUBLIC['journal_id'])
        self.assertEqual(1, empty.execute('SELECT COUNT(*) FROM sqlite_master').fetchone()[0])
        self.denied('schema', hj.create_new, self.db, self.scope, PUBLIC['journal_id'])
        self.denied('identity', hj.open_existing, self.connect(), self.scope, self.nonce(9))
        self.db.execute('UPDATE journal SET journal_id=?', (self.nonce(9),))
        self.db.commit()
        self.denied('identity', self.journal.current)

    def test_literal_sqlite_prefix_extra_trigger_cannot_erase_ticket(self):
        first = self.prepare()
        self.db.execute('CREATE TRIGGER sqliteXdrop_attempt AFTER INSERT ON attempts BEGIN DELETE FROM attempts WHERE nonce=NEW.nonce; END')
        self.denied('schema', self.journal.reserve_attempt, first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        self.denied('schema', self.open)
        self.assertEqual(0, self.db.execute('SELECT COUNT(*) FROM attempts').fetchone()[0])

    def test_corrupt_stored_body_manifest_and_metadata_admission(self):
        first = self.prepare()
        original = self.journal.request_body(first.intent_ids[0])
        self.db.execute('UPDATE intents SET body=? WHERE identity=?', (original+b' ', first.intent_ids[0]))
        self.db.commit()
        self.denied('corrupt', self.journal.current)
        self.db.execute('UPDATE intents SET body=? WHERE identity=?', (original, first.intent_ids[0]))
        self.db.execute('UPDATE journal SET profile=?', ('Δ'*70000,))
        self.db.commit()
        self.denied('identity', self.journal.current)
        self.db.execute('UPDATE journal SET profile=?', (hj.PROFILE,))
        self.db.execute('UPDATE generations SET manifest=? WHERE revision=1', (b'{}',))
        self.db.commit()
        self.denied('corrupt', self.open)

    def test_stored_text_nul_suffix_byte_admission_before_materialization(self):
        first = self.prepare()
        ticket = self.journal.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        cases = (('journal', 'team', PUBLIC['team_id'], 'identity'),
                 ('generations', 'generation', PUBLIC['generations'][0], 'corrupt'),
                 ('intents', 'digest', ticket.body_sha256, 'corrupt'),
                 ('attempts', 'nonce', ticket.nonce, 'corrupt'))
        for table, column, original, reason in cases:
            selection = ' WHERE identity=?' if table == 'intents' else ''
            suffix = (first.intent_ids[0],) if table == 'intents' else ()
            self.db.execute('UPDATE '+table+' SET '+column+'=?'+selection, (original+'\0'+'suffix'*50000,)+suffix)
            self.db.commit()
            materialized = []
            def trace(sql):
                if sql.startswith('SELECT * FROM '+table):
                    materialized.append(True)
            self.db.set_trace_callback(trace)
            try:
                self.denied(reason, self.journal.current)
            finally:
                self.db.set_trace_callback(None)
            self.assertEqual([], materialized)
            self.db.execute('UPDATE '+table+' SET '+column+'=?'+selection, (original,)+suffix)
            self.db.commit()
        self.assertEqual(first, self.open().current())

    def test_commit_error_returns_no_ticket_fences_until_strict_reopen(self):
        first = self.prepare()
        def authorizer(action, first_arg, second_arg, database, origin):
            if action == sqlite3.SQLITE_TRANSACTION and first_arg == 'COMMIT':
                return sqlite3.SQLITE_DENY
            return sqlite3.SQLITE_OK
        self.db.set_authorizer(authorizer)
        self.denied('storage', self.journal.reserve_attempt, first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        # Before Python 3.11, None is installed as the callback rather than
        # removing it, and every later statement is denied.
        self.db.set_authorizer(None if sys.version_info >= (3, 11) else lambda *_: sqlite3.SQLITE_OK)
        self.denied('fenced', self.journal.current)
        fresh = hj.open_existing(self.db, self.scope, PUBLIC['journal_id'])
        self.assertEqual(0, fresh.diagnostics()['recorded_attempts'])
        fresh.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][1], expected_revision=1)
        self.assertEqual(1, fresh.diagnostics()['recorded_attempts'])

    def test_other_writer_contention_preserves_its_transaction(self):
        self.prepare()
        other = self.connect()
        other.execute('BEGIN IMMEDIATE')
        self.denied('storage', self.journal.current)
        self.assertTrue(other.in_transaction)
        other.execute('ROLLBACK')
        self.assertEqual(1, self.journal.current().revision)

    def test_complete_historical_attempt_union_overflow_is_not_truncated(self):
        previous = None
        for generation in range(3):
            values = [{'path': 'docs/union-%d.md' % n, 'text': 'Public signal %d revision %d.\n' % (n, generation)}
                      for n in range(PUBLIC['stress']['sources_per_generation'])]
            selected = self.export(values)
            prepared = self.journal.prepare(selected, PUBLIC['generations'][generation], generation, previous_owned=previous)
            for n, identity in enumerate(prepared.intent_ids):
                ticket = self.journal.reserve_attempt(identity, self.nonce(generation*22+n), expected_revision=generation+1)
                self.journal.attest_terminal(ticket, 'success', profile=hj.QUIESCENT)
            previous = selected.catalogue
        self.assertEqual(PUBLIC['stress']['expected_union'], self.journal.diagnostics()['recorded_attempts'])
        self.denied('capacity', self.journal.challenge, self.nonce(100))
        self.assertEqual(0, self.journal.diagnostics()['pending_attempts'])

    def test_finite_generation_and_challenge_lifetime_admission(self):
        selected = self.select(3)
        for number in range(16):
            self.journal.prepare(selected, self.nonce(number), number)
        self.denied('capacity', self.journal.prepare, selected, self.nonce(17), 16)
        self.assertEqual(16, self.open().current().revision)
        for number in range(32):
            challenge = self.journal.challenge(self.nonce(100+number))
            self.assertEqual('supplied_selection_current', self.journal.evaluate(challenge, (), profile=hj.VISIBLE)['state'])
        self.denied('challenge', self.journal.challenge, self.nonce(132))
        self.denied('challenge', self.journal.challenge, self.nonce(100))

    def test_escaped_payload_budget_rolls_back_whole_generation(self):
        values = [{'path': 'docs/budget-%d.md' % n, 'text': '# Public whitespace note\n'+'\t'*90000}
                  for n in range(3)]
        selected = self.export(values)
        for number in range(15):
            self.journal.prepare(selected, self.nonce(number), number)
        before = self.journal.diagnostics()
        self.denied('capacity', self.journal.prepare, selected, self.nonce(15), 15)
        self.assertEqual(before, self.open().diagnostics())
        self.assertEqual((15, 45), (before['generations'], before['intents']))
        self.assertLessEqual(before['persisted_payload_bytes'], hj.MAX_PAYLOAD)

    def test_exact_public_type_boundaries_are_revalidated(self):
        first = self.prepare()
        ticket = self.journal.reserve_attempt(first.intent_ids[0], PUBLIC['nonces'][0], expected_revision=1)
        self.denied('input', self.journal.reserve_attempt, first.intent_ids[1], PUBLIC['nonces'][1], expected_revision=True)
        object.__setattr__(ticket, 'intent_id', [])
        self.denied('ticket', self.journal.attest_terminal, ticket, 'success', profile=hj.QUIESCENT)
        challenge = self.journal.challenge(self.nonce(100))
        forged = self.observations(challenge)
        object.__setattr__(forged[0], 'state', 'unauthorized-enum')
        self.denied('observation', self.journal.evaluate, challenge, forged, profile=hj.VISIBLE)


if __name__ == '__main__':
    unittest.main()
