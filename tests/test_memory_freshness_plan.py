"""Read-only review queue, not automatic attestation or a second mutator."""
from pathlib import Path
import sys
import unittest

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


if __name__ == '__main__': unittest.main()
