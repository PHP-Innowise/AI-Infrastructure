"""Fresh public offline export and desired-revision controls; no service/model."""
from dataclasses import FrozenInstanceError, replace
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import hindsight_export as he
from harness.hindsight_recall import Catalogue, Scope, SourceBinding

FIXTURE = Path(__file__).parent / 'fixtures'
PUBLIC = json.loads((FIXTURE / 'hindsight_export_public.json').read_text())
EXPECTED = json.loads((FIXTURE / 'hindsight_export_expected.json').read_text())


class PublicExportTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'clone'
        self.root.mkdir()
        self.scope = Scope(PUBLIC['team_id'], PUBLIC['project_id'])
        self.names = tuple(x['path'] for x in PUBLIC['sources'])
        self.raw = {x['path']: x['text'].encode() for x in PUBLIC['sources']}
        for name, raw in self.raw.items():
            self.write(name, raw)

    def write(self, name, raw):
        node = self.root / name
        node.parent.mkdir(parents=True, exist_ok=True)
        node.write_bytes(raw)

    def approve(self, name, raw=None, *, scope=None):
        image = self.raw[name] if raw is None else raw
        return he.PublicApproval(scope or self.scope, name, hashlib.sha256(image).hexdigest())

    def build(self, approvals=None, **options):
        if approvals is None:
            approvals = tuple(self.approve(n) for n in self.names)
        return he.build_public_export(self.root, self.scope, approvals, **options)

    def denied(self, code, approvals, **options):
        with self.assertRaises(he.ExportError) as result:
            self.build(approvals, **options)
        self.assertEqual(code, result.exception.reason)
        observation = result.exception.observation
        self.assertEqual(observation['opened_descriptors'], observation['closed_descriptors'])
        self.assertEqual(0, observation['close_errors'])
        self.assertEqual(code, str(result.exception))
        return observation

    def test_whole_literal_packet_requests_and_raw_bytes(self):
        with patch('socket.create_connection', side_effect=AssertionError('No network allowed')):
            result = self.build()
        self.assertEqual(EXPECTED['packet_v1'].encode(), result.serialized)
        self.assertEqual(EXPECTED['packet_v1_sha256'], hashlib.sha256(result.serialized).hexdigest())
        self.assertEqual(EXPECTED['bank_id'], result.catalogue.scope.bank_id)
        self.assertEqual(len(EXPECTED['documents_v1']), len(result.retain_drafts))
        for draft, expected in zip(result.retain_drafts, EXPECTED['documents_v1']):
            body = json.dumps(expected['retain_request'], ensure_ascii=False, separators=(',', ':')).encode()
            self.assertEqual(body, draft.body)
            self.assertEqual(('POST', '/v1/default/banks/' + EXPECTED['bank_id'] + '/memories'),
                             (draft.method, draft.resource))
            self.assertEqual(expected['document_id'], draft.binding.document_id)
            self.assertEqual(expected['source_id'], draft.source_id)
            self.assertNotIn(draft.binding.path.encode(), draft.body)
            actual_text = json.loads(draft.body)['items'][0]['content']
            self.assertEqual(self.raw[draft.binding.path], actual_text.encode())
        self.assertIn(b'\\r\\n', result.serialized)
        self.assertEqual(EXPECTED['raw_bytes_v1'], result.diagnostics['source_read_bytes'])
        self.assertEqual(EXPECTED['retain_body_bytes_v1'], result.diagnostics['retain_draft_body_bytes'])

    def test_clone_rename_worktree_and_project_scope(self):
        first = self.build()
        moved = self.root.with_name('renamed-same-basename')
        self.root.rename(moved)
        second = he.build_public_export(moved, self.scope, tuple(self.approve(n) for n in self.names))
        self.assertEqual(first.serialized, second.serialized)
        worktree = moved.with_name('worktree')
        worktree.mkdir()
        for name, raw in self.raw.items():
            node = worktree / name
            node.parent.mkdir(parents=True, exist_ok=True)
            node.write_bytes(raw)
        third = he.build_public_export(worktree, self.scope, tuple(self.approve(n) for n in self.names))
        self.assertEqual(first.serialized, third.serialized)
        other = Scope(PUBLIC['team_id'], PUBLIC['other_project_id'])
        different = he.build_public_export(worktree, other, tuple(self.approve(n, scope=other) for n in self.names))
        self.assertNotEqual(first.catalogue.scope.bank_id, different.catalogue.scope.bank_id)
        self.assertTrue({x.document_id for x in first.catalogue.sources}.isdisjoint(
                        x.document_id for x in different.catalogue.sources))
        with self.assertRaises(he.ExportError):
            he.build_public_export(worktree, other, tuple(self.approve(n) for n in self.names))

    def test_correction_removal_exact_delta_and_old_version_excluded(self):
        old = self.build()
        changed = PUBLIC['sources'][0]['changed_text'].encode()
        self.write(self.names[0], changed)
        (self.root / self.names[1]).unlink()
        new = self.build((self.approve(self.names[0], changed), self.approve(self.names[2])))
        self.assertEqual(EXPECTED['packet_v2'].encode(), new.serialized)
        self.assertEqual(old.retain_drafts[0].source_id, new.retain_drafts[0].source_id)
        plan = he.plan_public_ingestion(new, previous_owned=old.catalogue)
        oracle = EXPECTED['delta']
        self.assertEqual(oracle['retain_candidate_ids'], [x.binding.document_id for x in plan.retain_candidates])
        self.assertEqual(oracle['retire_candidate_ids'], [x.document_id for x in plan.retire_candidates])
        self.assertEqual(oracle['reconciliation_ids'], [x.document_id for x in plan.reconciliation])
        self.assertEqual(oracle['input_match_count'], plan.input_match_count)
        self.assertFalse({x.document_id for x in plan.retire_candidates} & {x.document_id for x in plan.current.sources})
        self.assertEqual(0, plan.diagnostics['authorized_mutations'])
        self.assertIsNone(plan.diagnostics['remote_noop'])

    def test_repeated_input_is_reconciliation_not_remote_noop(self):
        export = self.build()
        for _ in range(2):
            plan = he.plan_public_ingestion(export, previous_owned=export.catalogue)
            self.assertEqual(3, plan.input_match_count)
            self.assertEqual((), plan.retain_candidates)
            self.assertEqual((), plan.retire_candidates)
            self.assertEqual('reconciliation_required', plan.diagnostics['state'])
            self.assertIsNone(plan.diagnostics['remote_noop'])
            self.assertEqual(0, plan.diagnostics['authorized_mutations'])
            self.assertEqual(tuple(('GET', '/v1/default/banks/' + self.scope.bank_id + '/documents/' + x.document_id)
                                   for x in export.catalogue.sources), plan.reconciliation_drafts)

    def test_empty_selection_is_only_supplied_retirement_and_initial_unknown(self):
        old = self.build()
        empty = self.build(())
        plan = he.plan_public_ingestion(empty, previous_owned=old.catalogue)
        self.assertEqual((), empty.catalogue.sources)
        self.assertEqual(old.catalogue.sources, plan.retire_candidates)
        self.assertEqual('reconciliation_required', plan.diagnostics['state'])
        initial = he.plan_public_ingestion(empty)
        self.assertEqual(0, initial.diagnostics['authorized_mutations'])
        self.assertIsNone(initial.diagnostics['remote_noop'])
        self.assertEqual((), initial.reconciliation)

    def test_missing_or_changed_approval_aborts_whole_capture(self):
        (self.root / self.names[1]).unlink()
        observation = self.denied('unavailable', tuple(self.approve(n) for n in self.names))
        self.assertEqual(len(self.raw[self.names[0]]), observation['source_read_bytes'])
        self.write(self.names[1], self.raw[self.names[1]])
        self.write(self.names[2], b'Changed unapproved public bytes.\n')
        self.denied('source_changed', tuple(self.approve(n) for n in self.names))

    def test_operator_classification_and_no_repo_discovery(self):
        for classification in ('team', 'restricted', 'private', True, None):
            with self.assertRaises(he.ExportError):
                he.PublicApproval(self.scope, self.names[0], hashlib.sha256(self.raw[self.names[0]]).hexdigest(), classification)
        self.write('runtime.json', b'{"allowed_privacy":["public","team"],"all_files_public":true}')
        self.write('docs/unapproved.md', b'Unapproved file is outside the supplied selection.\n')
        result = self.build()
        self.assertEqual(3, len(result.catalogue.sources))
        self.assertNotIn(b'Unapproved file', result.serialized)
        self.assertNotIn(b'allowed_privacy', result.serialized)

    def test_bounded_unique_typed_approvals_before_io(self):
        one = self.approve(self.names[0])
        for invalid in ([one], (one, one), (one,) * 33, ({'scope': self.scope, 'path': self.names[0]},)):
            with patch.object(he, '_read', side_effect=AssertionError('Invalid input must not read')):
                self.denied('approval', invalid)

    def test_private_noncanonical_and_unicode_paths(self):
        for name in PUBLIC['unsafe_paths'] + ['docs/a\u202eb.md', 'docs/a\x00b.md', 'C:/docs/a.md',
                                             'docs/./a.md', 'docs/a\\b.md', '/docs/a.md', 'docs/file.zip']:
            with self.subTest(name=name), self.assertRaises(he.ExportError):
                he.PublicApproval(self.scope, name, 'a' * 64)

    def test_personal_blank_and_recognized_secret_content(self):
        negatives = [x['text'].encode() for x in PUBLIC['rejected_content']]
        negatives.append(b'API_KEY=fixtureHarbor123456789\n')
        for raw in negatives:
            self.write(self.names[0], raw)
            with self.assertRaises(he.ExportError) as caught:
                self.build((self.approve(self.names[0], raw),))
            self.assertIn(caught.exception.reason, ('private_content', 'source_text'))
            self.assertNotIn('fixtureHarbor', str(caught.exception))
            self.assertEqual(caught.exception.observation['opened_descriptors'],
                             caught.exception.observation['closed_descriptors'])

    def test_transcript_lines_with_cr_lf_crlf_preserve_detection(self):
        for separator in ('\r', '\n', '\r\n'):
            raw = ('Heading' + separator + 'log: public sailing output').encode()
            self.write(self.names[0], raw)
            self.denied('private_content', (self.approve(self.names[0], raw),))

    def test_invalid_utf8_nul_del_and_unicode_controls(self):
        for raw in (b'\xff', b'Public\x00image', b'Public\x7fimage', 'Public\u202eimage'.encode()):
            self.write(self.names[0], raw)
            self.denied('source_text', (self.approve(self.names[0], raw),))

    def test_leaf_and_directory_links_and_hardlinks_fifo(self):
        target = self.root / self.names[0]
        outside = self.root / 'outside.md'
        outside.write_bytes(self.raw[self.names[0]])
        target.unlink()
        target.symlink_to(outside)
        self.denied('unavailable', (self.approve(self.names[0]),))
        target.unlink()
        os.link(outside, target)
        self.denied('file_kind', (self.approve(self.names[0]),))
        target.unlink()
        os.mkfifo(target)
        self.denied('file_kind', (self.approve(self.names[0]),))
        target.unlink()
        target.write_bytes(self.raw[self.names[0]])
        (self.root / 'docs').rename(self.root / 'docs-real')
        (self.root / 'docs').symlink_to(self.root / 'docs-real', target_is_directory=True)
        self.denied('unavailable', (self.approve(self.names[0]),))

    def test_absolute_root_parent_link_and_unsupported_primitives(self):
        alias = self.root.with_name('alias')
        alias.symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(he.ExportError):
            he.build_public_export(alias, self.scope, tuple(self.approve(n) for n in self.names))
        with patch.object(os, 'supports_dir_fd', frozenset()):
            self.denied('unsupported', (self.approve(self.names[0]),))

    def test_named_parent_replacement_during_read_is_not_current(self):
        actual_read, changed = os.read, []
        def swapped(fd, count):
            raw = actual_read(fd, count)
            if raw and not changed:
                changed.append(True)
                (self.root / 'docs').rename(self.root / 'old-docs')
                self.write(self.names[0], self.raw[self.names[0]])
            return raw
        with patch.object(os, 'read', swapped):
            self.denied('source_changed', (self.approve(self.names[0]),))

    def test_leaf_modification_during_read_and_cancellation(self):
        actual_read, changed = os.read, []
        def edited(fd, count):
            raw = actual_read(fd, count)
            if raw and not changed:
                changed.append(True)
                self.write(self.names[0], b'Edited after capture begins.\n')
            return raw
        with patch.object(os, 'read', edited):
            self.denied('source_changed', (self.approve(self.names[0]),))
        self.write(self.names[0], self.raw[self.names[0]])
        self.denied('cancelled', (self.approve(self.names[0]),), active=lambda: False)
        answers = iter((True, True, False))
        self.denied('cancelled', (self.approve(self.names[0]),), active=lambda: next(answers))

    def test_source_aggregate_and_complete_serialization_limits(self):
        huge = b'a' * (he.SOURCE_BYTES + 1)
        self.write(self.names[0], huge)
        self.denied('source_bound', (self.approve(self.names[0], huge),))
        approvals = []
        for i in range(5):
            name, raw = 'docs/full%d.md' % i, b'a' * he.SOURCE_BYTES
            self.write(name, raw); approvals.append(self.approve(name, raw))
        self.denied('source_bound', tuple(approvals))
        quoted = b'\\' * he.SOURCE_BYTES
        self.write(self.names[0], quoted)
        self.denied('request_bound', (self.approve(self.names[0], quoted),))
        approvals = []
        for i in range(5):
            name, raw = 'docs/quote%d.md' % i, b'\\' * 104850
            self.write(name, raw); approvals.append(self.approve(name, raw))
        self.denied('packet_bound', tuple(approvals))

    def test_foreign_and_forged_previous_catalogues_do_not_target_deletes(self):
        export = self.build()
        other = Scope(PUBLIC['team_id'], PUBLIC['other_project_id'])
        foreign = Catalogue(other, (SourceBinding.from_bytes(other, self.names[0], self.raw[self.names[0]]),))
        original = export.catalogue.sources[0]
        forged = Catalogue(self.scope, (replace(original, document_id='server-chosen-target'),))
        for previous in (foreign, forged, {'success': True}, {'status': 'completed'}, {'catalogue': export.catalogue}):
            with self.assertRaises(he.ExportError):
                he.plan_public_ingestion(export, previous_owned=previous)

    def test_immutable_images_and_planned_costs_are_not_executed_usage(self):
        export = self.build()
        with self.assertRaises(FrozenInstanceError):
            export.retain_drafts[0].body = b'changed'
        old = export.serialized
        self.write(self.names[0], b'Later local changes do not mutate held image.\n')
        self.assertEqual(old, export.serialized)
        self.assertEqual(EXPECTED['packet_v1'].encode(), old)
        diagnostic = export.diagnostics
        self.assertEqual(0, diagnostic['executed_transport_calls'])
        self.assertFalse(diagnostic['total_cost_complete'])
        self.assertEqual(len(old), diagnostic['export_packet_bytes'])
        self.assertEqual(diagnostic['opened_descriptors'], diagnostic['closed_descriptors'])
        for key in ('provider_tokens', 'remote_tokens', 'remote_usd'):
            self.assertIsNone(diagnostic[key])
        self.assertTrue(all(x is None for x in diagnostic['remote_cost_stages'].values()))
        diagnostic['source_count'] = 99
        self.assertEqual(3, export.diagnostics['source_count'])
        for text in self.names + (PUBLIC['sources'][0]['text'], PUBLIC['team_id'], EXPECTED['scope']):
            self.assertNotIn(text, json.dumps(export.diagnostics))

    def test_unchecked_dataclass_inputs_cannot_forge_payload_or_diagnostics(self):
        export = self.build()
        draft = export.retain_drafts[0]
        for changes in ({'body': bytearray(draft.body)}, {'body': b'{"items":[]}'},
                        {'source_id': 'foreign-logical-id'},
                        {'binding': replace(draft.binding, document_id='foreign-document')}):
            with self.assertRaises(he.ExportError):
                replace(draft, **changes)
        for changes in ({'serialized': bytearray(export.serialized)}, {'serialized': b'{}'},
                        {'retain_drafts': list(export.retain_drafts)},
                        {'retain_drafts': tuple(reversed(export.retain_drafts))},
                        {'_io': export._io + (('raw_body', 'PRIVATE-OBSERVATION'),)},
                        {'_io': tuple((key, True) for key, _ in export._io)},
                        {'_io': tuple((key, 0 if key in ('source_read_calls', 'opened_descriptors', 'closed_descriptors') else count)
                                      for key, count in export._io)},
                        {'_io': tuple((key, count + 1 if key == 'source_read_bytes' else count)
                                      for key, count in export._io)}):
            with self.assertRaises(he.ExportError):
                replace(export, **changes)
        foreign = Scope(PUBLIC['team_id'], PUBLIC['other_project_id'])
        catalogue = Catalogue(foreign, ())
        with self.assertRaises(he.ExportError):
            replace(export, catalogue=catalogue)
        plan = he.plan_public_ingestion(export, previous_owned=export.catalogue)
        self.assertEqual(export.retain_drafts, plan.current_retain_drafts)
        self.assertEqual((), plan.retain_candidates)
        for changes in ({'current_retain_drafts': ()}, {'input_match_count': True},
                        {'retire_candidates': export.catalogue.sources},
                        {'reconciliation': ()},
                        {'reconciliation': plan.reconciliation + (SourceBinding.from_bytes(
                            self.scope, 'docs/unowned.md', b'Outside the prior/current union.\n'),)}):
            with self.assertRaises(he.ExportError):
                replace(plan, **changes)


if __name__ == '__main__':
    unittest.main()
