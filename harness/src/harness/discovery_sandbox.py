"""Discovery read allow-list: OS/CLI runtime, native account state, evidence only."""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

from ai_system_lib import inside, open_directory
from .creator import isolation_backend, provider_state_dirs, _sbpl
from .filesystem import fs
from .sessions import SessionError


GUIDE = 'See "Troubleshooting AI discovery" in docs/AI-SYSTEM-ORCHESTRATION.md.'
NATIVE_WINDOWS = os.name == 'nt'
# Windows scans run in Codex's elevated sandbox, as Creator phases do.
WINDOWS_REQUIRED = ('AI discovery on Windows needs the Codex CLI with permission profiles and its elevated '
                    'sandbox, even when the scan uses Claude or Cursor. Complete the sandbox setup in native '
                    'Codex first. ' + GUIDE)
# Before a Windows scan, every original file and folder is opened inside the sandbox, as many as
# Creator's own boundary check covers.
PROBE_LIMIT = 100000
TOO_LARGE = ('The selected folders hold more than 100,000 files and folders, more than the Windows sandbox '
             'check covers. Scan fewer or smaller service folders. ' + GUIDE)


def _sysctl(name):
    try:
        return Path('/proc/sys', *name.split('.')).read_text().strip()
    except OSError:
        return None


def namespace_hint():
    """Why this host may refuse unprivileged user namespaces, which bubblewrap needs."""
    if _sysctl('kernel.apparmor_restrict_unprivileged_userns') == '1':
        return ('AppArmor restricts unprivileged user namespaces on this host (Ubuntu 23.10 and later). '
                'Allow bubblewrap with an AppArmor profile, or set kernel.apparmor_restrict_unprivileged_userns=0. ')
    if _sysctl('kernel.unprivileged_userns_clone') == '0':
        return 'Unprivileged user namespaces are disabled: set kernel.unprivileged_userns_clone=1. '
    if _sysctl('user.max_user_namespaces') == '0':
        return 'user.max_user_namespaces is 0: raise it to allow user namespaces. '
    return 'User namespaces may be blocked by a container or security policy; run Harness on the host. '


def _probe(executable):
    # The namespaces and mounts every discovery sandbox needs, around a no-op.
    return subprocess.run([executable, '--unshare-pid', '--ro-bind', '/', '/', '--proc', '/proc',
                           '--dev', '/dev', 'true'], stdin=subprocess.DEVNULL, capture_output=True,
                          text=True, errors='replace', timeout=10, check=False)


def sandbox_problem(codex=None):
    """None when the isolation backend starts a process here; otherwise an actionable reason.

    codex is the available Codex CLI, whose sandbox confines a scan on Windows.
    """
    if NATIVE_WINDOWS:
        return None if codex and isolation_backend(codex) else WINDOWS_REQUIRED
    backend = isolation_backend()
    if not backend:
        return 'AI discovery requires bubblewrap on Linux or sandbox-exec on macOS. ' + GUIDE
    kind, executable = backend
    if kind != 'bwrap':
        return None
    try:
        probe = _probe(executable)
    except (OSError, subprocess.TimeoutExpired):
        return 'bubblewrap could not be started. Check its installation. ' + GUIDE
    if probe.returncode == 0:
        return None
    detail = next((line.strip() for line in reversed(probe.stderr.splitlines()) if line.strip()), 'no details')
    return ('The AI discovery sandbox cannot start (' + detail[:200] + '). ' + namespace_hint() + GUIDE)


def runtime_paths(command):
    paths = set()
    # On Windows an npm CLI starts as node and its package script, so both are the CLI's.
    for index, value in enumerate(command[:2] if NATIVE_WINDOWS else command[:1]):
        executable = Path(value).resolve()
        if index and not executable.is_file():
            continue
        # Packaged JS launchers resolve sibling platform packages through node_modules.
        paths.add(next((parent for parent in executable.parents if parent.name == 'node_modules'), executable))
        # Cursor's version directory contains index.js, Bun and helper binaries.
        if 'cursor-agent' in executable.parts and 'versions' in executable.parts:
            paths.add(executable.parent)
    # Windows grants no helper lookups: a Store alias such as python3 is a reparse point.
    for name in () if NATIVE_WINDOWS else ('node', 'bun', 'python3', 'bash', 'sh', 'git', 'rg'):
        found = shutil.which(name)
        if found:
            paths.add(Path(found).resolve())
    return sorted(paths, key=str)


def python():
    """The interpreter itself, not a link to it, for the trampoline and the probe. They use only the
    standard library, and a sandbox need not show the folder a link sits in."""
    return str(Path(sys.executable).resolve())


def codex_runtime(executable):
    """Python for the trusted trampoline, and the Codex CLI whose sandbox helpers start the scan."""
    from .windows_commands import command_argv
    outer = command_argv([executable], 'codex')
    paths = {Path(sys.base_prefix).resolve(), Path(sys.prefix).resolve(), Path(python()), *runtime_paths(outer)}
    if len(outer) == 1:
        paths.add(Path(outer[0]).resolve().parent)  # A standalone CLI keeps its helpers beside it.
    return sorted(paths, key=str)


