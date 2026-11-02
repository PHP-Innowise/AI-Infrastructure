"""Maintainer native A/B controller: private plans, safe images and bounded sessions."""
import contextlib
import hashlib
from datetime import date, timedelta
import json
import os
from pathlib import Path
import stat
import shlex
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
try:
    import memory_native_ab_io as io
    import memory_native_ab_session as session
except ImportError:
    io = session = None


class ImageTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(io, 'native A/B safe image support is missing')
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.source = self.root / 'source'; self.source.mkdir()
        self.script = self.source / 'bin/check.sh'; self.script.parent.mkdir()
        self.script.write_text('#!/bin/sh\nexit 0\n'); self.script.chmod(0o751)

    def test_physical_image_preserves_modes_and_binds_them(self):
        image = io.snapshot(self.source)
        out = self.root / 'copy'; digest = io.copy_image(image, out)
        self.assertEqual(0o751, stat.S_IMODE((out/'bin/check.sh').stat().st_mode))
        self.assertEqual(self.script.read_bytes(), (out/'bin/check.sh').read_bytes())
        self.script.chmod(0o644)
        self.assertNotEqual(digest, io.image_digest(io.snapshot(self.source)))
        self.assertEqual(1, (out/'bin/check.sh').stat().st_nlink)

    def test_execution_config_and_local_state_do_not_cross_tasks(self):
        for name in ['.codex/config.toml', '.codex/sessions/private.jsonl', '.cache/private',
                     '.claude/settings.json', '.mcp.json', '.context-handoff/private',
                     'memory-bank/local/private', 'project-brain/local/private', '.git/config']:
            p=self.source/name; p.parent.mkdir(parents=True,exist_ok=True); p.write_text('PRIVATE')
        p=self.source/'memory-bank/chunks/MEM-public.md'; p.parent.mkdir(parents=True); p.write_text('public')
        names={row[0] for row in io.snapshot(self.source)}
        self.assertEqual({'bin/check.sh','memory-bank/chunks/MEM-public.md'},names)

    def test_links_and_special_files_are_refused_before_copy(self):
        target=self.source/'public';target.write_text('public')
        for kind in ['symlink','hardlink','fifo']:
            with self.subTest(kind=kind):
                p=self.source/'unsafe'
                if kind=='symlink':p.symlink_to(target)
                elif kind=='hardlink':os.link(target,p)
                else:os.mkfifo(p)
                try:
                    with self.assertRaises(ValueError):io.snapshot(self.source)
                finally:p.unlink()

    def test_ancestor_replacement_cannot_read_outside_image(self):
        outside=self.root/'outside';outside.mkdir();(outside/'check.sh').write_text('OUTSIDE')
        original=os.open;swapped=False
        def swap(path,*args,**kwargs):
            nonlocal swapped
            if not swapped:
                swapped=True
                self.script.parent.rename(self.source/'old-bin')
                (self.source/'bin').symlink_to(outside,target_is_directory=True)
            return original(path,*args,**kwargs)
        with mock.patch.object(io.os,'open',side_effect=swap):
            with self.assertRaises((ValueError,OSError)):io.read_file(self.script)

    def test_plan_schema_numeric_and_unknown_keys_fail_closed(self):
        base={'schema':'memory-native-ab-v1','project_snapshot':str(self.source),
              'source':{'task_alias':'proof/source-alpha','prompt':'Research checked code'},
              'served':{'task_alias':'proof/served-omega','prompt':'Find previous rule'},
              'model':'gpt-6.1-sol','tool_cap':40,'deadline_seconds':120}
        plan=self.root/'plan.json'
        for update in [{'command':'echo PRIVATE'}, {'tool_cap':True}, {'deadline_seconds':float('inf')},
                       {'tool_cap':0}, {'served':base['source']}]:
            with self.subTest(update=update):
                plan.write_text(json.dumps({**base,**update}))
                with self.assertRaises(ValueError):io.load_config(plan)
        plan.write_text(json.dumps(base)); self.assertEqual(base,io.load_config(plan))
        plan.write_text('{"schema":1,"schema":2}')
        with self.assertRaises(ValueError):io.load_config(plan)

    def test_overlapping_or_linked_roots_fail_before_output(self):
        plan=self.root/'plan.json';plan.write_text('{}')
        for out in [self.source/'nested',self.source,self.root]:
            with self.subTest(out=out):
                with self.assertRaises(ValueError):io.validate_roots(self.source,plan,out)
        alias=self.root/'alias';alias.symlink_to(self.source,target_is_directory=True)
        with self.assertRaises(ValueError):io.validate_roots(alias,plan,self.root/'new')
        self.assertFalse((self.root/'new').exists())


class ProcessTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(session,'bounded native session runner is missing')
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)

    def run_code(self,code,**kwargs):
        return session.run_process([sys.executable,'-u','-c',code],self.root,self.root/'events.jsonl',
            timeout=kwargs.pop('timeout',2),cap=kwargs.pop('cap',10),**kwargs)

    def test_reconnect_multiple_turns_usage_and_repeated_item_ids(self):
        events=[{'type':'turn.started'}, {'type':'error','message':'Reconnecting'},
          {'type':'item.started','item':{'type':'command_execution','id':'item_0'}},
          {'type':'item.completed','item':{'type':'command_execution','id':'item_0'}},
          {'type':'turn.completed','usage':{'input_tokens':10,'output_tokens':2}},
          {'type':'turn.started'}, {'type':'item.completed','item':{'type':'command_execution','id':'item_0'}},
          {'type':'turn.completed','usage':{'input_tokens':20,'output_tokens':3}}]
        result=self.run_code('import json\nfor e in '+repr(events)+':print(json.dumps(e))')
        self.assertTrue(result['completed']);self.assertEqual(2,result['calls'])
        self.assertEqual({'input_tokens':30,'output_tokens':5},result['known_usage'])
        self.assertTrue(result['usage_complete'])

    def test_unfinished_continuation_and_bad_exit_are_incomplete(self):
        for tail in ["print('{\"type\":\"turn.started\"}')",'raise SystemExit(9)']:
            with self.subTest(tail=tail):
                result=self.run_code("print('{\"type\":\"turn.completed\"}')\n"+tail)
                self.assertFalse(result['completed'])
                (self.root/'events.jsonl').unlink()

    def test_output_limit_and_quiet_timeout_are_terminal(self):
        result=self.run_code("import os;os.write(1,b'x'*100000)",max_bytes=4096)
        self.assertEqual('output-limit',result['stopped']);self.assertFalse(result['completed'])
        (self.root/'events.jsonl').unlink()
        result=self.run_code('import time;time.sleep(10)',timeout=.15)
        self.assertEqual('timeout',result['stopped']);self.assertLess(result['seconds'],1.5)

    def test_invalid_budgets_do_not_launch_or_create_log(self):
        for kwargs in [{'max_bytes':float('nan')},{'max_bytes':True},{'timeout':10**500}]:
            with self.subTest(kwargs=kwargs), mock.patch.object(session.subprocess,'Popen') as launch:
                with self.assertRaises(ValueError):self.run_code('pass',**kwargs)
                launch.assert_not_called()
                self.assertFalse((self.root/'events.jsonl').exists())

    def test_tool_after_terminal_or_malformed_item_is_not_completion(self):
        for tail in [{'type':'item.started','item':{'type':'command_execution','id':'late'}},
                     {'type':'item.completed','item':[]}]:
            with self.subTest(tail=tail):
                events=[{'type':'turn.completed'},tail]
                result=self.run_code('import json\nfor e in '+repr(events)+':print(json.dumps(e))')
                self.assertFalse(result['completed'])
                (self.root/'events.jsonl').unlink()

    def test_cap_stops_at_boundary(self):
        events=[{'type':'item.started','item':{'type':'command_execution','id':'first'}},
                {'type':'item.started','item':{'type':'command_execution','id':'second'}},
                {'type':'turn.completed'}]
        result=self.run_code('import json\nfor e in '+repr(events)+':print(json.dumps(e))',cap=1)
        self.assertEqual(1,result['calls']);self.assertEqual('cap',result['stopped'])

    def test_trusted_preflight_output_is_bounded_and_stderr_retained(self):
        stderr=self.root/'private.stderr'
        result=self.run_code("import sys;print('probe-ok');print('failure-detail',file=sys.stderr)",
            native=False,stderr_path=stderr)
        self.assertTrue(result['completed'])
        self.assertEqual(b'probe-ok\n',(self.root/'events.jsonl').read_bytes())
        self.assertIn('failure-detail',stderr.read_text())
        self.assertEqual(0o600,stat.S_IMODE(stderr.stat().st_mode))

    def test_missing_usage_stays_unknown_and_log_is_private(self):
        result=self.run_code("print('{\"type\":\"turn.completed\"}')")
        self.assertTrue(result['completed']);self.assertFalse(result['usage_complete'])
        self.assertEqual(0o600,stat.S_IMODE((self.root/'events.jsonl').stat().st_mode))


