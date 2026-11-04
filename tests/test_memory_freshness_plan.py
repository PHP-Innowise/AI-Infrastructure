"""Read-only review queue, not automatic attestation or a second mutator."""
from pathlib import Path
from datetime import date, datetime, timedelta, timezone
import copy
import json
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import memory_freshness_plan as freshness


class FreshnessPacketTests(unittest.TestCase):
    def test_aggregate_reasons_prioritize_and_report_truncation(self):
        audit = {'chunks': 2, 'source_changed': [
            {'id': 'MEM-0001', 'chunk': 'memory-bank/chunks/MEM-0001-one.md', 'state': 'changed', 'source': 'src/a.py'}],
            'overdue_review': [{'id': 'MEM-0001', 'chunk': 'memory-bank/chunks/MEM-0001-one.md', 'review_after': '2026-01-01'}],
            'undigested': ['memory-bank/chunks/MEM-0002-two.md'], 'duplicates': [],
            'provenance': {'instruction_like': ['memory-bank/chunks/MEM-0002-two.md']}}
        packet = freshness.build_review_packet(audit, snapshot_digest='a' * 64, limit=1)
        self.assertEqual(2, packet['total_candidates']); self.assertTrue(packet['truncated'])
        self.assertEqual(1, packet['remaining'])
        self.assertEqual('memory-bank/chunks/MEM-0002-two.md', packet['queue'][0]['chunk'])
        complete = freshness.build_review_packet(audit, snapshot_digest='a' * 64)
        self.assertEqual(['changed', 'overdue'], complete['queue'][1]['reasons'])
        self.assertEqual('src/a.py', complete['queue'][1]['sources'][0]['source'])
        self.assertEqual(['2026-01-01'], complete['queue'][1]['review_dates'])
        self.assertTrue(complete['read_only']); self.assertEqual('not-performed', complete['attestation'])
        self.assertNotIn('human', str(complete))

    def test_invalid_limits_and_untrusted_paths_fail_closed(self):
        for limit in (False, 0, -1, 1.2):
            with self.assertRaises(ValueError): freshness.build_review_packet({}, snapshot_digest='a' * 64, limit=limit)
        with self.assertRaises(ValueError):
            freshness.build_review_packet({'undigested': ['../../.env']}, snapshot_digest='a' * 64)

    def test_private_source_details_are_withheld(self):
        packet = freshness.build_review_packet({'source_changed': [{'chunk': 'memory-bank/chunks/MEM-0001-one.md',
            'source': '/private/.env', 'state': 'missing'}]}, snapshot_digest='a' * 64)
        self.assertNotIn('/private', str(packet))
        self.assertTrue(packet['queue'][0]['sources'][0]['source_withheld'])

    def test_real_audit_does_not_change_original(self):
        from tests.test_memory_consolidation_pilot import ConsolidationPilotTests
        fixture = ConsolidationPilotTests(); fixture.setUp(); self.addCleanup(fixture.doCleanups)
        fixture.event(1); fixture.event(2)
        from memory_consolidation_pilot import run_pilot
        export = fixture.base / 'export'; run_pilot(fixture.root, variant='consolidate-verify', export=export)
        (export / 'specs/authority.md').write_text('Cobalt changed.\n')
        before = {p.relative_to(export).as_posix(): p.read_bytes() for p in export.rglob('*') if p.is_file()}
        result = freshness.plan_reviews(export)
        self.assertTrue(any('changed' in entry['reasons'] for entry in result['queue']))
        self.assertEqual(before, {p.relative_to(export).as_posix(): p.read_bytes() for p in export.rglob('*') if p.is_file()})


