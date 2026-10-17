"""NEW Waxwing actual native hook report transport and generator contract."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from uuid import UUID
from datetime import datetime

ROOT = Path(__file__).resolve().parents[1]
P = json.loads((ROOT/'tests/fixtures/memory_cursor_report_public.json').read_text(encoding='utf-8'))


class CursorReportStructureTests(unittest.TestCase):
    def setUp(self):
        scripts = ROOT/'Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts'
        old_path = sys.path[:]
        names = [p.stem for p in scripts.glob('*.py')]
        previous = {n:sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:
                sys.modules.pop(n,None)
            sys.modules.update(previous)
            sys.path[:] = old_path
        self.addCleanup(restore)
        spec = importlib.util.spec_from_file_location('waxwing_verifier',scripts/'validate_generated.py')
        self.v = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.v)

    def positioned(self,outside):
        lines = (ROOT/'PHP Core/.cursor/hooks/working-memory-read.sh').read_text(encoding='utf-8').splitlines(True)
        lines = [line for line in lines if line.strip()!='ARGUMENTS+=(--json)']
        start = next(i for i,line in enumerate(lines) if line.strip()=='if [ -n "$QUERY" ] && [ -n "$TASK_ID" ]; then')
        end = next(i for i in range(start+1,len(lines)) if lines[i].strip()=='fi')
        lines.insert(end+1 if outside else end,('' if outside else '  ')+'ARGUMENTS+=(--json)\n')
        return ''.join(lines)

    def test_native_inside_conditional_json_is_rejected(self):
        self.assertFalse(self.v.memory_hook_identity_contract(self.positioned(False),'cursor','working-memory-read.sh'))

    def test_unconditional_cursor_json_is_accepted(self):
        self.assertTrue(self.v.memory_hook_identity_contract(self.positioned(True),'cursor','working-memory-read.sh'))

    def test_claude_codex_profiles_remain_text(self):
        for tool in ('claude','codex'):
            source = (ROOT/'PHP Core'/('.'+tool)/'hooks/working-memory-read.sh').read_text(encoding='utf-8')
            self.assertNotIn('ARGUMENTS+=(--json)',source)
            self.assertTrue(self.v.memory_hook_identity_contract(source,tool,'working-memory-read.sh'))


@unittest.skipUnless(os.name=='posix' and shutil.which('bash'),'Native POSIX hook/custody profile')
class NativeCursorReportTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='waxwing-native-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)/'project'
        self.root.mkdir()
        self.env = {k:os.environ[k] for k in ('PATH','HOME','LANG','LC_ALL','LC_CTYPE',
                    'SystemRoot','WINDIR','COMSPEC','PATHEXT') if k in os.environ}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',
                        TMPDIR=temporary.name,XDG_CONFIG_HOME=str(self.root/'xdg'),CONTEXT_TASK_ID=P['task'])
        private_bin = Path(temporary.name)/'runtime-bin'
        private_bin.mkdir()
        (private_bin/'python3').symlink_to(sys.executable)
        self.env['PATH'] = str(private_bin)+os.pathsep+self.env.get('PATH','')
        self.run_process(['git','init','-q','-b','waxwing-public'])
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config',
                    'project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,
                            ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        (self.root/'.gitignore').write_text('/.cursor/rules/working-memory.mdc\n/memory-bank/local/\n/project-brain/local/\n',encoding='utf-8')
        cfg_path = self.root/'project-brain/config/runtime.json'
        cfg = json.loads(cfg_path.read_text(encoding='utf-8'))
        cfg.update(automatic_promotion=False,automatic_compaction=False)
        cfg_path.write_text(json.dumps(cfg),encoding='utf-8')
        self.hook = self.root/'.cursor/hooks/working-memory-read.sh'
        self.hook.parent.mkdir(parents=True)
        shutil.copyfile(ROOT/'PHP Core/.cursor/hooks/working-memory-read.sh',self.hook)
        self.hook.chmod(0o755)
        self.cli = self.root/'memory-bank/scripts/context.py'
        help_result = self.run_process([sys.executable,'-B',str(self.cli),'--help'])
        self.assertIn(b'cursor-rule',help_result.stdout)
        seeded = self.run_process([sys.executable,'-B',str(self.cli),'cursor-rule','--action','write',
            '--phase','prompt','--task-id',P['task']],P['old_capsule'].encode('utf-8'))
        self.assertEqual({'profile':'cursor-rule-custody-v1','state':'written','bytes_removed':0},json.loads(seeded.stdout))
        self.rule = self.root/'.cursor/rules/working-memory.mdc'
        self.assertEqual(self.expected(P['old_capsule']),self.rule.read_bytes())
        # A formerly cited public document is gone when the empty prompt arrives.
        gone = self.root/'docs/WaxwingSpool.md'
        gone.parent.mkdir()
        gone.write_bytes(b'# Waxwing spool\n\nOld waxwing copper pin.\n')
        gone.unlink()

    def run_process(self,command,data=b''):
        result = subprocess.run(command,cwd=self.root,env=self.env,input=data,capture_output=True,timeout=15)
        self.assertEqual(0,result.returncode,result.stderr.decode('utf-8','replace'))
        return result

    def expected(self,capsule):
        return (P['rule_prefix']+capsule.rstrip('\n')+P['rule_suffix']).encode('utf-8')

    def hook_call(self,payload,stderr=''):
        result = self.run_process([shutil.which('bash'),str(self.hook)],payload)
        self.assertEqual(P['stdout'].encode('utf-8'),result.stdout)
        self.assertEqual(stderr.encode('utf-8'),result.stderr)
        return result

    def replaced(self,payload):
        self.hook_call(payload)
        self.assertEqual(self.expected(P['current_capsule']),self.rule.read_bytes())
        self.assertNotIn(b'Old waxwing copper pin',self.rule.read_bytes())
        health = [json.loads(line) for line in (self.root/'memory-bank/local/refresh-health.ndjson').read_text(encoding='utf-8').splitlines()]
        self.assertEqual(0,health[-1]['hook_status'])
        self.assertEqual('hook',health[-1]['source'])
        runtime = [row for row in health if row['source']=='refresh']
        self.assertEqual(1,len(runtime))
        self.assertTrue(runtime[0]['layers_updated'])
        self.assertEqual(0,runtime[0]['warnings'])

    def test_missing_prompt_replaces_same_task_old_rule(self):
        self.replaced(b'{}')

    def test_empty_prompt_replaces_same_task_old_rule(self):
        self.replaced(b'{"prompt":""}')

    def test_nonstring_prompt_replaces_same_task_old_rule(self):
        self.replaced(b'{"prompt":["Waxwing public list"]}')

    def test_invalid_json_prompt_replaces_same_task_old_rule(self):
        self.replaced(b'{"prompt":')

    def test_whitespace_prompt_has_no_goal_fallback(self):
        self.populate_goal()
        self.replaced(b'{"prompt":"   "}')
        self.assertNotIn(P['goal_canary'].encode('utf-8'),self.rule.read_bytes())

    def test_privacy_withheld_prompt_has_no_goal_fallback(self):
        self.populate_goal()
        self.replaced(json.dumps({'prompt':P['private_prompt']}).encode('utf-8'))
        self.assertNotIn(P['private_prompt'].encode('utf-8'),self.rule.read_bytes())
        self.assertNotIn(P['goal_canary'].encode('utf-8'),self.rule.read_bytes())

    def populate_goal(self):
        result = self.run_process([sys.executable,'-B',str(self.cli),'--owner','waxwing-agent','start',
                                  '--task-id',P['task'],'--goal',P['goal_canary'],'--json'])
        value = json.loads(result.stdout)
        task_uuid = value.pop('task_uuid')
        created,updated = value.pop('created_at'),value.pop('updated_at')
        self.assertEqual(4,UUID(task_uuid).version)
        self.assertEqual(created,updated)
        self.assertIsNotNone(datetime.fromisoformat(created).tzinfo)
        self.assertEqual({'task_id':P['task'],'phase':None,'revision':1,'status':'active',
            'goal':P['goal_canary'],'progress':'','auto_checkpoint':None,'next_steps':[],
            'files':[],'sources':[],'authority':'project-brain'},value)
        record = self.root/'project-brain/dynamic/tasks'/(task_uuid+'.md')
        self.assertIn(P['goal_canary'],record.read_text(encoding='utf-8'))

    def test_custom_rule_is_preserved(self):
        self.rule.write_bytes(P['custom'].encode('utf-8'))
        before = (self.rule.read_bytes(),stat.S_IMODE(self.rule.stat().st_mode))
        self.hook_call(b'{}',P['preserved_stderr'])
        self.assertEqual(before,(self.rule.read_bytes(),stat.S_IMODE(self.rule.stat().st_mode)))

    def test_linked_rule_and_target_are_preserved(self):
        self.rule.unlink()
        target = self.root/'waxwing-team-rule.txt'
        target.write_bytes(P['custom'].encode('utf-8'))
        self.rule.symlink_to(target)
        before = (target.read_bytes(),stat.S_IMODE(target.stat().st_mode))
        self.hook_call(b'{}',P['preserved_stderr'])
        self.assertTrue(self.rule.is_symlink())
        self.assertEqual(before,(target.read_bytes(),stat.S_IMODE(target.stat().st_mode)))

    def test_hardlinked_rule_and_peer_are_preserved(self):
        peer = self.root/'waxwing-hardlink.txt'
        os.link(self.rule,peer)
        before = (peer.read_bytes(),peer.stat().st_ino,peer.stat().st_nlink)
        self.hook_call(b'{}',P['preserved_stderr'])
        self.assertEqual(before,(peer.read_bytes(),peer.stat().st_ino,peer.stat().st_nlink))
        self.assertEqual(peer.stat().st_ino,self.rule.stat().st_ino)


if __name__ == '__main__':
    unittest.main()
