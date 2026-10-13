"""Fresh native boundary regressions using public offline processes only."""
import json
import os
from pathlib import Path
import sys
import threading
import unittest
from unittest import mock
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'harness/src'))
from harness import cost_observation as cost, memory_recovery, native_diagnostics as native, sessions
from tests import test_harness_sessions as support


class NativeDiagnosticsTests(unittest.TestCase):
    def test_failed_terminal_usage_is_countable_and_private_fields_are_not_retained(self):
        observation = cost.LaunchObservation('codex')
        observation.new_phase('main'); observation.process('main', 'started')
        observation.raw('main', {'type': 'error', 'message': 'PRIVATE_ERROR', 'code': 'PRIVATE_CODE'})
        observation.raw('main', {'type': 'item.completed', 'item': {'type': 'error', 'message': 'PRIVATE_ITEM'}})
        observation.raw('main', {'type': 'turn.failed', 'error': {'message': 'PRIVATE_TERMINAL'},
            'usage': {'input_tokens': 11, 'output_tokens': 3}})
        observation.diagnostic('main', 'guard_exit_before_cleanup', 7)
        result = observation.finish('failed'); row = result['native_diagnostics']['main']
        self.assertEqual(14, result['provider']['provider_tokens'])
        self.assertFalse(result['observer_incomplete'])
        self.assertEqual((1, 1, 1), (row['provider_error_events'], row['nonfatal_item_error_events'], row['failed_terminals']))
        self.assertEqual({'code': 7, 'state': 'observed'}, row['guard_exits']['guard_exit_before_cleanup'])
        self.assertEqual('unmeasured', row['root_cause']); self.assertEqual('not_captured', row['stderr'])
        self.assertNotIn('PRIVATE_', json.dumps(result))

    def test_claude_unknown_and_contradictory_types_http_numeric_only(self):
        row = native.NativeDiagnostics('claude')
        for subtype, flag, status in [('success', True, 401), ('PRIVATE_SUBTYPE', None, 'PRIVATE_HTTP'),
                                      ('error_max_turns', False, True), ('success', False, None)]:
            row.observe({'type': 'result', 'subtype': subtype, 'is_error': flag, 'api_error_status': status,
                         'errors': ['PRIVATE_TEXT'], 'terminal_reason': 'PRIVATE_REASON'})
        result = row.finish()
        self.assertEqual({'success': 2, 'unknown': 1, 'turn_limit': 1}, result['reported_subtypes'])
        self.assertEqual((1, 2, 1, 1), (result['successful_terminals'], result['failed_terminals'],
                                     result['unknown_terminals'], result['contradictory_terminals']))
        self.assertEqual([{'status': 401, 'state': 'observed'}, {'status': None, 'state': 'invalid'},
                          {'status': None, 'state': 'invalid'}, {'status': None, 'state': 'unknown'}], result['reported_http_statuses'])
        self.assertNotIn('PRIVATE_', json.dumps(result))

    def test_claude_aborted_or_unknown_reason_never_implies_task_success(self):
        observed = native.NativeDiagnostics('claude')
        for reason in ('aborted_streaming', 'aborted_tools', 'completed', 'PRIVATE_REASON', [], None):
            observed.observe({'type': 'result', 'subtype': 'success', 'is_error': False, 'terminal_reason': reason})
        row = observed.finish()
        self.assertEqual({'aborted_streaming': 1, 'aborted_tools': 1, 'completed': 1, 'unknown': 2, 'invalid': 1},
                         row['reported_terminal_reasons'])
        self.assertEqual(6, row['successful_terminals'])  # Event predicate only, not task correctness.
        self.assertEqual('unmeasured', row['root_cause']); self.assertNotIn('PRIVATE_REASON', json.dumps(row))

    def test_guard_codes_are_bounded_and_conflicts_clear_the_code(self):
        for invalid in (True, False, 1.0, 'PRIVATE_CODE', [], -(2 ** 31) - 1, 2 ** 32):
            row = native.NativeDiagnostics('codex'); row.boundary('guard_exit_before_cleanup', invalid)
            self.assertEqual({'code': None, 'state': 'unknown'}, row.finish()['guard_exits']['guard_exit_before_cleanup'])
        row = native.NativeDiagnostics('codex'); row.boundary('guard_exit_before_cleanup', 143)
        row.boundary('guard_exit_after_cleanup', -9); row.boundary('guard_exit_before_cleanup', 127)
        result = row.finish()
        self.assertEqual({'code': None, 'state': 'conflict'}, result['guard_exits']['guard_exit_before_cleanup'])
        self.assertEqual({'code': -9, 'state': 'observed'}, result['guard_exits']['guard_exit_after_cleanup'])
        self.assertEqual('unmeasured', result['root_cause'])

    def test_capacity_and_atomic_freeze_ignore_late_callbacks_and_return_copies(self):
        row = native.NativeDiagnostics('codex')
        for _ in range(native.LIMIT + 1): row.observe({'type': 'error', 'message': 'PRIVATE'})
        for _ in range(native.LIMIT + 1): row.boundary('malformed_line')
        result = row.finish()
        self.assertEqual(native.LIMIT, result['events']); self.assertEqual(native.LIMIT, result['provider_error_events'])
        self.assertEqual(native.LIMIT, result['boundary_signals']['malformed_line']); self.assertTrue(result['overflow'])
        self.assertEqual('incomplete', result['coverage'])
        thread = threading.Thread(target=lambda: [row.observe({'type': 'turn.failed'}), row.boundary('guard_exit_after_cleanup', 9)])
        thread.start(); thread.join()
        self.assertEqual(result, row.finish())
        result['boundary_signals'].clear(); self.assertIn('malformed_line', row.snapshot()['boundary_signals'])

    def test_diagnostics_failure_does_not_change_tokens_and_attempt_survives_cost_phase_failure(self):
        observation = cost.LaunchObservation('codex'); observation.new_phase('main'); observation.process('main', 'started')
        with mock.patch.object(observation.diagnostics['main'], 'observe', side_effect=RuntimeError('PRIVATE')):
            observation.raw('main', {'type': 'turn.completed', 'usage': {'input_tokens': 5, 'output_tokens': 2}})
        with mock.patch.object(cost, 'Phase', side_effect=RuntimeError('PRIVATE_PHASE')):
            cost.safe(observation, 'new_phase', 'recovery')
        observation.diagnostic('recovery', 'launch_exception')
        result = observation.finish('completed')
        self.assertEqual(7, result['provider']['known_token_subtotal'])
        self.assertIsNone(result['provider']['provider_tokens'])
        self.assertEqual('incomplete', result['native_diagnostics']['main']['coverage'])
        self.assertEqual({'launch_exception': 1}, result['native_diagnostics']['recovery']['boundary_signals'])
        self.assertFalse(result['processes']['recovery']['started']); self.assertNotIn('PRIVATE', json.dumps(result))
        isolated = cost.LaunchObservation('codex'); isolated.new_phase('main'); isolated.process('main', 'started')
        with mock.patch.object(isolated.diagnostics['main'], 'observe', side_effect=RuntimeError('PRIVATE')):
            isolated.raw('main', {'type': 'turn.failed', 'usage': {'input_tokens': 5, 'output_tokens': 2}})
        self.assertEqual(7, isolated.finish('failed')['provider']['provider_tokens'])

    def test_main_recovery_and_unsupported_profiles_keep_separate_evidence(self):
        observed = cost.LaunchObservation('claude')
        for label in ('main', 'recovery'): observed.new_phase(label); observed.process(label, 'started')
        raw = {'type': 'result', 'subtype': 'success', 'is_error': False,
               'usage': {'input_tokens': 4, 'output_tokens': 2, 'cache_read_input_tokens': 3, 'cache_creation_input_tokens': 1}}
        observed.raw('main', raw); observed.raw('recovery', raw)
        observed.diagnostic('recovery', 'shared_budget')
        result = observed.finish('completed')
        self.assertEqual(20, result['provider']['provider_tokens'])
        self.assertEqual(1, result['native_diagnostics']['main']['successful_terminals'])
        self.assertEqual(1, result['native_diagnostics']['recovery']['successful_terminals'])
        self.assertEqual({}, result['native_diagnostics']['main']['boundary_signals'])
        cursor = native.NativeDiagnostics('cursor'); cursor.observe(raw)
        self.assertEqual('unsupported', cursor.finish()['profile'])