class DeliveryEvidenceTests(unittest.TestCase):
    def setUp(self):
        import memory_native_ab as native
        self.native = native
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.events = self.root / 'events.jsonl'
        self.frame = 'Checked memory: отказ от Redis.\n'.encode('utf-8')
        self.document = 'memory-bank/chunks/MEM-20261101-aabbccdd-rule.md'

    def observe(self, output, *, command=None, exit_code=0, task='task-a', render=None):
        document = {'path': self.document, 'document_state': 'match',
                    'sources': [{'state': 'match'}], 'render': {
                        'status': 'trace-reported', 'frame_sha256': io.sha(self.frame),
                        'frame_bytes': len(self.frame)}}
        if render is not None:
            document['render'].update(render)
        proof = {'frames': [{'task_uuid': task, 'origin': 'explicit', 'selected': [document]}]}
        event = {'type': 'item.completed', 'item': {'type': 'command_execution',
                 'exit_code': exit_code,
                 'command': command or 'python3 memory-bank/scripts/context.py retrieve --query Redis',
                 'aggregated_output': output}}
        self.events.write_text(json.dumps(event) + '\n', encoding='utf-8')
        with mock.patch('memory_delivery_proof.collect_delivery_proof', return_value=proof):
            return self.native.delivery(self.root, {'task_uuid': 'task-a'}, self.events,
                                        set(), {self.document}, [self.document])

    def test_exact_frame_and_frame_prefix_of_batched_output_are_observed(self):
        for suffix, match in [('', 'whole'), ('following source text\n', 'prefix')]:
            with self.subTest(suffix=suffix):
                proof, found = self.observe(self.frame.decode() + suffix,
                    command='python3 memory-bank/scripts/context.py retrieve --query Redis && cat source.txt')
                self.assertEqual([self.document], found)
                self.assertEqual(match, proof['frames'][0]['selected'][0]['render']['native_stdout_match'])

    def test_only_a_complete_exact_byte_prefix_matches(self):
        for output in ['leading text\n' + self.frame.decode(), self.frame.decode()[:-1],
                       self.frame.decode().replace('Redis', 'Memcached'),
                       self.frame.decode().replace('\n', '\r\n')]:
            with self.subTest(output=output):
                self.assertEqual([], self.observe(output)[1])

    def test_canonical_successful_retrieval_and_task_binding_remain_required(self):
        for options in [{'command': 'echo memory'}, {'exit_code': 1}, {'task': 'other-task'},
                        {'command': '/tmp/python3 memory-bank/scripts/context.py retrieve'}]:
            with self.subTest(options=options):
                self.assertEqual([], self.observe(self.frame.decode(), **options)[1])

    def test_nontext_stdout_and_invalid_frame_lengths_remain_unknown(self):
        for output in [None, 42, {'text': self.frame.decode()}]:
            with self.subTest(output=output):
                self.assertEqual([], self.observe(output)[1])
        for size in [True, 0, -1, len(self.frame) + 1, 2**40]:
            with self.subTest(size=size):
                self.assertEqual([], self.observe(self.frame.decode(), render={'frame_bytes': size})[1])


