"""Fresh Snipe operator-continuation and direct origin-mutation controls."""
from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_generator_continuation_public.json').read_text(encoding='utf-8'))


class ContinuationBoundaryTests(unittest.TestCase):
    def setUp(self):
        directory = Path(os.environ.get('SNIPE_VALIDATOR', str(ROOT / 'Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts')))
        old_path = list(sys.path)
        names = [entry.stem for entry in directory.glob('*.py')]
        previous = {name: sys.modules.pop(name) for name in names if name in sys.modules}
        spec = importlib.util.spec_from_file_location('snipe_validator', directory / 'validate_generated.py')
        self.validator = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = self.validator

        def restore():
            sys.path[:] = old_path
            sys.modules.pop(spec.name, None)
            for name in names:
                sys.modules.pop(name, None)
            sys.modules.update(previous)

        self.addCleanup(restore)
        spec.loader.exec_module(self.validator)

    def check(self, role, item, source, expected, variant):
        actual = self.validator.memory_hook_identity_contract(source, item['host'], item['name'])
        destination = os.environ.get('SNIPE_OBSERVATIONS')
        if destination:
            with Path(destination).open('a', encoding='utf-8') as stream:
                stream.write(json.dumps({'method': self._testMethodName, 'role': role, 'variant': variant, 'actual': actual, 'expected': expected}) + '\n')
        self.assertIs(actual, expected)

    def test_fresh_supported_roles(self):
        for role, item in PUBLIC['roles'].items():
            with self.subTest(role=role):
                self.check(role, item, item['positive'], PUBLIC['positive'], 'canonical')

    def test_direct_prefix_and_default_assignment_rejected(self):
        for role, item in PUBLIC['roles'].items():
            for index, source in enumerate(item['mutations']):
                with self.subTest(role=role, variant=index):
                    self.check(role, item, source, PUBLIC['rejected'], index)

    def test_continued_redirection_cannot_turn_data_into_code(self):
        for role, item in PUBLIC['roles'].items():
            for index, source in enumerate(item['split_data']):
                with self.subTest(role=role, variant=index):
                    self.check(role, item, source, PUBLIC['rejected'], index)


if __name__ == '__main__':
    unittest.main()
