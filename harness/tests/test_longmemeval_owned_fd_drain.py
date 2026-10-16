"""NEW synthetic owned-registry/Path/OS/full-observer controls, not kernel FD proof."""
import copy
import json
from pathlib import Path
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from harness import longmemeval_projection as projection
from harness import longmemeval_observed as observed

FIXTURE = Path(__file__).parent / 'fixtures/longmemeval_owned_fd_drain/controls.json'
NATIVE = projection.NativeProjectionMemory


class OwnerAbort(BaseException):
    pass


class OwnedFdDrainTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = json.loads(FIXTURE.read_text())

    def errors(self):
        return {'close_error': OSError('new-owned-close'), 'close_abort': OwnerAbort(),
                'close_other_abort': OwnerAbort(), 'body_error': OSError('new-owned-construct'),
                'body_abort': OwnerAbort(), 'helper_abort': OwnerAbort()}

    def snapshot(self, node):
        return {'fds': list(node._fds), 'closed': node._closed, 'fenced': node._fenced,
                'documents': dict(node._documents), 'file_ids': dict(node._file_ids),
                'bytes': node._bytes, 'queries': node._queries, 'last': copy.deepcopy(node._last),
                'custody_failure': list(node._custody_failure),
                'slots': {key: node.__dict__.get(attr) for key, attr in
                          [('root', '_root_fd'), ('specs', '_specs_fd'), ('source', '_source_fd')]}}

    def owner(self, state):
        node = NATIVE.__new__(NATIVE)
        node._lock = threading.RLock()
        node._fds = list(state['fds'])
        node._closed, node._fenced = state['closed'], state['fenced']
        node._documents, node._file_ids = dict(state['documents']), dict(state['file_ids'])
        node._bytes, node._queries, node._last = state['bytes'], state['queries'], copy.deepcopy(state['last'])
        node._custody_failure = tuple(state['custody_failure'])
        node._root_fd, node._specs_fd, node._source_fd = (state['slots'][key] for key in ('root', 'specs', 'source'))
        return node

    def closer(self, node, faults, errors, trace):
        def close(fd):
            trace.append({'call': 'close', 'fd': fd, 'registry_at_entry': list(node._fds),
                          'closed_at_entry': node._closed})
            if str(fd) in faults:
                raise errors[faults[str(fd)]]
        return close

    def run_close(self, name):
        case = self.data['close_cases'][name]
        node, errors, trace = self.owner(case['initial']), self.errors(), []
        self.assertEqual(self.snapshot(node), case['initial'])
        with patch.object(projection.os, 'close', self.closer(node, case['faults'], errors, trace)):
            if case['primary'] is None:
                self.assertIsNone(node.close())
            else:
                primary = errors[case['primary']]
                with self.assertRaises(type(primary)) as caught:
                    node.close()
                self.assertIs(caught.exception, primary)
                self.assertIsNone(primary.__context__)
            self.assertEqual(trace, case['trace'])
            self.assertEqual(self.snapshot(node), case['final'])
            self.assertEqual(node.custody_failure_observation(), self.data['custody'])
            before = copy.deepcopy(trace)
            self.assertIsNone(node.close())
            self.assertEqual(trace, before + case['repeat_trace'])
            self.assertEqual(self.snapshot(node), case['final'])
        # Do not call close with synthetic numbers outside the patches.

    def native_api(self, calls):
        def unexpected(*_, **__):
            calls.append('native')
            raise AssertionError('unexpected_native_call')
        return SimpleNamespace(**{name: unexpected for name in self.data['native_methods']})

    def constructor_patches(self, node, case, errors, trace):
        opened = 0

        def is_file(path):
            trace.append({'call': 'is_file', 'path': str(path)})
            return True

        def is_dir(path):
            trace.append({'call': 'is_dir', 'path': str(path)})
            return True

        def glob(path, pattern):
            trace.append({'call': 'glob', 'path': str(path), 'pattern': pattern})
            return iter([Path('owned-new.schema.json')])

        def open_fd(name, flags, *, dir_fd=None):
            nonlocal opened
            trace.append({'call': 'open', 'name': str(name), 'flags': flags, 'dir_fd': dir_fd})
            step = 0 if opened == 0 else 1 if opened == 1 else 3
            if case['fail_at'] == step:
                raise errors[case['primary']]
            value = 901 + opened
            opened += 1
            return value

        def mkdir(name, *, mode, dir_fd):
            trace.append({'call': 'mkdir', 'name': name, 'mode': mode, 'dir_fd': dir_fd})

        def fsync(fd):
            trace.append({'call': 'fsync', 'fd': fd})
            if case['fail_at'] == 4:
                raise errors[case['primary']]

        return (patch.object(Path, 'is_file', is_file), patch.object(Path, 'is_dir', is_dir),
                patch.object(Path, 'glob', glob), patch.object(projection.os, 'open', open_fd),
                patch.object(projection.os, 'mkdir', mkdir), patch.object(projection.os, 'fsync', fsync),
                patch.object(projection.os, 'close', self.closer(node, case['close_faults'], errors, trace)))

    def run_constructor(self, name):
        case = self.data['constructor_cases'][name]
        node, errors, trace, calls = NATIVE.__new__(NATIVE), self.errors(), [], []
        one, two, three, four, five, six, seven = self.constructor_patches(node, case, errors, trace)

        def helper_abort():
            trace.append({'call': 'helper_abort'})
            raise errors['helper_abort']

        helper = patch.object(node, '_drain_owned_fds', helper_abort) if case['helper_fault'] else patch.object(node, '_drain_owned_fds', node._drain_owned_fds)
        with one, two, three, four, five, six, seven, helper:
            primary = errors[case['primary']]
            with self.assertRaises(type(primary)) as caught:
                node.__init__(self.data['repository'], self.native_api(calls))
            self.assertIs(caught.exception, primary)
            self.assertIsNone(primary.__context__)
            self.assertEqual(trace, case['trace'])
            self.assertEqual(self.snapshot(node), case['state'])
            self.assertEqual(node.custody_failure_observation(), self.data['custody'])
            self.assertEqual(calls, [])
        # Failed normal construction does not return this inspection-only object.

    def clock(self, trace):
        ticks = iter(self.data['clock'])
        def now():
            trace.append('clock')
            return next(ticks)
        return now

    def test_close_success_consumes_owned_attempts(self):
        self.run_close('success')

    def test_close_first_error_preserves_identity_and_drains_remaining(self):
        self.run_close('first_error')

    def test_close_first_abort_outranks_later_errors(self):
        self.run_close('first_abort')

    def test_close_ordinary_first_keeps_later_abort_secondary(self):
        self.run_close('later_abort')

    def test_close_empty_registry_and_repeat_do_no_work(self):
        self.run_close('empty')

    def test_already_closed_ambiguous_registry_is_not_adopted(self):
        self.run_close('already_closed_ambiguous')

    def test_constructor_root_failure_has_no_owned_attempts(self):
        self.run_constructor('root_open_error')

    def test_constructor_specs_failure_preserves_primary_over_close_error(self):
        self.run_constructor('specs_open_error')

    def test_constructor_source_failure_drains_partial_registry(self):
        self.run_constructor('source_open_error')

    def test_constructor_abort_outranks_cleanup_errors(self):
        self.run_constructor('sync_abort')

    def test_constructor_helper_abort_does_not_replace_primary(self):
        self.run_constructor('helper_abort')

    def test_observed_failed_close_preserves_full_ledger_and_failure_view(self):
        case = self.data['close_cases']['later_abort']
        node, errors, trace = self.owner(case['initial']), self.errors(), []
        owner = observed.ObservedProjectionMemory(2, clock=self.clock(trace))
        self.assertEqual(owner.snapshot(), self.data['observed_close']['new'])

        def factory(*_):
            trace.append('factory')
            return node

        with patch.object(projection, 'NativeProjectionMemory', factory), \
                patch.object(projection.os, 'close', self.closer(node, case['faults'], errors, trace)):
            owner.open(self.data['repository'], object())
            self.assertEqual(owner.snapshot(), self.data['observed_close']['opened'])
            with self.assertRaises(OSError) as caught:
                owner.close()
            self.assertIs(caught.exception, errors['close_error'])
            self.assertEqual(owner.snapshot(), self.data['observed_close']['final'])
            self.assertEqual(owner.failure_observation(phase='cleanup'), self.data['failure_cleanup'])
            self.assertEqual(owner.failure_observation(phase='initialization'), self.data['failure_empty'])
            self.assertEqual(trace, self.data['observed_close_trace'])
            self.assertEqual(self.snapshot(node), case['final'])
            before = copy.deepcopy(trace)
            self.assertEqual(owner.close(), self.data['observed_close']['final'])
            self.assertEqual(trace, before)

    def test_observed_constructor_abort_keeps_full_cost_and_no_backend(self):
        case = self.data['constructor_cases']['sync_abort']
        node, errors, trace, calls = NATIVE.__new__(NATIVE), self.errors(), [], []
        owner = observed.ObservedProjectionMemory(2, clock=self.clock(trace))
        self.assertEqual(owner.snapshot(), self.data['observed_constructor']['new'])
        api = self.native_api(calls)

        def factory(repo, native):
            trace.append('factory')
            node.__init__(repo, native)
            return node

        one, two, three, four, five, six, seven = self.constructor_patches(node, case, errors, trace)
        with one, two, three, four, five, six, seven, patch.object(projection, 'NativeProjectionMemory', factory):
            with self.assertRaises(OwnerAbort) as caught:
                owner.open(self.data['repository'], api)
            self.assertIs(caught.exception, errors['body_abort'])
            self.assertEqual(owner.snapshot(), self.data['observed_constructor']['opened'])
            self.assertEqual(owner.failure_observation(phase='initialization'), self.data['failure_initialization'])
            self.assertIsNone(owner._backend)
            self.assertEqual(owner.close(), self.data['observed_constructor']['final'])
            self.assertEqual(owner.failure_observation(phase='cleanup'), self.data['failure_empty'])
            self.assertEqual(trace, self.data['observed_ctor_trace'])
            self.assertEqual(self.snapshot(node), case['state'])
            self.assertEqual(calls, [])
            before = copy.deepcopy(trace)
            self.assertEqual(owner.close(), self.data['observed_constructor']['final'])
            self.assertEqual(trace, before)
