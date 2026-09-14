"""Checkpointed Fleet process. Invoked by the stdlib server's guarded runner."""
from __future__ import annotations

import argparse
from contextlib import closing
from decimal import Decimal, ROUND_DOWN
import json
import os
from pathlib import Path
import selectors
import signal
import sqlite3
import stat
import subprocess
import sys
import threading
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from langgraph.checkpoint.sqlite import SqliteSaver

from harness import providers
from harness.blackboard import Blackboard, NullBlackboard
from harness.config import HarnessConfig
from harness.graphs.fleet_review import build_graph, dedupe, render_report, resume_command
from harness.workers import DryRunWorker, WorkerResult

OUTPUT_LOCK = threading.Lock()


def emit(event):
    with OUTPUT_LOCK:
        print(json.dumps(event, ensure_ascii=False, allow_nan=False), flush=True)


class NativeReviewer:
    """Reuse the browser's provider permissions, parsing and owner watchdog."""

    def __init__(self, request, lock_fd):
        self.request, self.lock_fd = request, lock_fd

    def reserve_budget(self, lens):
        # Checkpoint payloads retain old per-reviewer allowances. Allocate from the
        # current saved cap and the durable ledger on every invocation instead.
        settings = self.request['session']['fleet']
        budget = settings['budget_usd']
        path = Path(self.request['state_dir']) / 'budget.sqlite'
        legacy = not path.exists() and self.request['action'] != 'start'
        with closing(sqlite3.connect(path, timeout=10)) as ledger, ledger:
            ledger.execute('CREATE TABLE IF NOT EXISTS reservations (id TEXT PRIMARY KEY,lens TEXT,allowance REAL,cost REAL)')
            ledger.execute('BEGIN IMMEDIATE')
            if legacy:
                ledger.execute('INSERT INTO reservations VALUES (?,?,NULL,NULL)', ('legacy-unknown',lens))
            allowance = None
            if budget is not None:
                if ledger.execute('SELECT 1 FROM reservations WHERE cost IS NULL AND allowance IS NULL LIMIT 1').fetchone():
                    emit({'kind':'status','text':'Prior uncapped spending is unknown. Start a new Fleet to enforce a USD cap.'})
                    raise ValueError('Prior Fleet spending is unknown.')
                spent = ledger.execute('SELECT COALESCE(SUM(COALESCE(cost,allowance)),0) FROM reservations').fetchone()[0]
                remaining = Decimal(str(budget)) - Decimal(str(spent))
                share = Decimal(str(budget)) / len(settings['lenses'])
                allowance = float(min(share, max(Decimal(0), remaining)).quantize(Decimal('.000001'), rounding=ROUND_DOWN))
                if allowance < .000001:
                    emit({'kind':'status','text':'No unreserved USD budget remains. Interrupted calls retain their allocation when cost is unknown.'})
                    raise ValueError('Fleet budget exhausted.')
            reservation = uuid.uuid4().hex
            ledger.execute('INSERT INTO reservations(id,lens,allowance) VALUES (?,?,?)', (reservation,lens,allowance))
        return reservation, allowance

    def settle_budget(self, reservation, cost):
        if reservation is not None and cost is not None:
            with closing(sqlite3.connect(Path(self.request['state_dir']) / 'budget.sqlite', timeout=10)) as ledger, ledger:
                ledger.execute('UPDATE reservations SET cost=? WHERE id=?', (cost, reservation))

    def recorded_cost(self, lens):
        with closing(sqlite3.connect(Path(self.request['state_dir']) / 'budget.sqlite')) as ledger:
            costs = [row[0] for row in ledger.execute('SELECT cost FROM reservations WHERE lens=?', (lens,))]
        return None if any(cost is None for cost in costs) else sum(costs)

    def __call__(self, prompt, config):
        session = self.request['session']
        provider = session['provider']
        prompt += '\n\n' + self.request['context']
        command = providers.build_command(provider, self.request['executable'], config.project, prompt,
                                          mode='plan', model=config.model, agents_enabled=False,
                                          thinking_effort=config.thinking_effort)
        if provider == 'claude' and self.request.get('attachment_dirs'):
            command.extend(['--add-dir', *self.request['attachment_dirs']])
        reservation, allowance = self.reserve_budget(config.lenses[0])
        if allowance is not None:
            if provider != 'claude':
                raise ValueError('This provider cannot enforce a native USD limit.')
            command.extend(['--max-budget-usd', str(allowance)])
        environment = {**os.environ, **providers.agent_environment(provider, False, 1)}
        read_fd, write_fd = os.pipe()
        process = None
        selector = selectors.DefaultSelector()
        writer = None
        started = time.monotonic()
        try:
            try:
                process = subprocess.Popen(
                    [sys.executable, str(Path(__file__).with_name('process_guard.py')), str(read_fd), '--', *command],
                    cwd=config.project, env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                    stderr=subprocess.DEVNULL, start_new_session=True, pass_fds=(read_fd, self.lock_fd))
            finally:
                os.close(read_fd)

            def write_input():
                try:
                    text = providers.input_text(provider, prompt)
                    if text is not None:
                        process.stdin.write(text.encode('utf-8'))
                        process.stdin.flush()
                except (OSError, ValueError):
                    pass
                finally:
                    process.stdin.close()

            writer = threading.Thread(target=write_input, daemon=True)
            writer.start()
            selector.register(process.stdout, selectors.EVENT_READ)
            buffer, total, text, cost, terminal, failed = b'', 0, '', None, False, False
            limit_reached = None
            while True:
                if time.monotonic() - started > config.worker_timeout_seconds:
                    raise TimeoutError('Reviewer time limit reached.')
                ready = selector.select(.1)
                if not ready:
                    if process.poll() is not None:
                        break
                    continue
                chunk = os.read(process.stdout.fileno(), 65536)
                eof = not chunk
                if eof and not buffer:
                    break
                total += len(chunk)
                buffer += chunk if not eof else b'\n'
                if len(buffer) > 2 * 1024 * 1024 or total > 4 * 1024 * 1024:
                    raise ValueError('Reviewer output limit reached.')
                lines = buffer.split(b'\n')
                buffer = lines.pop()
                for line in lines:
                    try:
                        event = json.loads(line)
                    except (ValueError, UnicodeError):
                        continue
                    if provider == 'claude' and isinstance(event, dict) and event.get('subtype') == 'error_max_budget_usd':
                        limit_reached = 'USD'
                    for clean in providers.normalize_event(provider, event):
                        if clean['kind'] == 'text':
                            text = clean['text'][:32000]
                        elif clean['kind'] == 'usage':
                            emit({**clean, 'lens':config.lenses[0]})
                            if 'cost_usd' in clean: cost = clean['cost_usd']
                        elif clean['kind'] == 'error':
                            failed = True
                        elif clean['kind'] == 'result':
                            terminal = True
                            failed = failed or clean.get('ok') is not True
                            if clean.get('ok') and clean.get('text'):
                                text = clean['text'][:32000]
                if eof:
                    break
            code = process.wait(timeout=3)
            ok = code == 0 and terminal and not failed
            self.settle_budget(reservation, cost)
            error = None if ok else 'Native reviewer failed; no completed review was returned.'
            if allowance is not None and cost is not None and cost > allowance:
                ok = False
                limit_reached = 'USD'
                error = f'Reviewer exceeded its USD budget: ${cost:.4f} reported for ${allowance:.4f} allocated. No automatic retry.'
            recorded_cost = self.recorded_cost(config.lenses[0]) if reservation is not None else cost
            return WorkerResult(text=text, ok=ok, cost_usd=recorded_cost, duration_seconds=time.monotonic() - started,
                                error=error, limit_reached=limit_reached)
        except (OSError, ValueError, TimeoutError, subprocess.TimeoutExpired):
            return WorkerResult(text='', ok=False, duration_seconds=time.monotonic() - started,
                                error='Native reviewer could not finish within its execution limits.')
        finally:
            os.close(write_fd)
            if process is not None:
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                process.stdout.close()
                if writer:
                    writer.join(timeout=1)
            selector.close()


