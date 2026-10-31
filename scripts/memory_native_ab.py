#!/usr/bin/env python3
"""Opt-in native A→fresh-B availability experiment; never a causal verdict."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import sqlite3
import sys
import time
from itertools import islice

import memory_native_ab_io as io
from memory_native_ab_session import run_process
from memory_native_ab_evidence import capture, metadata, command_args
from memory_pilot_support import child_env

ROOT = Path(__file__).resolve().parents[1]
CANON = ROOT / 'PHP Core'
FORMAT = 'memory-native-ab-run-v1'
SAVE_FORMAT = ("\nThis is a read-only research experiment. Do not modify application files or delegate. "
    "Do not manually create, update or promote reusable findings, decisions or memory chunks before "
    "the internal Stop continuation. Do not call record-result or memory_write early; retain checked "
    "learning for the native-result continuation. "
    "Follow the native memory workflow. If an internal Stop requests native-result, invoke its exact "
    "python3 memory-bank/scripts/context.py command directly with a single quoted heredoc "
    "<<'NATIVE_RESULT' containing your own JSON, followed by NATIVE_RESULT on a new line. "
    "Do not wrap the save in another program. Each learning consequence is at most 1000 characters. "
    "Run an explicit retrieve in its own shell call so its exact stdout can be checked against the render trace. "
    "When consulting a prior memory, cite its MEM identifier. verified=true is agent attestation.")


def runtime_digest():
    import memory_outcome_ab as ab
    edition = ab.stand.Edition('PHP Core')
    rows = []
    for _, name in edition.files:
        raw, mode = io.read_file(edition.source(name))
        rows.append((name, io.sha(raw), mode))
    for path in sorted((ROOT / 'scripts').glob('memory_native_ab*.py')):
        raw, mode = io.read_file(path)
        rows.append((path.name, io.sha(raw), mode))
    for name in ('memory_outcome_ab.py', 'memory_outcome_isolation.py',
                 'memory_delivery_proof.py', 'memory_pilot_support.py', 'memory_eval.py'):
        raw, mode = io.read_file(ROOT / 'scripts' / name)
        rows.append((name, io.sha(raw), mode))
    return io.digest(rows)


def install_runtime(corpus):
    import memory_outcome_ab as ab
    for name in ('.codex', '.Codex', '.claude', '.cursor', '.agents'):
        path = corpus / name
        if path.exists():
            shutil.rmtree(path)
    if (corpus / '.mcp.json').exists():
        (corpus / '.mcp.json').unlink()
    for name in ('memory-bank/scripts', 'project-brain/scripts'):
        path = corpus / name
        if path.exists():
            shutil.rmtree(path)
    ab.stand.overlay(corpus, ab.stand.Edition('PHP Core'))
    for name in ('runtime.json', 'providers.json', 'telemetry.json'):
        source = CANON / 'project-brain/config' / name
        if source.exists():
            shutil.copyfile(source, corpus / 'project-brain/config' / name)
    for name in ('memory-bank', 'project-brain', '.context-handoff'):
        (corpus / name).mkdir(exist_ok=True, mode=0o700)
    for path in (corpus / '.codex/hooks').glob('*.sh'):
        if not os.access(path, os.X_OK):
            raise io.PilotError('non-executable canonical hook')


def make_view(corpus, private, task_alias, command, *, credentials=True):
    from memory_outcome_isolation import prepare_agent_view
    evidence = corpus / 'memory-bank/local/native-ab'
    for name in ('home', 'tmp'):
        (evidence / name).mkdir(parents=True, exist_ok=True, mode=0o700)
    view = prepare_agent_view(corpus, private, command=command, credentials=credentials)
    prefix = list(view['prefix'])
    position = next((i for i, x in enumerate(prefix)
        if x == '--ro-bind' and prefix[i + 2] == '/home/agent/.codex/auth.json'), len(prefix))
    prefix[position:position] = ['--bind', str(evidence / 'home'), '/home/agent/.codex']
    for i, value in enumerate(prefix):
        if value == '--tmpfs' and prefix[i+1] == '/tmp':
            prefix[i:i+2] = ['--bind', str(evidence / 'tmp'), '/tmp']
            break
    # Native workspace-write creates empty denial stubs in the corpus root.
    # Pin existing entries individually; writable top-level mounts pin runtime ancestors.
    writable = {'memory-bank', 'project-brain', '.context-handoff'}
    for path in sorted(corpus.iterdir()):
        if path.name not in writable:
            prefix += ['--ro-bind', str(path), str(path)]
    for name in ('memory-bank', 'project-brain', '.context-handoff'):
        prefix += ['--bind', str(corpus / name), str(corpus / name)]
    for name in ('memory-bank/scripts', 'memory-bank/templates', 'project-brain/config',
                 'project-brain/schemas', 'project-brain/scripts', 'project-brain/templates'):
        if (corpus / name).exists():
            prefix += ['--ro-bind', str(corpus / name), str(corpus / name)]
    prefix += ['--setenv', 'TMPDIR', '/tmp',
        '--setenv', 'CONTEXT_TASK_ID', task_alias, '--setenv', 'PROJECT_BRAIN_OWNER', 'local',
        '--setenv', 'PYTHONDONTWRITEBYTECODE', '1', '--setenv', 'GIT_OPTIONAL_LOCKS', '0']
    view['prefix'] = prefix
    return view


def native_command(corpus, config, task):
    import memory_outcome_ab as ab
    hooks = io.read_json(CANON / '.codex/hooks.json')['hooks']
    command = ['codex', 'exec', '--json', '--ignore-user-config', '--skip-git-repo-check',
        '--dangerously-bypass-hook-trust', '-s', 'workspace-write', '-C', str(corpus),
        '-m', config['model'], '-c', 'features.hooks=true', '-c', 'features.multi_agent=false',
        '-c', 'agents.enabled=false', '-c', 'project_doc_max_bytes=131072']
    for event, groups in hooks.items():
        command += ['-c', 'hooks.' + event + '=' + ab._toml(groups)]
    return [*command, task['prompt'] + SAVE_FORMAT]


def pending_state(corpus, *, deadline=None):
    io.check_deadline(deadline)
    database = corpus / 'memory-bank/local/context.db'
    raw = io.read_file(database, io.MAX_TREE)[0]
    if raw[:16] != b'SQLite format 3\0' or raw[18:20] != b'\x01\x01':
        raise io.PilotError('unsupported pending-state database')
    connection = sqlite3.connect(database.resolve().as_uri() + '?mode=ro', uri=True, timeout=.2)
    try:
        if deadline is not None:
            connection.set_progress_handler(lambda: int(time.monotonic() >= deadline), 1000)
        connection.execute('PRAGMA query_only=ON')
        count = connection.execute('SELECT count(*) FROM turn_deltas').fetchone()[0]
        tasks = [row[0] for row in connection.execute('SELECT DISTINCT task_id FROM turn_deltas LIMIT 33')]
    finally:
        connection.close()
    requests = list(islice((corpus / 'project-brain/local/native-results').glob('*.json'), 33))
    if len(requests) > 32:
        raise io.PilotError('native request bound')
    unresolved = sum(io.read_json(p).get('phase') != 'saved' for p in requests)
    queued = visited = 0
    for folder, dirs, files in os.walk(corpus / 'memory-bank/local/native-ab/tmp', followlinks=False):
        io.check_deadline(deadline)
        visited += len(dirs) + len(files) + 1
        if visited > 10000:
            raise io.PilotError('temporary queue scan bound')
        if any((Path(folder) / name).is_symlink() for name in dirs):
            raise io.PilotError('linked temporary directory')
        if any(p.startswith(('ai-accelerator-native-spool-', 'ai-accelerator-memory-queue-'))
               for p in Path(folder).parts):
            queued += sum('.json' in name or '.attempt' in name for name in files)
    return {'turn_deltas': count, 'turn_task_ids': tasks,
            'unresolved_requests': unresolved, 'queued_requests': queued,
            'retained': True, 'lossless_cache_deletion': False}


def native_journal_healthy(corpus, *, deadline=None):
    io.check_deadline(deadline)
    raw = io.read_file(corpus / 'memory-bank/local/native-results.ndjson', 1024*1024)[0]
    rows = [io._json(line) for line in raw.splitlines()]
    exits = [r for r in rows if r.get('stage') == 'stop-hook' and r.get('event') == 'exited']
    return len(exits) >= 2 and all(type(r.get('status')) is int and r['status'] == 0 for r in exits)


def application_digest(image):
    return io.image_digest([row for row in image
                            if not row[0].startswith(('memory-bank/', 'project-brain/'))])


def delivery(corpus, identity, events, before_manifests, known, expected, *, deadline=None):
    from memory_delivery_proof import collect_delivery_proof
    import memory_outcome_ab as ab
    folders = ('memory-bank/local/retrieval-manifests', 'project-brain/control/retrieval-manifests')
    paths = [p for name in folders for p in islice((corpus / name).glob('*.json'), 1001)
             if p.relative_to(corpus).as_posix() not in before_manifests]
    if len(paths) > 1000:
        raise io.PilotError('manifest scan bound')
    proof = collect_delivery_proof(corpus, paths, known_paths=known, deadline=deadline)
    outputs = set()
    for line in io.read_file(events, 8*1024*1024)[0].splitlines():
        io.check_deadline(deadline)
        event = io._json(line)
        item = event.get('item', {})
        if event.get('type') != 'item.completed' or not isinstance(item, dict):
            continue
        if item.get('type') != 'command_execution' or item.get('exit_code') != 0:
            continue
        argv = command_args(item.get('command', ''), corpus)
        if argv and argv[0] in ('retrieve', 'context'):
            outputs.add(io.sha(item.get('aggregated_output', '').encode()))
    found = []
    for frame in proof['frames']:
        io.check_deadline(deadline)
        if frame['task_uuid'] != identity['task_uuid'] or frame['origin'] != 'explicit':
            continue
        for doc in frame['selected']:
            if (doc['path'] in expected and doc['document_state'] == 'match' and doc['sources']
                    and all(r['state'] == 'match' for r in doc['sources'])
                    and doc['render'].get('frame_sha256') in outputs):
                found.append(doc['path'])
    return proof, sorted(set(found))


def execute_stage(run_root, seal, role, source_result=None):
    began = time.monotonic()
    config = seal['config']
    deadline = began + config['deadline_seconds']
    control, work = run_root / 'control', run_root / 'work'
    work.mkdir(exist_ok=True, mode=0o700)
    corpus, task = work / role, config[role]
    command = native_command(corpus, config, task)
    if sys.platform != 'linux' or not shutil.which('bwrap') or not shutil.which(command[0]):
        raise io.PilotError('native execution prerequisites unavailable')

    def remaining():
        left = deadline - time.monotonic()
        if left <= 0:
            raise io.PilotError('stage deadline')
        return left

    if role == 'source':
        image = io.snapshot(Path(config['project_snapshot']), deadline=deadline)
    else:
        image = io.snapshot(work / 'source', deadline=deadline)
        if io.image_digest(image) != source_result['handoff_sha256']:
            raise io.PilotError('source handoff changed')
    io.copy_image(image, corpus)
    remaining()
    install_runtime(corpus)
    remaining()
    env = child_env()
    env.update(PYTHONDONTWRITEBYTECODE='1', GIT_OPTIONAL_LOCKS='0', PROJECT_BRAIN_OWNER='local',
               GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull, GIT_ATTR_NOSYSTEM='1')

    def trusted(label, command):
        target = control / (role + '-' + label + '.stdout')
        outcome = run_process(command, corpus, target, timeout=remaining(), cap=100, env=env,
            native=False, stderr_path=control / (role + '-' + label + '.stderr'), max_bytes=1024*1024)
        if not outcome['completed']:
            raise io.PilotError('trusted preparation failed')
        return io.read_file(target, 1024*1024)[0]

    trusted('git', ['git', 'init', '-q', '--initial-branch', task['task_alias']])
    identity = io._json(trusted('start', [sys.executable, '-B', 'memory-bank/scripts/context.py',
        'start', '--task-id', task['task_alias'], '--goal', 'Independent native-memory research', '--json']))
    io.write_json(control / (role + '-identity.json'), identity)
    if role == 'served' and identity['task_uuid'] == source_result['identity']['task_uuid']:
        raise io.PilotError('same task UUID')
    # Only this freshly created, screened copy is staged; client Git history is never reused.
    trusted('baseline-add', ['git', '-c', 'core.hooksPath=/dev/null', 'add', '--', '.'])
    trusted('baseline-commit', ['git', '-c', 'core.hooksPath=/dev/null',
        '-c', 'user.name=Native Memory Experiment', '-c', 'user.email=fixture@example.invalid',
        '-c', 'commit.gpgsign=false', 'commit', '-qm', 'Screened native-memory baseline', '--allow-empty'])
    before_image = io.snapshot(corpus, deadline=deadline)
    before = {n for n, _, _ in before_image}
    io.write_json(control / (role + '-before.json'), {
        'image_sha256': io.image_digest(before_image),
        'files': [{'path': name, 'sha256': io.sha(raw), 'mode': mode}
                  for name, raw, mode in before_image]})
    from memory_delivery_proof import snapshot_known_paths
    known = snapshot_known_paths(corpus)
    folders = ('memory-bank/local/retrieval-manifests', 'project-brain/control/retrieval-manifests')
    before_manifests = {p.relative_to(corpus).as_posix()
                       for name in folders for p in (corpus / name).glob('*.json')}
    view = make_view(corpus, control, task['task_alias'], command)
    probes = [str(control / 'plan.json'), config['project_snapshot']]
    if role == 'served':
        probes.append(str(work / 'source'))
    probe_code = ('from pathlib import Path;import sys;'
                  'sys.exit(9 if any(Path(p).exists() for p in sys.argv[1:]) else 0)')
    checked = run_process([*view['prefix'], '--', str(Path(sys.executable).resolve()), '-c', probe_code, *probes],
        corpus, control / (role + '-probe.stdout'), timeout=remaining(), cap=100,
        native=False, env=view['env'], max_bytes=4096)
    if not checked['completed']:
        raise io.PilotError('read-isolation probe failed')
    events = control / (role + '-events.jsonl')
    result = run_process([*view['prefix'], '--', *view['command']], corpus, events,
        timeout=remaining(), cap=config['tool_cap'], env=view['env'],
        stderr_path=control / (role + '-native.stderr'))
    result.update(identity=identity, native_saved=False, new_chunks=[])
    result['before_inventory_sha256'] = io.sha(io.read_file(
        control / (role + '-before.json'), io.MAX_TREE)[0])
    io.write_json(control / (role + '-process.json'), result)
    if len(result['sessions']) != 1 or not result['completed']:
        return result
    if role == 'served' and result['sessions'] == source_result['sessions']:
        raise io.PilotError('same native session')
    if application_digest(io.snapshot(corpus, deadline=deadline)) != application_digest(before_image):
        raise io.PilotError('unexpected application image change')
    result.update(capture(corpus, identity, result['sessions'][0], events, before, deadline=deadline))
    result['hooks_healthy'] = native_journal_healthy(corpus, deadline=deadline)
    result['pending'] = pending_state(corpus, deadline=deadline)
    remaining()
    if role == 'source':
        pending = result['pending']
        if (result['native_saved'] and result['hooks_healthy'] and pending['turn_deltas']
                and not pending['unresolved_requests'] and not pending['queued_requests']
                and set(pending['turn_task_ids']) <= {task['task_alias'], identity['task_uuid']}):
            images = {name: io.sha(io.read_file(corpus / name)[0]) for name in result['new_chunks']}
            result['pending_before_settle'] = pending
            trusted('settle', [sys.executable, '-B', 'memory-bank/scripts/context.py',
                              'turn', '--flush', '--task-id', task['task_alias'], '--json'])
            result['pending'] = pending_state(corpus, deadline=deadline)
            if any(io.sha(io.read_file(corpus / name)[0]) != digest for name, digest in images.items()):
                raise io.PilotError('knowledge changed while settling source observations')
        clear = not any(result['pending'][k]
                        for k in ('turn_deltas', 'unresolved_requests', 'queued_requests'))
        result['handoff_ready'] = bool(clear and result['native_saved']
                                      and result['hooks_healthy'] and result['new_chunks'])
        if result['handoff_ready']:
            result['handoff_sha256'] = io.image_digest(io.snapshot(corpus, deadline=deadline))
    else:
        proof, found = delivery(corpus, identity, events, before_manifests,
                                known, source_result['new_chunks'], deadline=deadline)
        io.write_json(control / 'served-delivery.json', proof)
        result['delivered_chunks'] = found
        result['attribution_observed'] = any(metadata(corpus / name)['id']
                                            in result['answer'] for name in found)
    result['seconds'] = round(time.monotonic() - began, 3)
    remaining()
    return result


def summary(state):
    slots = state['stages']
    statuses = {'planned', 'running', 'captured', 'consulted', 'not-observed', 'failed',
                'interrupted', 'blocked-source', 'no-learning', 'pending-source'}
    for slot in slots.values():
        if (not isinstance(slot, dict) or type(slot.get('attempted')) is not bool
                or not isinstance(slot.get('status'), str) or slot['status'] not in statuses):
            raise io.PilotError('invalid stage summary')
    return {'schema': FORMAT, 'cohort_role': 'development-smoke', 'planned': 2,
        'attempted': sum(s['attempted'] for s in slots.values()),
        'stages': {name: s['status'] for name, s in slots.items()},
        'causal_benefit': None, 'provider_cost_savings': None, 'natural_delayed_efficacy': False}


def plan(config_path, output):
    config_path, output = Path(config_path), Path(output)
    config = io.load_config(config_path)
    source, _, output = io.validate_roots(Path(config['project_snapshot']), config_path, output)
    seal = {'schema': FORMAT, 'config': config,
        'source_sha256': io.image_digest(io.snapshot(source)), 'runtime_sha256': runtime_digest(),
        'created_at': time.time(), 'cohort_role': 'development-smoke'}
    output.mkdir(mode=0o700)
    control = output / 'control'
    control.mkdir(mode=0o700)
    io.write_json(control / 'plan.json', seal)
    state = {'schema': FORMAT, 'plan_sha256': io.sha(io.read_file(control / 'plan.json')[0]),
        'stages': {name: {'status': 'planned', 'attempted': False} for name in ('source', 'served')}}
    io.write_json(control / 'state.json', state)
    return summary(state)


def load_run(root):
    root = io.ordinary(root)
    control = root / 'control'
    seal, state = io.read_json(control / 'plan.json'), io.read_json(control / 'state.json')
    if (seal.get('schema') != FORMAT or state.get('schema') != FORMAT
            or state.get('plan_sha256') != io.sha(io.read_file(control / 'plan.json')[0])
            or set(state.get('stages', {})) != {'source', 'served'}):
        raise io.PilotError('invalid run state')
    return root, seal, state


def inspect(root):
    return summary(load_run(Path(root))[2])


def run(root, stage='both'):
    root, seal, state = load_run(Path(root))
    control = root / 'control'
    if stage not in ('both', 'source', 'served'):
        raise io.PilotError('invalid stage')
    roles = ('source', 'served') if stage == 'both' else (stage,)
    lock = os.open(root / 'run.lock', os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    os.close(lock)
    try:
        root, seal, state = load_run(root)
        summary(state)
        if any(state['stages'][name]['attempted'] for name in roles):
            raise io.PilotError('implicit retry refused; create a new plan')
        for role in roles:
            slot, source = state['stages'][role], state['stages']['source']
            if role == 'served' and source['status'] != 'captured':
                slot['status'] = 'blocked-source'
                io.write_json(control / 'state.json', state)
                break
            slot.update(status='running', attempted=True, started_at=time.time())
            io.write_json(control / 'state.json', state)
            try:
                if runtime_digest() != seal['runtime_sha256']:
                    raise io.PilotError('runtime changed after plan')
                if (role == 'source' and io.image_digest(io.snapshot(
                        Path(seal['config']['project_snapshot']))) != seal['source_sha256']):
                    raise io.PilotError('source changed after plan')
                result = execute_stage(root, seal, role, source.get('result'))
                slot['result'] = result
                if (not result.get('completed') or not result.get('native_saved')
                        or result.get('hooks_healthy') is False):
                    slot['status'] = 'failed'
                elif role == 'source':
                    slot['status'] = ('captured' if result.get('handoff_ready')
                        else ('no-learning' if not result.get('new_chunks') else 'pending-source'))
                else:
                    slot['status'] = ('consulted' if result.get('delivered_chunks')
                                     and result.get('attribution_observed') else 'not-observed')
            except KeyboardInterrupt:
                slot['status'] = 'interrupted'
                io.write_json(control / 'state.json', state)
                raise
            except Exception:
                slot['status'], slot['failure'] = 'failed', 'stage-unavailable'
                process_result = control / (role + '-process.json')
                if process_result.exists():
                    slot['result'] = io.read_json(process_result)
            io.write_json(control / 'state.json', state)
            if role == 'source' and slot['status'] != 'captured':
                state['stages']['served']['status'] = 'blocked-source'
                io.write_json(control / 'state.json', state)
                break
        return summary(state)
    finally:
        (root / 'run.lock').unlink()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    p = commands.add_parser('plan')
    p.add_argument('--config', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    for name in ('run', 'inspect'):
        p = commands.add_parser(name)
        p.add_argument('--run', type=Path, required=True)
        if name == 'run':
            p.add_argument('--stage', choices=('both', 'source', 'served'), default='both')
    args = parser.parse_args(argv)
    try:
        value = (plan(args.config, args.out) if args.command == 'plan' else
                 (inspect(args.run) if args.command == 'inspect' else run(args.run, args.stage)))
        print(json.dumps(value, allow_nan=False))
        return (0 if args.command != 'run' or value['stages']['served'] == 'consulted'
                or (args.stage == 'source' and value['stages']['source'] == 'captured') else 1)
    except (OSError, ValueError, TypeError, KeyError, UnicodeError, RecursionError):
        print('native memory experiment unavailable; private inputs or prerequisites invalid',
              file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
