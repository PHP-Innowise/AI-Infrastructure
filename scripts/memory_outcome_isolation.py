"""Maintainer-only Linux read allowlist for private task-outcome experiments.

There is no host fallback. Account access is the native CLI's auth.json alone;
private labels, controller inputs and sibling corpora are never mounted. This
outer mount namespace shares networking for the native provider transport; the
native workspace-write tool sandbox continues to control tool networking.
"""
from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import sys
from typing import Any, Dict, Sequence


class IsolationError(ValueError):
    pass


PUBLIC = ('/usr', '/bin', '/sbin', '/lib', '/lib64', '/etc/ssl/certs', '/etc/ssl/cert.pem',
          '/etc/pki/tls/certs', '/etc/ld.so.cache', '/etc/nsswitch.conf',
          '/etc/passwd', '/etc/group', '/etc/hosts', '/etc/resolv.conf')


def overlaps(a: Path, b: Path) -> bool:
    a, b = a.resolve(), b.resolve()
    return a == b or a in b.parents or b in a.parents


def prepare_agent_view(corpus: Path, private_root: Path, *, command: Sequence[str] = (),
                       credentials: bool = True, citation_snapshot: Path = None) -> Dict[str, Any]:
    if sys.platform != 'linux' or not shutil.which('bwrap'):
        raise IsolationError('task-complete requires Linux bubblewrap read isolation')
    corpus, private_root = corpus.resolve(), private_root.resolve()
    if not corpus.is_dir() or overlaps(corpus, private_root):
        raise IsolationError('private inputs overlap the agent corpus')
    roots = [Path(path) for path in PUBLIC if Path(path).exists()]
    # The probe and trusted Python oracle need the actual interpreter's
    # standard library too (e.g. a uv installation outside /usr). This is
    # one runtime prefix, never its user's home or sibling installations.
    python_runtime = Path(sys.base_prefix).resolve()
    if not any(overlaps(python_runtime, root) for root in roots):
        roots.append(python_runtime)
    if command:
        executable = shutil.which(command[0])
        if executable is None:
            raise IsolationError('native executable unavailable')
        resolved = Path(executable).resolve()
        # JS launchers load their own package; standalone native helpers may
        # live beside the executable. Never expose an entire home directory.
        package = next((p for p in resolved.parents if p.name == 'node_modules'), resolved.parent)
        if not any(overlaps(resolved, root) for root in roots):
            roots.append(package)
    auth = Path(os.environ.get('CODEX_HOME', str(Path.home() / '.codex'))) / 'auth.json'
    if any(overlaps(private_root, root) or overlaps(corpus, root) for root in roots):
        raise IsolationError('private inputs or corpus overlap allowed runtime roots')
    if credentials and auth.is_file() and overlaps(private_root, auth):
        raise IsolationError('private inputs overlap native account state')
    prefix = [shutil.which('bwrap'), '--die-with-parent', '--new-session', '--unshare-pid',
              '--tmpfs', '/', '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/tmp',
              '--dir', '/home', '--dir', '/home/agent', '--dir', '/home/agent/.codex']
    for root in roots:
        prefix += ['--ro-bind', str(root), str(root)]
    prefix += ['--bind', str(corpus), str(corpus), '--chdir', str(corpus), '--clearenv',
               '--setenv', 'HOME', '/home/agent', '--setenv', 'CODEX_HOME', '/home/agent/.codex',
               '--setenv', 'PATH', '/usr/local/bin:/usr/bin:/bin', '--setenv', 'LANG', 'C.UTF-8']
    if citation_snapshot is not None:
        from memory_response_intake import MOUNT_NAME
        from memory_native_ab_io import ordinary
        citation_snapshot = ordinary(citation_snapshot)
        if (not citation_snapshot.is_dir() or overlaps(citation_snapshot, corpus)
                or overlaps(citation_snapshot, private_root)
                or any(overlaps(citation_snapshot, root) for root in roots)):
            raise IsolationError('citation snapshot overlaps private or mutable roots')
        mount = corpus / MOUNT_NAME
        if mount.exists() or mount.is_symlink():
            raise IsolationError('reserved citation mount path already exists')
        mount.mkdir(mode=0o700)
        prefix += ['--ro-bind', str(citation_snapshot), str(mount)]
    if credentials and auth.is_file():
        prefix += ['--ro-bind', str(auth.resolve()), '/home/agent/.codex/auth.json']
    normalized = [str(Path(shutil.which(command[0])).resolve()), *command[1:]] if command else []
    return {'prefix': prefix, 'command': normalized, 'env': {}, 'corpus': str(corpus),
            'backend': 'linux-bwrap-allowlist-v1'}


def run_probe(view: Dict[str, Any], command: Sequence[str]) -> subprocess.CompletedProcess:
    executable = shutil.which(command[0])
    normalized = [str(Path(executable).resolve()), *command[1:]] if executable else list(command)
    return subprocess.run([*view['prefix'], '--', *normalized], cwd=view['corpus'],
                          env=view['env'], capture_output=True, timeout=10)


def probe_agent_view(view: Dict[str, Any], sentinel: Path) -> bool:
    """Prove both that this namespace runs and that open() cannot read gold.

    An unavailable namespace is false, never evidence of a successful denial.
    No sentinel contents enter argv, output or persistent metadata.
    """
    code = ('import pathlib,sys; p=pathlib.Path(sys.argv[1]); '
            'accessible=p.exists(); '
            'print("OUTCOME-READ-DENIED" if not accessible else "OUTCOME-READ-EXPOSED"); '
            'sys.exit(1 if accessible else 0)')
    try:
        result = run_probe(view, [str(Path(sys.executable).resolve()), '-c', code, str(sentinel.resolve())])
        return result.returncode == 0 and result.stdout.strip() == b'OUTCOME-READ-DENIED'
    except (OSError, subprocess.TimeoutExpired):
        return False


def verifier_command(snapshot: Path, argv: Sequence[str], read_roots: Sequence[str]) -> Sequence[str]:
    """Private verifier runs only after the agent exits, in its own namespace.

    It may read its sealed implementation/labels, cannot change those mounts,
    sees no host home, and has no network. The candidate may write only in its
    disposable verification copy. No gold-bearing feedback returns to the agent.
    """
    sentinel_root = snapshot.parent / '.not-an-agent-input'
    view = prepare_agent_view(snapshot, sentinel_root, command=argv, credentials=False)
    prefix = list(view['prefix'])
    for raw in read_roots:
        path = Path(raw).resolve()
        if not path.exists() or overlaps(path, snapshot):
            raise IsolationError('verifier read root unavailable or overlaps candidate')
        prefix += ['--ro-bind', str(path), str(path)]
    return [*prefix, '--unshare-net', '--', *view['command']]