class CheckedCopyReplayTests(unittest.TestCase):
    def setUp(self):
        from tests.test_memory_result_revalidation import RevalidationFixture
        self.fixture = RevalidationFixture(); self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.root = self.fixture.root
        self.chunk = self.fixture.old_chunk.relative_to(self.root).as_posix()

    def image(self):
        return {p.relative_to(self.root).as_posix(): p.read_bytes()
                for p in self.root.rglob('*') if p.is_file()}

    def packet(self):
        return freshness.evidence_plan(self.root, selected_chunks=[self.chunk])

    def decision(self, packet, action='confirm', **changes):
        row = packet['items'][0]
        return {'chunk': row['chunk'], 'row_sha256': row['sha256'], 'action': action,
                'attestation': 'agent', 'reason': 'Checked the complete Cedar current claim and source.',
                'evidence': [{'path': source['path'], 'sha256': source['sha256'],
                              'quote': source['text'].splitlines()[0]} for source in row['sources']], **changes}

    def replay(self, packet, decisions):
        export = self.root.parent / (self.root.name + '-review-export')
        self.addCleanup(lambda: __import__('shutil').rmtree(export, ignore_errors=True))
        return freshness.replay_reviews(self.root, packet, decisions, export=export), export

    def test_fresh_explicit_capture_preserves_historical_source_bytes_after_edit(self):
        before = self.image(); packet = self.packet(); captured = copy.deepcopy(packet)
        self.assertEqual(before, self.image())
        row = packet['items'][0]
        self.assertEqual('ready', row['state'])
        self.assertTrue(row['source_image_matches_record'])
        self.assertEqual(self.fixture.source.read_text(), row['sources'][0]['text'])
        self.assertEqual('not-performed', packet['attestation'])
        self.fixture.change_source()
        self.assertEqual(captured, packet)
        self.assertNotEqual(self.fixture.source.read_text(), row['sources'][0]['text'])
        with self.assertRaisesRegex(ValueError, 'changed|image'):
            self.replay(packet, [self.decision(packet)])

    def test_confirm_and_correct_use_checked_reference_and_preserve_original(self):
        for action in ('confirm', 'correct'):
            with self.subTest(action=action):
                self.fixture.change_source(); packet = self.packet(); before = self.image()
                extra = {'consequence': 'Cedar now reserves eight bounded slots.'} if action == 'correct' else {}
                report, export = self.replay(packet, [self.decision(packet, action, **extra)])
                self.assertEqual(before, self.image())
                self.assertTrue(report['success']); self.assertEqual('applied', report['actions'][0]['state'])
                brain = self.fixture.brain
                record = brain.get_record(export, self.fixture.record_id)
                self.assertEqual(self.fixture.record_id, record['id'])
                self.assertEqual(self.fixture.target()['revision'] + 1, record['revision'])
                old, body = brain._parse_chunk(export / self.chunk)
                self.assertEqual('superseded', old['status'])
                self.assertEqual(brain._parse_chunk(self.fixture.old_chunk)[1], body)
                self.assertEqual([], brain.audit_bank(export)['source_changed'])
                self.assertFalse(json.loads((export/'project-brain/config/runtime.json').read_text())['automatic_consolidation'])
                __import__('shutil').rmtree(export)

    def test_unselected_eligible_backlog_is_not_promoted(self):
        cfg = json.loads(self.fixture.config.read_text()); cfg['automatic_promotion'] = False
        self.fixture.config.write_text(json.dumps(cfg))
        other = {**self.fixture.learning, 'title': 'Separate Cedar boundary',
                 'consequence': 'A separate guard rejects empty Cedar reservations.'}
        result = self.fixture.save(self.fixture.a, 'backlog', other)
        backlog = result['records'][0]['id']
        self.fixture.change_source(); packet = self.packet(); before = self.image()
        report, export = self.replay(packet, [self.decision(packet)])
        self.assertTrue(report['success']); self.assertEqual(before, self.image())
        self.assertFalse(any(backlog in text for text in (p.read_text() for p in (export/'memory-bank/chunks').glob('*.md'))))
        self.assertIn('automatic_promotion', report['copy_overrides'])

    def test_missing_binary_private_and_instruction_like_evidence_is_held(self):
        for value in (b'\x00binary', b'token=sk-proj-' + b'A'*60):
            with self.subTest(value=value[:10]):
                self.fixture.source.write_bytes(value)
                packet = self.packet()
                self.assertEqual('held', packet['items'][0]['state'])
                self.assertNotIn('sk-proj-', json.dumps(packet))
        self.fixture.source.unlink(); packet = self.packet()
        self.assertEqual('held', packet['items'][0]['state'])

    def test_bad_decision_is_held_without_hiding_unprocessed_or_mutating_input(self):
        self.fixture.change_source(); packet = self.packet(); before = self.image()
        bad = self.decision(packet, attestation='person')
        report, _ = self.replay(packet, [bad])
        self.assertFalse(report['success']); self.assertEqual('held', report['actions'][0]['state'])
        self.assertEqual(before, self.image())

    def test_confirm_renews_overdue_unchanged_knowledge_without_rewriting_history(self):
        brain = self.fixture.brain
        metadata, body = brain._parse_chunk(self.root / self.chunk)
        yesterday = (date.today() - timedelta(days=1)).isoformat()
        metadata.update(created=yesterday, last_verified=yesterday, review_after=yesterday)
        (self.root / self.chunk).write_text(brain._render_chunk(metadata, body))
        brain.reindex_bank(self.root)
        packet = self.packet(); before = self.image()
        record_before = self.fixture.target()
        report, export = self.replay(packet, [self.decision(packet)])
        self.assertTrue(report['success'])
        self.assertEqual('applied', report['actions'][0]['state'])
        self.assertIn('renewal', report['actions'][0])
        renewed, renewed_body = brain._parse_chunk(export / self.chunk)
        self.assertEqual(metadata['id'], renewed['id'])
        self.assertEqual(body, renewed_body)
        self.assertEqual(datetime.now(timezone.utc).date().isoformat(), renewed['last_verified'])
        self.assertGreater(renewed['review_after'], date.today().isoformat())
        self.assertEqual(record_before, brain.get_record(export, self.fixture.record_id))
        self.assertEqual(before, self.image())
        self.assertFalse(report['audit_after']['queue'])

    def test_source_change_during_overdue_renewal_is_a_visible_failure(self):
        brain = self.fixture.brain
        metadata, body = brain._parse_chunk(self.root / self.chunk)
        yesterday = (date.today() - timedelta(days=1)).isoformat()
        metadata.update(created=yesterday, last_verified=yesterday, review_after=yesterday)
        (self.root / self.chunk).write_text(brain._render_chunk(metadata, body))
        brain.reindex_bank(self.root)
        packet = self.packet(); before = self.image()
        from memory_pilot_support import copy_snapshot
        temporary = self.root.parent / (self.root.name + '-renewal-race-copy')
        self.addCleanup(lambda: __import__('shutil').rmtree(temporary, ignore_errors=True))
        copy_snapshot(self.root, temporary)
        runtime = freshness.canonical_module('brain_runtime'); original = runtime.reverify_chunk
        def interrupted(*arguments, **options):
            (temporary / 'src/rule.txt').write_text('Cedar source changed during review.\n')
            return original(*arguments, **options)
        with mock.patch.object(runtime, 'reverify_chunk', side_effect=interrupted) as renewal:
            report = freshness._worker(temporary, 'replay',
                {'packet': packet, 'decisions': [self.decision(packet)]})
        self.assertEqual(1, renewal.call_count)
        self.assertFalse(report['success'])
        self.assertEqual('failed', report['actions'][0]['state'])
        self.assertTrue(report['actions'][0]['copy_effects_may_have_applied'])
        self.assertEqual(before, self.image())

    def test_fabricated_quote_missing_decision_and_tampered_packet_fail_closed(self):
        packet = self.packet(); bad = self.decision(packet)
        bad['evidence'][0]['quote'] = 'A quote that never appeared.'
        report, export = self.replay(packet, [bad])
        self.assertFalse(report['success']); __import__('shutil').rmtree(export)
        report, export = self.replay(packet, [])
        self.assertEqual('held', report['actions'][0]['state']); __import__('shutil').rmtree(export)
        packet['items'][0]['sources'][0]['text'] = 'Changed packet source text.'
        with self.assertRaises(ValueError): self.replay(packet, [])

    def test_explicit_retire_requires_selected_fresh_successor(self):
        packet = self.packet(); before = self.image()
        report, _ = self.replay(packet, [self.decision(packet, 'retire',
            successor='MEM-9999', valid_to=self.fixture.brain._parse_chunk(self.fixture.old_chunk)[0]['valid_from'])])
        self.assertFalse(report['success']); self.assertEqual('held', report['actions'][0]['state'])
        self.assertEqual(before, self.image())

    def successor(self):
        brain = self.fixture.brain
        metadata, body = brain._parse_chunk(self.fixture.old_chunk)
        metadata = {**metadata, 'id':'MEM-9999', 'supersedes':[], 'superseded_by':None}
        successor = self.root/'memory-bank/chunks/MEM-9999-newer-cedar.md'
        successor.write_text(brain._render_chunk(metadata,body))
        brain.reindex_bank(self.root)
        return successor.relative_to(self.root).as_posix()

    def test_retire_preserves_both_bodies_and_reciprocal_successor_link(self):
        successor = self.successor()
        packet = freshness.evidence_plan(self.root,selected_chunks=[self.chunk,successor])
        before = self.image()
        defer = {'chunk':successor,'row_sha256':packet['items'][1]['sha256'],
                 'action':'defer','reason':'Fresh successor is retained.'}
        report, export = self.replay(packet,[self.decision(packet,'retire',
            successor='MEM-9999',valid_to=self.fixture.brain._parse_chunk(self.fixture.old_chunk)[0]['valid_from']),defer])
        self.assertTrue(report['success']);self.assertEqual(before,self.image())
        brain=self.fixture.brain
        old,body=brain._parse_chunk(export/self.chunk)
        new,new_body=brain._parse_chunk(export/successor)
        self.assertEqual('superseded',old['status']);self.assertEqual('MEM-9999',old['superseded_by'])
        self.assertIn(old['id'],new['supersedes'])
        self.assertEqual(brain._parse_chunk(self.fixture.old_chunk)[1],body)
        self.assertEqual(brain._parse_chunk(self.root/successor)[1],new_body)

    def test_source_drift_during_retire_reports_failed_partial_copy_effects(self):
        from memory_pilot_support import copy_snapshot
        successor = self.successor()
        packet = freshness.evidence_plan(self.root, selected_chunks=[self.chunk, successor])
        decisions = [self.decision(packet, 'retire', successor='MEM-9999',
                                  valid_to=date.today().isoformat()),
                     {'chunk': successor, 'row_sha256': packet['items'][1]['sha256'],
                      'action': 'defer', 'reason': 'Retain selected replacement.'}]
        before = self.image()
        temporary = self.root.parent/(self.root.name+'-retire-race-copy')
        self.addCleanup(lambda: __import__('shutil').rmtree(temporary, ignore_errors=True))
        copy_snapshot(self.root, temporary)
        brain = freshness.canonical_module('brain_runtime'); original = brain.retire_chunk
        def drift(*args, **kwargs):
            (temporary/'src/rule.txt').write_text('Source changed during retirement.\n')
            return original(*args, **kwargs)
        with mock.patch.object(brain, 'retire_chunk', side_effect=drift):
            report = freshness._worker(temporary, 'replay', {'packet': packet, 'decisions': decisions})
        self.assertFalse(report['success'])
        self.assertEqual('failed', report['actions'][0]['state'])
        self.assertTrue(report['actions'][0]['copy_effects_may_have_applied'])
        self.assertEqual(before, self.image())

    def test_successor_with_uncaptured_extra_chunk_citation_is_held(self):
        successor = self.successor(); brain = self.fixture.brain
        extra = self.root/'src/additional.txt'; extra.write_text('Additional public evidence.\n')
        metadata, body = brain._parse_chunk(self.root/successor)
        metadata['sources'].append('src/additional.txt')
        metadata['source_digests'].append(brain.fingerprint(self.root, 'src/additional.txt'))
        (self.root/successor).write_text(brain._render_chunk(metadata, body))
        brain.reindex_bank(self.root)
        packet = freshness.evidence_plan(self.root, selected_chunks=[self.chunk, successor])
        self.assertEqual('held', packet['items'][1]['state'])
        report, export = self.replay(packet, [self.decision(packet, 'retire',
            successor='MEM-9999', valid_to=date.today().isoformat())])
        self.assertFalse(report['success'])
        self.assertEqual('held', report['actions'][0]['state'])
        self.assertFalse(report['actions'][0].get('copy_effects_may_have_applied', False))
        self.assertEqual('active', brain._parse_chunk(export/self.chunk)[0]['status'])

    def test_bounded_unicode_packet_round_trips_with_one_utf8_size_policy(self):
        text = 'Cedar uses four reservation slots.\n' + 'Я'*90000
        self.fixture.source.write_text(text, encoding='utf-8')
        (self.root/'src/other.txt').write_text(text, encoding='utf-8')
        saved = self.fixture.save(self.fixture.b, 'unicode-sources', {
            **self.fixture.learning, 'title': 'Cedar Unicode source check',
            'sources': ['src/rule.txt', 'src/other.txt']})
        memory_id = saved['promotion']['promoted'][0]['memory_id']
        chunk = next((self.root/'memory-bank/chunks').glob(memory_id+'-*.md'))
        relative = chunk.relative_to(self.root).as_posix()
        packet = freshness.evidence_plan(self.root, selected_chunks=[relative])
        self.assertEqual('ready', packet['items'][0]['state'])
        self.assertLess(len(json.dumps(packet, ensure_ascii=False).encode('utf-8')), freshness.PACKET_BYTES)
        self.assertGreater(len(json.dumps(packet).encode('utf-8')), freshness.PACKET_BYTES)
        decision = {'chunk': relative, 'row_sha256': packet['items'][0]['sha256'],
                    'action': 'defer', 'reason': 'Retain current checked claim.'}
        report, _ = self.replay(packet, [decision])
        self.assertTrue(report['success'])
        self.assertEqual('deferred', report['actions'][0]['state'])

    def test_future_successor_cannot_retire_current_knowledge(self):
        successor = self.successor(); brain = self.fixture.brain
        metadata, body = brain._parse_chunk(self.root / successor)
        metadata['valid_from'] = (date.today() + timedelta(days=1)).isoformat()
        (self.root / successor).write_text(brain._render_chunk(metadata, body))
        brain.reindex_bank(self.root)
        packet = freshness.evidence_plan(self.root, selected_chunks=[self.chunk, successor])
        before = self.image()
        defer = {'chunk': successor, 'row_sha256': packet['items'][1]['sha256'],
                 'action': 'defer', 'reason': 'Wait for replacement applicability.'}
        report, export = self.replay(packet, [self.decision(packet, 'retire',
            successor='MEM-9999', valid_to=date.today().isoformat()), defer])
        self.assertFalse(report['success'])
        self.assertEqual('held', report['actions'][0]['state'])
        self.assertFalse(report['actions'][0].get('copy_effects_may_have_applied', False))
        self.assertEqual('active', brain._parse_chunk(export / self.chunk)[0]['status'])
        self.assertEqual(before, self.image())

    def test_successor_start_expiry_and_review_boundaries_include_today(self):
        successor = self.successor(); brain = self.fixture.brain
        metadata, body = brain._parse_chunk(self.root / successor)
        for key in ('valid_from', 'valid_to', 'review_after'):
            metadata[key] = date.today().isoformat()
        (self.root / successor).write_text(brain._render_chunk(metadata, body))
        brain.reindex_bank(self.root)
        packet = freshness.evidence_plan(self.root, selected_chunks=[self.chunk, successor])
        defer = {'chunk': successor, 'row_sha256': packet['items'][1]['sha256'],
                 'action': 'defer', 'reason': 'Current replacement is retained.'}
        report, _ = self.replay(packet, [self.decision(packet, 'retire',
            successor='MEM-9999', valid_to=date.today().isoformat()), defer])
        self.assertTrue(report['success'])

    def test_partial_success_is_visible_and_not_reported_as_all_green(self):
        successor=self.successor();packet=freshness.evidence_plan(self.root,
            selected_chunks=[self.chunk,successor]);before=self.image()
        report,export=self.replay(packet,[self.decision(packet,'retire',
            successor='MEM-9999',valid_to=self.fixture.brain._parse_chunk(self.fixture.old_chunk)[0]['valid_from'])])
        self.assertEqual(['applied','held'],[row['state'] for row in report['actions']])
        self.assertFalse(report['success']);self.assertEqual(before,self.image())
        self.assertEqual('superseded',self.fixture.brain._parse_chunk(export/self.chunk)[0]['status'])

    def test_project_privacy_exclusion_and_source_budget_fail_closed(self):
        flags=json.loads(self.fixture.config.read_text());flags['allowed_privacy']=['public']
        self.fixture.config.write_text(json.dumps(flags))
        self.assertEqual('held',self.packet()['items'][0]['state'])
        flags['allowed_privacy']=['public','team'];self.fixture.config.write_text(json.dumps(flags))
        self.fixture.source.write_text('x'*(freshness.SOURCE_BYTES+1))
        self.assertEqual('held',self.packet()['items'][0]['state'])

    def test_post_effect_failure_is_reported_as_failed_with_possible_copy_effects(self):
        self.fixture.change_source();packet=self.packet();before=self.image()
        from memory_pilot_support import copy_snapshot
        temporary=self.root.parent/(self.root.name+'-failed-copy')
        self.addCleanup(lambda: __import__('shutil').rmtree(temporary,ignore_errors=True))
        copy_snapshot(self.root,temporary)
        results=freshness._runtime_results();original=results.record_result
        def interrupted(*arguments,**options):
            original(*arguments,**options)
            raise OSError('Failure after a checked record effect')
        with mock.patch.object(results,'record_result',side_effect=interrupted):
            report=freshness._worker(temporary,'replay',{'packet':packet,'decisions':[self.decision(packet)]})
        self.assertFalse(report['success']);self.assertEqual('failed',report['actions'][0]['state'])
        self.assertTrue(report['actions'][0]['copy_effects_may_have_applied'])
        self.assertEqual(before,self.image())

    def test_cli_capture_and_copy_replay_use_external_private_artifacts(self):
        self.fixture.change_source();before=self.image()
        packet_path=self.root.parent/(self.root.name+'-packet.json')
        decisions_path=self.root.parent/(self.root.name+'-decisions.json')
        report_path=self.root.parent/(self.root.name+'-report.json')
        export=self.root.parent/(self.root.name+'-cli-export')
        for path in (packet_path,decisions_path,report_path):self.addCleanup(path.unlink,missing_ok=True)
        self.addCleanup(lambda:__import__('shutil').rmtree(export,ignore_errors=True))
        script=Path(freshness.__file__)
        command=[sys.executable,'-B',str(script),'--root',str(self.root)]
        captured=subprocess.run([*command,'--evidence','--chunk',self.chunk,'--out',str(packet_path)],
            capture_output=True,text=True)
        self.assertEqual(0,captured.returncode,captured.stderr)
        packet=json.loads(packet_path.read_text());decisions_path.write_text(json.dumps([self.decision(packet)]))
        replayed=subprocess.run([*command,'--packet',str(packet_path),'--decisions',str(decisions_path),
            '--export',str(export),'--out',str(report_path)],capture_output=True,text=True)
        self.assertEqual(0,replayed.returncode,replayed.stderr)
        self.assertTrue(json.loads(report_path.read_text())['success'])
        self.assertEqual(before,self.image())


