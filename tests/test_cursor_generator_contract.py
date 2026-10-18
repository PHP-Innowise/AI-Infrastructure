"""Actual generated-hook validator and native maintenance CLI contracts."""
from __future__ import annotations
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[1]
PUBLIC=json.loads((ROOT/'tests/fixtures/cursor_generator_contract_public.json').read_text(encoding='utf-8'))


class CursorGeneratorContractTests(unittest.TestCase):
    def setUp(self):
        clean={k:v for k,v in os.environ.items() if not k.startswith(('GIT_','CONTEXT_','ACCELERATOR_'))}
        clean['GIT_OPTIONAL_LOCKS']='0';environment=mock.patch.dict(os.environ,clean,clear=True)
        environment.start();self.addCleanup(environment.stop)
        temporary=tempfile.TemporaryDirectory(prefix='heron-generator-cursor-');self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name).resolve()
        spec=importlib.util.spec_from_file_location('heron_generator_validator',
            ROOT/'Infrastructure-Creator/.agents/skills/bootstrap-verifier/scripts/validate_generated.py')
        self.validator=importlib.util.module_from_spec(spec);spec.loader.exec_module(self.validator)
        shutil.copytree(ROOT/'PHP Core/.cursor/hooks',self.root/'.cursor/hooks')
        self.files={leaf.relative_to(self.root).as_posix():{} for leaf in (self.root/'.cursor/hooks').glob('*') if leaf.is_file()}

    def errors(self):
        errors=[];self.validator.validate_hooks(self.root,['cursor'],self.files,errors);return errors

    def test_actual_native_hook_delegates_pass_the_generated_contract(self):
        self.assertEqual([],self.errors())

    def test_old_shell_rule_writer_is_refused(self):
        for name in PUBLIC['phases']:
            (self.root/'.cursor/hooks'/name).write_text(
                '#!/bin/bash\n# .cursor/rules/working-memory.mdc\n# alwaysApply: true\nexit 0\n',encoding='utf-8')
        errors=self.errors()
        self.assertEqual(3,sum('does not delegate rule publication' in x for x in errors))
        self.assertEqual(3,sum('inlines working-memory frontmatter' in x for x in errors))

    def test_wrong_native_phase_is_refused(self):
        leaf=self.root/'.cursor/hooks/working-memory-read.sh'
        leaf.write_text(leaf.read_text(encoding='utf-8').replace('--phase prompt','--phase turn'),encoding='utf-8')
        self.assertTrue(any('native cursor-rule --action write --phase prompt' in x for x in self.errors()))

    @unittest.skipUnless(shutil.which('git'),'actual selected repository index')
    def test_actual_cli_reports_fixed_refusal_without_custom_content(self):
        shutil.copytree(ROOT/'PHP Core/memory-bank/scripts',self.root/'memory-bank/scripts',
                        ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        subprocess.run(['git','init','-q',str(self.root)],check=True,capture_output=True)
        (self.root/'.gitignore').write_text('.cursor/rules/working-memory.mdc\n',encoding='utf-8')
        rule=self.root/'.cursor/rules/working-memory.mdc';rule.parent.mkdir(parents=True)
        raw=b'Heron private custom instruction kept locally';rule.write_bytes(raw)
        process=subprocess.run([sys.executable,str(self.root/'memory-bank/scripts/context.py'),
            'cursor-rule','--action','write','--task-id','HERON-CURSOR','--diagnostic'],
            input='working: Heron current context\n',text=True,encoding='utf-8',capture_output=True,check=False)
        self.assertEqual(0,process.returncode,process.stderr)
        self.assertEqual('preserved',json.loads(process.stdout)['state'])
        self.assertEqual(PUBLIC['diagnostic']['custom_deviation']+'\n',process.stderr)
        self.assertEqual(raw,rule.read_bytes());self.assertNotIn(raw.decode(),process.stdout+process.stderr)

    @unittest.skipUnless(shutil.which('git') and os.name=='posix','actual selected repository index and links')
    def test_a_linked_rules_folder_is_named_and_never_written_through(self):
        # A .cursor/rules shared between projects through a link would carry
        # this project's capsule to every project sharing it.
        shutil.copytree(ROOT/'PHP Core/memory-bank/scripts',self.root/'memory-bank/scripts',
                        ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        subprocess.run(['git','init','-q',str(self.root)],check=True,capture_output=True)
        (self.root/'.gitignore').write_text('.cursor/rules/working-memory.mdc\n',encoding='utf-8')
        shared=self.root.parent/(self.root.name+'-shared-rules');shared.mkdir()
        self.addCleanup(shutil.rmtree,shared,True)
        (shared/'team.mdc').write_text('Heron team rule\n',encoding='utf-8')
        for linked in ('rules','cursor'):
            with self.subTest(linked=linked):
                if linked=='rules':
                    os.symlink(shared,self.root/'.cursor/rules')
                else:
                    os.unlink(self.root/'.cursor/rules');shutil.move(str(self.root/'.cursor'),str(self.root/'cursor-real'))
                    os.symlink(shared,self.root/'cursor-real/rules');os.symlink(self.root/'cursor-real',self.root/'.cursor')
                process=subprocess.run([sys.executable,str(self.root/'memory-bank/scripts/context.py'),
                    'cursor-rule','--action','write','--task-id','HERON-CURSOR','--diagnostic'],
                    input='working: Heron current context\n',text=True,encoding='utf-8',capture_output=True,check=False)
                self.assertEqual(0,process.returncode,process.stderr)
                reply=json.loads(process.stdout)
                self.assertEqual(('preserved','linked_directory'),(reply['state'],reply.get('reason')))
                self.assertEqual(PUBLIC['diagnostic']['linked_directory']+'\n',process.stderr)
                self.assertEqual(['team.mdc'],sorted(leaf.name for leaf in shared.iterdir()))

    def test_forge_instructions_require_native_delegation(self):
        text=(ROOT/'Infrastructure-Creator/.agents/skills/hook-forge/SKILL.md').read_text(encoding='utf-8')
        for token in ('cursor-rule --action write --phase prompt','--action prompt-held','--diagnostic',
                      'three seconds','reserved generated','supported Windows backend'):
            self.assertIn(token,text)
        self.assertNotIn('atomic temp-file-then-`mv` replace',text)

    @unittest.skipUnless(os.name=='posix' and shutil.which('bash') and shutil.which('python3'),
                         'actual POSIX hook shell failure channel')
    def test_host_delivered_failure_uses_only_fixed_stderr(self):
        scripts=self.root/'memory-bank/scripts';scripts.mkdir(parents=True)
        (scripts/'context.py').write_text(
            'import sys\nprint("Heron unexpected private error text",file=sys.stderr)\nsys.exit(1)\n',encoding='utf-8')
        environment=dict(os.environ,CONTEXT_CAPSULE_DELIVERED='1')
        process=subprocess.run(['bash',str(self.root/'.cursor/hooks/working-memory-read.sh')],
            env=environment,input='{}',text=True,encoding='utf-8',capture_output=True,check=False)
        self.assertEqual(0,process.returncode)
        self.assertEqual({'continue':True},json.loads(process.stdout))
        self.assertEqual('Cursor memory rule was not updated: maintenance_unavailable.\n',process.stderr)
        self.assertNotIn('private error text',process.stdout+process.stderr)


if __name__=='__main__':unittest.main()
