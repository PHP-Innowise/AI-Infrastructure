"""SDD artifact paths must work with the installed accelerator policy."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import sdd
from harness.sessions import SessionError


class SddPathsTests(unittest.TestCase):
    def test_new_phase_documents_pass_real_edition_hooks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = {'phase': 'specify', 'feature': 'login'}
            files = sdd.artifacts(root, settings)
            for item in files:
                for edition in ('PHP Core', 'Laravel', 'Symfony'):
                    hook = ROOT / edition / '.claude/hooks/file-naming-validator.sh'
                    result = subprocess.run(['bash', str(hook)], input=json.dumps({
                        'tool_name': 'Write', 'tool_input': {'file_path': item['path']}}),
                        text=True, capture_output=True, cwd=ROOT / edition)
                    self.assertEqual(result.returncode, 0, result.stderr)
            for phase, name in [('specify', 'spec'), ('plan', 'plan'), ('tasks', 'tasks'),
                                ('implement', 'progress'), ('review', None)]:
                settings['phase'] = phase
                sdd.check(root, settings)
                self.assertEqual('Register feature documents' in sdd.instructions(settings, root), phase != 'review')
                if name:
                    path = root / f'specs/login/sdd-{name}.md'
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(f'# {name}\nVerified fixture.\n')
            self.assertIn('sdd-spec.md', sdd.instructions(settings, root))

    def test_existing_legacy_documents_remain_readable_and_block_clarifications(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            folder = root / 'specs/login'
            folder.mkdir(parents=True)
            (folder / 'spec.md').write_text('# Legacy specification\n')
            settings = {'phase': 'plan', 'feature': 'login'}
            sdd.check(root, settings)
            self.assertEqual(sdd.artifacts(root, settings)[1]['path'], 'specs/login/spec.md')
            self.assertIn('spec.md', sdd.instructions(settings, root))
            (folder / 'spec.md').write_text('[NEEDS CLARIFICATION: which account?]')
            with self.assertRaisesRegex(SessionError, 'CLARIFICATION'):
                sdd.check(root, settings)


if __name__ == '__main__':
    unittest.main()