class ArchivedOriginMaintenanceTests(unittest.TestCase):
    image = CheckedCopyReplayTests.image
    packet = CheckedCopyReplayTests.packet
    decision = CheckedCopyReplayTests.decision
    replay = CheckedCopyReplayTests.replay

    def archived(self, kind='finding', *, attestation='agent', decision_terminal=False):
        from tests.test_memory_result_revalidation import RevalidationFixture
        fixture = RevalidationFixture(); fixture.setUp()
        self.addCleanup(fixture.doCleanups)
        self.fixture, self.root = fixture, fixture.root
        return self.publish(kind, attestation=attestation, decision_terminal=decision_terminal)

    def publish(self, kind='finding', *, attestation='agent', decision_terminal=False,
                alias='archive-expiry', title='Quartz message expiry',
                claim='Quartz messages expire after ten minutes.', source_name='src/expiry.txt'):
        fixture = self.fixture
        brain = fixture.brain
        source = self.root/source_name
        source.write_text(claim+'\n')
        record = brain.create_record(self.root, kind, alias+'-'+kind,
            title, [], [source_name], owner='local',
            privacy='team', goal='Check the Quartz expiry policy.')
        record = brain.update_record(self.root, record['id'], expected_revision=record['revision'],
            progress=claim, next_steps=[], files=[], sources=[],
            actor='local', transition_to='resolved' if kind=='finding' else 'accepted',
            authority='verified', attestation=attestation)
        promotion = brain.auto_promote(self.root, owner='local', only={record['id']}, limit=1)
        self.assertEqual(1, len(promotion['promoted']))
        memory_id = promotion['promoted'][0]['memory_id']
        chunk = next((self.root/'memory-bank/chunks').glob(memory_id+'-*.md'))
        if decision_terminal:
            record = brain.update_record(self.root, record['id'], expected_revision=record['revision'],
                progress=None, next_steps=[], files=[], sources=[], actor='local', transition_to='superseded')
        dynamic = brain.dynamic_path(self.root, record)
        original = dynamic.read_bytes()
        should_archive = kind=='finding' or decision_terminal
        self.assertEqual(int(should_archive), brain.compact(self.root, only_external_id=record['external_id'])['moved'])
        archived = self.root/'project-brain/archive'/kind/(record['id']+'.md')
        origin = archived if should_archive else dynamic
        self.assertEqual(original, origin.read_bytes())
        self.assertEqual(not should_archive, dynamic.exists())
        self.chunk = chunk.relative_to(self.root).as_posix()
        metadata, _ = brain._parse_chunk(chunk)
        self.assertIn(origin.relative_to(self.root).as_posix(), metadata['sources'])
        if should_archive:
            self.assertNotIn(dynamic.relative_to(self.root).as_posix(), metadata['sources'])
        return brain, source, origin, record, memory_id

    def knowledge(self, root):
        prefixes = ('src/', 'project-brain/dynamic/', 'project-brain/archive/',
                    'project-brain/control/', 'memory-bank/chunks/')
        return {p.relative_to(root).as_posix():p.read_bytes() for p in root.rglob('*')
                if p.is_file() and p.relative_to(root).as_posix().startswith(prefixes)}

    def test_canonical_archived_agent_origins_capture_read_only_with_capabilities(self):
        brain, source, origin, record, _ = self.archived()
        before = self.image(); packet = self.packet(); row = packet['items'][0]
        self.assertEqual('ready', row['state'])
        self.assertEqual('archive', row['origin_placement'])
        self.assertEqual('unchanged-evidence-only', row['maintenance_capabilities']['confirm'])
        self.assertFalse(row['maintenance_capabilities']['origin_record_mutable'])
        self.assertEqual(origin.relative_to(self.root).as_posix(), row['record_path'])
        self.assertEqual(record['revision'], row['expected_revision'])
        self.assertEqual(source.read_text(), row['sources'][0]['text'])
        self.assertEqual('agent', brain.record_attestation(record))
        self.assertEqual(before, self.image())

    def test_fresh_archived_confirm_is_noop_without_new_task_or_record(self):
        for action in ('confirm', 'correct'):
            with self.subTest(action=action):
                _, _, origin, _, _ = self.archived()
                packet = self.packet(); before = self.image()
                extra = {'consequence':packet['items'][0]['consequence']} if action=='correct' else {}
                report, export = self.replay(packet, [self.decision(packet, action, **extra)])
                self.assertTrue(report['success']); self.assertTrue(report['actions'][0]['unchanged'])
                self.assertEqual(origin.read_bytes(), (export/origin.relative_to(self.root)).read_bytes())
                self.assertEqual(self.knowledge(self.root), self.knowledge(export))
                self.assertEqual(before, self.image())

    def test_decisions_keep_their_canonical_lifecycle_and_dynamic_packet(self):
        brain, _, origin, record, _ = self.archived('decision')
        self.assertIn('/dynamic/decisions/', origin.as_posix())
        packet = self.packet(); row = packet['items'][0]
        self.assertEqual('ready', row['state'])
        self.assertNotIn('origin_placement', row)
        self.assertNotIn('maintenance_capabilities', row)
        before = self.image()
        report, export = self.replay(packet, [self.decision(packet)])
        self.assertTrue(report['success'])
        self.assertEqual(origin.read_bytes(), (export/origin.relative_to(self.root)).read_bytes())
        self.assertEqual('accepted', brain.get_record(export, record['id'])['status'])
        self.assertEqual(before, self.image())
        brain, _, origin, record, _ = self.archived('decision', decision_terminal=True)
        self.assertIn('/archive/decision/', origin.as_posix())
        brain.validate_record(record, archived=True)
        self.assertEqual('superseded', record['status'])
        self.assertEqual('held', self.packet()['items'][0]['state'])

    def overdue(self):
        brain = self.fixture.brain
        metadata, body = brain._parse_chunk(self.root/self.chunk)
        yesterday = (date.today()-timedelta(days=1)).isoformat()
        metadata.update(created=yesterday, last_verified=yesterday, review_after=yesterday)
        (self.root/self.chunk).write_text(brain._render_chunk(metadata, body))
        brain.reindex_bank(self.root)
        return body

    def test_overdue_archive_confirmation_renews_only_bank_review_metadata(self):
        for action in ('confirm', 'correct'):
            with self.subTest(action=action):
                brain, _, origin, record, _ = self.archived()
                body = self.overdue(); packet = self.packet(); before = self.image()
                extra = {'consequence':packet['items'][0]['consequence']} if action=='correct' else {}
                report, export = self.replay(packet, [self.decision(packet, action, **extra)])
                self.assertTrue(report['success']); self.assertIn('renewal', report['actions'][0])
                renewed, renewed_body = brain._parse_chunk(export/self.chunk)
                self.assertEqual(body, renewed_body)
                self.assertEqual(date.today().isoformat(), renewed['last_verified'])
                self.assertGreater(renewed['review_after'], date.today().isoformat())
                self.assertEqual(origin.read_bytes(), (export/origin.relative_to(self.root)).read_bytes())
                self.assertEqual(record, brain.find_record(export, record['id'], include_archive=True)[1])
                self.assertEqual(set(self.knowledge(self.root)), set(self.knowledge(export)))
                self.assertEqual(before, self.image())

    def test_archive_revision_requests_hold_before_any_knowledge_effect(self):
        for action, changed_source in (('confirm', True), ('correct', True), ('correct', False)):
            with self.subTest(action=action, changed_source=changed_source):
                _, source, _, _, _ = self.archived()
                if changed_source: source.write_text(source.read_text()+'A new expiry guard was added.\n')
                packet = self.packet(); self.assertEqual('ready', packet['items'][0]['state'])
                before = self.image()
                extra = {'consequence':'Quartz messages expire after twenty minutes.'} if action=='correct' else {}
                report, export = self.replay(packet, [self.decision(packet, action, **extra)])
                self.assertFalse(report['success'])
                self.assertEqual('held', report['actions'][0]['state'])
                self.assertEqual('archived-origin-revision-required', report['actions'][0]['reason'])
                self.assertFalse(report['actions'][0].get('copy_effects_may_have_applied', False))
                self.assertEqual(self.knowledge(self.root), self.knowledge(export))
                self.assertEqual(before, self.image())

    def test_expired_archive_claim_cannot_be_renewed_into_current_knowledge(self):
        brain, _, origin, _, _ = self.archived(); self.overdue()
        metadata, body = brain._parse_chunk(self.root/self.chunk)
        metadata.update(valid_from=(date.today()-timedelta(days=2)).isoformat(),
                        valid_to=(date.today()-timedelta(days=1)).isoformat())
        (self.root/self.chunk).write_text(brain._render_chunk(metadata, body))
        brain.reindex_bank(self.root)
        packet = self.packet(); before = self.image()
        report, export = self.replay(packet, [self.decision(packet)])
        self.assertFalse(report['success']); self.assertEqual('failed', report['actions'][0]['state'])
        self.assertTrue(report['actions'][0]['copy_effects_may_have_applied'])
        self.assertEqual(self.knowledge(self.root), self.knowledge(export))
        self.assertEqual(origin.read_bytes(), (export/origin.relative_to(self.root)).read_bytes())
        self.assertEqual(before, self.image())

    def test_retirement_preserves_both_archived_origins_and_chunk_bodies(self):
        brain, _, old_origin, _, memory_id = self.archived()
        old_chunk = self.chunk
        _, _, new_origin, _, successor_id = self.publish(alias='archive-lease',
            title='Quartz lease replacement', claim='Signed leases preserve Quartz messages beyond expiry.',
            source_name='src/lease.txt')
        successor_chunk = self.chunk; self.chunk = old_chunk
        packet = freshness.evidence_plan(self.root, selected_chunks=[old_chunk, successor_chunk])
        self.assertTrue(all(row['state']=='ready' and row['origin_placement']=='archive'
                            for row in packet['items']))
        before = self.image()
        defer = {'chunk':successor_chunk, 'row_sha256':packet['items'][1]['sha256'],
                 'action':'defer', 'reason':'Retain the current replacement.'}
        report, export = self.replay(packet, [self.decision(packet, 'retire',
            successor=successor_id, valid_to=date.today().isoformat()), defer])
        self.assertTrue(report['success'])
        old, old_body = brain._parse_chunk(export/old_chunk)
        new, new_body = brain._parse_chunk(export/successor_chunk)
        self.assertEqual(('superseded', successor_id), (old['status'], old['superseded_by']))
        self.assertIn(memory_id, new['supersedes'])
        self.assertEqual(brain._parse_chunk(self.root/old_chunk)[1], old_body)
        self.assertEqual(brain._parse_chunk(self.root/successor_chunk)[1], new_body)
        for origin in (old_origin, new_origin):
            self.assertEqual(origin.read_bytes(), (export/origin.relative_to(self.root)).read_bytes())
        self.assertEqual(before, self.image())

    def test_unknown_and_person_archive_attestation_remain_held(self):
        for attestation in (None, 'person'):
            with self.subTest(attestation=attestation):
                brain, _, _, record, _ = self.archived(attestation=attestation)
                self.assertEqual(attestation or '', brain.record_attestation(record))
                before = self.image(); packet = self.packet()
                self.assertEqual('held', packet['items'][0]['state'])
                self.assertEqual([], packet['items'][0]['sources'])
                self.assertEqual(before, self.image())

    def rewrite_origin(self, origin, change):
        brain = self.fixture.brain
        metadata, body = brain.parse_markdown_record(origin)
        change(metadata)
        origin.write_text(brain.render_markdown_record(metadata, body))
        chunk, chunk_body = brain._parse_chunk(self.root/self.chunk)
        for pin in chunk['source_digests']:
            if pin['path']==origin.relative_to(self.root).as_posix():
                pin['sha256'] = brain.fingerprint(self.root, pin['path'])['sha256']
        (self.root/self.chunk).write_text(brain._render_chunk(chunk, chunk_body))

    def test_archive_identity_and_terminal_validation_fail_closed(self):
        for corruption in ('uuid', 'duplicate', 'nonterminal', 'plural-path', 'wrong-kind-path'):
            with self.subTest(corruption=corruption):
                brain, _, origin, record, _ = self.archived()
                if corruption=='uuid':
                    self.rewrite_origin(origin, lambda md:md.update(id='48d5c1da-8f09-4ac1-bf6b-2e6850d34527'))
                elif corruption=='duplicate':
                    dynamic = brain.dynamic_path(self.root, record)
                    dynamic.parent.mkdir(parents=True, exist_ok=True); dynamic.write_bytes(origin.read_bytes())
                elif corruption=='nonterminal':
                    def reopen(md):
                        md['status']='open'
                        md['transitions']=[row for row in md['transitions'] if row['to']!='resolved']
                    self.rewrite_origin(origin, reopen)
                else:
                    metadata, body = brain._parse_chunk(self.root/self.chunk)
                    original = origin.relative_to(self.root).as_posix()
                    replacement = original.replace('/finding/', '/findings/' if corruption=='plural-path' else '/decision/')
                    metadata['sources']=[replacement if name==original else name for name in metadata['sources']]
                    for pin in metadata['source_digests']:
                        if pin['path']==original: pin['path']=replacement
                    (self.root/self.chunk).write_text(brain._render_chunk(metadata, body))
                before = self.image(); packet = self.packet()
                self.assertEqual('held', packet['items'][0]['state'])
                self.assertEqual(before, self.image())

    def test_archive_admission_preserves_owner_privacy_and_dependency_guards(self):
        for exclusion in ('owner', 'privacy', 'authority', 'extra-source', 'missing-source', 'linked-origin'):
            with self.subTest(exclusion=exclusion):
                brain, source, origin, _, _ = self.archived()
                if exclusion in ('owner', 'privacy', 'authority'):
                    flags = json.loads(self.fixture.config.read_text())
                    flags.update({'owners':['other']} if exclusion=='owner' else
                                 {'allowed_privacy':['public']} if exclusion=='privacy' else
                                 {'allowed_authority':['observed']})
                    self.fixture.config.write_text(json.dumps(flags))
                elif exclusion=='extra-source':
                    extra = self.root/'src/dependency.txt'; extra.write_text('Quartz needs a lease.\n')
                    metadata, body = brain._parse_chunk(self.root/self.chunk)
                    metadata['sources'].append('src/dependency.txt')
                    metadata['source_digests'].append(brain.fingerprint(self.root, 'src/dependency.txt'))
                    (self.root/self.chunk).write_text(brain._render_chunk(metadata, body))
                elif exclusion=='missing-source': source.unlink()
                else:
                    outside = self.root/'origin-copy.md'; outside.write_bytes(origin.read_bytes())
                    origin.unlink(); origin.symlink_to(outside)
                before = self.image()
                self.assertEqual('held', self.packet()['items'][0]['state'])
                self.assertEqual(before, self.image())
        self.archived()
        packet = freshness.evidence_plan(self.root, selected_chunks=[self.chunk], owner='other')
        self.assertEqual('held', packet['items'][0]['state'])

    def test_archive_capture_replay_refuses_frontier_or_scope_drift(self):
        for frontier in ('source', 'record', 'chunk', 'configuration'):
            with self.subTest(frontier=frontier):
                _, source, origin, _, _ = self.archived(); packet = self.packet()
                target = {'source':source, 'record':origin, 'chunk':self.root/self.chunk,
                          'configuration':self.fixture.config}[frontier]
                target.write_bytes(target.read_bytes()+b'\n')
                before = self.image()
                with self.assertRaises(ValueError): self.replay(packet, [self.decision(packet)])
                self.assertEqual(before, self.image())

    def test_source_drift_during_archive_renewal_is_a_visible_partial_failure(self):
        _, source, origin, _, _ = self.archived(); self.overdue()
        packet = self.packet(); before = self.image()
        from memory_pilot_support import copy_snapshot
        temporary = self.root.parent/(self.root.name+'-archive-renewal-race')
        self.addCleanup(lambda:__import__('shutil').rmtree(temporary, ignore_errors=True))
        copy_snapshot(self.root, temporary)
        runtime = freshness.canonical_module('brain_runtime'); original = runtime.reverify_chunk
        def interrupted(*arguments, **options):
            (temporary/source.relative_to(self.root)).write_text('Quartz source changed during review.\n')
            return original(*arguments, **options)
        with mock.patch.object(runtime, 'reverify_chunk', side_effect=interrupted):
            report = freshness._worker(temporary, 'replay',
                {'packet':packet, 'decisions':[self.decision(packet)]})
        self.assertFalse(report['success']); self.assertEqual('failed', report['actions'][0]['state'])
        self.assertTrue(report['actions'][0]['copy_effects_may_have_applied'])
        self.assertEqual(origin.read_bytes(), (temporary/origin.relative_to(self.root)).read_bytes())
        self.assertEqual(before, self.image())

    def renewal_origin(self, archived):
        if archived:
            self.archived()
        else:
            from tests.test_memory_result_revalidation import RevalidationFixture
            fixture = RevalidationFixture(); fixture.setUp()
            self.addCleanup(fixture.doCleanups)
            self.fixture, self.root = fixture, fixture.root
            self.chunk = fixture.old_chunk.relative_to(self.root).as_posix()
        self.overdue()

    def renewal_with_foreign_change(self, change):
        packet = self.packet(); before = self.image()
        from memory_pilot_support import copy_snapshot
        temporary = self.root.parent/(self.root.name+'-foreign-renewal')
        self.addCleanup(lambda:__import__('shutil').rmtree(temporary, ignore_errors=True))
        copy_snapshot(self.root, temporary)
        runtime = freshness.canonical_module('brain_runtime'); original = runtime.reverify_chunk
        def interrupted(*arguments, **options):
            metadata, body = runtime._parse_chunk(temporary/self.chunk)
            metadata, body = change(metadata, body)
            (temporary/self.chunk).write_text(runtime._render_chunk(metadata, body))
            return original(*arguments, **options)
        with mock.patch.object(runtime, 'reverify_chunk', side_effect=interrupted):
            report = freshness._worker(temporary, 'replay',
                {'packet':packet, 'decisions':[self.decision(packet)]})
        self.assertFalse(report['success'])
        self.assertEqual('failed', report['actions'][0]['state'])
        self.assertTrue(report['actions'][0]['copy_effects_may_have_applied'])
        self.assertEqual(before, self.image())
        return runtime._parse_chunk(temporary/self.chunk)

    def test_renewal_rejects_exact_body_drift_for_dynamic_and_archived_origins(self):
        for archived in (False, True):
            for placement in ('append', 'leading-newline'):
                with self.subTest(archived=archived, placement=placement):
                    self.renewal_origin(archived)
                    original_body = self.fixture.brain._parse_chunk(self.root/self.chunk)[1]
                    foreign_body = (original_body+'\nQuartz messages expire after twenty minutes.\n'
                                    if placement=='append' else '\n'+original_body)
                    _, actual_body = self.renewal_with_foreign_change(
                        lambda metadata, body:(metadata, foreign_body))
                    self.assertEqual(foreign_body, actual_body)

    def test_renewal_rejects_unexpected_metadata_for_dynamic_and_archived_origins(self):
        for archived in (False, True):
            with self.subTest(archived=archived):
                self.renewal_origin(archived)
                def foreign_scope(metadata, body):
                    metadata['scope']=['foreign-environment']
                    return metadata, body
                metadata, _ = self.renewal_with_foreign_change(foreign_scope)
                self.assertEqual(['foreign-environment'], metadata['scope'])


if __name__ == '__main__': unittest.main()
