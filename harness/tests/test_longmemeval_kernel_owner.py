"""Real temporary descriptor witnesses; supplied faults are not OS failures.

Only an inspection-only Native owner is used. No constructor, corpus, custody,
bank import, query or model is run. Four persistent fixture handles are owned;
interpreter/tempfile internals and aggregate host resources are outside scope.
"""
import errno
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
import threading
import unittest
from unittest.mock import patch

from harness.longmemeval_projection import NativeProjectionMemory


CONTROLS = Path(__file__).parent / 'fixtures/longmemeval_kernel_owner/controls.json'
BODY = b'public replacement reference\n'
LABELS = ('alpha', 'beta', 'gamma', 'source')


class SuppliedAbort(BaseException):
    pass


class KernelOwnerTests(unittest.TestCase):
    def _case(self, name):
        expected = next(row for row in json.loads(CONTROLS.read_text())['cases']
                        if row['name'] == name)
        # Each test owns its progress. Unknown/unreached fields remain null.
        self.observation = {key: None for key in expected}
        seen = self.observation
        seen.update(name=name, assertions_completed=False, callbacks=[],
                    api_actions=[], injected_faults=[], cleanup_actions=[],
                    cleanup_issues=[], cleanup_probes={},
                    callback_refusals={'count': 0, 'overflow': False})
        real_open, real_write, real_fstat = os.open, os.write, os.fstat
        real_close, real_dup2, real_pread = os.close, os.dup2, os.pread
        fds, identities = {}, {}
        first_error = OSError('supplied_first_error')
        later_abort = SuppliedAbort('supplied_later_abort')

        def identity(value):
            return value.st_dev, value.st_ino, stat.S_IFMT(value.st_mode)

        def probe(label):
            if label not in fds or label not in identities:
                return {'state': 'unobserved'}
            fd = fds[label]
            try:
                value = real_fstat(fd)
            except OSError as error:
                if error.errno == errno.EBADF:
                    return {'state': 'closed', 'errno': errno.EBADF}
                return {'state': 'unavailable'}
            current = identity(value)
            if label != 'source' and current == identities[label]:
                return {'state': 'original_directory', 'identity_match': True,
                        'directory': stat.S_ISDIR(value.st_mode)}
            if ((label == 'source' or label == 'gamma')
                    and current == identities.get('source')
                    and stat.S_ISREG(value.st_mode)):
                raw = real_pread(fd, len(BODY) + 1, 0)
                return {'state': 'source_file' if label == 'source' else 'replacement_file',
                        'identity_match': True, 'regular': True, 'bytes': len(raw),
                        'sha256': hashlib.sha256(raw).hexdigest()}
            return {'state': 'foreign_or_ambiguous'}

        def owner_image(owner):
            by_fd = {value: key for key, value in fds.items()}
            return {'closed': owner._closed,
                    'registry': [by_fd.get(fd, 'unexpected') for fd in owner._fds]}

        def close_action(label):
            action = {'operation': 'close', 'label': label, 'returned': None}
            seen['api_actions'].append(action)
            real_close(fds[label])
            action['returned'] = True

        # Finalization is outside the close patch and before TemporaryDirectory
        # cleanup. Only fixed, captured identities may receive one close attempt.
        def finalize():
            for label in LABELS:
                if label not in fds or label not in identities:
                    seen['cleanup_issues'].append({'label': label, 'kind': 'unobserved'})
                    continue
                fd = fds[label]
                try:
                    value = real_fstat(fd)
                except OSError as error:
                    if error.errno == errno.EBADF:
                        seen['cleanup_actions'].append({'label': label, 'action': 'already_closed'})
                    else:
                        seen['cleanup_issues'].append({'label': label, 'kind': 'stat_unavailable'})
                    continue
                allowed = {identities[label]}
                if label == 'gamma' and 'source' in identities:
                    allowed.add(identities['source'])
                if identity(value) not in allowed:
                    seen['cleanup_issues'].append({'label': label, 'kind': 'foreign_or_ambiguous'})
                    continue
                action = {'label': label, 'action': 'close_attempt_unknown'}
                seen['cleanup_actions'].append(action)
                try:
                    real_close(fd)
                except BaseException:
                    seen['cleanup_issues'].append({'label': label, 'kind': 'close_unavailable'})
                else:
                    action['action'] = 'closed_matching_identity'
            for label in LABELS:
                seen['cleanup_probes'][label] = probe(label)

        with tempfile.TemporaryDirectory(prefix='lme-kernel-owner-') as directory:
            try:
                root = Path(directory)
                fd = real_open(root / 'replacement.txt',
                               os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
                fds['source'] = fd
                identities['source'] = identity(real_fstat(fd))
                offset = 0
                while offset < len(BODY):
                    wrote = real_write(fd, BODY[offset:])
                    self.assertGreater(wrote, 0)
                    offset += wrote
                for label in LABELS[:3]:
                    node = root / label
                    node.mkdir(mode=0o700)
                    fd = real_open(node, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
                    fds[label] = fd
                    identities[label] = identity(real_fstat(fd))
                seen['distinct_fixture_handles'] = len(set(fds.values()))
                self.assertEqual(seen['distinct_fixture_handles'], 4)
                seen['held_source_before_drain'] = probe('source') == expected['before_probes']['source']
                self.assertTrue(seen['held_source_before_drain'])
                owner = NativeProjectionMemory.__new__(NativeProjectionMemory)
                owner._lock = threading.RLock()
                owner._closed = False
                owner._fds = [fds[label] for label in LABELS[:3]]
                seen['before_owner'] = owner_image(owner)
                seen['before_probes'] = {label: probe(label) for label in LABELS}
                admitted = {fds[label]: label for label in LABELS[:3]}
                attempted = set()

                def callback(value):
                    # Refuse unexpected/repeated handles before touching them.
                    if type(value) is not int or value not in admitted or value in attempted:
                        refused = seen['callback_refusals']
                        if refused['count'] < 4:
                            refused['count'] += 1
                        else:
                            refused['overflow'] = True
                        raise AssertionError('unexpected_or_repeated_owned_close')
                    attempted.add(value)
                    label = admitted[value]
                    seen['callbacks'].append({'label': label, 'owner_closed': owner._closed,
                                              'remaining': owner_image(owner)['registry']})
                    if label == 'gamma' and name == 'postclose_reuse_error':
                        action = {'operation': 'close_then_dup2', 'label': label,
                                  'source': 'source', 'returned': None}
                        seen['api_actions'].append(action)
                        # Adjacent saved C APIs: no descriptor allocation/path IO
                        # may intervene. Group success requires both returns.
                        real_close(value)
                        real_dup2(fds['source'], value)
                        action['returned'] = True
                        seen['injected_faults'].append({'label': label, 'kind': 'postclose_first_error'})
                        raise first_error
                    if label == 'gamma' and name == 'preclose_error_retains_uncertain_handle':
                        seen['injected_faults'].append({'label': label, 'kind': 'preclose_first_error'})
                        raise first_error
                    if name == 'mixed_postclose_error_preclose_abort':
                        if label == 'gamma':
                            close_action(label)
                            seen['injected_faults'].append({'label': label, 'kind': 'postclose_first_error'})
                            raise first_error
                        if label == 'beta':
                            seen['injected_faults'].append({'label': label, 'kind': 'preclose_later_abort'})
                            raise later_abort
                    close_action(label)

                with patch.object(os, 'close', callback):
                    caught = None
                    try:
                        returned = owner.close()
                        self.assertIsNone(returned)
                    except BaseException as error:
                        caught = error
                    seen['outward'] = (None if caught is None else
                                       'first_error' if caught is first_error else 'unexpected')
                    seen['outward_context'] = (None if caught is None or caught.__context__ is None
                                               else 'unexpected_context')
                    seen['after_owner'] = owner_image(owner)
                    seen['after_probes'] = {label: probe(label) for label in LABELS}
                    if name == 'success_three_directory_handles':
                        self.assertIsNone(caught)
                    else:
                        self.assertIs(caught, first_error)
                        self.assertIsNone(caught.__context__)
                    count = len(seen['callbacks'])
                    seen['repeat_return'] = owner.close()
                    seen['repeat_new_callbacks'] = len(seen['callbacks']) - count
                    seen['repeat_probes'] = {label: probe(label) for label in LABELS}
            finally:
                finalize()
        # Do not mark completion until the entire observed record agrees. Worker
        # independently detaches/bounds and compares all four literal oracles.
        actual = {key: value for key, value in seen.items() if key != 'assertions_completed'}
        oracle = {key: value for key, value in expected.items() if key != 'assertions_completed'}
        self.assertEqual(actual, oracle)
        seen['assertions_completed'] = True

    def test_success_three_directory_handles(self):
        self._case('success_three_directory_handles')

    def test_postclose_reuse_error(self):
        self._case('postclose_reuse_error')

    def test_preclose_error_retains_uncertain_handle(self):
        self._case('preclose_error_retains_uncertain_handle')

    def test_mixed_postclose_error_preclose_abort(self):
        self._case('mixed_postclose_error_preclose_abort')


if __name__ == '__main__':
    unittest.main()
