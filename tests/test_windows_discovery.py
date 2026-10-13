"""Opt-in native Windows acceptance for the AI discovery sandbox; no model calls.

Runs only with HARNESS_WINDOWS_CREATOR_SANDBOX=1 on a Windows runner whose Codex
CLI has its elevated sandbox set up: the runner of the manual Windows Creator
sandbox workflow.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
sys.path.insert(0, str(ROOT / "harness/src"))
import ai_system_execution as execution
from harness import discovery_runner, discovery_sandbox, windows_creator
from harness.sessions import SessionError


@unittest.skipUnless(os.name == "nt" and os.environ.get("HARNESS_WINDOWS_CREATOR_SANDBOX") == "1",
                     "native elevated Codex sandbox acceptance")
class WindowsDiscoverySandboxTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="discovery-windows-"))
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.codex = os.environ.get("HARNESS_CODEX_BIN") or shutil.which("codex")
        self.assertIsNotNone(windows_creator.backend(self.codex), windows_creator.REQUIRED)
        self.directory = self.root / "state" / "system-discovery" / "run"
        self.workspace = self.directory / "agent"
        (self.workspace / "evidence").mkdir(parents=True)
        (self.workspace / "evidence" / "000000.txt").write_text("safe evidence", encoding="utf-8")
        self.source = self.root / "services" / "orders"
        (self.source / "src").mkdir(parents=True)
        (self.source / "src" / "order.py").write_text("ORIGINAL SOURCE", encoding="utf-8")
        (self.source / ".env").write_text("SECRET=1", encoding="utf-8")
        self.launcher, self.probe = discovery_runner.helpers(self.directory)
        self.agent = [sys.executable, "-c", "pass"]

    def verify(self):
        discovery_sandbox.verify_boundary(self.directory, self.workspace, self.agent, "codex", [self.source],
                                          self.codex, self.launcher, self.probe, execution.run_process)

    def unreadable_to_the_scan(self, *paths):
        check = self.directory / "check.py"
        check.write_text("import sys\nfrom pathlib import Path\n"
                         "for name in sys.argv[1:]:\n"
                         "    try: Path(name).read_bytes()\n"
                         "    except (FileNotFoundError, PermissionError): pass\n"
                         "    else: raise SystemExit('readable: ' + name)\n"
                         "assert Path('evidence/000000.txt').read_text() == 'safe evidence'\n"
                         "Path('scratch.txt').write_text('allowed')\n", encoding="utf-8")
        command = discovery_sandbox.sandbox_command(self.workspace, self.agent, "codex", [self.source], self.codex,
                                                    [discovery_sandbox.python(), str(check), *map(str, paths)],
                                                    self.launcher)
        result = execution.run_process(command, self.workspace, "", 180, stderr_tail=True)
        self.assertEqual(0, result["returncode"], result)

    def test_boundary_holds_for_original_files_and_the_scan_reads_only_evidence(self):
        self.verify()
        self.unreadable_to_the_scan(self.source / "src" / "order.py", self.source / ".env")

    def test_a_file_with_its_own_allow_entry_is_never_read(self):
        exposed = self.source / "shared.txt"
        exposed.write_text("EXPLICITLY SHARED", encoding="utf-8")
        granted = subprocess.run(["icacls", str(exposed), "/grant", "*S-1-1-0:(R)"],
                                 capture_output=True, text=True, check=False)
        self.assertEqual(0, granted.returncode, granted.stdout + granted.stderr)
        try:
            self.verify()
        except SessionError:
            return  # The probe found the file readable, so no scan would start.
        self.unreadable_to_the_scan(exposed)


if __name__ == "__main__":
    unittest.main()