def execute(request, lock_fd):
    os.umask(0o077)
    session, settings = request['session'], request['session']['fleet']
    state_dir = Path(request['state_dir'])
    if state_dir.parent.parent.resolve() != state_dir.parent.parent:
        raise ValueError('Unsafe checkpoint parent.')
    state_dir.parent.mkdir(mode=0o700, exist_ok=True)
    if state_dir.parent.resolve() != state_dir.parent:
        raise ValueError('Unsafe checkpoint parent.')
    state_dir.mkdir(mode=0o700, exist_ok=True)
    if state_dir.resolve() != state_dir:
        raise ValueError('Unsafe checkpoint directory.')
    checkpoint = state_dir / 'checkpoints.sqlite'
    if checkpoint.is_symlink() or (checkpoint.exists() and (not checkpoint.is_file() or checkpoint.stat().st_nlink != 1)):
        raise ValueError('Unsafe checkpoint database.')
    config = HarnessConfig(project=Path(session['project_path']), worker=session['provider'],
                           lenses=tuple(settings['lenses']), model=session['model'],
                           thinking_effort=session['thinking_effort'], budget_usd=settings['budget_usd'],
                           worker_timeout_seconds=settings['worker_timeout'], delegate_to_roster=False,
                           state_dir=state_dir, reports_dir=state_dir / 'reports')
    if settings['dry_run']:
        script = {lens: json.dumps({'findings': [{'file': 'DEMO.md', 'line': 1, 'severity': 'low',
                   'claim': 'Offline demonstration finding (' + lens + ')',
                   'evidence': 'Synthetic finding; no project code was reviewed.'}]}) for lens in settings['lenses']}
        worker = DryRunWorker(script=script, cost_per_call_usd=0.0)
        blackboard = NullBlackboard()
    else:
        worker = NativeReviewer(request, lock_fd)
        blackboard = Blackboard(Path(session.get('brain_root', config.project)))
    with closing(sqlite3.connect(checkpoint, check_same_thread=False)) as connection:
        saver = SqliteSaver(connection)
        graph = build_graph(config, worker, blackboard, saver, observer=emit)
        options = {'configurable': {'thread_id': session['id']}, 'max_concurrency': session['agent_count']}
        snapshot = graph.get_state(options)
        action = request['action']
        if not snapshot.values:
            if action in ('approve', 'reject'):
                raise ValueError('The report checkpoint is missing.')
            initial = {'scope': request['scope'], 'task_id': session.get('task_id', 'harness/' + session['id']),
                       'thread_id': session['id'], 'lenses': settings['lenses']}
            graph.invoke(initial, options)
        elif snapshot.next:
            paused = any(task.interrupts for task in snapshot.tasks)
            if paused and action in ('approve', 'reject'):
                graph.invoke(resume_command(action == 'approve'), options)
            elif not paused:
                graph.invoke(None, options)
        snapshot = graph.get_state(options)
        values = snapshot.values
        awaiting = bool(snapshot.next) and any(task.interrupts for task in snapshot.tasks)
        if snapshot.next and not awaiting:
            raise RuntimeError('The graph did not reach a report checkpoint.')
        status = 'awaiting_approval' if awaiting else 'completed' if values.get('approved') else 'rejected'
        findings = dedupe(values.get('findings', []))
        report = values.get('report') or ''
        preview = render_report({**values, 'findings': findings})
        if settings['dry_run']:
            prefix = '> Offline dry-run demonstration. No project code was reviewed; no models were called.\n\n'
            preview = prefix + preview
            if report:
                report = prefix + report
                (config.reports_dir / (session['id'] + '.md')).write_text(report, encoding='utf-8')
        emit({'kind': 'fleet_state', 'status': status, 'findings': findings,
              'cost_usd': values.get('cost_usd'), 'skipped': values.get('skipped', []),
              'report': report, 'preview': preview, 'dry_run': settings['dry_run'],
              'brain_available': blackboard.available, 'task_id': values['task_id']})
        emit({'kind': 'result', 'ok': True, 'text': ''})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--runner-lock', type=int, required=True)
    arguments = parser.parse_args()
    try:
        if not stat.S_ISREG(os.fstat(arguments.runner_lock).st_mode):
            raise ValueError('Missing runner lock.')
        raw = sys.stdin.buffer.read(262145)
        if len(raw) > 262144:
            raise ValueError('Fleet request is too large.')
        execute(json.loads(raw), arguments.runner_lock)
        return 0
    except Exception:
        emit({'kind': 'error', 'text': 'Fleet review stopped. Inspect reviewer status and project runtime; resume retries unfinished graph work.'})
        emit({'kind': 'result', 'ok': False, 'text': ''})
        return 1


if __name__ == '__main__':
    sys.exit(main())
