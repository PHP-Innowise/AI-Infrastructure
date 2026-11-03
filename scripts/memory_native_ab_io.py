"""Private, bounded physical images for the maintainer native-memory experiment."""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import tempfile
import time

from memory_delivery_proof import _json
from memory_pilot_support import ROOT, SKIP_PARTS, SKIP_PREFIXES, PilotError, public_text, relative_path

MAX_FILE = 4 * 1024 * 1024
MAX_TREE = 64 * 1024 * 1024
SKIP = SKIP_PARTS | {'.cache', '.codex', '.Codex', '.claude', '.cursor', '.agents'}
ALIAS = re.compile(r'[a-z0-9][a-z0-9_/-]{0,79}\Z')


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def digest(value):
    return sha(json.dumps(value, sort_keys=True, ensure_ascii=False, allow_nan=False).encode())


def ordinary(path):
    path = Path(os.path.abspath(path))
    for parent in reversed([path, *path.parents]):
        if parent.is_symlink(): raise PilotError('linked path refused')
    return path


def overlaps(a, b):
    a, b = Path(a).resolve(), Path(b).resolve()
    return a == b or a in b.parents or b in a.parents


def change_time_ns(image):
    """st_ctime_ns as stat and fstat both report it: Python 3.12+ on Windows
    gives os.stat() and os.lstat() the creation time there and os.fstat() the
    last change time, and both the creation time as st_birthtime_ns."""
    birth = getattr(image, 'st_birthtime_ns', None) if os.name == 'nt' else None
    return image.st_ctime_ns if birth is None else birth


def read_file(path, limit=MAX_FILE):
    path = ordinary(path)
    descriptors = []
    try:
        if os.name == 'posix':
            parent = os.open(path.anchor, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            descriptors.append(parent)
            for part in path.parts[1:-1]:
                parent = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=parent)
                descriptors.append(parent)
            fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent)
        else:
            fd = os.open(path, os.O_RDONLY | getattr(os,'O_BINARY',0))
        descriptors.append(fd)
        before = os.fstat(fd)
        if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_size > limit:
            raise PilotError('unsafe or oversized file')
        chunks, size = [], 0
        while True:
            block = os.read(fd, min(65536, limit + 1 - size))
            if not block: break
            chunks.append(block); size += len(block)
            if size > limit: raise PilotError('file byte limit')
        after = os.fstat(fd)
        named = os.stat(path.name,dir_fd=parent,follow_symlinks=False) if os.name=='posix' else path.lstat()
        identity = lambda s: (s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,change_time_ns(s),s.st_nlink,s.st_mode)
        if identity(before) != identity(after) or identity(after) != identity(named):
            raise PilotError('file changed while reading')
        return b''.join(chunks), stat.S_IMODE(before.st_mode) & 0o777
    finally:
        for descriptor in reversed(descriptors):os.close(descriptor)


def read_json(path):
    return _json(read_file(path, 1024 * 1024)[0])


def write_json(path, value):
    path = ordinary(path)
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2, allow_nan=False).encode() + b'\n'
    if len(raw) > MAX_TREE: raise PilotError('artifact byte limit')
    fd, name = tempfile.mkstemp(prefix='.native-ab-', dir=str(path.parent))
    try:
        with os.fdopen(fd, 'wb') as f: f.write(raw)
        os.replace(name, path)
    finally:
        if os.path.exists(name): os.unlink(name)


def load_config(path):
    value = read_json(path)
    required = {'schema','project_snapshot','source','served','model','tool_cap','deadline_seconds'}
    if not isinstance(value,dict) or set(value) != required or value['schema'] != 'memory-native-ab-v1':
        raise PilotError('invalid native experiment configuration')
    for name in ('source','served'):
        item = value[name]
        if not isinstance(item,dict) or set(item) != {'task_alias','prompt'}: raise PilotError('invalid task')
        if not isinstance(item['task_alias'],str) or not ALIAS.fullmatch(item['task_alias']): raise PilotError('invalid task alias')
        prompt = item['prompt']
        if not isinstance(prompt,str) or not 1 <= len(prompt.encode('utf-8')) <= 16000 or '\0' in prompt:
            raise PilotError('invalid prompt')
        public_text(prompt)
    if value['source']['task_alias'] == value['served']['task_alias']: raise PilotError('tasks must differ')
    if not isinstance(value['model'],str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,79}',value['model']):
        raise PilotError('invalid model')
    for name, maximum in [('tool_cap',100),('deadline_seconds',900)]:
        if type(value[name]) is not int or not 1 <= value[name] <= maximum: raise PilotError('invalid budget')
    if not isinstance(value['project_snapshot'],str) or not Path(value['project_snapshot']).is_absolute():
        raise PilotError('absolute project snapshot required')
    return value


def validate_roots(source, config, output):
    source, config, output = map(ordinary, (source,config,output))
    if not source.is_dir() or output.exists() or overlaps(source,ROOT) or overlaps(output,ROOT):
        raise PilotError('use a fresh private output and separate project snapshot')
    if any(overlaps(a,b) for a,b in [(source,config),(source,output),(config,output)]):
        raise PilotError('experiment paths overlap')
    if not output.parent.is_dir(): raise PilotError('output parent must already exist')
    return source,config,output


def check_deadline(deadline):
    if deadline is not None and time.monotonic() >= deadline:
        raise PilotError('collection deadline')


def snapshot(root, *, deadline=None):
    root = ordinary(root)
    if not root.is_dir(): raise PilotError('ordinary project directory required')
    from memory_pilot_support import canonical_module
    auto = canonical_module('automatic_query')
    result, total, visited = [], 0, 0
    for base, directories, files in os.walk(root,followlinks=False):
        check_deadline(deadline)
        folder = Path(base)
        visited += len(directories) + len(files) + 1
        if visited > 50000: raise PilotError('snapshot scan limit')
        def excluded(name):
            value = (folder/name).relative_to(root).as_posix()
            return (name in SKIP or name in {'.mcp.json','accelerator-attach.json','credentials.json'}
                    or name.casefold().startswith('.env') or (value+'/').startswith(SKIP_PREFIXES)
                    or auto.SOURCE_PATH_DENYLIST.search(value)
                    or any(part.casefold() in auto.BLOCKED_SOURCE_PARTS for part in Path(value).parts)
                    or Path(value).suffix.casefold() in auto.PRIVATE_SOURCE_SUFFIXES)
        directories[:] = sorted(n for n in directories if not excluded(n))
        for name in directories:
            if (folder/name).is_symlink(): raise PilotError('linked snapshot directory')
        for name in sorted(n for n in files if not excluded(n)):
            check_deadline(deadline)
            value = (folder/name).relative_to(root).as_posix();relative_path(value,memory=True)
            raw, mode = read_file(folder/name)
            total += len(raw)
            if total > MAX_TREE or len(result) >= 20000: raise PilotError('snapshot byte/file limit')
            result.append((value,raw,mode))
    return sorted(result)


def image_digest(image):
    return digest([(name,sha(raw),mode) for name,raw,mode in image])


def copy_image(image, destination):
    destination = ordinary(destination)
    destination.mkdir(mode=0o700)
    try:
        for name,raw,mode in image:
            relative_path(name,memory=True)
            path = destination/name;path.parent.mkdir(parents=True,exist_ok=True,mode=0o700)
            fd = os.open(path, os.O_WRONLY|os.O_CREAT|os.O_EXCL, 0o600)
            with os.fdopen(fd,'wb') as stream:stream.write(raw)
            path.chmod(mode & 0o777)
    except BaseException:
        shutil.rmtree(destination)
        raise
    return image_digest(image)
