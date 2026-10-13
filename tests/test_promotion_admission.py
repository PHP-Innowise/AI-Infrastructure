"""Fresh filesystem regressions for automatic promotion admission fairness."""
from pathlib import Path
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SCRIPTS=Path(__file__).resolve().parents[1]/'PHP Core/memory-bank/scripts'
sys.path.insert(0,str(SCRIPTS))
try:
    import brain_runtime as brain
finally:
    sys.path.pop(0)


class PromotionAdmissionTest(unittest.TestCase):
    SHARED=('Gate cobalt requests before dispatch so queued messages cannot bypass '
            'the canonical authority rule while workers await their bounded execution lease.')
    OTHER=('Inspect amber storage humidity sensors daily and replace exhausted silica '
           'cartridges before sealed wooden containers absorb moisture during extended warehouse transit.')
    THIRD=('Require indigo ledger reconciliation after each accounting window closes '
           'and compare explicit currency minor units against the signed settlement document.')

    def setUp(self):
        self.temporary=tempfile.TemporaryDirectory(prefix='promotion-admission-')
        self.addCleanup(self.temporary.cleanup)
        self.repository=Path(self.temporary.name)
        subprocess.run(['git','-c','init.defaultBranch=main','init','--quiet',str(self.repository)],check=True,
            capture_output=True,env={**os.environ,'GIT_CONFIG_GLOBAL':'/dev/null','GIT_CONFIG_NOSYSTEM':'1'})
        (self.repository/'memory-bank/chunks').mkdir(parents=True)
        (self.repository/'memory-bank/README.md').write_text('# Memory Bank\n')
        (self.repository/'memory-bank/INDEX.md').write_text('# Memory Index\n\n| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n')
        (self.repository/'specs').mkdir()
        (self.repository/'specs/authority.md').write_text('# Public fixture\n\n'+self.SHARED+'\n'+self.OTHER+'\n'+self.THIRD+'\n')
        config=self.repository/'project-brain/config/runtime.json';config.parent.mkdir(parents=True)
        config.write_text(json.dumps({'mode':'governed','automatic_promotion':True}))

    def resolved(self, number, title, consequence):
        # Chunk identities use the UUID prefix; keep both record order and
        # destination identities distinct rather than manufacturing collisions.
        uid=f'{number:08x}-0000-4000-8000-000000000001'
        with mock.patch.object(brain,'new_uuid',return_value=uid):
            record=brain.create_record(self.repository,'finding',f'ADMISSION-{number}',title,[],['specs/authority.md'],owner='alice')
        brain.update_record(self.repository,uid,expected_revision=1,progress=None,next_steps=[],files=[],sources=[],
            actor='alice',authority='verified',reason='Verified against public fixture')
        brain.update_record(self.repository,uid,expected_revision=2,progress=consequence,next_steps=[],files=[],sources=[],
            actor='alice',transition_to='resolved',reason='Resolved fixture finding')
        return brain.find_record(self.repository,uid)[1]

    def test_duplicate_prefix_does_not_starve_independent_fact_or_repeat_it(self):
        first=self.resolved(1,'Cobalt request gate',self.SHARED)
        seed=brain.auto_promote(self.repository,owner='alice')['promoted'][0]['memory_id']
        duplicates=[self.resolved(n,'Cobalt request gate',self.SHARED) for n in range(2,7)]
        independent=self.resolved(7,'Amber humidity maintenance',self.OTHER)
        candidates,_=brain.promotable_records(self.repository,brain.load_config(self.repository),allow_changed_sources=True)
        self.assertEqual([r['id'] for r in duplicates]+[independent['id']],[x['record']['id'] for x in candidates])
        result=brain.auto_promote(self.repository,owner='alice',limit=5)
        self.assertEqual([independent['id']],[x['record_id'] for x in result['promoted']])
        self.assertEqual(5,len(result['blocked']));self.assertTrue(all(seed in x['reason'] for x in result['blocked']))
        self.assertEqual(0,result['skipped']);self.assertEqual([],result['failed'])
        before={str(p.relative_to(self.repository)):p.read_bytes() for p in self.repository.rglob('*') if p.is_file() and '.git' not in p.parts}
        again=brain.auto_promote(self.repository,owner='alice',limit=5)
        self.assertEqual([],again['promoted']);self.assertEqual([],again['failed']);self.assertEqual(5,len(again['blocked']))
        self.assertEqual(before,{str(p.relative_to(self.repository)):p.read_bytes() for p in self.repository.rglob('*') if p.is_file() and '.git' not in p.parts})

    def test_same_batch_duplicate_preserves_write_cap_and_exact_tail(self):
        a=self.resolved(1,'Cobalt request gate',self.SHARED)
        duplicate=self.resolved(2,'Cobalt request gate',self.SHARED)
        b=self.resolved(3,'Amber humidity maintenance',self.OTHER)
        c=self.resolved(4,'Indigo ledger reconciliation',self.THIRD)
        result=brain.auto_promote(self.repository,owner='alice',limit=2)
        self.assertEqual([a['id'],b['id']],[x['record_id'] for x in result['promoted']])
        self.assertEqual([duplicate['id']],[x['record_id'] for x in result['blocked']])
        self.assertEqual(1,result['skipped']);self.assertNotIn(c['id'],brain.promoted_source_ids(self.repository))

    def test_systematic_failure_consumes_admissions_and_is_reported(self):
        for n,(title,content) in enumerate([('Cobalt',self.SHARED),('Amber',self.OTHER),('Indigo',self.THIRD)],1):self.resolved(n,title,content)
        with mock.patch.object(brain,'create_promotion',side_effect=brain.BrainError('Synthetic unavailable destination')) as create:
            result=brain.auto_promote(self.repository,owner='alice',limit=2)
        self.assertEqual(2,create.call_count);self.assertEqual(2,len(result['failed']));self.assertEqual(1,result['skipped'])
        self.assertEqual([],result['promoted']);self.assertEqual([],brain.active_chunk_texts(self.repository))

    def test_stalled_retry_keeps_separate_budget_and_source_proposal_identity(self):
        a=self.resolved(1,'Cobalt request gate',self.SHARED)
        proposal=brain.create_promotion(self.repository,[a['id']],a['title'],brain.promotion_content(a),proposer='alice',review_mode='automatic')
        brain.auto_review_promotion(self.repository,proposal['id'])
        b=self.resolved(2,'Amber humidity maintenance',self.OTHER)
        with mock.patch.object(brain,'create_promotion',wraps=brain.create_promotion) as create:
            result=brain.auto_promote(self.repository,owner='alice',limit=1)
        self.assertEqual([a['id'],b['id']],[x['record_id'] for x in result['promoted']])
        self.assertEqual(1,create.call_count);self.assertEqual([],result['failed'])
        self.assertEqual([],brain.stalled_automatic_promotions(self.repository))
        self.assertEqual(0,result['skipped'])


if __name__=='__main__':unittest.main(verbosity=2)
