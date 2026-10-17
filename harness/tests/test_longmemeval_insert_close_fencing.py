"""Fresh supplied-owner/OS controls; descriptor numbers are synthetic, never real."""
import copy
import json
from pathlib import Path
import threading
import unittest
from unittest.mock import patch

from harness import longmemeval_projection as projection

FIXTURE = Path(__file__).parent / 'fixtures/longmemeval_insert_close_fencing/prior-controls.json'


class CloseAbort(BaseException):
    pass


class NoNative:
    def __init__(self):
        self.calls = 0

    def __getattr__(self, _):
        self.calls += 1
        raise AssertionError('unexpected_native_access')


class InsertCloseFencingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads(FIXTURE.read_text())

    def snapshot(self, node):
        documents = {}
        for key, item in node._documents.items():
            documents[key] = {'trajectory_id': item.trajectory_id, 'relative': item.relative,
                              'body_utf8': item.body.decode('utf-8'), 'sha256': item.sha256,
                              'input_sha256': item.input_sha256, 'states': item.states}
        return {'documents': documents,
                'file_ids': {key: list(value) for key, value in node._file_ids.items()},
                'bytes': node._bytes, 'queries': node._queries, 'last': copy.deepcopy(node._last),
                'closed': node._closed, 'fenced': node._fenced, 'fds': list(node._fds),
                'source_fd': node._source_fd, 'custody_failure': list(node._custody_failure)}

    def owner(self):
        # Exact canonical owner and normal private fields, no constructor/FS calls.
        node = projection.NativeProjectionMemory.__new__(projection.NativeProjectionMemory)
        node._lock = threading.RLock()
        initial = self.data['initial_state']
        node._documents, node._file_ids = {}, {}
        node._bytes = initial['bytes']
        node._queries, node._last = initial['queries'], copy.deepcopy(initial['last'])
        node._closed, node._fenced = initial['closed'], initial['fenced']
        node._fds, node._source_fd = list(initial['fds']), initial['source_fd']
        node._custody_failure = tuple(initial['custody_failure'])
        node.native = NoNative()
        return node

    def run_case(self, name):
        case, data = self.data['cases'][name], self.data
        image = data['projection']
        item = projection.Projection(image['trajectory_id'], image['relative'],
                                     image['body_utf8'].encode('utf-8'), image['sha256'],
                                     image['input_sha256'], image['states'])
        node = self.owner()
        self.assertEqual(self.snapshot(node), data['initial_state'])
        trace = []
        # Independent exception instances avoid cross-case context contamination.
        errors = {'close_error': OSError('new-close-error'), 'close_abort': CloseAbort(),
                  'write_error': OSError('new-write-error'), 'open_error': OSError('new-open-error'),
                  'fsync_error': OSError('new-fsync-error')}
        fault = case['fault']

        def project(value):
            trace.append({'call': 'project', 'input': copy.deepcopy(value)})
            return item

        def custody():
            trace.append({'call': 'custody_ready'})
            node._ready()

        def opened(filename, flags, mode, *, dir_fd):
            trace.append({'call': 'open', 'name': filename, 'flags': flags, 'mode': mode, 'dir_fd': dir_fd})
            if fault == 'open_error':
                raise errors['open_error']
            return 744

        def written(fd, pending):
            trace.append({'call': 'write', 'fd': fd, 'payload_hex': bytes(pending).hex()})
            if fault in ('write_error', 'double_fault'):
                raise errors['write_error']
            return len(pending)

        def synced(fd):
            trace.append({'call': 'fsync', 'fd': fd})
            if fault == 'fsync_error':
                raise errors['fsync_error']

        def identity(fd):
            trace.append({'call': 'identity', 'fd': fd})
            return (2117, 7289)

        def closed(fd):
            trace.append({'call': 'close', 'fd': fd})
            if fault in ('close_error', 'double_fault'):
                raise errors['close_error']
            if fault == 'close_abort':
                raise errors['close_abort']

        node._custody = custody
        with patch.object(projection, 'project_trajectory', project), \
                patch.object(projection.os, 'open', opened), \
                patch.object(projection.os, 'write', written), \
                patch.object(projection.os, 'fsync', synced), \
                patch.object(projection, '_identity', identity), \
                patch.object(projection.os, 'close', closed):
            if case['primary'] is None:
                node.insert(copy.deepcopy(data['input']))
            else:
                primary = errors[case['primary']]
                with self.assertRaises(type(primary)) as caught:
                    node.insert(copy.deepcopy(data['input']))
                self.assertIs(caught.exception, primary)
                context = None if case['context'] is None else errors[case['context']]
                self.assertIs(caught.exception.__context__, context)
            self.assertEqual(trace, case['trace'])
            self.assertEqual(sum(row['call'] == 'close' for row in trace), case['close_calls'])
            self.assertEqual(self.snapshot(node), case['state'])
            self.assertEqual(node.custody_failure_observation(), data['custody_observation'])
            self.assertEqual(node.native.calls, 0)
            if case['state']['fenced']:
                # Real query availability guard, valid input, no downstream work.
                prior_trace = copy.deepcopy(trace)
                with patch.object(projection.time, 'monotonic', side_effect=AssertionError('unexpected_clock')) as clock:
                    with self.assertRaises(projection.ProjectionError) as caught:
                        node.query(data['later_query'])
                    self.assertEqual(list(caught.exception.args), data['later_refusal']['args'])
                    self.assertEqual(type(caught.exception).__name__, data['later_refusal']['exception_type'])
                    self.assertEqual(clock.call_count, data['later_refusal']['clock_calls'])
                self.assertEqual(trace, prior_trace)
                self.assertEqual(node.native.calls, data['later_refusal']['native_calls'])
                self.assertEqual(self.snapshot(node), case['state'])
            else:
                node._ready()
                self.assertEqual(self.snapshot(node), case['state'])
        # Never owner.close(): _fds contains supplied numbers, not owned handles.

    def test_success_retains_publication_and_availability(self):
        self.run_case('success')

    def test_final_close_error_fences_retained_publication(self):
        self.run_case('close_error')

    def test_final_close_abort_fences_retained_publication(self):
        self.run_case('close_abort')

    def test_write_error_keeps_primary_and_one_close(self):
        self.run_case('write_error')

    def test_double_fault_keeps_close_primary_and_write_context(self):
        self.run_case('double_fault')

    def test_open_error_has_no_close_or_publication(self):
        self.run_case('open_error')

    def test_fsync_error_keeps_primary_without_publication(self):
        self.run_case('fsync_error')
