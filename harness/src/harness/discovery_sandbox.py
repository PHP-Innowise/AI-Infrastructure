"""Discovery read allow-list: OS/CLI runtime, native account state, evidence only."""
from __future__ import annotations

import hashlib
from pathlib import Path
import shutil

from ai_system_lib import inside
from .creator import isolation_backend, provider_state_dirs, _sbpl
from .sessions import SessionError


def runtime_paths(command):
    executable = Path(command[0]).resolve()
    paths = {executable}
    # Packaged JS launchers resolve sibling platform packages through node_modules.
    for parent in executable.parents:
        if parent.name == 'node_modules':
            paths = {parent}
            break
    # Cursor's version directory contains index.js, Bun and helper binaries.
    if 'cursor-agent' in executable.parts and 'versions' in executable.parts:
        paths.add(executable.parent)
    for name in ('node', 'bun', 'python3', 'bash', 'sh', 'git', 'rg'):
        found = shutil.which(name)
        if found:
            paths.add(Path(found).resolve())
    return sorted(paths, key=str)


def sandbox_command(workspace, command, provider, source_roots):
    backend = isolation_backend()
    if not backend:
        raise SessionError('AI discovery requires bubblewrap or sandbox-exec.')
    workspace = Path(workspace).resolve()
    homes = [p.resolve() for p in provider_state_dirs(provider)]
    runtime = runtime_paths(command)
    # Account state is deliberately available to its native CLI for login/session data.
    # Source projects must never be hidden among those privileged directories.
    if any(inside(root, allowed) or inside(allowed, root) for root in source_roots for allowed in homes + runtime + [workspace.parent]):
        raise SessionError('Source folders overlap native CLI account/runtime directories. Choose ordinary project folders.')
    public = [Path(p) for p in ('/usr', '/bin', '/sbin', '/lib', '/lib64',
              '/etc/ssl/certs', '/etc/pki/tls/certs', '/etc/ca-certificates',
              '/etc/ld.so.cache', '/etc/ld.so.conf', '/etc/nsswitch.conf', '/etc/passwd',
              '/etc/group', '/etc/hosts', '/etc/resolv.conf') if Path(p).exists()]
    # Also refuse projects under OS runtime roots instead of silently exposing them.
    if any(inside(root, allowed.resolve()) or inside(allowed.resolve(), root) for root in source_roots for allowed in public):
        raise SessionError('Source folders must be outside OS runtime directories.')
    kind, executable = backend
    if kind == 'bwrap':
        args = [executable, '--die-with-parent', '--new-session', '--unshare-pid',
                '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/tmp']
        for path in public + runtime:
            args += ['--ro-bind', str(path), str(path)]
        for home in homes:
            args += ['--bind', str(home), str(home)]
        args += ['--ro-bind', str(workspace.parent), str(workspace.parent),
                 '--bind', str(workspace), str(workspace),
                 '--ro-bind', str(workspace / 'evidence'), str(workspace / 'evidence'),
                 '--chdir', str(workspace)]
        return args + ['--', *command]
    # macOS needs its standard frameworks and loader as well as CLI runtime paths.
    public += [Path(p) for p in ('/System', '/Library/Apple', '/private/etc/ssl',
               '/private/etc/hosts', '/private/etc/resolv.conf') if Path(p).exists()]
    if any(inside(root, allowed.resolve()) or inside(allowed.resolve(), root) for root in source_roots for allowed in public):
        raise SessionError('Source folders must be outside OS runtime directories.')
    allowed = public + runtime + homes + [workspace.parent]
    lines = ['(version 1)', '(allow default)', '(deny file-read*)', '(deny file-write*)']
    lines += [f'(allow file-read* (subpath {_sbpl(p)}))' for p in allowed]
    lines += [f'(allow file-write* (subpath {_sbpl(p)}))' for p in homes + [workspace]]
    lines += ['(allow file-read* file-write* (subpath "/dev"))',
              f'(deny file-write* (subpath {_sbpl(workspace / "evidence")}))']
    profile = workspace.parent / ('discovery-' + hashlib.sha256('\n'.join(lines).encode()).hexdigest()[:16] + '.sb')
    profile.write_text('\n'.join(lines) + '\n'); profile.chmod(0o600)
    temporary = workspace / 'tmp'; temporary.mkdir(mode=0o700, exist_ok=True)
    environment = ['/usr/bin/env'] + [f'{name}={temporary}' for name in ('TMPDIR', 'TMP', 'TEMP')]
    return [executable, '-f', str(profile), '--', *environment, *command]
