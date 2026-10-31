"""Actual public read-isolation probe where user namespaces are available."""
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'scripts'))
import memory_outcome_isolation as isolation

class IsolationTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name); self.corpus=self.root/'corpus'; self.corpus.mkdir()
        self.private=self.root/'private'; self.private.mkdir()
        self.sentinel=self.private/'gold'; self.sentinel.write_text('PUBLIC-PRIVATE-CANARY')

    def test_unavailable_boundary_never_falls_back_to_host(self):
        with patch.object(isolation.shutil,'which',return_value=None):
            with self.assertRaises(isolation.IsolationError):
                isolation.prepare_agent_view(self.corpus,self.private)

    def test_allowlist_has_no_host_root_or_private_parent(self):
        with patch.object(isolation.shutil,'which',return_value='/usr/bin/bwrap'):
            view=isolation.prepare_agent_view(self.corpus,self.private)
        prefix=view['prefix']
        self.assertNotIn('--ro-bind / /',' '.join(prefix))
        self.assertNotIn(str(self.private),prefix)
        self.assertNotIn(str(self.root),prefix)
        self.assertIn('--clearenv',prefix)
        self.assertNotIn('OPENAI_API_KEY',view['env'])

    def test_relocated_controller_python_runtime_is_available_to_probe(self):
        prefix = self.root / 'relocated-python'; (prefix / 'bin').mkdir(parents=True)
        executable = prefix / 'bin/python3'; executable.write_text('public runtime fixture')
        with patch.object(isolation.shutil, 'which', return_value='/usr/bin/bwrap'), \
                patch.object(isolation.sys, 'base_prefix', str(prefix)), \
                patch.object(isolation.sys, 'executable', str(executable)):
            view = isolation.prepare_agent_view(self.corpus, self.private)
        mounted = [Path(view['prefix'][i + 1]) for i, value in enumerate(view['prefix']) if value == '--ro-bind']
        self.assertTrue(any(executable == root or root in executable.parents for root in mounted))

    def test_agent_cannot_read_private_sentinel(self):
        try: view=isolation.prepare_agent_view(self.corpus,self.private)
        except isolation.IsolationError as error: self.skipTest(str(error))
        if not isolation.probe_agent_view(view,self.sentinel):
            self.skipTest('bubblewrap/user namespaces unavailable; private runs refuse')
        link=self.corpus/'alias'
        try: link.symlink_to(self.sentinel)
        except OSError: self.skipTest('symlinks unavailable')
        self.assertTrue(isolation.probe_agent_view(view,self.sentinel))
        result=isolation.run_probe(view,[sys.executable,'-c',"from pathlib import Path; import sys; sys.exit(0 if not Path(sys.argv[1]).exists() else 1)",str(link)])
        self.assertEqual(0,result.returncode)
