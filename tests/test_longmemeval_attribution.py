"""Accompanying license/provenance checks; no benchmark imports or gold reads."""
import hashlib
from pathlib import Path
import unittest

FIXTURES = Path(__file__).parent / 'fixtures'
PINS = {
    'longmemeval_memory_api_2cc8c540.py': '607ca21320067df2abc3583b80debeb819781e90955776bdea4313a8b93a0586',
    'longmemeval_reader_calls_2cc8c540.py': '06bb4d73591af5ea8cbaa11ad1cd94a51e49a25f900da8ff4de9433b3227f009',
    'longmemeval_reader_helpers_2cc8c540.py': '2d2ebbcc6f7efed44f98f8194ddcb94cbc26df08a65d115a838a2cf9567e914d',
    'longmemeval_judge_calls_2cc8c540.py': '42c17caa741deea9d7cc01d35aea2655fe9b2135cf97bf17cca9fc888c4b7786',
}


class AttributionTests(unittest.TestCase):
    def test_exact_upstream_license_accompanies_every_unchanged_source_fixture(self):
        raw = (FIXTURES / 'LongMemEval-V2-LICENSE').read_bytes()
        self.assertEqual(11357, len(raw))
        self.assertEqual('c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4', hashlib.sha256(raw).hexdigest())
        notice = (FIXTURES / 'LONGMEMEVAL-V2-NOTICES.md').read_text(encoding='utf-8')
        self.assertIn('Apache License 2.0', notice)
        self.assertIn('2cc8c540bdb87fe6761629b585e727e1c4704520', notice)
        for name, expected in PINS.items():
            with self.subTest(fixture=name):
                self.assertEqual(expected, hashlib.sha256((FIXTURES / name).read_bytes()).hexdigest())
                self.assertIn(name, notice)
                self.assertIn(expected, notice)
        self.assertIn('Include both companion files', notice)


if __name__ == '__main__':
    unittest.main()