class CaptureEvidenceTests(unittest.TestCase):
    def test_unrelated_heredoc_and_nontext_commands_are_ignored(self):
        import memory_native_ab_evidence as evidence
        script="python3 - <<'PY'\n# don't parse Python comments as shell quoting\nprint('ok')\nPY"
        for command in ['/usr/bin/bash -lc '+shlex.quote(script),None,42]:
            self.assertIsNone(evidence.command_args(command,Path('/public')))

    def test_emitted_cd_wrapper_requires_the_exact_corpus(self):
        import memory_native_ab_evidence as evidence
        root=Path('/public/project')
        direct=("python3 memory-bank/scripts/context.py --root /public/project native-result --request "+
                'a'*64+' --token '+'b'*32+" --input - --json")
        command="(cd /public/project && "+direct+") <<'NATIVE_RESULT'\n{}\nNATIVE_RESULT"
        self.assertIsNotNone(evidence.save_call('/usr/bin/bash -lc '+shlex.quote(command),root))
        self.assertIsNone(evidence.save_call(command.replace('cd /public/project','cd /outside'),root))

    def test_literal_cat_pipeline_binds_exact_root_and_payload(self):
        import memory_native_ab_evidence as evidence
        root=Path('/public/project with spaces')
        invocation=('(cd '+shlex.quote(str(root))+' && python3 memory-bank/scripts/context.py --root '+
                    shlex.quote(str(root))+' native-result --request '+'a'*64+' --token '+'b'*32+
                    ' --input - --json)')
        payload={'progress':'Checked $(not-executed) and `literal` text.','verified':True,'learnings':[]}
        for cat in ('cat','/bin/cat','/usr/bin/cat'):
            for target in (invocation,invocation.split(' && ',1)[1][:-1]):
                script=cat+" <<'JSON' | "+target+'\n'+json.dumps(payload)+'\nJSON'
                for command in (script,'/usr/bin/bash -lc '+shlex.quote(script)):
                    with self.subTest(cat=cat,grouped=target==invocation,wrapped=command!=script):
                        self.assertEqual(('a'*64,'b'*32,payload),evidence.save_call(command,root))

    def test_double_quoted_pwd_root_preserves_literal_payload(self):
        import memory_native_ab_evidence as evidence
        root=Path('/public/project with spaces')
        direct=('python3 memory-bank/scripts/context.py --root "$PWD" native-result --request '+
                'a'*64+' --token '+'b'*32+' --input - --json')
        payload={'progress':'Literal $HOME and $(never-run).','verified':True,'learnings':[]}
        for invocation in (direct,'(cd '+shlex.quote(str(root))+' && '+direct+')'):
            for script in (invocation+" <<'JSON'\n"+json.dumps(payload)+'\nJSON',
                           "cat <<'JSON' | "+invocation+'\n'+json.dumps(payload)+'\nJSON'):
                for command in (script,'/usr/bin/bash -lc '+shlex.quote(script)):
                    with self.subTest(command=command):
                        self.assertEqual(('a'*64,'b'*32,payload),evidence.save_call(command,root))

    def test_pwd_root_rejects_other_expansions_literal_variables_and_shell_steps(self):
        import memory_native_ab_evidence as evidence
        root=Path('/public/project')
        direct=('python3 memory-bank/scripts/context.py --root "$PWD" native-result --request '+
                'a'*64+' --token '+'b'*32+' --input - --json')
        invalid=[direct.replace('"$PWD"',value) for value in
                 ("'$PWD'",'$PWD','"${PWD}"','"$HOME"','"$PWD/other"','"$(pwd)"')]
        invalid += ['PWD=/outside '+direct,'(cd /outside && '+direct+')',
                    direct.replace('--root "$PWD"','--root "$PWD" --root /outside'),
                    direct.replace('--root "$PWD"',"'--root' \"$PWD\""),
                    direct+' && echo extra',direct+' > /tmp/other']
        for invocation in invalid:
            for script in (invocation+" <<'JSON'\n{}\nJSON",
                           "cat <<'JSON' | "+invocation+'\n{}\nJSON'):
                with self.subTest(command=script):self.assertIsNone(evidence.save_call(script,root))

    def test_cat_pipeline_rejects_other_commands_roots_and_nonliteral_input(self):
        import memory_native_ab_evidence as evidence
        root=Path('/public/project')
        invocation=('(cd /public/project && python3 memory-bank/scripts/context.py native-result --request '+
                    'a'*64+' --token '+'b'*32+' --input - --json)')
        script="cat <<'JSON' | "+invocation+'\n{}\nJSON'
        invalid=[script.replace('cat ',name+' ',1) for name in ('/tmp/cat','cat -','env cat','tee')]
        invalid += [script.replace("<<'JSON'",'<<JSON'),script.replace("<<'JSON'","<<-'JSON'"),
                    script.replace('cd /public/project','cd /outside'),
                    script.replace('&& python3','&& echo fake && python3'),
                    script.replace(' --json)',' --json && echo fake)'),
                    script.replace(' | (',' | tee /tmp/copy | ('),
                    script.replace(')\n',') > /tmp/output\n'),script+'\necho extra',
                    script.replace('python3','/tmp/python3',1)]
        for command in invalid:
            with self.subTest(command=command):self.assertIsNone(evidence.save_call(command,root))

    def test_learning_only_and_decision_saves_are_valid(self):
        import memory_native_ab_evidence as evidence
        from tests.test_memory_native_capture import NativeCaptureFixture
        for variant in ('decision','learning-only'):
            with self.subTest(variant=variant):
                case=NativeCaptureFixture();case.setUp()
                try:
                    before={n for n,_,_ in io.snapshot(case.root)}
                    case.begin();case.stop();path,state=case.state()
                    payload=case.payload.copy()
                    if variant=='decision':
                        payload['learnings']=[{**payload['learnings'][0],'type':'decision'}]
                    else:
                        payload.pop('progress');payload.pop('next_steps')
                    answer=json.loads(case.save(data=payload).stdout);_,state=case.state()
                    script=('python3 memory-bank/scripts/context.py native-result --request '+path.stem+
                        ' --token '+state['token']+" --input - --json <<'NATIVE_RESULT'\n"+
                        json.dumps(payload)+'\nNATIVE_RESULT')
                    item={'type':'command_execution','id':'save','exit_code':0,'command':script,
                          'aggregated_output':json.dumps(answer)}
                    events=case.root/'events.jsonl'
                    events.write_text(json.dumps({'type':'item.completed','item':item})+'\n')
                    checked=evidence.capture(case.root,{'task_id':'research','task_uuid':state['task_uuid']},
                                             'session-a',events,before)
                    self.assertTrue(checked['native_saved']);self.assertEqual(1,len(checked['new_chunks']))
                finally:case.doCleanups()

    def test_fake_executable_paths_do_not_count_as_canonical(self):
        import memory_native_ab_evidence as evidence
        script=("python3 memory-bank/scripts/context.py native-result --request "+'a'*64+
                ' --token '+'b'*32+" --input - --json <<'NATIVE_RESULT'\n{}\nNATIVE_RESULT")
        self.assertIsNotNone(evidence.save_call(script,Path('/public')))
        for command in [script.replace('python3','/tmp/python3',1),'/tmp/bash -lc '+shlex.quote(script)]:
            self.assertIsNone(evidence.save_call(command,Path('/public')))

    def test_real_native_result_binds_new_knowledge_and_rejects_old_or_echoed(self):
        try: import memory_native_ab_evidence as evidence
        except ImportError: evidence=None
        self.assertIsNotNone(evidence,'native capture evidence validator is missing')
        from tests.test_memory_native_capture import NativeCaptureFixture
        case=NativeCaptureFixture();case.setUp()
        try:
            before={n for n,_,_ in io.snapshot(case.root)}
            case.begin();case.stop();_,request=case.state()
            answer=json.loads(case.save().stdout);case.stop(active=True)
            _,state=case.state()
            body=json.dumps(case.payload)
            script=('python3 memory-bank/scripts/context.py --root '+shlex.quote(str(case.root))+
                ' native-result --request '+next((case.root/'project-brain/local/native-results').glob('*.json')).stem+
                ' --token '+request['token']+" --input - --json <<'NATIVE_RESULT'\n"+body+'\nNATIVE_RESULT')
            event={'type':'item.completed','item':{'type':'command_execution','id':'save',
                'command':'/usr/bin/bash -lc '+shlex.quote(script),'exit_code':0,'aggregated_output':json.dumps(answer)}}
            events=case.root/'events.jsonl';events.write_text(json.dumps(event)+'\n')
            identity={'task_id':'research','task_uuid':state['task_uuid']}
            checked=evidence.capture(case.root,identity,'session-a',events,before)
            self.assertTrue(checked['native_saved']);self.assertEqual(1,len(checked['new_chunks']))
            self.assertFalse(checked['producer_authenticated'])
            # Matching local receipts are insufficient without an observed real command.
            event['item']['command']='echo '+shlex.quote(script)
            events.write_text(json.dumps(event)+'\n')
            self.assertFalse(evidence.capture(case.root,identity,'session-a',events,before)['native_saved'])
            events.write_text(json.dumps({**event,'item':{**event['item'],'command':'/usr/bin/bash -lc '+shlex.quote(script)}})+'\n')
            replay=evidence.capture(case.root,identity,'session-a',events,{n for n,_,_ in io.snapshot(case.root)})
            self.assertEqual([],replay['new_chunks'])
        finally:case.doCleanups()

    def test_progress_only_is_saved_without_knowledge(self):
        try: import memory_native_ab_evidence as evidence
        except ImportError: evidence=None
        self.assertIsNotNone(evidence,'native capture evidence validator is missing')
        from tests.test_memory_native_capture import NativeCaptureFixture
        case=NativeCaptureFixture();case.setUp()
        try:
            before={n for n,_,_ in io.snapshot(case.root)}
            case.begin();case.stop();path,state=case.state()
            payload={'progress':'Checked the public rule.','verified':True,'learnings':[]}
            answer=json.loads(case.save(data=payload).stdout);_,state=case.state()
            script=('python3 memory-bank/scripts/context.py native-result --request '+path.stem+
                ' --token '+state['token']+" --input - --json <<'NATIVE_RESULT'\n"+json.dumps(payload)+'\nNATIVE_RESULT')
            event={'type':'item.completed','item':{'type':'command_execution','id':'save','exit_code':0,
                'command':script,'aggregated_output':json.dumps(answer)}}
            events=case.root/'events.jsonl';events.write_text(json.dumps(event)+'\n')
            checked=evidence.capture(case.root,{'task_id':'research','task_uuid':state['task_uuid']},'session-a',events,before)
            self.assertTrue(checked['native_saved']);self.assertEqual([],checked['new_chunks'])
            # Real CLI sessions also send the same literal JSON through cat into
            # the emitted cd wrapper. A saved progress-only result is not a miss.
            pipeline=("cat <<'JSON' | (cd "+shlex.quote(str(case.root))+
                ' && python3 memory-bank/scripts/context.py --root '+shlex.quote(str(case.root))+
                ' native-result --request '+path.stem+' --token '+state['token']+
                ' --input - --json)\n'+json.dumps(payload)+'\nJSON')
            plain=("cat <<'JSON' | python3 memory-bank/scripts/context.py --root "+shlex.quote(str(case.root))+
                ' native-result --request '+path.stem+' --token '+state['token']+
                ' --input - --json\n'+json.dumps(payload)+'\nJSON')
            pwd_script=script.replace('context.py native-result','context.py --root "$PWD" native-result')
            for observed in (pipeline,plain,pwd_script):
                event['item']['command']='/usr/bin/bash -lc '+shlex.quote(observed)
                events.write_text(json.dumps(event)+'\n')
                checked=evidence.capture(case.root,{'task_id':'research','task_uuid':state['task_uuid']},'session-a',events,before)
                self.assertTrue(checked['native_saved']);self.assertEqual([],checked['record_ids'])
                self.assertEqual([],checked['new_chunks'])
            self.assertFalse(evidence.capture(case.root,{'task_id':'research','task_uuid':state['task_uuid']},
                                              'other-session',events,before)['native_saved'])
        finally:case.doCleanups()


