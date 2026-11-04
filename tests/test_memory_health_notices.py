"""NEW Iris fixed typed health source-notice aggregation oracles."""
import importlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
ROOT=Path(__file__).resolve().parents[1]
P=json.loads((ROOT/'tests/fixtures/memory_health_notices_public.json').read_text(encoding='utf-8'))
class HealthNoticeTests(unittest.TestCase):
    def setUp(self):
        t=tempfile.TemporaryDirectory(prefix='iris-public-health-');self.addCleanup(t.cleanup)
        self.root=Path(t.name)/'project';self.root.mkdir()
        self.env={k:os.environ[k] for k in ('PATH','HOME','LANG','LC_ALL','LC_CTYPE','SystemRoot','WINDIR','COMSPEC','PATHEXT') if k in os.environ}
        self.env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',XDG_CONFIG_HOME=str(self.root/'xdg'),TMPDIR=t.name)
        patch=mock.patch.dict(os.environ,self.env,clear=True);patch.start();self.addCleanup(patch.stop)
        runtime=ROOT/'PHP Core/memory-bank/scripts';names=[p.stem for p in runtime.glob('*.py')]
        previous={n:sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:sys.modules.pop(n,None)
            sys.modules.update(previous)
        self.addCleanup(restore);sys.path.insert(0,str(runtime))
        try:self.c=importlib.import_module('context')
        finally:sys.path.remove(str(runtime))
        self.journal=self.root/'memory-bank/local/refresh-health.ndjson';self.journal.parent.mkdir(parents=True)
        self.manifest=self.journal.parent/'manifests'/'iris-private-canary.json';self.manifest.parent.mkdir();self.manifest.write_bytes(b'IRIS_PRIVATE_MANIFEST_CANARY')
    def put(self,records):
        self.journal.write_bytes(b'\n'+b'\n'.join(json.dumps(x).encode('utf-8') for x in records)+b'\ninvalid-json\n')
        self.image=(self.journal.read_bytes(),stat.S_IMODE(self.journal.stat().st_mode))
    def health(self,window=None):
        with mock.patch.object(self.c,'read_manifests',side_effect=AssertionError('health must not read manifests')):
            value=self.c.refresh_health(self.root,window)
        self.assertEqual(self.image,(self.journal.read_bytes(),stat.S_IMODE(self.journal.stat().st_mode)))
        self.assertEqual(b'IRIS_PRIVATE_MANIFEST_CANARY',self.manifest.read_bytes())
        self.assertNotIn('private/path/canary',json.dumps(value));self.assertNotIn('IRIS_PRIVATE_MANIFEST_CANARY',json.dumps(value))
        return value
    def test_occurrences_and_record_counts_are_exact(self):
        self.put(P['records']);value=self.health();self.assertEqual(4,value['turns']);self.assertEqual(P['total'],value.get('source_notices'))
        self.assertEqual({'0':1},value['hook_status']);self.assertEqual(0.0,value['timeout_rate'])
    def test_window_is_applied_before_notice_totals(self):
        self.put(P['records']);self.assertEqual(P['window3'],self.health(3).get('source_notices'))
        self.assertNotIn('source_notices',self.health(1))
    def test_invalid_types_unknown_reasons_and_nonpositive_counts_are_absent(self):
        self.put([{'source_notices':x} for x in P['invalid']]);self.assertNotIn('source_notices',self.health())
    def test_empty_and_legacy_records_keep_optional_field_absent(self):
        self.put([]);v=self.health();self.assertEqual(0,v['turns']);self.assertIsNone(v['timeout_rate']);self.assertNotIn('source_notices',v)
        self.put([{},{}]);v=self.health();self.assertEqual(2,v['turns']);self.assertNotIn('source_notices',v)
    def test_actual_cli_json_and_text_deliver_only_fixed_notice_summary(self):
        self.put(P['records'])
        for rel in ('memory-bank/scripts','project-brain/config','project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        command=[sys.executable,'-B',str(self.root/'memory-bank/scripts/context.py'),'--root',str(self.root),'health']
        outputs=[]
        for flags in (['--json'],[]):
            r=subprocess.run(command+flags,cwd=self.root,env=self.env,capture_output=True,timeout=20)
            self.assertEqual(0,r.returncode,r.stderr.decode('utf-8','replace'));self.assertEqual(b'',r.stderr);outputs.append(r.stdout.decode('utf-8'))
        self.assertEqual(P['total'],json.loads(outputs[0]).get('source_notices'))
        self.assertTrue(outputs[1].endswith(P['text']),outputs[1]);self.assertEqual(1,outputs[1].count('sources omitted:'))
        self.assertNotIn('private/path/canary',''.join(outputs));self.assertNotIn('IRIS_PRIVATE_MANIFEST_CANARY',''.join(outputs))
        self.assertEqual(self.image,(self.journal.read_bytes(),stat.S_IMODE(self.journal.stat().st_mode)))
    def broken(self,count=0):
        """The series with a line a full disk cut inside a character, as bytes."""
        good=[json.dumps({'source':'hook','hook_status':0}).encode('utf-8')]*count
        return b'\n'.join([json.dumps({'source':'hook','hook_status':0}).encode('utf-8'),
                            b'{"source":"hook","task":"feature/\xd0',*good,
                            json.dumps({'source':'hook','hook_status':124}).encode('utf-8')])+b'\n'
    def test_a_line_cut_inside_a_character_is_one_bad_line(self):
        # It stopped every read of the series: `health` failed on it.
        self.journal.write_bytes(self.broken())
        self.image=(self.journal.read_bytes(),stat.S_IMODE(self.journal.stat().st_mode))
        value=self.health();self.assertEqual(2,value['turns']);self.assertEqual({'0':1,'124':1},value['hook_status'])
    def test_an_append_after_a_line_cut_inside_a_character_still_trims_the_series(self):
        # It stopped every trim: the series grew for good, read whole on every prompt.
        self.journal.write_bytes(self.broken(4))
        with mock.patch.object(self.c,'refresh_health_retention',return_value=3):
            self.c.append_refresh_health(self.root,{'source':'hook','hook_status':0})
        lines=self.journal.read_bytes().decode('utf-8').splitlines()
        self.assertEqual(3,len(lines));self.assertEqual({'source':'hook','hook_status':0},json.loads(lines[-1]))
if __name__=='__main__':unittest.main()