def sandbox_command(workspace, command, provider, source_roots, codex=None, inner=None, launcher=None):
    """command inside the discovery sandbox; inner, when given, runs there instead under the same profile.

    codex is the Codex CLI that sandboxes a Windows scan; launcher is a trusted copy of its trampoline.
    """
    backend = isolation_backend(codex) if NATIVE_WINDOWS else isolation_backend()
    if not backend or backend[0] not in ('bwrap', 'seatbelt', 'windows-codex'):
        raise SessionError(WINDOWS_REQUIRED if NATIVE_WINDOWS else 'AI discovery requires bubblewrap or sandbox-exec.')
    kind, executable = backend
    inner = command if inner is None else inner
    workspace = Path(workspace).resolve()
    homes = [p.resolve() for p in provider_state_dirs(provider)]
    runtime = runtime_paths(command) + (codex_runtime(executable) if kind == 'windows-codex' else [])
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
        return args + ['--', *inner]
    if kind == 'windows-codex':
        from .windows_creator import sandbox_argv
        # An allow-list like the other sandboxes: platform minimum, CLI runtime, run folder and account
        # state. Each source folder is also denied outright, because Codex's sandbox account may read
        # a folder outside the user profile that Windows permissions leave open to every user.
        entries = {':minimal': 'read'}
        entries.update((str(path), 'read') for path in [*runtime, workspace.parent])
        entries.update((str(home), 'write') for home in homes)
        entries[str(workspace)] = 'write'
        entries[str(workspace / 'evidence')] = 'read'
        entries.update((str(root), 'deny') for root in source_roots)
        return sandbox_argv(executable, workspace, entries, inner, launcher, 'harnessdiscovery', python())
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
    return [executable, '-f', str(profile), '--', *environment, *inner]


def original_paths(roots):
    """Every file and folder under the source roots, without following links; a link is listed itself."""
    paths = []

    def visit(descriptor, folder, depth):
        if depth > 64:
            raise SessionError(TOO_LARGE)
        try:
            with fs.scandir(descriptor) as entries:
                names = sorted(entry.name for entry in entries)
        except OSError:
            return  # Unlisted here; the probe still checks this folder.
        for name in names:
            if len(paths) >= PROBE_LIMIT:
                raise SessionError(TOO_LARGE)
            path = folder / name
            paths.append(str(path))
            try:
                if not stat.S_ISDIR(fs.stat(name, dir_fd=descriptor, follow_symlinks=False).st_mode):
                    continue
                child = fs.open(name, os.O_RDONLY | fs.O_DIRECTORY | fs.O_NOFOLLOW, dir_fd=descriptor)
            except (OSError, ValueError):
                continue  # A link or a Windows reparse point: probed, never followed.
            try:
                visit(child, path, depth + 1)
            finally:
                fs.close(child)

    for root in roots:
        if any(other != root and inside(Path(root), Path(other)) for other in roots):
            continue  # A service folder inside the system folder is walked with it.
        paths.append(str(root))
        descriptor = open_directory(root)
        try:
            visit(descriptor, Path(root), 0)
        finally:
            fs.close(descriptor)
    return paths


def verify_boundary(directory, workspace, command, provider, source_roots, codex, launcher, probe, run):
    """Before a Windows scan, prove inside the scan's own sandbox that no original file is readable.

    Codex denies each source folder with an inherited Windows ACE, which a file with its own allow
    entry or a protected DACL escapes. So the probe opens every original file and folder, and a
    sentinel in the user's temporary folder, which the sandbox account must not reach either.
    run is the process runner; the scan starts only if this returns.
    """
    paths = original_paths(source_roots)
    descriptor, sentinel = tempfile.mkstemp(prefix='harness-discovery-sentinel-')
    os.close(descriptor)
    listing = Path(directory) / 'boundary-paths.txt'
    evidence = Path(workspace) / 'evidence'
    try:
        readable = sorted(str(path) for path in evidence.iterdir())[:3]
        listing.write_text('\n'.join([*paths, sentinel, '--readable', *readable]) + '\n', encoding='utf-8')
        checks = [python(), '-B', str(probe), str(listing), str(workspace), str(evidence)]
        result = run(sandbox_command(workspace, command, provider, source_roots, codex, checks, launcher),
                     workspace, '', 900, stderr_tail=True)
    finally:
        listing.unlink(missing_ok=True)
        Path(sentinel).unlink(missing_ok=True)
    if not result['error'] and result['returncode'] == 0:
        return
    found = [line.strip() for line in result['stdout'].decode('utf-8', 'replace').splitlines()
             if line.startswith(('The sandbox could', 'The AI discovery sandbox boundary'))]
    if found:  # The probe ran and named what it could reach.
        raise SessionError('The Codex sandbox did not keep the original folders unreadable, so AI discovery did not '
                           'start. ' + found[-1][:400] + ' ' + GUIDE)
    errors = [line.strip() for line in result.get('stderr_tail', b'').decode('utf-8', 'replace').splitlines() if line.strip()]
    detail = errors[-1][:300] if errors else result['error'] or 'exit code ' + str(result['returncode'])
    raise SessionError('The Codex sandbox could not start the check that runs before every Windows scan ('
                       + detail + '). ' + WINDOWS_REQUIRED)