class MemoryPreparationTests(unittest.TestCase):
    def setUp(self):
        import memory_native_ab
        self.controller = memory_native_ab
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.corpus = self.root / 'copy'
        self.calls = []
        self.env = self.controller.child_env()
        self.env.update(HOME=str(self.root), TMPDIR=str(self.root), PYTHONDONTWRITEBYTECODE='1')

    def image(self, *, source='rules.md', invalid=False):
        today = date.today()
        metadata = dict(id='MEM-9001', title='Amber retry boundary', type='constraint',
                        status='active', scope=['tooling'], tags=['retry'],
                        created=today.isoformat(), last_verified=today.isoformat(),
                        review_after=(today + timedelta(days=365)).isoformat(),
                        sources=[source], supersedes=[], superseded_by=None)
        if invalid:
            metadata['type'] = 'invalid-kind'
        chunk = ('---\n' + json.dumps(metadata) + '\n---\n# Amber retry boundary\n'
                 'Only idempotent requests can be retried.\n').encode()
        return [('memory-bank/chunks/MEM-9001-amber-retry.md', chunk, 0o640),
                ('rules.md', b'Only idempotent requests can be retried.\n', 0o644)]

    def trusted(self, label, command):
        self.calls.append(label)
        checked = subprocess.run(command, cwd=self.corpus, env=self.env,
                                 capture_output=True, timeout=30)
        if checked.returncode:
            raise io.PilotError(label + ': ' + checked.stdout.decode() + checked.stderr.decode())
        return checked.stdout

    def test_imported_chunks_rebuild_newer_blank_catalog_and_search_index(self):
        from memory_native_ab_admission import MaterializedGate
        image = self.image()
        io.copy_image(image, self.corpus)
        self.controller.install_runtime(self.corpus)
        gate = MaterializedGate(self.corpus, image)
        chunk = self.corpus / image[0][0]
        os.utime(chunk, (1, 1))  # Fresh overlay INDEX is newer; mtime healing skips it.
        index = self.corpus / 'memory-bank/INDEX.md'
        self.assertNotIn('MEM-9001', index.read_text())
        self.controller.prepare_memory(self.trusted)
        self.assertIn('MEM-9001', index.read_text())
        self.assertEqual((image[0][1], image[0][2]), io.read_file(chunk))
        with contextlib.closing(sqlite3.connect(self.corpus / 'memory-bank/local/context.db')) as db:
            self.assertIn(image[0][0], {row[0] for row in db.execute('SELECT path FROM documents')})
        before = (index.read_bytes(), index.stat().st_mtime_ns)
        self.controller.prepare_memory(self.trusted)
        self.assertEqual(before, (index.read_bytes(), index.stat().st_mtime_ns))
        gate.seal_generated()
        gate.checkpoint()
        self.assertTrue(gate.verify()['verified'])

    def test_empty_bank_prepares_and_each_validation_failure_stops_indexing(self):
        for variant in ('empty', 'metadata', 'missing-source'):
            with self.subTest(variant=variant):
                self.corpus = self.root / variant
                image = [] if variant == 'empty' else self.image(
                    source='absent.md' if variant == 'missing-source' else 'rules.md',
                    invalid=variant == 'metadata')
                io.copy_image(image, self.corpus)
                self.controller.install_runtime(self.corpus)
                self.calls.clear()
                if variant == 'empty':
                    self.controller.prepare_memory(self.trusted)
                    self.assertEqual(['bank-reindex', 'bank-validate', 'brain-validate', 'index'], self.calls)
                else:
                    with self.assertRaisesRegex(ValueError, 'bank-validate'):
                        self.controller.prepare_memory(self.trusted)
                    self.assertEqual(['bank-reindex', 'bank-validate'], self.calls)
                    self.assertEqual((image[0][1], image[0][2]), io.read_file(self.corpus / image[0][0]))


