"""Trusted queue worker for reviewed system plans; no browser-supplied command."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import signal
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / 'scripts'))
import ai_system_execution as execution
from ai_system_lib import System, SystemError


def emit(kind, text, **extra):
    print(json.dumps({'kind': kind, 'text': text[:4000], **extra}), flush=True)


def interrupted(*_):
    raise KeyboardInterrupt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--request', type=Path, required=True)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    execution.PROCESS_GUARD = Path(__file__).with_name('process_guard.py')
    try:
        request = execution.load(args.request.parent, args.request.name)
        directory = Path(request['directory'])
        if directory != args.request.parent:
            raise SystemError('System request directory changed')
        system = System(request['system_file'], request['allowed_roots'])
        journal = directory / 'execution'
        if (journal / 'run.json').exists():
            state = execution.validate_state(system, journal)
            if state['executable'] != execution.executable_path('codex', request['executable']):
                raise SystemError('Configured provider changed; prepare a new plan')
        else:
            plan = execution.load(directory, 'approved-plan.json')
            state = execution.create_run(system, plan, journal, 'codex', request['executable'],
                                         request['mode'], request['timeout'])
        emit('status', 'Sequential system dispatch started. Progress and receipts are saved after each dispatch.')
        state = execution.drive_run(system, journal, state, request['retry_step'], request['accept_source_changes'])
        ok = state['status'] == 'completed'
        emit('text', state['error'] or 'Worker-reported checks completed; native tasks closed and handoff saved.')
        emit('result', 'System run ' + state['status'], ok=ok)
        return 0 if ok else 1
    except KeyboardInterrupt:
        emit('result', 'System run interrupted; inspect saved dispatches before recovery.', ok=False)
        return 1
    except (SystemError, OSError, ValueError, KeyError) as error:
        emit('error', str(error) if isinstance(error, SystemError) else 'System runner could not read its saved inputs.')
        emit('result', 'System run blocked. Inspect the plan and execution journal.', ok=False)
        return 1


if __name__ == '__main__':
    sys.exit(main())
