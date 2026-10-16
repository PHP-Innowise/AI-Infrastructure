"""Fresh Wren/Marten copied-CLI continuity and persistent-rule assertions."""
from __future__ import annotations
from datetime import datetime, timezone
import importlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
import time
from unittest import mock
import importlib.util
import hashlib

ROOT = Path(__file__).resolve().parents[1]
PUBLIC = json.loads((ROOT/'tests/fixtures/cursor_continuation_state_public.json').read_text(encoding='utf-8'))
PRECHANGE = os.environ.get('CURSOR_CONTINUITY_PRECHANGE') == '1'


class CursorContinuityStateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='wren-cursor-state-')
        self.addCleanup(temporary.cleanup);self.root=Path(temporary.name)
        self.env={k:v for k,v in os.environ.items() if not k.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONIOENCODING='utf-8',PYTHONDONTWRITEBYTECODE='1')
        for relative in ('memory-bank/scripts','memory-bank/templates','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/relative,self.root/relative,
                            ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        (self.root/'memory-bank/chunks').mkdir()
        (self.root/'.gitignore').write_text('.cursor/rules/working-memory.mdc\nmemory-bank/local/\nproject-brain/local/\n',encoding='utf-8')
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps(
            {'mode':'governed','automatic_promotion':False}),encoding='utf-8')
        for name,text in [(PUBLIC['source_path'],PUBLIC['source']),(PUBLIC['other_path'],PUBLIC['other_source']),
                          *PUBLIC['background_sources'].items()]:
            leaf=self.root/name;leaf.parent.mkdir(exist_ok=True);leaf.write_text(text,encoding='utf-8')
        self.git('init','-q','-b','main')
        self.cli=self.root/'memory-bank/scripts/context.py'
        self.task=PUBLIC['task_id'];self.mode='governed'
        self.rule=self.root/'.cursor/rules/working-memory.mdc'

    def git(self,*args):
        result=subprocess.run(['git','-C',str(self.root),*args],env=self.env,capture_output=True,timeout=10)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'));return result.stdout

    def call(self,*args,stdin=None):
        result=subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(self.root),
            '--mode',self.mode,*args],input=(json.dumps(stdin).encode() if stdin is not None else None),
            env=self.env,cwd=self.root,capture_output=True,timeout=20)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'))
        return json.loads(result.stdout)

    def profile(self):
        return [] if PRECHANGE else ['--delivery-profile','cursor-rule']

    def refresh(self,query,*,gate='shadow',profile=True,host='cursor'):
        return self.call('refresh','--query',query,'--task-id',self.task,'--host',host,
            '--sanitize','--ephemeral','--gate',gate,'--json',*(self.profile() if profile else []))

    def publish(self,reply):
        return self.call('cursor-rule','--action','write','--phase','prompt','--task-id',self.task,
                         '--report-json',stdin=reply)

    def paths(self,reply):
        return sorted(item['path'] for item in reply['capsule']['semantic'])

    def seed(self,*,goal=None,gate='shadow'):
        if goal is not None:self.call('start','--task-id',self.task,'--goal',goal,'--json')
        reply=self.refresh(PUBLIC['substantive_query'],gate=gate)
        self.assertEqual(PUBLIC['oracles']['continued_paths'],self.paths(reply))
        self.publish(reply);self.assertIn(PUBLIC['initial_fact'],self.rule.read_text(encoding='utf-8'))
        return reply

    def report(self,**fields):
        leaf=self.root/'memory-bank/local/last-turn-report.json';leaf.parent.mkdir(exist_ok=True)
        leaf.write_text(json.dumps({'timestamp':datetime.now(timezone.utc).isoformat(),**fields}),encoding='utf-8')

    def test_warming_referential_forms_keep_current_topic_in_all_gate_modes(self):
        self.seed()
        for gate in ('off','shadow','enforce'):
            for query in PUBLIC['continuations']:
                with self.subTest(gate=gate,query=query):
                    reply=self.refresh(query,gate=gate)
                    self.assertEqual(PUBLIC['oracles']['continued_paths'],self.paths(reply))
                    self.assertEqual('continuation',reply['capsule']['query_source'])
                    self.publish(reply);self.assertIn(PUBLIC['initial_fact'],self.rule.read_text(encoding='utf-8'))

    def derived_goal(self,mode):
        self.mode=mode;self.task=PUBLIC['task_id']+'-'+mode
        self.call('start','--task-id',self.task,'--goal',PUBLIC['oracles']['derived_goals'][mode],'--json')
        self.seed();reply=self.refresh('yes do it')
        self.assertEqual(PUBLIC['oracles']['continued_paths'],self.paths(reply))
        self.assertEqual('continuation',reply['capsule']['query_source'])

    def test_governed_derived_goal_uses_the_substantive_seed(self):
        self.derived_goal('governed')

    def test_lightweight_derived_goal_uses_the_substantive_seed(self):
        self.derived_goal('lightweight')

    def test_routine_counters_leave_actual_rule_bytes_and_inode_unchanged(self):
        self.seed();before=self.rule.read_bytes();identity=self.rule.stat().st_ino
        self.report(flushed=True,files=7,promoted=[{'record_id':'public-ignored-counter'}],archived=3)
        reply=self.refresh(PUBLIC['substantive_query']);published=self.publish(reply)
        self.assertEqual('unchanged',published['state'])
        self.assertEqual(before,self.rule.read_bytes());self.assertEqual(identity,self.rule.stat().st_ino)
        self.assertNotIn('Last turn:',reply['capsule_text'])

    def test_enforced_repeat_renders_a_new_failure_instead_of_raw_keep(self):
        self.seed(goal=PUBLIC['explicit_goal'],gate='enforce')
        self.report(error='Wren checkpoint is incomplete')
        reply=self.refresh(PUBLIC['substantive_query'],gate='enforce')
        self.assertEqual('retrieve',reply['capsule']['gate']['decision'])
        self.assertEqual('persistent-rule-repeat',reply['capsule']['gate']['reason'])
        self.assertEqual('written',self.publish(reply)['state'])
        self.assertIn('Wren checkpoint is incomplete',self.rule.read_text(encoding='utf-8'))
        self.assertIn(PUBLIC['initial_fact'],self.rule.read_text(encoding='utf-8'))

    def test_same_topic_commit_and_unborn_branch_are_supported(self):
        self.seed();reply=self.refresh('да');self.assertEqual(PUBLIC['oracles']['continued_paths'],self.paths(reply))
        self.git('add','specs');self.git('-c','user.name=Wren Fixture','-c','user.email=wren@example.test','commit','-qm','Wren public fixture')
        self.assertEqual(PUBLIC['oracles']['continued_paths'],self.paths(self.refresh('да')))

    def test_fresh_canonical_edit_and_deletion_do_not_reuse_old_rule_facts(self):
        self.seed();source=self.root/PUBLIC['source_path']
        source.write_text(PUBLIC['source'].replace(PUBLIC['initial_fact'],PUBLIC['changed_fact']),encoding='utf-8')
        reply=self.refresh('fix it');self.publish(reply)
        self.assertIn(PUBLIC['changed_fact'],self.rule.read_text(encoding='utf-8'))
        self.assertNotIn(PUBLIC['initial_fact'],self.rule.read_text(encoding='utf-8'))
        source.unlink();reply=self.refresh('why?',gate='enforce');self.publish(reply)
        self.assertEqual([],self.paths(reply));self.assertNotIn(PUBLIC['source_path'],self.rule.read_text(encoding='utf-8'))

    def test_known_empty_hook_context_replaces_stale_facts(self):
        self.seed(goal=PUBLIC['explicit_goal']);(self.root/PUBLIC['source_path']).unlink()
        reply=self.call('hook-context','--task-id',self.task,'--host','cursor','--gate','enforce','--json',*self.profile())
        self.assertEqual([],reply['semantic']);self.assertIsNotNone(reply['working'])

    def test_explicit_new_task_and_branch_cannot_adopt_an_old_topic(self):
        self.seed();self.call('start','--task-id',self.task,'--goal','New unrelated ibis diffraction work','--json')
        reply=self.refresh('yes do it');self.assertEqual([],self.paths(reply))
        self.git('symbolic-ref','HEAD','refs/heads/feature-other')
        self.task='feature/other';reply=self.refresh('yes do it');self.assertEqual([],self.paths(reply))

    def test_latest_meaningful_unmatched_and_redacted_prompts_do_not_revive_seed(self):
        self.seed();self.refresh(PUBLIC['unmatched_query']);self.assertEqual([],self.paths(self.refresh('да')))
        self.refresh(PUBLIC['substantive_query']);self.refresh('Wren\nuser: copied conversation')
        reply=self.refresh('да');self.assertNotEqual('continuation',reply['capsule']['query_source'])

    def local_module(self):
        scripts=self.cli.parent;sys.path.insert(0,str(scripts))
        self.addCleanup(lambda:sys.path.remove(str(scripts)))
        spec=importlib.util.spec_from_file_location('wren_context_control',self.cli)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        return module

    def database(self):
        connection=sqlite3.connect(self.root/'memory-bank/local/context.db')
        connection.row_factory=sqlite3.Row;self.addCleanup(connection.close)
        return connection

    def test_utf8_cache_bound_is_applied_before_sql_transport_and_repair(self):
        self.seed();module=self.local_module();connection=self.database()
        with connection:connection.execute("UPDATE index_state SET value=? WHERE key='cursor-topic-v1'",('😀'*1537,))
        plan=module._cursor_topic_begin(connection,self.root,self.task,self.mode)
        self.assertIsNone(plan[1]);self.assertIsNone(plan[2])
        value={'profile':module.CURSOR_TOPIC_KEY,'scope':plan[0],'query':PUBLIC['substantive_query'],'written_at':time.time()}
        self.assertTrue(module._cursor_topic_write(connection,None,value))
        self.assertIsNotNone(module._cursor_topic_begin(connection,self.root,self.task,self.mode)[2])
        self.assertFalse(module._cursor_topic_write(connection,None,{**value,'query':'foreign replacement'}))

    def test_deep_cache_json_abstains_without_aborting_refresh(self):
        self.seed();module=self.local_module();connection=self.database()
        with connection:connection.execute("UPDATE index_state SET value=? WHERE key='cursor-topic-v1'",('['*1500+'0'+']'*1500,))
        self.assertIsNone(module._cursor_topic_begin(connection,self.root,self.task,self.mode)[2])
        self.assertNotEqual('continuation',self.refresh('yes do it')['capsule']['query_source'])

    def test_huge_cache_timestamp_abstains_without_aborting_refresh(self):
        self.seed();module=self.local_module();connection=self.database()
        value=json.loads(connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone()[0])
        value['written_at']=10**400
        with connection:connection.execute("UPDATE index_state SET value=? WHERE key='cursor-topic-v1'",(json.dumps(value),))
        self.assertIsNone(module._cursor_topic_begin(connection,self.root,self.task,self.mode)[2])
        self.assertNotEqual('continuation',self.refresh('yes do it')['capsule']['query_source'])

    def test_unseeded_ack_is_not_stored_as_a_substantive_topic(self):
        self.refresh('yes do it');connection=self.database()
        self.assertIsNone(connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone())

    def test_a_prompt_that_selected_nothing_is_not_a_topic_to_continue(self):
        # A reply that retrieved nothing used to become the topic, and every
        # whitelisted "yes" after it searched by that reply.
        self.seed();self.refresh(PUBLIC['unmatched_query']);connection=self.database()
        self.assertIsNone(connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone())
        self.assertNotEqual('continuation',self.refresh('yes do it')['capsule']['query_source'])
        # A prompt that selected something is still the topic a "yes" continues.
        self.refresh(PUBLIC['substantive_query'])
        reply=self.refresh('yes do it')
        self.assertEqual('continuation',reply['capsule']['query_source'])
        self.assertEqual(PUBLIC['oracles']['continued_paths'],self.paths(reply))

    def test_optional_topic_contention_does_not_consume_delivery_timeout(self):
        self.seed();module=self.local_module();connection=self.database();other=self.database()
        plan=module._cursor_topic_begin(connection,self.root,self.task,self.mode)
        other.execute('BEGIN IMMEDIATE');self.addCleanup(other.rollback)
        start=time.monotonic()
        saved=module._cursor_topic_write(connection,plan[1],{**plan[2],'query':'another topic'})
        elapsed=time.monotonic()-start
        self.assertFalse(saved);self.assertLess(elapsed,0.5)
        self.assertEqual(5000,connection.execute('PRAGMA busy_timeout').fetchone()[0])
        self.assertFalse(connection.in_transaction)
        self.assertEqual(plan[1],connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone()[0])

    def test_lightweight_projection_uses_one_normalized_current_snapshot(self):
        self.seed();module=self.local_module();connection=self.database()
        working={'task_id':self.task,'goal':'Wren public operation','progress':'  Wren\n operation   complete  ',
                 'auto_checkpoint':'Wren routine counter 71','next_steps':[],'files':[],'sources':[]}
        with mock.patch.object(module,'find_working_task',return_value=working) as read:
            reply=module.assemble_capsule(connection,self.root,mode='lightweight',query=PUBLIC['substantive_query'],
                task_id=self.task,limit=3,ephemeral=True,refresh_index=False,index_warnings=['Wren local index unavailable'],
                host='cursor',entry_point='refresh',delivery_profile='cursor-rule')
        self.assertEqual(1,read.call_count)
        self.assertEqual('Wren operation complete',reply['working']['progress'])
        with mock.patch.object(module,'find_working_task',return_value={**working,'auto_checkpoint':'user: copied conversation'}):
            with self.assertRaises(module.ContextError):
                module.assemble_capsule(connection,self.root,mode='lightweight',query=PUBLIC['substantive_query'],
                    task_id=self.task,limit=3,ephemeral=True,refresh_index=False,index_warnings=['Wren local index unavailable'],
                    host='cursor',entry_point='refresh',delivery_profile='cursor-rule')

    def test_final_text_uses_the_same_profile_as_json_and_manifest(self):
        self.seed();expected=self.refresh('yes do it')['capsule_text']
        result=subprocess.run([sys.executable,'-B',str(self.cli),'--root',str(self.root),'--mode',self.mode,
            'refresh','--query','yes do it','--task-id',self.task,'--host','cursor','--sanitize','--ephemeral',*self.profile()],
            env=self.env,cwd=self.root,capture_output=True,timeout=20)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'))
        self.assertTrue(result.stdout.decode('utf-8').endswith(expected+'\n'))

    def test_ordinary_ack_grammar_is_not_extended_by_the_rule_profile(self):
        module=self.local_module()
        for literal in ('yes do it','yes','why?','fix it','да','сделай'):
            self.assertFalse(module._cursor_continuation_prompt(literal))
            self.assertTrue(module._cursor_continuation_prompt(literal,persistent=True))
        for literal in ('continue','go on','дальше'):
            self.assertTrue(module._cursor_continuation_prompt(literal))

    def test_expired_ack_does_not_renew_the_substantive_topic(self):
        self.seed();connection=self.database()
        value=json.loads(connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone()[0])
        value['written_at']=0
        with connection:connection.execute("UPDATE index_state SET value=? WHERE key='cursor-topic-v1'",(json.dumps(value),))
        reply=self.refresh('yes do it');self.assertNotEqual('continuation',reply['capsule']['query_source'])
        self.assertIsNone(connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone())

    def test_topic_write_preserves_an_existing_callers_transaction(self):
        self.seed();module=self.local_module();connection=self.database()
        connection.execute("INSERT INTO index_state(key,value) VALUES ('wren-uncommitted','owned caller')")
        self.assertFalse(module._cursor_topic_write(connection,None,{}))
        self.assertTrue(connection.in_transaction)
        connection.rollback()
        self.assertIsNone(connection.execute("SELECT 1 FROM index_state WHERE key='wren-uncommitted'").fetchone())

    def test_handoff_requires_exact_creation_scope_and_preserves_competing_cas(self):
        self.seed();module=self.local_module();connection=self.database()
        plan=module._cursor_topic_begin(connection,self.root,self.task,self.mode)
        current={**plan[0],'kind':'bound','task_uuid':'wren-created-uuid','generation':'wren-created-generation',
            'goal':hashlib.sha256(module.derive_goal(self.task).encode('utf-8')).hexdigest()}
        proof=[{'task_uuid':current['task_uuid'],'generation':current['generation']}]
        for key in ('task_uuid','generation','config','branch','goal'):
            with self.subTest(changed=key),mock.patch.object(module,'_cursor_topic_scope',return_value={**current,key:'foreign'}):
                module._cursor_topic_handoff(connection,self.root,self.task,self.mode,plan,proof)
                self.assertEqual(plan[1],connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone()[0])
        competing=json.dumps({**plan[2],'query':'Marten public topic'})
        with connection:connection.execute("UPDATE index_state SET value=? WHERE key='cursor-topic-v1'",(competing,))
        with mock.patch.object(module,'_cursor_topic_scope',return_value=current):
            module._cursor_topic_handoff(connection,self.root,self.task,self.mode,plan,proof)
        self.assertEqual(competing,connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone()[0])

    def test_failed_checkpoint_cannot_transfer_topic_or_remove_pending_turn(self):
        self.seed();(self.root/'specs/wren-work.md').write_text('Wren public staged operation',encoding='utf-8')
        self.call('turn','--task-id',self.task,'--flush-after','5','--json')
        module=self.local_module();connection=self.database()
        before=connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone()[0]
        pending=len(module.pending_turn_deltas(connection,self.task));self.assertGreater(pending,0)
        with mock.patch.object(module,'apply_governed_update',side_effect=module.ContextError('Wren injected checkpoint interruption')):
            with mock.patch.object(module,'_cursor_topic_handoff') as handoff:
                with self.assertRaises(module.ContextError):module.flush_turn_deltas(connection,self.root,self.task,self.mode,'local')
                handoff.assert_not_called()
        self.assertEqual(before,connection.execute("SELECT value FROM index_state WHERE key='cursor-topic-v1'").fetchone()[0])
        self.assertEqual(pending,len(module.pending_turn_deltas(connection,self.task)))
        self.assertIsNone(module._cursor_topic_begin(connection,self.root,self.task,self.mode)[2])

    @unittest.skipIf(os.name=='nt','actual Bash hook requires Git Bash; native Windows CLI controls run separately')
    def test_actual_installed_read_hook_keeps_facts_on_short_replies_and_stable_counts(self):
        shutil.copytree(ROOT/'PHP Core/.cursor/hooks',self.root/'.cursor/hooks')
        self.env['CONTEXT_TASK_ID']=self.task
        hook=self.root/'.cursor/hooks/working-memory-read.sh'
        for prompt in (PUBLIC['substantive_query'],'yes do it'):
            result=subprocess.run(['bash',str(hook)],input=json.dumps({'prompt':prompt,'session_id':'public-wren-session'}).encode(),
                env=self.env,cwd=self.root,capture_output=True,timeout=20)
            self.assertEqual(0,result.returncode)
            self.assertEqual({'continue':True},json.loads(result.stdout))
            self.assertEqual(b'',result.stderr)
            self.assertIn(PUBLIC['initial_fact'],self.rule.read_text(encoding='utf-8'))
        before=self.rule.read_bytes();identity=self.rule.stat().st_ino
        self.report(flushed=True,files=77,archived=17)
        result=subprocess.run(['bash',str(hook)],input=b'{"prompt":"yes do it"}',
            env=self.env,cwd=self.root,capture_output=True,timeout=20)
        self.assertEqual({'continue':True},json.loads(result.stdout));self.assertEqual(b'',result.stderr)
        self.assertEqual(before,self.rule.read_bytes());self.assertEqual(identity,self.rule.stat().st_ino)

    def test_all_native_and_generator_templates_use_the_same_delivery_profile(self):
        for edition in ('PHP Core','Laravel','Symfony','Cms/wordpress'):
            for hook in ('working-memory-read.sh','working-memory-write.sh','local-context.sh'):
                raw=(ROOT/edition/'.cursor/hooks'/hook).read_text(encoding='utf-8')
                self.assertIn('--delivery-profile cursor-rule',raw)

                self.assertNotIn('ARGUMENTS+=(--session-id',raw)
        for tool in ('.agents','.claude','.cursor'):
            raw=(ROOT/'Infrastructure-Creator'/tool/'skills/memory-seed/assets/scripts/context_retrieval.py').read_text(encoding='utf-8')
            self.assertIn('--delivery-profile cursor-rule',raw)
            for skill in ('hook-forge','bootstrap-verifier'):
                raw=(ROOT/'Infrastructure-Creator'/tool/'skills'/skill/'SKILL.md').read_text(encoding='utf-8')
                self.assertIn('--delivery-profile cursor-rule',raw)

    def test_event_continuity_hooks_do_not_receive_capsule_only_arguments(self):
        for edition in ('PHP Core','Laravel','Symfony','Cms/wordpress'):
            raw=(ROOT/edition/'.cursor/hooks/context-continuity.sh').read_text(encoding='utf-8')
            self.assertNotIn('--delivery-profile',raw)

    def test_fifth_turn_checkpoint_transfers_only_its_new_native_generation(self):
        self.seed()
        for index in range(5):
            (self.root/'specs/wren-work.md').write_text('Wren fixture operation '+str(index),encoding='utf-8')
            self.call('turn','--task-id',self.task,'--flush-after','5','--json')
        reply=self.refresh('yes do it')
        self.assertIsNotNone(reply['capsule']['task_uuid'])
        self.assertEqual(PUBLIC['oracles']['continued_paths'],self.paths(reply))
        self.assertEqual('continuation',reply['capsule']['query_source'])

    def test_overcap_or_malformed_seed_is_not_used(self):
        self.seed()
        database=self.root/'memory-bank/local/context.db'
        with sqlite3.connect(database) as connection:
            connection.execute("UPDATE index_state SET value=? WHERE key='cursor-topic-v1'",('x'*7000,))
        reply=self.refresh('yes do it');self.assertEqual([],self.paths(reply))
        self.assertNotEqual('continuation',reply['capsule']['query_source'])

    def test_ordinary_cursor_and_other_hosts_keep_ordinary_repeat_profile(self):
        self.seed(goal=PUBLIC['explicit_goal'])
        for host in ('cursor','claude','codex'):
            first=self.refresh(PUBLIC['substantive_query'],gate='enforce',profile=False,host=host)
            second=self.refresh(PUBLIC['substantive_query'],gate='enforce',profile=False,host=host)
            self.assertEqual('repeat-retrieval',second['capsule']['gate']['reason'])
            self.assertEqual('skip',second['capsule']['gate']['decision'])

    def test_manifest_selection_matches_the_final_persistent_render(self):
        reply=self.seed();reply=self.refresh('yes do it',gate='enforce')
        manifest=json.loads((self.root/reply['capsule']['manifest']).read_text(encoding='utf-8'))
        paths=sorted(item['path'] for item in manifest['selected'])
        self.assertEqual(self.paths(reply),paths)
        for path in paths:self.assertIn(path,reply['capsule_text'])
        self.assertEqual('continuation',manifest['query_source'])


if __name__=='__main__':unittest.main()