class ControllerTests(unittest.TestCase):
    def setUp(self):
        try: import memory_native_ab as controller
        except ImportError: controller=None
        self.assertIsNotNone(controller,'native A/B controller is missing');self.controller=controller
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.source=self.root/'source';self.source.mkdir()
        (self.source/'rules.md').write_text('Public retry rule.\n')
        self.config=self.root/'private.json'
        self.config.write_text(json.dumps({'schema':'memory-native-ab-v1','project_snapshot':str(self.source),
            'source':{'task_alias':'proof/source-alpha','prompt':'PRIVATE_SOURCE_PROMPT research rules'},
            'served':{'task_alias':'proof/served-omega','prompt':'PRIVATE_SERVED_PROMPT find rule'},
            'model':'gpt-6.1-sol','tool_cap':20,'deadline_seconds':60}))
        self.run=self.root/'experiment'

    def drive_fixture(self, extra=()):
        fixture=Path(__file__).resolve().parent/'fixtures/memory_native_ab_cli.py'
        (self.source/'rules.md').write_text('The Amber dispatcher retries only idempotent requests.\n')
        (self.source/'fixture_cli.py').write_bytes(fixture.read_bytes())
        self.controller.plan(self.config,self.run)
        original=self.controller.make_view
        with mock.patch.object(self.controller,'native_command',
                side_effect=lambda corpus,config,task:[sys.executable,str(corpus/'fixture_cli.py'),*extra]):
            with mock.patch.object(self.controller,'make_view',
                    side_effect=lambda *a,**k:original(*a,**k,credentials=False)):
                return self.controller.run(self.run)

    def test_plan_is_private_immutable_and_inspect_read_only(self):
        summary=self.controller.plan(self.config,self.run)
        self.assertEqual(2,summary['planned'])
        self.assertNotIn('PRIVATE',json.dumps(summary))
        self.assertEqual(0o700,stat.S_IMODE(self.run.stat().st_mode))
        self.assertEqual(0o600,stat.S_IMODE((self.run/'control/plan.json').stat().st_mode))
        before={p:p.read_bytes() for p in self.run.rglob('*') if p.is_file()}
        with mock.patch.object(subprocess,'Popen',side_effect=AssertionError('inspect launched process')):
            self.controller.inspect(self.run)
        self.assertEqual(before,{p:p.read_bytes() for p in self.run.rglob('*') if p.is_file()})
        with self.assertRaises(ValueError):self.controller.plan(self.config,self.run)

    def test_native_invocation_keeps_all_hooks_and_defers_manual_capture(self):
        config=io.load_config(self.config)
        command=self.controller.native_command(self.source,config,config['source'])
        self.assertNotIn('--ephemeral',command)
        self.assertIn('workspace-write',command)
        for name in ('SessionStart','UserPromptSubmit','Stop','PreToolUse','PostToolUse'):
            self.assertTrue(any(value.startswith('hooks.'+name+'=') for value in command))
        self.assertIn(config['source']['prompt'],command[-1])
        self.assertIn('record-result',command[-1])

    def test_private_state_status_cannot_be_exported_as_arbitrary_metadata(self):
        self.controller.plan(self.config,self.run)
        p=self.run/'control/state.json';state=io.read_json(p)
        state['stages']['source']['status']={'prompt':'PRIVATE_METADATA'}
        io.write_json(p,state)
        with self.assertRaises(ValueError):self.controller.inspect(self.run)

    @unittest.skipUnless(sys.platform=='linux' and shutil.which('bwrap'),'Linux bubblewrap fixture')
    def test_canonical_hooks_and_read_only_sources_with_writable_memory(self):
        from memory_outcome_isolation import run_probe
        corpus=self.root/'corpus';corpus.mkdir()
        (corpus/'rules.md').write_text('Public rule')
        evil=corpus/'.codex';evil.mkdir();(evil/'hooks.json').write_text('{"command":"PRIVATE_BAD_HOOK"}')
        self.controller.install_runtime(corpus)
        canonical=Path(__file__).resolve().parents[1]/'PHP Core/.codex/hooks.json'
        self.assertEqual(canonical.read_bytes(),(corpus/'.codex/hooks.json').read_bytes())
        self.assertTrue(os.access(corpus/'.codex/hooks/working-memory-write.sh',os.X_OK))
        view=self.controller.make_view(corpus,self.root/'controller', 'proof/source-alpha',
            [sys.executable],credentials=False)
        code="""from pathlib import Path
p=Path('rules.md')
try:p.write_text('bad')
except OSError:pass
else:raise SystemExit(5)
try:Path('memory-bank/scripts/context.py').write_text('bad')
except OSError:pass
else:raise SystemExit(6)
try:Path('.codex').rename('.codex-moved')
except OSError:pass
else:raise SystemExit(7)
Path('memory-bank/local/probe').write_text('ok')
Path('.sandbox-placeholder').touch()
print('native-preflight-ok')
"""
        checked=run_probe(view,[sys.executable,'-c',code])
        self.assertEqual(0,checked.returncode,checked.stderr)
        self.assertEqual('Public rule',(corpus/'rules.md').read_text())
        self.assertTrue((corpus/'memory-bank/local/probe').exists())

    @unittest.skipUnless(sys.platform=='linux' and shutil.which('bwrap'),'Linux bubblewrap fixture')
    def test_complete_fixture_runs_real_hooks_capture_and_fresh_B(self):
        fixture=Path(__file__).resolve().parent/'fixtures/memory_native_ab_cli.py'
        self.assertTrue(fixture.exists(),'offline native lifecycle driver is missing')
        (self.source/'rules.md').write_text('The Amber dispatcher retries only idempotent requests.\n')
        (self.source/'fixture_cli.py').write_bytes(fixture.read_bytes())
        self.controller.plan(self.config,self.run)
        original=self.controller.make_view
        def fixture_view(*args,**kwargs):
            return original(*args,**kwargs,credentials=False)
        with mock.patch.object(self.controller,'native_command',
                side_effect=lambda corpus,config,task:[sys.executable,str(corpus/'fixture_cli.py')]):
            with mock.patch.object(self.controller,'make_view',side_effect=fixture_view):
                result=self.controller.run(self.run)
        self.assertEqual({'source':'captured','served':'consulted'},result['stages'])
        self.assertEqual(2,result['attempted'])
        before=io.read_json(self.run/'control/source-before.json')
        self.assertIn('rules.md',{row['path'] for row in before['files']})
        state=io.read_json(self.run/'control/state.json')
        a=state['stages']['source']['result'];b=state['stages']['served']['result']
        self.assertNotEqual(a['identity']['task_uuid'],b['identity']['task_uuid'])
        self.assertNotEqual(a['sessions'],b['sessions'])
        self.assertEqual(1,len(a['new_chunks']))
        self.assertEqual(a['new_chunks'],b['delivered_chunks'])
        self.assertTrue(b['hooks_healthy'])
        for stage in ('source', 'served'):
            self.assertIn('validation passed',
                (self.run / 'control' / (stage + '-bank-validate.stdout')).read_text())
        for chunk in a['new_chunks']:
            self.assertIn(Path(chunk).name,
                (self.run / 'work/served/memory-bank/INDEX.md').read_text())
        self.assertFalse((self.run/'work/served/memory-bank/local/native-ab/home'/
                         a['sessions'][0]).exists())

    @unittest.skipUnless(sys.platform=='linux' and shutil.which('bwrap'),'Linux bubblewrap fixture')
    def test_foreign_pending_observations_block_without_flushing_under_A(self):
        original=self.controller.pending_state
        def foreign(corpus,**kwargs):
            result=original(corpus,**kwargs)
            return {**result,'turn_deltas':1,'turn_task_ids':['foreign/task']}
        with mock.patch.object(self.controller,'pending_state',side_effect=foreign):
            result=self.drive_fixture()
        self.assertEqual({'source':'pending-source','served':'blocked-source'},result['stages'])
        self.assertFalse((self.run/'control/source-settle.stdout').exists())
        self.assertFalse((self.run/'work/served').exists())

    @unittest.skipUnless(sys.platform=='linux' and shutil.which('bwrap'),'Linux bubblewrap fixture')
    def test_unexpected_public_file_never_crosses_the_handoff(self):
        result=self.drive_fixture(('--extra-public-file',))
        self.assertEqual({'source':'failed','served':'blocked-source'},result['stages'])
        self.assertFalse((self.run/'work/served').exists())

    @unittest.skipUnless(sys.platform=='linux' and shutil.which('bwrap'),'Linux bubblewrap fixture')
    def test_changed_knowledge_during_settle_blocks_and_retains_usage(self):
        original_pending=self.controller.pending_state
        original_process=self.controller.run_process
        def pending(corpus,**kwargs):
            result=original_pending(corpus,**kwargs)
            return {**result,'turn_deltas':1,'turn_task_ids':['proof/source-alpha']}
        def process(*args,**kwargs):
            result=original_process(*args,**kwargs)
            if Path(args[2]).name=='source-settle.stdout':
                chunk=next((Path(args[1])/'memory-bank/chunks').glob('MEM-*.md'))
                chunk.write_text('changed after native save')
            return result
        with mock.patch.object(self.controller,'pending_state',side_effect=pending):
            with mock.patch.object(self.controller,'run_process',side_effect=process):
                result=self.drive_fixture()
        self.assertEqual({'source':'failed','served':'blocked-source'},result['stages'])
        state=io.read_json(self.run/'control/state.json')
        self.assertEqual(10,state['stages']['source']['result']['known_usage']['input_tokens'])

    def test_failed_source_retains_attempt_and_never_prepares_B_or_retries(self):
        self.controller.plan(self.config,self.run)
        with mock.patch.object(self.controller,'execute_stage',return_value={
                'completed':False,'native_saved':False,'new_chunks':[], 'stopped':'timeout'}) as launch:
            summary=self.controller.run(self.run)
        self.assertEqual(1,launch.call_count)
        self.assertEqual(1,summary['attempted']);self.assertEqual('blocked-source',summary['stages']['served'])
        self.assertFalse((self.run/'work/served').exists())
        with self.assertRaises(ValueError):self.controller.run(self.run)

    @unittest.skipUnless(sys.platform == 'linux' and shutil.which('bwrap'), 'Linux bubblewrap fixture')
    def test_invalid_imported_bank_fails_before_git_and_native_dispatch(self):
        fixture = MemoryPreparationTests()
        fixture.setUp()
        try:
            # Text is safe to admit, but its active source is missing: Bank
            # validation must reject it rather than silently dropping it.
            for name, raw, mode in fixture.image(source='absent.md'):
                target = self.source / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(raw)
                target.chmod(mode)
            self.controller.plan(self.config, self.run)
            original = self.controller.run_process
            def no_native(*args, **kwargs):
                self.assertFalse(kwargs.get('native', True), 'invalid bank reached model dispatch')
                return original(*args, **kwargs)
            with mock.patch.object(self.controller, 'native_command', return_value=[sys.executable, 'unused.py']), \
                 mock.patch.object(self.controller, 'run_process', side_effect=no_native):
                result = self.controller.run(self.run, stage='source')
            self.assertEqual('failed', result['stages']['source'])
            self.assertFalse((self.run / 'work/source/.git').exists())
            self.assertIn('absent.md', (self.run / 'control/source-bank-validate.stderr').read_text())
            self.assertFalse((self.run / 'control/source-index.stdout').exists())
            self.assertFalse((self.run / 'work/served').exists())
        finally:
            fixture.doCleanups()

    def test_stale_reader_cannot_overwrite_a_finished_attempt_after_lock(self):
        self.controller.plan(self.config,self.run)
        original=os.open
        def winner(path,*args,**kwargs):
            if Path(path)==self.run/'run.lock':
                p=self.run/'control/state.json';state=io.read_json(p)
                state['stages']['source'].update(attempted=True,status='failed')
                io.write_json(p,state)
            return original(path,*args,**kwargs)
        with mock.patch.object(self.controller.os,'open',side_effect=winner):
            with mock.patch.object(self.controller,'execute_stage') as launch:
                with self.assertRaises(ValueError):self.controller.run(self.run)
        launch.assert_not_called()
        self.assertTrue(io.read_json(self.run/'control/state.json')['stages']['source']['attempted'])

    def test_extra_project_runtime_modules_are_removed_before_host_preparation(self):
        corpus=self.root/'corpus';(corpus/'memory-bank/scripts').mkdir(parents=True)
        (corpus/'memory-bank/scripts/json.py').write_text('raise RuntimeError("CLIENT_CODE")')
        self.controller.install_runtime(corpus)
        self.assertFalse((corpus/'memory-bank/scripts/json.py').exists())

    def test_runtime_or_source_drift_blocks_all_native_execution(self):
        self.controller.plan(self.config,self.run)
        (self.source/'rules.md').write_text('Changed after planning.')
        with mock.patch.object(self.controller,'execute_stage') as launch:
            summary=self.controller.run(self.run)
        launch.assert_not_called();self.assertEqual('failed',summary['stages']['source'])
        self.assertEqual('blocked-source',summary['stages']['served'])

    def test_progress_only_source_does_not_claim_knowledge_or_launch_B(self):
        self.controller.plan(self.config,self.run)
        with mock.patch.object(self.controller,'execute_stage',return_value={
                'completed':True,'native_saved':True,'new_chunks':[], 'stopped':'finished'}) as launch:
            summary=self.controller.run(self.run)
        self.assertEqual(1,launch.call_count)
        self.assertEqual('no-learning',summary['stages']['source'])
        self.assertIsNone(summary['causal_benefit']);self.assertIsNone(summary['provider_cost_savings'])


if __name__=='__main__':unittest.main()
