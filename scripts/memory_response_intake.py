"""Opt-in v2 custody and isolated literal evidence for the completion stand.

Private references never enter the public citation image or effective prompt.
This is evaluation tooling, not an installed memory or retrieval policy.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import time

import memory_response_contract as contract

MOUNT_NAME = '.memory-citation-corpus'
MAX_PROMPT_BYTES = 2 * 1024 * 1024
MAX_PACKET_BYTES = 32 * 1024 * 1024
MAX_RECEIPT_BYTES = 8 * 1024 * 1024


def digest(value):
    return hashlib.sha256(json.dumps(value, ensure_ascii=False, sort_keys=True,
        separators=(',', ':'), allow_nan=False).encode('utf-8')).hexdigest()


def configuration(task, case):
    """Validate explicit opt-in; the legacy answer/patch oracle stays separate."""
    public = contract.validate_contract(task['response_contract'])
    if public['schema'] != 'memory-response-contract-v2' or public['case_id'] != task['id']:
        raise ValueError('completion intake requires v2 and the exact task case_id')
    private = case.get('response')
    if type(private) is not dict or set(private) != {'reference', 'answer_field'}:
        raise ValueError('v2 requires a private reference and explicit answer_field')
    field = private['answer_field']
    if task['task_mode'] == 'answer':
        if (type(field) is not str or public['answer_fields'] != {
                field: {'type': 'string', 'nullable': False}}
                or type(case.get('equals')) is not str):
            raise ValueError('answer intake grades exactly one nonnullable string field')
    elif field is not None:
        raise ValueError('patch intake requires null answer_field')
    contract.preflight_reference(public, private['reference'])
    if field is not None:
        value = json.loads(private['reference'].split('\n', 1)[1])['answer'][field]
        if value.strip() != case['equals'].strip():
            raise ValueError('private reference disagrees with the answer oracle')
    return public


def effective_prompt(task, case):
    public = configuration(task, case)
    if type(task.get('prompt')) is not str or '\0' in task['prompt']:
        raise ValueError('invalid v2 task prompt')
    field = case['response']['answer_field']
    instructions = ('\n\nCitation evidence is the immutable pre-task snapshot under '
        + MOUNT_NAME + '/. Read the listed path beneath that directory; cite its '
        'original relative path, without the snapshot prefix. Edits to working '
        'files do not change these citation bytes. Unlisted runtime files remain '
        'ineligible. The complete corpus is common to both arms and is additional '
        'public context; it is not an absence-of-memory control.\n')
    if field is not None:
        instructions += ('The task answer graded by the private answer oracle is '
                         'the string field ' + json.dumps(field) + '.\n')
    else:
        instructions += 'Patch correctness is graded independently of response shape and literal citations.\n'
    raw = (task['prompt'] + instructions + contract.render_contract(public)).encode('utf-8')
    if len(raw) > MAX_PROMPT_BYTES:
        raise ValueError('complete effective prompt exceeds the intake byte bound')
    return raw


def source_map(root, public, *, exact=False):
    """Read only caller-approved paths, never model citation paths."""
    from memory_native_ab_io import ordinary, read_file
    from memory_pilot_support import public_text, relative_path
    root = ordinary(root)
    if not root.is_dir():
        raise ValueError('citation snapshot must be an ordinary existing directory')
    sources, rows = {}, []
    expected_dirs = {'.'}
    for entry in public['citation_corpus']['files']:
        relative = relative_path(entry['path'], memory=True)
        path = root.joinpath(*relative.parts)
        if stat.S_IMODE(path.lstat().st_mode) & 0o7000:
            raise ValueError('special citation permissions refused')
        raw, mode = read_file(path, contract.CORPUS_MAX_FILE_BYTES)
        public_text(raw.decode('utf-8'))
        sources[entry['path']] = raw
        rows.append([entry['path'], hashlib.sha256(raw).hexdigest(), len(raw), mode])
        expected_dirs.update(parent.as_posix() for parent in relative.parents)
    # Even an invalid answer validates the entire source context first.
    contract.check_citations(public, '', sources)
    if exact:
        found_files, found_dirs = set(), {'.'}
        def fail(error):
            raise error
        for base, directories, files in os.walk(root, followlinks=False, onerror=fail):
            for name in [*directories, *files]:
                path = Path(base) / name
                relative = path.relative_to(root).as_posix()
                image = path.lstat()
                if stat.S_ISDIR(image.st_mode):
                    found_dirs.add(relative)
                    if stat.S_IMODE(image.st_mode) & 0o7000:
                        raise ValueError('special citation directory permissions refused')
                elif stat.S_ISREG(image.st_mode):
                    found_files.add(relative)
                else:
                    raise ValueError('citation snapshot contains a link or special file')
                if len(found_dirs) + len(found_files) > 8192:
                    raise ValueError('citation snapshot entry bound exceeded')
        if found_files != set(sources) or found_dirs != expected_dirs:
            raise ValueError('citation snapshot membership changed')
        rows += [[name, 'directory', stat.S_IMODE((root / name).stat().st_mode)]
                 for name in sorted(found_dirs)]
    return sources, digest(rows)


def prepare(task, case, root, target):
    public = configuration(task, case)
    sources, _ = source_map(root, public)
    if not contract.check_citations(public, case['response']['reference'], sources)['valid']:
        raise ValueError('private reference has invalid literal citations')
    target.mkdir(mode=0o700)
    for name, raw in sources.items():
        path = target / name
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open('xb') as stream:
            stream.write(raw)
        path.chmod(0o444)
    _, image = source_map(target, public, exact=True)
    prompt = effective_prompt(task, case)
    return {'schema': 'memory-response-intake-v1', 'path': str(target),
            'source_image_sha256': image, 'contract_sha256': contract.contract_digest(public),
            'prompt_sha256': hashlib.sha256(prompt).hexdigest(), 'prompt_bytes': len(prompt)}


def validate_prepared(task, case, prepared, root=None):
    public = configuration(task, case)
    prompt = effective_prompt(task, case)
    sources, image = source_map(Path(prepared['path']) if root is None else root, public, exact=True)
    if (prepared.get('schema') != 'memory-response-intake-v1'
            or image != prepared.get('source_image_sha256')
            or contract.contract_digest(public) != prepared.get('contract_sha256')
            or hashlib.sha256(prompt).hexdigest() != prepared.get('prompt_sha256')
            or len(prompt) != prepared.get('prompt_bytes')):
        raise ValueError('sealed response corpus, contract or effective prompt changed')
    return sources, prompt


def _strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError('duplicate intake JSON key')
            result[key] = value
        return result
    def invalid(value):
        raise ValueError('nonfinite intake JSON value')
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=invalid)


def evidence(packet):
    """Pure worker operation over complete immutable bytes, not candidate files."""
    public = contract.validate_contract(packet['contract'])
    sources = {name: base64.b64decode(raw, validate=True) for name, raw in packet['sources'].items()}
    # Context drift has precedence over response errors and remains unknown.
    contract.check_citations(public, '', sources)
    capture = packet['capture']
    if (type(capture) is not dict or set(capture) != {'schema', 'messages', 'overflow'}
            or capture['schema'] != 'memory-assistant-capture-v1'
            or type(capture['overflow']) is not bool):
        raise ValueError('invalid current-run assistant capture')
    artifact = contract.make_artifact(public, capture['messages'], packet['context'])
    selected = artifact['selection']
    checked = contract.check_citations(public, selected['text'], sources)
    if capture['overflow']:
        checked = {'valid': False, 'errors': ['assistant transcript exceeded declared limits']}
    answer = None
    if checked['valid']:
        field = packet['answer_field']
        answer = (json.loads(selected['text'].split('\n', 1)[1])['answer'][field]
                  if field is not None else selected['text'])
    return {'schema': 'memory-response-evidence-v1', 'artifact': artifact,
            'citation_check': checked, 'answer_text': answer}


def grade(snapshot, capture_path, case, task, prepared, citation_root, context, *, capture_sha256=None):
    """Isolated literal checks followed by the existing independent task oracle."""
    from memory_native_ab_io import read_file
    from memory_outcome_isolation import IsolationError, verifier_command
    from memory_outcome_verify import verify_task, verifier_digest
    started = time.monotonic()
    result = {'status': 'unavailable', 'correct': None, 'verifier_digest': None,
              'artifact_digest': None, 'seconds': None}
    try:
        result['verifier_digest'] = verifier_digest(case)
        if context['verifier_sha256'] != result['verifier_digest']:
            raise ValueError('current response verifier association changed')
        sources, _ = validate_prepared(task, case, prepared, citation_root)
        if context['source_image_sha256'] != prepared['source_image_sha256']:
            raise ValueError('current response source association changed')
        capture = _strict_json(read_file(capture_path, 8 * 1024 * 1024)[0])
        if capture_sha256 is not None and digest(capture) != capture_sha256:
            raise ValueError('current-run assistant capture association changed')
        packet = {'contract': task['response_contract'], 'capture': capture, 'context': context,
                  'answer_field': case['response']['answer_field'],
                  'sources': {name: base64.b64encode(raw).decode('ascii') for name, raw in sources.items()}}
        raw = json.dumps(packet, ensure_ascii=False, allow_nan=False).encode('utf-8')
        if len(raw) > MAX_PACKET_BYTES:
            raise ValueError('response packet byte bound exceeded')
        with tempfile.TemporaryDirectory(prefix='response-oracle-', dir=snapshot.parent) as temporary:
            private = Path(temporary) / 'private'; private.mkdir(mode=0o700)
            candidate = Path(temporary) / 'candidate'; candidate.mkdir()
            request = private / 'request.json'; request.write_bytes(raw); request.chmod(0o600)
            module = Path(__file__).resolve()
            argv = [str(Path(sys.executable).resolve()), '-B', str(module), '--oracle', str(request)]
            command = verifier_command(candidate, argv, [str(module), str(Path(contract.__file__).resolve()), str(private)])
            with tempfile.TemporaryFile() as output:
                checked = subprocess.run(command, cwd=candidate, env={}, stdin=subprocess.DEVNULL,
                                         stdout=output, stderr=subprocess.DEVNULL, timeout=30)
                output.seek(0); receipt_raw = output.read(MAX_RECEIPT_BYTES + 1)
            if checked.returncode != 0 or len(receipt_raw) > MAX_RECEIPT_BYTES:
                raise ValueError('isolated response evidence unavailable')
            receipt = _strict_json(receipt_raw)
            # Recompute the unsigned associations locally; never adopt another slot's receipt.
            if receipt != evidence(packet):
                raise ValueError('isolated response receipt association mismatch')
            result['response'] = receipt
            result['artifact_digest'] = digest(receipt['artifact'])
            validate_prepared(task, case, prepared, citation_root)
            if not receipt['citation_check']['valid']:
                result.update(status='failed', correct=False)
            else:
                answer = private / 'answer.txt'; answer.write_text(receipt['answer_text'], encoding='utf-8')
                semantic_case = {key: value for key, value in case.items() if key != 'response'}
                semantic = verify_task(snapshot, answer, semantic_case)
                result.update(status=semantic['status'], correct=semantic['correct'], semantic=semantic)
                result['artifact_digest'] = digest([result['artifact_digest'], semantic['artifact_digest']])
            validate_prepared(task, case, prepared, citation_root)
            if verifier_digest(case) != result['verifier_digest']:
                result.update(status='unavailable', correct=None)
    except (OSError, ValueError, TypeError, KeyError, UnicodeError, IsolationError, subprocess.SubprocessError):
        result.update(status='unavailable', correct=None)
    finally:
        result['seconds'] = round(time.monotonic() - started, 4)
    return result


def main():
    if len(sys.argv) != 3 or sys.argv[1] != '--oracle':
        return 2
    fd = os.open(sys.argv[2], os.O_RDONLY | getattr(os, 'O_NOFOLLOW', 0) | getattr(os, 'O_NONBLOCK', 0))
    with os.fdopen(fd, 'rb') as stream:
        image = os.fstat(stream.fileno())
        if not stat.S_ISREG(image.st_mode) or image.st_size > MAX_PACKET_BYTES:
            return 2
        raw = stream.read(MAX_PACKET_BYTES + 1)
    if len(raw) > MAX_PACKET_BYTES:
        return 2
    result = evidence(_strict_json(raw))
    sys.stdout.write(json.dumps(result, ensure_ascii=False, allow_nan=False))
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, ValueError, TypeError, KeyError, UnicodeError):
        raise SystemExit(2)
