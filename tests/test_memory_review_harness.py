"""Heron full Harness -> prepared CLI -> observed recovery; no model calls."""
from __future__ import annotations
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'harness/src'))
from harness import cost_observation, memory_attempts, memory_draft, sessions
from harness.knowledge import KnowledgeManager
from harness.task_context import TaskContext
PUBLIC = json.loads((ROOT / 'tests/fixtures/memory_review_harness_public.json').read_text(encoding='utf-8'))
RECALL = json.loads((ROOT / 'tests/fixtures/memory_review_recall_public.json').read_text(encoding='utf-8'))


class HarnessObservedRecoveryTests(unittest.TestCase):
    def run_cut(self, cut, *, promotion=False):
        clean = {k:v for k,v in os.environ.items() if not k.startswith(('GIT_', 'CONTEXT_', 'ACCELERATOR_'))}
        clean.update(GIT_OPTIONAL_LOCKS='0',PROJECT_BRAIN_OWNER='local')
        environment=mock.patch.dict(os.environ,clean,clear=True);environment.start();self.addCleanup(environment.stop)
        temporary=tempfile.TemporaryDirectory(prefix='heron-harness-frontier-');self.addCleanup(temporary.cleanup)
        parent=Path(temporary.name);project=parent/'project';project.mkdir()
        from tests.test_harness_knowledge import install_knowledge_fixture
        install_knowledge_fixture(project)
        (project/PUBLIC['source_path']).write_text(PUBLIC['source'],encoding='utf-8')
        (project/'project-brain/config/runtime.json').write_text(json.dumps(
            {'mode':'governed','automatic_promotion':promotion}),encoding='utf-8')
        module=project/'memory-bank/scripts/memory_results.py'
        body=module.read_text(encoding='utf-8')
        hook = ("                _checkpoint_attempt(root, attempt_path, attempt, identifier, current, state, False)\n"
                if cut=='after_created_frontier' else
                "            _checkpoint_attempt(root, attempt_path, attempt, 'task', updated, 'task', True)\n")
        spaces='                ' if cut=='after_created_frontier' else '            '
        fault=(spaces+"if not (root / '.heron-owned-cut').exists():\n"+
               spaces+"    (root / '.heron-owned-cut').write_text('one owned fault', encoding='utf-8')\n"+
               spaces+"    raise OSError('owned Heron post-frontier interruption')\n")
        if cut is not None:
            self.assertEqual(1,body.count(hook));module.write_text(body.replace(hook,hook+fault),encoding='utf-8')
        for owner,name,value in [(sessions.providers,'discover_providers',[{'id':'codex','available':True,'name':'No model','executable':'unused'}]),
                (sessions.Sessions,'_worker',None),(sessions.Sessions,'_keep_accelerators_current',None)]:
            patcher=mock.patch.object(owner,name,return_value=value);patcher.start();self.addCleanup(patcher.stop)
        forbidden=mock.patch.object(sessions.providers,'build_command',side_effect=AssertionError('No model allowed'))
        forbidden.start();self.addCleanup(forbidden.stop)
        store=sessions.Sessions(parent/'state',[project]);self.addCleanup(store.close)
        manager=KnowledgeManager(store);self.addCleanup(manager.close);store.knowledge=manager
        project_id=next(iter(store.projects))
        session=store.create({'project_id':project_id,'provider':'codex','mode':'plan','prompt':PUBLIC['goal'],
            'brain':{'bank':'memory-bank','task_id':PUBLIC['task'],'query':'Heron keeper handoff expiry',
                     'goal':PUBLIC['goal'],'create':True,'review':True}})
        sid=session['id'];context=TaskContext(manager);prepared=context.prepare(session);store._save_brain(sid,prepared)
        generation=store.generations[sid];store.results.start_launch(sid,generation);store.launch_ids[sid]=generation
        store._event(sid,{'kind':'result','text':'```memory-draft\n'+json.dumps(PUBLIC['draft'])+'\n```'})
        event_id=store.events(sid)[-1]['id'];data=memory_draft.latest(store,sid)['draft']
        observation=cost_observation.LaunchObservation('codex');observation.memory_owner=(sid,generation)
        context=TaskContext(manager,observation=observation)
        def save():
            return context.save_memory(store.get(sid),data,automatic=True,generation=generation,origin_event_id=event_id)
        def domain():
            return {leaf.relative_to(project).as_posix():leaf.read_bytes()
                    for folder in ('project-brain/dynamic','project-brain/indexes','project-brain/control/handoffs')
                    for leaf in (project/folder).rglob('*') if leaf.is_file()}
        first=save();self.assertEqual(cut is None,first['ok'],first)
        interrupted=domain()
        [created]=[json.loads(raw.decode('utf-8').split('---',2)[1]) for name,raw in interrupted.items()
                   if name.endswith('.md') and '/findings/' in name]
        binding=memory_attempts.lookup(store,sid,generation);self.assertIsNotNone(binding)
        second=save();self.assertTrue(second['ok'],second)
        self.assertEqual(binding,memory_attempts.lookup(store,sid,generation))
        if cut=='after_task_frontier':self.assertEqual(interrupted,domain())
        before=domain();third=save();self.assertTrue(third['ok'],third);self.assertEqual(before,domain())
        records=[]
        for leaf in (project/'project-brain/dynamic').rglob('*.md'):
            records.append(json.loads(leaf.read_text(encoding='utf-8').split('---',2)[1]))
        [task]=[x for x in records if x['type']=='task']
        [learning]=[x for x in records if x['type']=='finding']
        self.assertEqual(created['id'],learning['id'])
        self.assertEqual(created['revision']+(1 if cut=='after_created_frontier' else 0),learning['revision'])
        self.assertEqual(PUBLIC['oracles']['task_progress'],task['progress'])
        self.assertEqual(PUBLIC['oracles']['task_next_steps'],task['next_steps'])
        self.assertEqual('resolved',learning['status']);self.assertEqual('verified',learning['authority'])
        self.assertEqual(PUBLIC['oracles']['learning_progress'],learning['progress'])
        self.assertEqual(PUBLIC['oracles']['operations'],[x['operation'] for x in observation.operations if x['operation']!='inspect'])
        self.assertTrue(all(any(io['started'] for io in x['runtime_io']) for x in observation.operations if x['operation']!='inspect'))
        def recall(query):
            response=manager.run(project_id,{'bank':'memory-bank','action':'retrieve',
                'task_id':PUBLIC['task'],'query':query})
            self.assertTrue(response['ok'],response)
            return response['result']['selected']
        initial=recall(RECALL['query'])
        finding_path='project-brain/dynamic/findings/'+learning['id']+'.md'
        self.assertFalse(any(item['path']==finding_path for item in initial),initial)
        chunks=list((project/'memory-bank/chunks').glob('MEM-*.md'))
        self.assertEqual(1 if promotion else 0,len(chunks))
        if promotion:
            chunk_path=chunks[0].relative_to(project).as_posix()
            metadata=json.loads(chunks[0].read_text(encoding='utf-8').split('---',2)[1])
            self.assertIn(finding_path,metadata['sources'])
            self.assertIn(PUBLIC['source_path']+'#heron-allocation',metadata['sources'])
            self.assertIn('agent-attested',metadata['tags'])
            [item]=[item for item in initial if item['path']==chunk_path]
            self.assertEqual('agent',item['attestation'])
            self.assertIn(RECALL['initial_fact'],item['snippet'])
        else:
            self.assertTrue(any(item['path']==PUBLIC['source_path'] for item in initial),initial)
            self.assertIn(RECALL['initial_fact'],json.dumps(initial))
        learning_leaf=project/finding_path;stored=learning_leaf.read_bytes()
        source=project/PUBLIC['source_path']
        source.write_text(PUBLIC['source'].replace(RECALL['initial_fact'],RECALL['changed_fact']),encoding='utf-8')
        changed=recall('Heron packet active keepers')
        self.assertFalse(any(item['path']==finding_path for item in changed),changed)
        self.assertIn(RECALL['changed_fact'],json.dumps(changed))
        for item in changed:
            if item['path'].startswith('memory-bank/chunks/'):
                self.assertTrue(item.get('source_changed'),item)
        self.assertEqual(stored,learning_leaf.read_bytes())
        source.unlink();missing=recall('Heron packet active keepers')
        self.assertFalse(any(item['path']==PUBLIC['source_path'] or item['path'].startswith('memory-bank/chunks/')
                             for item in missing),missing)
        self.assertEqual(stored,learning_leaf.read_bytes())

    def test_owned_created_frontier_replays_through_actual_copied_cli(self):
        self.run_cut('after_created_frontier')

    def test_owned_task_frontier_replays_without_second_task_update(self):
        self.run_cut('after_task_frontier')

    def test_default_promotion_returns_saved_agent_attested_knowledge(self):
        self.run_cut(None,promotion=True)


if __name__=='__main__':unittest.main()
