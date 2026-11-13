"""Real canonical consolidation/promotion on physical copies, never the input."""
import json
import os
from pathlib import Path
import sys
import subprocess
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import memory_consolidation_pilot as pilot
from memory_pilot_support import canonical_module, snapshot_digest


class ConsolidationPilotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name); self.root = self.base / 'input'; self.root.mkdir()
        (self.root / 'specs').mkdir(); (self.root / 'specs/authority.md').write_text('Cobalt is canonical.\n')
        (self.root / 'memory-bank/chunks').mkdir(parents=True)
        (self.root / 'memory-bank/README.md').write_text('# Memory Bank\n')
        (self.root / 'project-brain/config').mkdir(parents=True)
        (self.root / 'project-brain/config/runtime.json').write_text(json.dumps({
            'mode': 'governed', 'owners': ['*'], 'automatic_promotion': True,
            'automatic_compaction': True, 'compaction_threshold': 1,
            'automatic_consolidation': False, 'automatic_consolidation_verification': False}))
        self.brain = canonical_module('brain_runtime')

    def event(self, number, verified=True, **kwargs):
        return self.brain.create_record(self.root, 'event', f'EV-{number}',
            f'Completed SHOP-741: phase {number}', [], ['specs/authority.md'],
            owner=kwargs.pop('owner', 'local'), goal=f'Outcome {number}; do not remove exceptions.' +
            (' Verification: public checks passed.' if verified else ''), **kwargs)

    def image(self):
        return {p.relative_to(self.root).as_posix(): (p.read_bytes(), p.stat().st_mtime_ns)
                for p in self.root.rglob('*') if p.is_file() and not p.is_symlink()}

    def test_pipeline_exports_provenance_preserves_all_outcomes_and_source(self):
        self.event(1); self.event(2)
        before = self.image(); export = self.base / 'export'
        result = pilot.run_pilot(self.root, variant='consolidate-verify', export=export)
        self.assertEqual(before, self.image())
        self.assertEqual('not-measured', result['task_outcome'])
        self.assertEqual([], result['validation_errors'])
        self.assertTrue(result['idempotent'])
        self.assertEqual(1, len(result['mappings']))
        self.assertEqual(2, len(result['mappings'][0]['events']))
        chunks = list((export / 'memory-bank/chunks').glob('*.md'))
        self.assertEqual(1, len(chunks))
        text = chunks[0].read_text()
        self.assertIn('Outcome 1', text); self.assertIn('Outcome 2', text)
        self.assertIn('auto-promoted', text)
        self.assertEqual('agent', result['mappings'][0]['attestation'])
        self.assertEqual(1, len(result['mappings'][0]['chunks']))
        self.assertEqual(64, len(result['export_digest']))
        self.assertIn('validate.py', result['runtime_inputs'])

    def test_unverified_and_foreign_events_are_not_laundered(self):
        self.event(1); self.event(2, verified=False); self.event(3, owner='other')
        export = self.base / 'export'
        result = pilot.run_pilot(self.root, variant='consolidate-verify', export=export)
        self.assertEqual([], list((export / 'memory-bank/chunks').glob('*.md')))
        self.assertEqual('observed', result['mappings'][0]['authority'])
        self.assertEqual(2, len(result['mappings'][0]['events']))

    @unittest.skipUnless(os.name == 'posix', 'POSIX executable hook fixture')
    def test_export_preserves_actual_prompt_hook_execution(self):
        hooks = self.root/'.codex/hooks'; hooks.mkdir(parents=True)
        hook = hooks/'working-memory-read.sh'
        hook.write_text("#!/bin/sh\nprintf 'fixture-hook-ok\\n'\n"); hook.chmod(0o751)
        configuration = Path(pilot.ROOT)/'PHP Core/.codex/hooks.json'
        (self.root/'.codex/hooks.json').write_bytes(configuration.read_bytes())
        command = json.loads(configuration.read_text())['hooks']['UserPromptSubmit'][0]['hooks'][0]['command']
        self.assertEqual(0, subprocess.run(['/bin/sh', '-c', command], cwd=self.root,
                                         capture_output=True).returncode)
        export = self.base/'export'
        pilot.run_pilot(self.root, variant='consolidate-verify', export=export)
        after = subprocess.run(['/bin/sh', '-c', command], cwd=export, capture_output=True)
        self.assertEqual(0, after.returncode, after.stderr)
        self.assertEqual(0o751, (export/'.codex/hooks/working-memory-read.sh').stat().st_mode & 0o777)

    @unittest.skipUnless(os.name == 'posix', 'POSIX mode identity fixture')
    def test_mode_only_change_is_part_of_snapshot_identity(self):
        source = self.root/'specs/authority.md'; source.chmod(0o644)
        before = snapshot_digest(self.root)
        source.chmod(0o755)
        self.assertNotEqual(before, snapshot_digest(self.root))

    def test_baseline_and_copy_refuse_attachment_and_external_links(self):
        self.event(1); self.event(2)
        (self.root / '.git').write_text('gitdir: /outside/original')
        (self.root / 'accelerator-attach.json').write_text('{}')
        (self.root / 'launch').mkdir(); (self.root / 'launch/run').write_text('danger')
        (self.root / 'credentials.json').write_text('{"private":"must not copy"}')
        (self.root / '.env-backup').mkdir(); (self.root / '.env-backup/data.json').write_text('{}')
        (self.root / '.agents/skills/example').mkdir(parents=True)
        (self.root / '.agents/skills/example/SKILL.md').write_text('A canonical public constraint.\n')
        (self.root / 'link').symlink_to(self.base)
        export = self.base / 'export'
        result = pilot.run_pilot(self.root, variant='baseline', export=export)
        self.assertEqual([], result['mappings'])
        self.assertFalse((export / '.git').exists())
        self.assertFalse((export / 'link').exists())
        self.assertFalse((export / 'accelerator-attach.json').exists())
        self.assertFalse((export / 'launch').exists())
        self.assertFalse((export / 'credentials.json').exists())
        self.assertFalse((export / '.env-backup').exists())
        self.assertTrue((export / '.agents/skills/example/SKILL.md').is_file())


if __name__ == '__main__': unittest.main()
