"""Trusted foreground trampoline; the outer Windows sandbox owns descendants."""
import os
import subprocess
import sys

if __name__ == '__main__':
    environment = os.environ.copy()
    for key in ('TMP', 'TEMP', 'TMPDIR'):
        environment[key] = sys.argv[1]
    child = subprocess.Popen(sys.argv[2:], env=environment)
    raise SystemExit(child.wait())
