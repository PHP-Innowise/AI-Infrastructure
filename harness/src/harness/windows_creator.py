"""Provider-neutral Creator write isolation through the native Codex sandbox.

The outer CLI enforces a per-invocation permission profile; agent flags cannot
widen it. No batch shell, global config edit, or unelevated fallback is used.
"""
from __future__ import annotations

from functools import lru_cache
import json
import os
from pathlib import Path
import shutil
import subprocess
import uuid
import stat

from .filesystem import existing_directory
from .filesystem import fs
from .windows_commands import command_argv


REQUIRED = ('Windows Creator requires Codex CLI with permission-profile support '
            'and a configured elevated Windows sandbox. Complete sandbox setup '
            'in native Codex first; unelevated execution is not accepted.')


@lru_cache(maxsize=8)
def _capability(executable, modified, size):
    try:
        result = subprocess.run(command_argv([executable, 'sandbox', '--help'], 'codex'),
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, timeout=5)
        help_text = result.stdout[:65536].decode(errors='replace')
        return result.returncode == 0 and all(option in help_text for option in
            ('--permission-profile', '--cd', '--include-managed-config'))
    except (OSError, ValueError, subprocess.TimeoutExpired):
        return False


def backend(executable=None):
    executable = executable or shutil.which('codex')
    if not executable:
        return None
    try:
        info = Path(executable).stat()
        if _capability(executable, info.st_mtime_ns, info.st_size):
            return 'windows-codex', executable
    except OSError:
        pass
    return None


def _toml(value):
    if isinstance(value, dict):
        return '{' + ','.join(json.dumps(str(key), ensure_ascii=False) + '=' + _toml(item)
                              for key, item in value.items()) + '}'
    if isinstance(value, bool):
        return 'true' if value else 'false'
    return json.dumps(value, ensure_ascii=False)


def validation_copy(target, workspace):
    """Copy accelerator inputs via retained handles; runtime writes stay disposable.

    Application evidence is still read from the original target by validators.
    Unrelated application files and the old disposable SQLite cache are omitted.
    """
    from .creator import allowed_output
    from .setup import _read
    directory = workspace / ('.harness-validation-' + uuid.uuid4().hex)
    directory.mkdir(mode=0o700)
    total, count = 0, 0
    def copy_tree(descriptor, prefix=''):
        nonlocal total, count
        for name in fs.listdir(descriptor):
            relative = prefix + name
            if not prefix and not allowed_output(relative):
                continue
            if relative in ('memory-bank/local', 'project-brain/local'):
                continue
            from .creator import publication_helpers
            publication_helpers().normalize_relative_path(relative)
            info = fs.stat(name, dir_fd=descriptor, follow_symlinks=False)
            if stat.S_ISDIR(info.st_mode):
                child = fs.open(name, os.O_RDONLY | fs.O_DIRECTORY | fs.O_NOFOLLOW, dir_fd=descriptor)
                try:
                    (directory / relative).mkdir()
                    copy_tree(child, relative + '/')
                finally:
                    fs.close(child)
            else:
                value = _read(descriptor, name, required=True)
                count += 1; total += value['bytes']
                if count > 50000 or total > 256 * 1024 * 1024:
                    raise ValueError('Accelerator validation copy exceeds its size limit.')
                destination = directory / relative
                destination.write_bytes(value['body'])
                # Avoid a Windows READONLY attribute that would block cache cleanup.
                if os.name != 'nt':
                    destination.chmod(value['mode'] & 0o777)
    descriptor = None
    try:
        descriptor = fs.open_target_directory(target)
        copy_tree(descriptor)
        return directory
    except BaseException:
        shutil.rmtree(directory)
        raise
    finally:
        if descriptor is not None:
            fs.close(descriptor)


def command(executable, workspace, target, argv, homes=(), common=None, source=None, protected_roots=()):
    workspace, target = existing_directory(workspace), existing_directory(target)
    control = existing_directory(workspace.parent)
    private_tmp = workspace / '.harness-tmp'
    private_tmp.mkdir(mode=0o700, exist_ok=True)
    existing_directory(private_tmp)
    # A random name prevents ambient user/project profile tables from merging
    # extra writable roots or an `extends` setting into the enforced profile.
    name = 'harnesscreator' + uuid.uuid4().hex
    entries = {':root': 'read', str(workspace): 'write', str(control): 'read', str(target): 'read'}
    for home in homes:
        home = existing_directory(home)
        for protected in (target, control, source, common, *protected_roots):
            if protected:
                left, right = os.path.normcase(str(home)), os.path.normcase(str(protected))
                try:
                    overlap = os.path.commonpath((left, right)) in (left, right)
                except ValueError:
                    overlap = False
                if overlap:
                    raise ValueError('Provider account state overlaps a protected Creator directory.')
        entries[str(home)] = 'write'
    for protected in (target, control, source, common, *protected_roots):
        if protected:
            entries[str(existing_directory(protected))] = 'read'
    profile = {'filesystem': entries, 'network': {'enabled': True},
               'workspace_roots': {str(workspace): True}}
    outer = command_argv([executable], 'codex')
    child = command_argv(list(argv))
    # A trusted Python trampoline changes only the child's private temporary
    # directory. argv remains a list all the way to CreateProcess.
    launcher = Path(__file__).with_name('windows_creator_child.py')
    import sys
    return [*outer, 'sandbox', '-c', 'windows.sandbox="elevated"',
            '-c', 'permissions=' + _toml({name: profile}),
            '-P', name, '-C', str(workspace), '--include-managed-config', '--',
            sys.executable, '-B', str(launcher), str(private_tmp), *child]
