"""NEW Tanager strict policy names; no skip-to-clean parity or digest."""
import contextlib
import hashlib
import importlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
P = json.loads((ROOT/'tests/fixtures/memory_policy_utf8_public.json').read_text(encoding='utf-8'))
POSIX = unittest.skipUnless(os.name == 'posix', 'Raw POSIX policy filename bytes unavailable')


class PolicyUtf8Tests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='tanager-policy-')
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)/'project'
        self.root.mkdir()
        env = {k: os.environ[k] for k in ('PATH','HOME','LANG','LC_ALL','LC_CTYPE',
               'SystemRoot','WINDIR','COMSPEC','PATHEXT') if k in os.environ}
        env.update(GIT_OPTIONAL_LOCKS='0',PYTHONDONTWRITEBYTECODE='1',
                   TMPDIR=temporary.name,XDG_CONFIG_HOME=str(self.root/'xdg'))
        patch = mock.patch.dict(os.environ,env,clear=True)
        patch.start()
        self.addCleanup(patch.stop)
        subprocess.run(['git','-C',str(self.root),'init','-q','-b','tanager-policy'],
                       env=env,capture_output=True,check=True,timeout=8)
        runtime = ROOT/'PHP Core/memory-bank/scripts'
        names = [p.stem for p in runtime.glob('*.py')]
        previous = {n:sys.modules.pop(n) for n in names if n in sys.modules}
        def restore():
            for n in names:
                sys.modules.pop(n,None)
            sys.modules.update(previous)
        self.addCleanup(restore)
        sys.path.insert(0,str(runtime))
        try:
            self.r = importlib.import_module('context_retrieval')
            self.c = importlib.import_module('context')
        finally:
            sys.path.remove(str(runtime))
        for rel in ('memory-bank/scripts','memory-bank/templates','project-brain/config',
                    'project-brain/schemas','project-brain/templates'):
            shutil.copytree(ROOT/'PHP Core'/rel,self.root/rel,
                            ignore=shutil.ignore_patterns('__pycache__','*.pyc'))
        cfg = json.loads((self.root/'project-brain/config/runtime.json').read_text(encoding='utf-8'))
        cfg.update(automatic_promotion=False,automatic_compaction=False)
        (self.root/'project-brain/config/runtime.json').write_text(json.dumps(cfg),encoding='utf-8')
        self.bad = bytes.fromhex(P['bad_hex']).decode('ascii','surrogateescape')

    def write(self,rel,text=None,root=None):
        p = (root or self.root)/rel
        p.parent.mkdir(parents=True,exist_ok=True)
        p.write_bytes((P['body'] if text is None else text).encode('utf-8'))
        os.utime(p,ns=(P['mtime_ns'],P['mtime_ns']))
        return p

    def malformed(self,edition='.agents',suffix='/SKILL.md',root=None):
        return self.write(edition+'/skills/'+self.bad+suffix,root=root)

    def refused(self,call):
        with self.assertRaises(self.r.RetrievalError) as raised:
            call()
        self.assertEqual(P['error'],str(raised.exception))
        self.assertTrue(str(raised.exception).isascii())

    def cli(self,*args):
        out,err = io.StringIO(),io.StringIO()
        with mock.patch.object(self.c,'default_root',return_value=self.root), \
                mock.patch.object(sys,'argv',['context.py',*args]), \
                contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
            status = self.c.main()
        out.getvalue().encode('utf-8')
        err.getvalue().encode('utf-8')
        return status,out.getvalue(),err.getvalue()

    def test_supplied_relative_key_refused(self):
        stats = [('.agents',self.bad+'/SKILL.md',P['mtime_ns'],44)]
        self.refused(lambda:self.r.skill_tree_fingerprint(self.root,stats))

    def test_supplied_edition_key_refused(self):
        stats = [(self.bad,P['valid'],P['mtime_ns'],44)]
        self.refused(lambda:self.r.skill_tree_fingerprint(self.root,stats))

    @POSIX
    def test_direct_fingerprint_refused(self):
        self.malformed()
        self.refused(lambda:self.r.skill_tree_fingerprint(self.root))

    @POSIX
    def test_identical_malformed_light_mirrors_never_clean(self):
        self.malformed('.agents')
        self.malformed('.cursor')
        self.refused(lambda:self.r.skill_mirror_drift(self.root,'.agents'))

    @POSIX
    def test_light_reverse_malformed_never_drift_payload(self):
        self.write('.agents/skills/'+P['valid'])
        self.malformed('.cursor')
        self.refused(lambda:self.r.skill_mirror_drift(self.root,'.agents'))

    @POSIX
    def test_full_canonical_asset_refused_before_transform(self):
        self.malformed('.agents',suffix='/latch.txt')
        self.malformed('.cursor',suffix='/latch.txt')
        with mock.patch.dict(self.r._MIRROR_TRANSFORMS,{'copy':mock.Mock()}):
            self.refused(lambda:self.r.full_mirror_drift(self.root))
            self.r._MIRROR_TRANSFORMS['copy'].assert_not_called()

    @POSIX
    def test_full_reverse_asset_refused_with_matching_counterpart(self):
        self.write('.agents/skills/'+P['valid'])
        self.write('.cursor/skills/'+P['valid'])
        self.malformed('.cursor',suffix='/latch.txt')
        self.refused(lambda:self.r.full_mirror_drift(self.root))

    @POSIX
    def test_full_reverse_asset_refused_before_partial_install_shortcut(self):
        self.write('.agents/skills/'+P['valid'])
        self.malformed('.cursor',suffix='/latch.txt')
        self.refused(lambda:self.r.full_mirror_drift(self.root))

    def test_supplied_only_key_refused_before_lookup(self):
        calls = mock.Mock(return_value=False)
        with mock.patch.object(Path,'is_file',calls):
            self.refused(lambda:list(self.r._mirror_iter_canonical(self.root,[self.bad+'/missing.md'])))
        calls.assert_not_called()

    @POSIX
    def test_fresh_cached_parity_cannot_accept_malformed_skill(self):
        self.write('.agents/skills/'+P['valid'])
        connection = self.c.connect(self.root/'memory-bank/local/context.db')
        self.addCleanup(connection.close)
        self.c.index_repository(connection,self.root)
        stored = list(connection.execute('SELECT * FROM index_state ORDER BY key'))
        documents = list(connection.execute('SELECT * FROM documents'))
        self.malformed()
        with self.assertRaisesRegex(self.r.RetrievalError,'invalid-utf8-path'):
            self.c.index_repository(connection,self.root,incremental=True)
        self.assertEqual(stored,list(connection.execute('SELECT * FROM index_state ORDER BY key')))
        self.assertEqual(documents,list(connection.execute('SELECT * FROM documents')))

    def test_valid_unicode_direct_and_supplied_fingerprint_unchanged(self):
        p = self.write('.agents/skills/'+P['valid'])
        # Independent original serialization oracle; no production helper used.
        raw = ('.agents\0'+P['valid']+'\0'+str(P['mtime_ns'])+'\0'+str(len(P['body'].encode('utf-8')))+'\n').encode('utf-8')
        expected = hashlib.sha256(raw).hexdigest()
        self.assertEqual(expected,self.r.skill_tree_fingerprint(self.root))
        self.assertEqual(expected,self.r.skill_tree_fingerprint(self.root,[('.agents',P['valid'],P['mtime_ns'],len(p.read_bytes()))]))

    def test_valid_unicode_clean_and_readonly(self):
        paths = [self.write(e+'/skills/'+P['valid']) for e in ('.agents','.cursor')]
        before = [(p.read_bytes(),p.stat().st_mtime_ns) for p in paths]
        self.assertEqual([],self.r.skill_mirror_drift(self.root,'.agents'))
        self.assertEqual([],self.r.full_mirror_drift(self.root))
        self.assertEqual(before,[(p.read_bytes(),p.stat().st_mtime_ns) for p in paths])

    def test_valid_partial_install_still_skips_absent_class(self):
        self.write('.agents/skills/'+P['valid'])
        self.write('.cursor/skills/unrelated/latch.txt')
        self.assertEqual([],self.r.full_mirror_drift(self.root))

    def test_valid_drift_shapes_unchanged(self):
        self.write('.agents/skills/'+P['valid'])
        self.write('.cursor/skills/'+P['valid'],P['different'])
        self.assertEqual([{'logical_path':P['valid'],'canonical':'.agents','mismatched':['.cursor'],'reason':'content differs'}],self.r.skill_mirror_drift(self.root,'.agents'))
        self.assertEqual([{'path':'.cursor/skills/'+P['valid'],'class':'skills','reason':'differs from canon (.agents/skills/'+P['valid']+')'}],self.r.full_mirror_drift(self.root))

    @POSIX
    def test_cli_skills_refuses_with_fixed_error(self):
        self.malformed()
        self.assertEqual((1,'','context: '+P['error']+'\n'),self.cli('parity','--skills-only','--json'))

    @POSIX
    def test_cli_full_reverse_refuses_with_fixed_error(self):
        self.write('.agents/skills/'+P['valid'])
        self.malformed('.cursor',suffix='/latch.txt')
        self.assertEqual((1,'','context: '+P['error']+'\n'),self.cli('parity','--json'))

    def siblings(self,rel):
        base = self.root/'siblings'
        roots = [base/n for n in ('Laravel','Symfony')]
        for root in roots:
            self.write(rel,root=root)
        return roots

    @POSIX
    def test_cross_edition_matching_bad_keys_never_clean(self):
        roots = self.siblings('memory-bank/templates/'+self.bad+'.md')
        self.refused(lambda:self.r.cross_edition_drift(roots[0]))

    def test_cross_edition_unicode_and_drift_shapes_unchanged(self):
        rel = 'memory-bank/templates/tanager-é.md'
        roots = self.siblings(rel)
        self.assertEqual([],self.r.cross_edition_drift(roots[0]))
        self.write(rel,P['different'],root=roots[1])
        self.assertEqual([{'path':rel,'reason':'content differs','editions':['Laravel','Symfony']}],self.r.cross_edition_drift(roots[0]))

    @POSIX
    def test_ignored_suffix_and_link_do_not_become_policy_files(self):
        self.write('.agents/skills/'+P['valid'])
        self.write('.cursor/skills/'+P['valid'])
        self.malformed('.cursor',suffix='/cache.pyc')
        link = self.root/('.cursor/skills/'+self.bad+'/link.txt')
        link.symlink_to(self.root/('.agents/skills/'+P['valid']))
        self.assertEqual([],self.r.skill_mirror_drift(self.root,'.agents'))
        self.assertEqual([],self.r.full_mirror_drift(self.root))
        self.assertTrue(link.is_symlink())


if __name__ == '__main__':
    unittest.main()