OFFLINE_CLI = r'''
import json, sys
config=json.loads(sys.argv[1]); sys.stdin.read()
print('malformed-public-fixture',flush=True)
print('[]',flush=True)
print(json.dumps({'type':'thread.started','thread_id':'native-original'}),flush=True)
print(json.dumps({'type':'error','message':'PRIVATE_RAW_ERROR'}),flush=True)
print(json.dumps({'type':'turn.failed','error':{'message':'PRIVATE_FAILURE'},'usage':{'input_tokens':9,'output_tokens':2}}),flush=True)
sys.exit(7)
'''


@unittest.skipUnless(hasattr(os, 'killpg'), 'Offline process integration requires POSIX guards')
class NativeBoundaryIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.fixture = support.SessionTests('runTest'); self.fixture.setUp()
        self.addCleanup(self.fixture.doCleanups)
        self.fixture.fake.write_text(OFFLINE_CLI)

    def full_cost(self, manager, sid):
        with manager.lock:
            raw = manager.db.execute('SELECT receipt FROM launches WHERE session_id=? ORDER BY id DESC', (sid,)).fetchone()
        return json.loads(raw['receipt'])['full_cost']

    def test_primary_failed_public_process_retains_boundary_evidence_and_usage(self):
        manager = self.fixture.manager(); sid = self.fixture.create(manager, 'new-diagnostic-fixture')
        self.assertEqual('failed', self.fixture.settled(manager, sid)['status'])
        result = self.full_cost(manager, sid); row = result['native_diagnostics']['main']
        self.assertEqual(11, result['provider']['provider_tokens'])
        self.assertEqual((1, 1), (row['provider_error_events'], row['failed_terminals']))
        self.assertEqual(1, row['boundary_signals']['malformed_line']); self.assertEqual(1, row['boundary_signals']['non_object_line'])
        self.assertEqual({'code': 7, 'state': 'observed'}, row['guard_exits']['guard_exit_before_cleanup'])
        self.assertEqual(1, row['boundary_signals']['cleanup_confirmed']); self.assertEqual(1, len(self.fixture.calls))
        self.assertNotIn('PRIVATE_', json.dumps(result))

    def test_primary_launch_exception_stays_attempted_with_unknown_start_and_no_private_text(self):
        manager = self.fixture.manager()
        with mock.patch.object(sessions.process_runtime, 'launch_guarded', side_effect=OSError('PRIVATE_SPAWN')):
            sid = self.fixture.create(manager, 'new-spawn-failure'); self.fixture.settled(manager, sid)
        result = self.full_cost(manager, sid)
        self.assertFalse(result['processes']['main']['started']); self.assertIsNone(result['provider']['provider_tokens'])
        self.assertEqual({'launch_exception': 1, 'lifecycle_exception': 1}, result['native_diagnostics']['main']['boundary_signals'])
        self.assertEqual('unmeasured', result['native_diagnostics']['main']['root_cause'])
        self.assertNotIn('PRIVATE_SPAWN', json.dumps(result))

    def test_budget_on_final_unterminated_terminal_records_one_enforced_stop(self):
        self.fixture.fake.write_text("import json,sys\njson.loads(sys.argv[1]);sys.stdin.read()\n"
            "print(json.dumps({'type':'thread.started','thread_id':'native-original'}),flush=True)\n"
            "sys.stdout.write(json.dumps({'type':'turn.completed','usage':{'input_tokens':9,'output_tokens':2}}));sys.stdout.flush()\n")
        manager=self.fixture.manager();sid=self.fixture.create(manager,'new-eof-token-budget',budgets={'tokens':11})
        self.assertEqual('failed',self.fixture.settled(manager,sid)['status'])
        result=self.full_cost(manager,sid);row=result['native_diagnostics']['main']
        self.assertEqual(1,row['boundary_signals']['token_budget'])
        self.assertEqual(1,row['successful_terminals']);self.assertEqual(11,result['provider']['provider_tokens'])
        self.assertEqual(1,len(self.fixture.calls));self.assertEqual('unmeasured',row['root_cause'])

    def test_eof_controller_stop_records_one_observation(self):
        self.fixture.fake.write_text("import json,sys\njson.loads(sys.argv[1]);sys.stdin.read()\n"
            "print(json.dumps({'type':'thread.started','thread_id':'native-original'}),flush=True)\n"
            "sys.stdout.write(json.dumps({'type':'turn.completed','usage':{'input_tokens':4,'output_tokens':1}}));sys.stdout.flush()\n")
        normalizer=sessions.providers.normalize_event
        for signal,status in [('cancelled','cancelled'),('shutdown','interrupted')]:
            with self.subTest(signal=signal):
                manager=self.fixture.manager(state=self.fixture.root/signal)
                def cancel_on_terminal(provider,event,**options):
                    clean=normalizer(provider,event,**options)
                    if event.get('type')=='turn.completed':
                        with manager.lock:
                            if signal=='shutdown':manager.stopping.set()
                            else:manager.cancelled.update(manager.generations)
                    return clean
                with mock.patch.object(sessions.providers,'normalize_event',side_effect=cancel_on_terminal):
                    sid=self.fixture.create(manager,'new-eof-stop');self.assertEqual(status,self.fixture.settled(manager,sid)['status'])
                row=self.full_cost(manager,sid)['native_diagnostics']['main']
                self.assertEqual(1,row['boundary_signals'][signal]);self.assertEqual(1,row['successful_terminals'])
                self.assertEqual('unmeasured',row['root_cause'])

    def test_recovery_raw_is_once_before_normalization_and_boundary_callbacks_are_optional(self):
        manager = self.fixture.manager()
        # No primary turn is launched: the recovery's caller-owned public session is synthetic.
        sid='public-recovery'; generation=71; manager.generations[sid]=generation
        session={'provider':'codex','native_session_id':'native-original','budgets':{'tokens':None,'usd':None,'seconds':None},
                 'model':'fixture-model','budget_usage':{}}
        observed=cost.LaunchObservation('codex'); observed.new_phase('recovery','native-original')
        normalized=[]
        result=memory_recovery.recover(manager,sid,session,generation,self.fixture.project,dict(os.environ),
            remaining_seconds=5,exhausted=False,observe=lambda raw,clean: normalized.append(clean) or False,
            observe_raw=lambda raw: observed.raw('recovery',raw),on_launch=lambda: observed.process('recovery','started'),
            observe_transport=lambda count,state: observed.transport('recovery',count,state),
            observe_boundary=lambda signal,code=None: observed.diagnostic('recovery',signal,code))
        self.assertEqual('failed',result['state']); self.assertTrue(result['launched'])
        receipt=observed.finish('failed'); row=receipt['native_diagnostics']['recovery']
        self.assertEqual(3,row['events']); self.assertGreater(len(normalized),row['events'])
        self.assertEqual(11,receipt['provider']['provider_tokens']); self.assertEqual(1,row['failed_terminals'])
        self.assertEqual(7,row['guard_exits']['guard_exit_before_cleanup']['code'])
        self.assertEqual(1,row['boundary_signals']['cleanup_confirmed']); self.assertNotIn('PRIVATE_',json.dumps(receipt))
        self.assertNotIn(sid,manager.active)
        # A new skipped operation proves observer exceptions do not change the ordinary result.
        with mock.patch.object(memory_recovery.providers,'build_command',side_effect=AssertionError('Must not launch')):
            skipped=memory_recovery.recover(manager,sid,{**session,'native_session_id':None},generation,self.fixture.project,{},
                remaining_seconds=5,exhausted=False,observe=lambda *_:False,
                observe_boundary=lambda *_: (_ for _ in ()).throw(RuntimeError('PRIVATE_CALLBACK')))
        self.assertEqual('skipped',skipped['state']); self.assertFalse(skipped['launched'])

    def test_recovery_setup_exception_uses_required_two_argument_boundary_contract(self):
        manager=self.fixture.manager();sid='public-setup';manager.generations[sid]=72
        session={'provider':'codex','native_session_id':'native-original','budgets':{'tokens':None,'usd':None},
                 'model':'fixture-model','budget_usage':{}}
        boundaries=[]
        with mock.patch.object(memory_recovery.providers,'build_command',side_effect=RuntimeError('PRIVATE_SETUP')):
            result=memory_recovery.recover(manager,sid,session,72,self.fixture.project,{},remaining_seconds=5,
                exhausted=False,observe=lambda *_:False,observe_boundary=lambda signal,code:boundaries.append((signal,code)))
        self.assertEqual('failed',result['state']);self.assertFalse(result['launched'])
        self.assertEqual([('lifecycle_exception',None)],boundaries);self.assertEqual([],self.fixture.calls)
        self.assertNotIn('PRIVATE_SETUP',json.dumps(result))


if __name__ == '__main__': unittest.main(verbosity=2)
