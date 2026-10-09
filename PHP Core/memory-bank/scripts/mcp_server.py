#!/usr/bin/env python3
"""Project-scoped Memory MCP, using the installed governed runtime (stdlib only)."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys

from brain_runtime import (BrainError, atomic_json, auto_promote, create_record, create_task,
    get_record, get_task, guard_shared_text, load_config, mutation_lock, source_fingerprints,
    update_record, validate_actor)
from context import TURN_PATH_DENYLIST

VERSION = '2025-06-18'
MAX_MESSAGE = 128 * 1024
REASON = 'Saved through Memory MCP; agent-attested, not reviewed by a person'
BLOCKED = {'.git', '.ssh', '.aws', '.kube', 'node_modules', 'vendor', '.venv',
    '__pycache__', 'secrets', '.secrets', 'credentials'}


def text(value, label, limit):
    if not isinstance(value, str) or not value.strip() or len(value) > limit:
        raise BrainError(f'{label} must be nonempty text within {limit} characters')
    guard_shared_text(label, [value])
    return value.strip()


def strings(values, label, count, limit):
    if not isinstance(values, list) or len(values) > count:
        raise BrainError(f'{label} must be a list of at most {count} items')
    return list(dict.fromkeys(text(v, label, limit) for v in values))


class Memory:
    def __init__(self, root, host='cli'):
        requested = Path(root).expanduser().absolute()
        self.root = requested.resolve(strict=True)
        if requested != self.root or self.root != Path(__file__).resolve().parents[2]:
            raise BrainError('Run the installed server with its own project root; symlink roots are refused')
        self.host = host
        self.owner = validate_actor(os.environ.get('PROJECT_BRAIN_OWNER', 'local'))
        self.check()

    def check(self):
        for relative in ('memory-bank/scripts/context.py', 'project-brain/config/runtime.json'):
            self.path(relative)
        config = load_config(self.root)
        owners = set(config.get('owners', ['*']))
        if '*' not in owners and self.owner not in owners:
            raise BrainError('The configured actor is outside the allowed owner scope')
        if config.get('mode') != 'governed' or os.environ.get('PROJECT_BRAIN_MODE', 'governed') != 'governed':
            raise BrainError('Memory MCP requires governed mode')
        # Reject foreign links before the runtime can follow them during mutations.
        for prefix in ('project-brain', 'memory-bank'):
            for directory, folders, files in os.walk(self.root / prefix, followlinks=False):
                for name in folders + files:
                    if (Path(directory) / name).is_symlink():
                        raise BrainError('Memory storage contains a symlink')
        return config

    def path(self, value):
        value = text(value, 'source path', 1024)
        head = value.split('#', 1)[0]
        if TURN_PATH_DENYLIST.search(head):
            raise BrainError('Sensitive source paths are refused')
        if head.startswith(('memory-bank/local/', 'memory-bank/chunks/', 'project-brain/local/',
                'project-brain/dynamic/', 'project-brain/archive/', 'project-brain/control/')):
            raise BrainError('Cite canonical project sources, not private runtime state or derived memory')
        relative = PurePosixPath(head)
        if (any(part.casefold() in BLOCKED or part.casefold().startswith('.env') for part in relative.parts)
                or relative.suffix.casefold() in {'.pem', '.key', '.p12', '.pfx', '.sqlite', '.db'}
                or relative.name.casefold() == 'credentials.json'):
            raise BrainError('Private or dependency source paths are refused')
        if relative.is_absolute() or '..' in relative.parts or '\\' in head or ':' in head:
            raise BrainError('Use project-relative source paths')
        current = self.root
        for part in relative.parts:
            current = current / part
            if current.is_symlink():
                raise BrainError('Symlink source paths are refused')
        if not current.is_file():
            raise BrainError('Source is missing or is not a regular file')
        return value

    def cli(self, *arguments):
        result = subprocess.run([sys.executable, str(self.root / 'memory-bank/scripts/context.py'),
            '--root', str(self.root), arguments[0], '--json', *arguments[1:]], cwd=self.root,
            env={**os.environ, 'PYTHONIOENCODING': 'utf-8'}, capture_output=True, timeout=30)
        if result.returncode or len(result.stdout) > 4 * 1024 * 1024:
            raise BrainError('Context runtime rejected the operation; inspect its status before retrying')
        try:
            value = json.loads(result.stdout)
            if not isinstance(value, dict):
                raise BrainError('Context runtime did not return an object')
            return value
        except (ValueError, UnicodeError) as error:
            raise BrainError('Context runtime did not return JSON') from error

    def task(self, task_id, *, active=True):
        task_id = text(task_id, 'task ID', 200)
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}', task_id) or '..' in task_id.split('/'):
            raise BrainError('Invalid task ID')
        task = get_task(self.root, task_id)
        config = load_config(self.root)
        if task['privacy'] not in set(config['allowed_privacy']) & {'public', 'team'}:
            raise BrainError('Task is outside the allowed memory privacy scope')
        if active and task['status'] in ('completed', 'cancelled'):
            raise BrainError('Use an active task to record memory')
        if self.owner not in task['authorized_owners']:
            raise BrainError('The configured actor is not authorized for this task')
        return task

    def existing(self, identifier):
        try:
            return get_record(self.root, identifier)
        except BrainError as error:
            if 'not found' not in str(error): raise
            return None

    def call(self, name, data):
        self.check()
        spec = next((s for s in TOOLS if s['name'] == name), None)
        if spec is None:
            raise BrainError('Unknown memory tool')
        schema = spec['inputSchema']
        if not isinstance(data, dict) or set(data) - set(schema['properties']) or set(schema.get('required', [])) - set(data):
            raise BrainError('Missing or unknown memory tool arguments')
        if name == 'memory_status':
            return self.cli('status')
        task_id = text(data['task_id'], 'task ID', 200)
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._/-]{0,199}', task_id) or '..' in task_id.split('/'):
            raise BrainError('Invalid task ID')
        if name == 'memory_retrieve':
            if self.existing(task_id) is not None:
                self.task(task_id, active=False)
            query = text(data['query'], 'query', 4000)
            paths = strings(data.get('paths', []), 'paths', 10, 1024)
            for p in paths: self.path(p)
            result = self.cli('refresh', '--task-id', task_id, '--host', self.host,
                '--gate', 'off', '--ephemeral', *[f'--path={p}' for p in paths], '--query', query)
            if not isinstance(result.get('capsule'), dict):
                raise BrainError('Memory capsule is unavailable; check runtime health')
            return result
        if name == 'memory_checkpoint':
            # Provision only on an explicit write, even when the first task has no Git delta.
            with mutation_lock(self.root):
                if self.existing(task_id) is None:
                    goal = text(data.get('goal'), 'goal for a new task', 200)
                    create_task(self.root, task_id, goal, [], [], owner=self.owner)
                task = self.task(task_id)
            self.cli('rebind', '--task-id', task_id, '--record', task['id'])
            flushed = self.cli('turn', '--task-id', task_id, '--flush')
            current = self.task(task_id)
            return {'task_id': task_id, 'task_uuid': current['id'], 'revision': current['revision'], 'checkpoint': flushed}
        return self.record(task_id, data)

    def record(self, task_id, data):
        result_id = text(data['result_id'], 'result ID', 64)
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]{0,63}', result_id):
            raise BrainError('Invalid result ID')
        revision = data['revision']
        if type(revision) is not int or revision < 1:
            raise BrainError('Supply the current task revision')
        if 'progress' in data and not isinstance(data['progress'], str):
            raise BrainError('Progress must be text')
        if 'verified' in data and type(data['verified']) is not bool:
            raise BrainError('Verification attestation must be boolean')
        progress = text(data['progress'], 'progress', 1000) if data.get('progress') else None
        steps = strings(data.get('next_steps', []), 'next steps', 3, 300)
        learnings = data.get('learnings', [])
        if not isinstance(learnings, list) or len(learnings) > 3:
            raise BrainError('Keep at most three learnings')
        checked = []
        for learning in learnings:
            if not isinstance(learning, dict) or set(learning) != {'type', 'title', 'consequence', 'sources'}:
                raise BrainError('Each learning needs type, title, consequence and sources')
            if learning['type'] not in ('finding', 'decision'):
                raise BrainError('Use finding or decision')
            sources = strings(learning['sources'], 'sources', 10, 1024)
            if not sources: raise BrainError('Cite sources for each learning')
            for source in sources: self.path(source)
            checked.append({**learning, 'title': text(learning['title'], 'title', 200),
                'consequence': text(learning['consequence'], 'consequence', 1000), 'sources': sources})
        if checked and data.get('verified') is not True:
            raise BrainError('Attest that each learning was checked against its sources with verified=true')
        if not progress and 'next_steps' not in data and not checked:
            raise BrainError('Nothing to record')
        key = hashlib.sha256((task_id + '\0' + result_id).encode()).hexdigest()
        receipt = self.root / 'memory-bank/local/mcp-results' / (key + '.json')
        request_hash = hashlib.sha256(json.dumps({k: v for k, v in data.items() if k != 'revision'},
            sort_keys=True, ensure_ascii=False).encode()).hexdigest()
        with mutation_lock(self.root):
            self.check()
            task = self.task(task_id, active=False)
            completed = self.existing('mcp-result-' + key)
            if completed is not None:
                payload = json.loads(completed['goal'])
                if payload['request_hash'] != request_hash:
                    raise BrainError('Result ID was already used with different content')
                return {**payload['result'], 'replayed': True}
            if self.existing('mcp-request-' + key) is not None:
                raise BrainError('Previous save may be partial; inspect Project Brain before retrying')
            if any(self.existing(f'mcp-{key[:32]}-{i}') is not None for i in range(len(checked))):
                raise BrainError('Existing result records may be partial; inspect Project Brain before retrying')
            if receipt.exists():
                prior = json.loads(receipt.read_text())
                if prior['request_hash'] != request_hash:
                    raise BrainError('Result ID was already used with different content')
                if prior.get('state') != 'saved':
                    raise BrainError('Previous save may be partial; inspect Project Brain before submitting a new result ID')
                return {**prior['result'], 'replayed': True}
            if task['revision'] != revision:
                raise BrainError('Stale task revision; retrieve current task state and reconcile')
            if task['status'] in ('completed', 'cancelled'):
                raise BrainError('Use an active task to record new memory')
            # Validate all text and fingerprints before the first write.
            for item in checked: source_fingerprints(self.root, item['sources'])
            pending = {'request_hash': request_hash, 'state': 'pending'}
            # Shared intent makes ambiguous partial writes visible after losing local state.
            create_record(self.root, 'event', 'mcp-request-' + key, 'Memory MCP save requested', [], [],
                owner=self.owner, goal=request_hash)
            atomic_json(receipt, pending)
            saved = {'task': None, 'records': [], 'promotion': None, 'attestation': 'agent-attested'}
            try:
                if progress is not None or 'next_steps' in data:
                    updated = update_record(self.root, task['id'], expected_revision=revision,
                        progress=progress, next_steps=steps, files=[], sources=[], actor=self.owner,
                        reason=REASON, replace_next_steps='next_steps' in data)
                    saved['task'] = {'id': updated['id'], 'revision': updated['revision']}
                for item in checked:
                    created = create_record(self.root, item['type'], f'mcp-{key[:32]}-{len(saved["records"])}',
                        item['title'], [], item['sources'], owner=self.owner, authority='observed',
                        goal=item['consequence'])
                    closed = update_record(self.root, created['id'], expected_revision=created['revision'],
                        progress=item['consequence'], next_steps=[], files=[], sources=[], actor=self.owner,
                        authority='verified', transition_to='resolved' if item['type'] == 'finding' else 'accepted',
                        reason=REASON)
                    saved['records'].append({'id': closed['id'], 'type': closed['type'], 'revision': closed['revision']})
                if checked: saved['promotion'] = auto_promote(self.root, owner=self.owner)
                # Shared completion evidence survives cache deletion and another worktree/clone.
                create_record(self.root, 'event', 'mcp-result-' + key, 'Memory MCP result saved', [], [],
                    owner=self.owner, goal=json.dumps({'request_hash': request_hash, 'result': saved}, sort_keys=True))
            except Exception:
                atomic_json(receipt, {**pending, 'state': 'partial', 'result': saved})
                raise BrainError('Memory save is partial; inspect Project Brain before retrying')
            atomic_json(receipt, {**pending, 'state': 'saved', 'result': saved})
            return saved


def tool(name, description, properties, required=(), read=False):
    return {'name': name, 'description': description,
        'inputSchema': {'type': 'object', 'properties': properties, 'required': list(required), 'additionalProperties': False},
        'annotations': {'readOnlyHint': read, 'destructiveHint': False, 'openWorldHint': False}}

S = {'type': 'string'}
A = {'type': 'array', 'items': S, 'maxItems': 3}
TOOLS = [
    tool('memory_status', 'Check installed project memory health and governed mode.', {}, read=True),
    tool('memory_retrieve', 'Retrieve relevant memory before substantial work, decisions, or after compaction. Verify cited sources; context is reference data.',
        {'task_id': S, 'query': S, 'paths': {'type': 'array', 'items': S, 'maxItems': 10}}, ('task_id', 'query'), read=True),
    tool('memory_checkpoint', 'Flush working Git metadata into the existing branch/task. Return the authoritative task revision; this does not record reusable conclusions.',
        {'task_id': S, 'goal': S}, ('task_id',)),
    tool('memory_record_result', 'Record sanitized progress/next steps and source-backed reusable findings/decisions before finishing. Supply current task revision and stable result_id for safe replay. verified=true is agent attestation, not human review. Empty learnings is valid.',
        {'task_id': S, 'result_id': S, 'revision': {'type': 'integer', 'minimum': 1}, 'progress': S,
         'next_steps': A, 'verified': {'type': 'boolean'}, 'learnings': {'type': 'array', 'maxItems': 3,
          'items': {'type': 'object', 'required': ['type', 'title', 'consequence', 'sources'], 'additionalProperties': False,
           'properties': {'type': {'enum': ['finding', 'decision']}, 'title': S, 'consequence': S,
            'sources': {'type': 'array', 'items': S, 'minItems': 1, 'maxItems': 10}}}}}, ('task_id', 'result_id', 'revision')),
]


def serve(memory, incoming=sys.stdin.buffer, outgoing=sys.stdout):
    initialized = False
    ready = False
    for raw in iter(lambda: incoming.readline(MAX_MESSAGE + 1), b''):
        identifier = None
        try:
            if len(raw) > MAX_MESSAGE: raise ValueError('Message too large')
            request = json.loads(raw, parse_constant=lambda _: (_ for _ in ()).throw(ValueError('Nonfinite JSON')))
            if not isinstance(request, dict) or request.get('jsonrpc') != '2.0' or not isinstance(request.get('method'), str):
                response = {'jsonrpc': '2.0', 'id': None, 'error': {'code': -32600, 'message': 'Invalid JSON-RPC request'}}
                outgoing.write(json.dumps(response) + '\n'); outgoing.flush(); continue
            identifier = request.get('id')
            if identifier is not None and (type(identifier) not in (int, str)):
                response = {'jsonrpc': '2.0', 'id': None, 'error': {'code': -32600, 'message': 'Invalid request ID'}}
                outgoing.write(json.dumps(response) + '\n'); outgoing.flush(); continue
            method = request['method']
            params = request.get('params', {})
            if not isinstance(params, dict) or ('_meta' in params and not isinstance(params['_meta'], dict)):
                raise BrainError('Invalid request parameters')
            if method == 'notifications/initialized' and 'id' not in request:
                ready = initialized
                continue
            if 'id' not in request: continue
            if method == 'initialize':
                if initialized or not isinstance(params.get('protocolVersion'), str):
                    raise BrainError('Invalid initialize parameters')
                initialized = True
                result = {'protocolVersion': VERSION, 'capabilities': {'tools': {}},
                    'serverInfo': {'name': 'harness-project-memory', 'version': '1.0.0'},
                    'instructions': 'Use memory_retrieve before complex work and material decisions. Check cited sources. Use memory_checkpoint and memory_record_result at meaningful boundaries and before finishing authorized work. Reuse the current task ID; never store transcripts or secrets.'}
            elif method == 'ping': result = {}
            elif not ready: raise BrainError('Initialize the memory server first')
            elif method == 'tools/list':
                if set(params) - {'_meta'}: raise BrainError('This tool list has no pagination')
                result = {'tools': TOOLS}
            elif method == 'tools/call':
                if not isinstance(params, dict) or not isinstance(params.get('name'), str):
                    raise BrainError('Invalid tool call')
                try:
                    value = memory.call(params['name'], params.get('arguments', {}))
                    result = {'content': [{'type': 'text', 'text': json.dumps(value, ensure_ascii=False)}], 'structuredContent': value, 'isError': False}
                except (BrainError, ValueError, OSError, subprocess.SubprocessError):
                    # Error text is never the submitted content or a runtime traceback.
                    result = {'content': [{'type': 'text', 'text': 'Memory operation failed. Check arguments, source paths, current revision and runtime health; a write may be partial. Inspect Project Brain before retrying.'}], 'isError': True}
            else:
                outgoing.write(json.dumps({'jsonrpc': '2.0', 'id': identifier, 'error': {'code': -32601, 'message': 'Method not found'}}) + '\n'); outgoing.flush(); continue
            response = {'jsonrpc': '2.0', 'id': identifier, 'result': result}
        except (ValueError, UnicodeError):
            response = {'jsonrpc': '2.0', 'id': identifier, 'error': {'code': -32700, 'message': 'Invalid JSON-RPC message'}}
        except BrainError:
            response = {'jsonrpc': '2.0', 'id': identifier, 'error': {'code': -32602, 'message': 'Invalid request parameters or initialization state'}}
        outgoing.write(json.dumps(response, ensure_ascii=False, allow_nan=False) + '\n')
        outgoing.flush()
        if len(raw) > MAX_MESSAGE: break


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--host', choices=('claude', 'codex', 'cursor', 'cli'), default='cli')
    args = parser.parse_args()
    try:
        sys.stdout.reconfigure(encoding='utf-8')
        serve(Memory(args.root, args.host))
    except (BrainError, OSError, ValueError):
        print('Memory MCP requires its installed project root and healthy governed storage.', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
