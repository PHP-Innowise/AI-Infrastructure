"""Shell launcher contract, exercised without a real Harness server."""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "harness-server"


class HarnessLauncherTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        git = shutil.which("git")
        bundled = Path(git).parents[1] / "bin/bash.exe" if git and os.name == "nt" else None
        self.bash = str(bundled) if bundled and bundled.is_file() else shutil.which("bash")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.launcher = self.root / "harness-server"
        shutil.copy2(LAUNCHER, self.launcher)
        self.web = self.root / "harness" / "src" / "harness" / "web.py"
        self.web.parent.mkdir(parents=True)
        self.web.write_text("# launcher fixture\n", encoding="utf-8")

    def interpreter(self, name, *, valid, label):
        path = self.bin / name
        if valid:
            body = f'''#!/usr/bin/env bash
if [ "$1" = "-c" ]; then
    exec "$HARNESS_REAL_PYTHON" "$@"
fi
printf '%s\\0' "{label}" "$@" > "$HARNESS_LAUNCH_LOG"
exit 23
'''
        else:
            body = "#!/usr/bin/env bash\nexit 1\n"
        path.write_text(body, encoding="utf-8")
        path.chmod(0o755)

    def run_launcher(self, *args):
        log = self.root / "argv.bin"
        environment = {
            **os.environ,
            "PATH": str(self.bin) + os.pathsep + os.environ.get("PATH", ""),
            "HARNESS_REAL_PYTHON": sys.executable.replace("\\", "/"),
            "HARNESS_LAUNCH_LOG": str(log).replace("\\", "/"),
        }
        completed = subprocess.run(
            [self.bash, str(self.launcher), *args], cwd=self.root, env=environment,
            capture_output=True, text=True, check=False,
        )
        values = log.read_bytes().split(b"\0")[:-1] if log.exists() else []
        decoded = [value.decode() for value in values]
        if os.name == 'nt' and len(decoded) > 1:
            converted = subprocess.run([self.bash, '-c', 'cygpath -w "$1"', 'harness-test', decoded[1]],
                                       capture_output=True, text=True, check=True)
            decoded[1] = converted.stdout.strip()
        return completed, decoded

    def test_prefers_python3_when_it_meets_the_310_floor(self):
        self.interpreter("python3", valid=True, label="python3")
        self.interpreter("python", valid=True, label="python")
        completed, argv = self.run_launcher("status", "--state-dir", "state folder")
        self.assertEqual(completed.returncode, 23, completed.stderr)
        self.assertEqual(argv, ["python3", str(self.web), "status", "--state-dir", "state folder"])

    def test_falls_back_to_python_and_preserves_literal_arguments(self):
        self.interpreter("python3", valid=False, label="python3")
        self.interpreter("python", valid=True, label="python")
        literal = 'state & | < > ^ % ! "folder with spaces"'
        completed, argv = self.run_launcher("status", "--state-dir", literal)
        self.assertEqual(completed.returncode, 23, completed.stderr)
        self.assertEqual(argv, ["python", str(self.web), "status", "--state-dir", literal])

    def test_rejects_interpreters_below_python_310_or_missing_from_path(self):
        for name in ("python3", "python"):
            path = self.bin / name
            path.write_text("#!/usr/bin/env bash\nif [ \"$1\" = \"-c\" ]; then exit 1; fi\nexit 99\n", encoding="utf-8")
            path.chmod(0o755)
        completed, argv = self.run_launcher("status")
        self.assertEqual(argv, [])
        self.assertEqual(completed.returncode, 127)
        self.assertIn("Python 3.10+ is required", completed.stderr)


if __name__ == "__main__":
    unittest.main()
