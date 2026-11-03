#!/usr/bin/python3
"""Opt-in EntryFetchThrottle artifact oracle v2; stdout is a bounded receipt.

Supply immutable --source and --public-test files in explicit sealed read roots.
This version adds exact-source include_path collisions; it never updates v1 studies.

Every candidate execution is in a fresh nested namespace containing only its
script, selected source and optional public shadow, public test, and PHP runtime.
No label/reference/oracle directory is mounted into that namespace.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import subprocess
import sys
import tempfile
import time

PROFILE = 'entryfetch-throttle-oracle-v2'
PRIVATE_ROOT = Path(__file__).resolve().parent
SOURCE_REL = Path('packages/website-public-api/app/src/Service/EntryFetchThrottle.php')
TEST_REL = Path('packages/website-public-api/app/tests/Unit/Service/EntryFetchThrottleTest.php')
PHP = '/usr/bin/php'
BWRAP = '/usr/bin/bwrap'
OUTPUT_CAP = 65536
TIMEOUT = 5
MAX_SCRIPT_BYTES = 1024 * 1024
DISABLED = ','.join(('sleep', 'usleep', 'time_nanosleep', 'time_sleep_until',
                     'time', 'microtime', 'hrtime', 'gettimeofday', 'date', 'gmdate'))

class InfrastructureUnknown(Exception):
    pass


def system_links(root='/'):
    """/bin, /lib and /lib64 laid out as this system lays them out.

    Arch links /lib64 to usr/lib, Debian and Ubuntu to usr/lib64, where their
    loader is; a system without a merged /usr keeps folders, bound read-only.
    """
    mounts = []
    for name in ('bin', 'lib', 'lib64'):
        path = os.path.join(root, name)
        if os.path.islink(path):
            mounts += ['--symlink', os.readlink(path), '/' + name]
        elif os.path.isdir(path):
            mounts += ['--ro-bind', path, '/' + name]
    return mounts


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def public_inputs(source_file, test_file):
    return source_file.read_bytes(), test_file.read_bytes()


def variants(source):
    """All negative controls are valid PHP behavior changes to public criteria."""
    s = source.decode('utf-8')
    def replace(old, new):
        if s.count(old) != 1:
            raise InfrastructureUnknown('mutation anchor changed')
        return s.replace(old, new).encode('utf-8')
    allowed = "return $this->consume('car_financing_entry_fetch_' . sha1($this->callerId()), self::MAX_ATTEMPTS)\n            && $this->consume('car_financing_entry_fetch_global', self::MAX_GLOBAL_ATTEMPTS);"
    caller = "$this->consume('car_financing_entry_fetch_' . sha1($this->callerId()), self::MAX_ATTEMPTS)"
    glob = "$this->consume('car_financing_entry_fetch_global', self::MAX_GLOBAL_ATTEMPTS)"
    deny = "if ($attempts >= $maxAttempts) {\n            return false;\n        }"
    controls = [
        ('caller-cap-59', replace('private const MAX_ATTEMPTS = 60;', 'private const MAX_ATTEMPTS = 59;')),
        ('caller-cap-61', replace('private const MAX_ATTEMPTS = 60;', 'private const MAX_ATTEMPTS = 61;')),
        ('global-cap-5999', replace('private const MAX_GLOBAL_ATTEMPTS = 6000;', 'private const MAX_GLOBAL_ATTEMPTS = 5999;')),
        ('global-cap-6001', replace('private const MAX_GLOBAL_ATTEMPTS = 6000;', 'private const MAX_GLOBAL_ATTEMPTS = 6001;')),
        ('window-59', replace('private const WINDOW_SECONDS = 60;', 'private const WINDOW_SECONDS = 59;')),
        ('window-61', replace('private const WINDOW_SECONDS = 60;', 'private const WINDOW_SECONDS = 61;')),
        ('allow-all', replace(allowed, 'return true;')),
        ('deny-all', replace(allowed, 'return false;')),
        ('no-global-charge', replace(allowed, 'return ' + caller + ';')),
        ('no-caller-charge', replace(allowed, 'return ' + glob + ';')),
        ('global-first', replace(allowed, 'return ' + glob + ' && ' + caller + ';')),
        ('eager-global-on-caller-denial', replace(allowed, 'return (bool) (' + caller + ' & ' + glob + ');')),
        ('shared-all-callers', replace("sha1($this->callerId())", "sha1('all-callers')")),
        ('request-object-identity', replace("sha1($this->callerId())", "sha1((string) spl_object_id($this->requestStack->getCurrentRequest()))")),
        ('instance-local-identity', replace("sha1($this->callerId())", "sha1($this->callerId() . ':' . spl_object_id($this))")),
        ('denial-increments', replace(deny, "if ($attempts >= $maxAttempts) {\n            $item->set([$attempts + 1, $windowStartedAt]);\n            $item->expiresAfter($remaining);\n            $this->cache->save($item);\n            return false;\n        }")),
        ('denial-resaves', replace(deny, "if ($attempts >= $maxAttempts) {\n            $item->set([$attempts, $windowStartedAt]);\n            $item->expiresAfter($remaining);\n            $this->cache->save($item);\n            return false;\n        }")),
        ('global-denial-increments', replace(deny, "if ($attempts >= $maxAttempts) {\n            if ($key === 'car_financing_entry_fetch_global') {\n                $item->set([$attempts + 1, $windowStartedAt]);\n                $item->expiresAfter($remaining);\n                $this->cache->save($item);\n            }\n            return false;\n        }")),
        ('global-denial-resaves', replace(deny, "if ($attempts >= $maxAttempts) {\n            if ($key === 'car_financing_entry_fetch_global') {\n                $item->set([$attempts, $windowStartedAt]);\n                $item->expiresAfter($remaining);\n                $this->cache->save($item);\n            }\n            return false;\n        }")),
        ('ttl-only-first-save', replace('$item->expiresAfter($remaining);', 'if ($attempts === 0) { $item->expiresAfter($remaining); }')),
        ('ttl-always-full-window', replace('$item->expiresAfter($remaining);', '$item->expiresAfter(self::WINDOW_SECONDS);')),
        ('sliding-window', replace('$item->set([$attempts + 1, $windowStartedAt]);', '$windowStartedAt = time();\n        $remaining = self::WINDOW_SECONDS;\n        $item->set([$attempts + 1, $windowStartedAt]);')),
        ('saved-start-changes', replace('$item->set([$attempts + 1, $windowStartedAt]);', '$item->set([$attempts + 1, time()]);')),
        ('counter-stays-one', replace('$item->set([$attempts + 1, $windowStartedAt]);', '$item->set([1, $windowStartedAt]);')),
        ('reset-after-boundary', replace('if ($remaining <= 0) {', 'if ($remaining < 0) {')),
        ('no-hit-expiry-reset', replace('if ($remaining <= 0) {', 'if (false && $remaining <= 0) {')),
        ('ignore-valid-hit-pair', replace('$item->isHit() && is_array($item->get())', 'false')),
        ('nonarray-hit-starts-used', replace(': [0, time()];', ': [$item->isHit() ? 59 : 0, time()];')),
        ('miss-starts-two', replace(': [0, time()];', ': [1, time()];')),
        ('separate-fallback-identities', replace("?? 'unknown-caller'", "?? ($this->requestStack->getCurrentRequest() === null ? 'no-request' : 'no-ip')")),
        ('fallback-unlimited', replace(allowed, "if ($this->requestStack->getCurrentRequest()?->getClientIp() === null) { return true; }\n        " + allowed)),
        ('fallback-skips-global', replace(allowed, 'return ' + caller + ' && ($this->requestStack->getCurrentRequest()?->getClientIp() === null || ' + glob + ');')),
    ]
    # Bucket-specific controls prevent a caller-only TTL test from standing in
    # for the separately disclosed global TTL/reset requirements (and vice versa).
    for bucket, condition in (('global', "$key === 'car_financing_entry_fetch_global'"),
                              ('caller', "$key !== 'car_financing_entry_fetch_global'")):
        controls += [
            (bucket + '-ttl-never-set', replace('$item->expiresAfter($remaining);',
                'if (!(' + condition + ')) { $item->expiresAfter($remaining); }')),
            (bucket + '-ttl-only-first', replace('$item->expiresAfter($remaining);',
                'if (!(' + condition + ') || $attempts === 0) { $item->expiresAfter($remaining); }')),
            (bucket + '-ttl-always-full', replace('$item->expiresAfter($remaining);',
                '$item->expiresAfter((' + condition + ') ? self::WINDOW_SECONDS : $remaining);')),
            (bucket + '-saved-start-slides', replace('$item->set([$attempts + 1, $windowStartedAt]);',
                '$item->set([$attempts + 1, (' + condition + ') ? time() : $windowStartedAt]);')),
            (bucket + '-late-boundary-reset', replace('if ($remaining <= 0) {',
                'if ($remaining < 0 || ($remaining === 0 && !(' + condition + '))) {')),
            (bucket + '-no-hit-expiry-reset', replace('if ($remaining <= 0) {',
                'if ($remaining <= 0 && !(' + condition + ')) {')),
            (bucket + '-nonarray-starts-used', replace(': [0, time()];',
                ': [($item->isHit() && (' + condition + ')) ? 59 : 0, time()];')),
            (bucket + '-miss-starts-two', replace(': [0, time()];',
                ': [(! $item->isHit() && (' + condition + ')) ? 1 : 0, time()];')),
        ]
    memo = s.replace('private const WINDOW_SECONDS = 60;', 'private array $seen = [];\n    private const WINDOW_SECONDS = 60;')
    memo = memo.replace(allowed, "$request = $this->requestStack->getCurrentRequest();\n        if ($request !== null) {\n            $id = spl_object_id($request);\n            if (isset($this->seen[$id])) { return true; }\n            $this->seen[$id] = true;\n        }\n        " + allowed)
    controls.append(('memoize-one-request', memo.encode()))
    coupled = s.replace('private const WINDOW_SECONDS = 60;', 'private ?int $sharedStart = null;\n    private const WINDOW_SECONDS = 60;')
    coupled = coupled.replace('$remaining = self::WINDOW_SECONDS - (time() - $windowStartedAt);', '$this->sharedStart ??= $windowStartedAt;\n        $windowStartedAt = $this->sharedStart;\n        $remaining = self::WINDOW_SECONDS - (time() - $windowStartedAt);')
    controls.append(('coupled-bucket-window-starts', coupled.encode()))
    equivalent = re.sub(r'/\*.*?\*/', '', s, flags=re.S)
    equivalent = re.sub(r'^\s*#[^\n]*$', '', equivalent, flags=re.M)
    equivalent = equivalent.replace('\n', '\n\n')
    return [('original', source, True), ('equivalent-comments-whitespace', equivalent.encode(), True)] + [(name, raw, False) for name, raw in controls]


def execute(argv, timeout=TIMEOUT, *, startup_marker=None):
    """Bound elapsed time and output before waiting; never unbounded communicate()."""
    output = bytearray()
    output_bound = OUTPUT_CAP + (len(startup_marker) if startup_marker else 0)
    process = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, env={}, start_new_session=True)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    deadline = time.monotonic() + timeout
    reason = None
    try:
        while selector.get_map():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                reason = 'timeout'
                break
            for key, _ in selector.select(min(remaining, .1)):
                block = os.read(key.fd, 8192)
                if not block:
                    selector.unregister(key.fileobj)
                else:
                    output.extend(block)
                    if len(output) > output_bound:
                        reason = 'output-cap'
                        break
            if reason:
                break
        if reason:
            os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=1)
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        raise
    finally:
        selector.close()
        process.stdout.close()
    raw = bytes(output[:output_bound])
    if startup_marker is not None:
        # The trusted bootstrap writes atomically to this merged pipe before
        # candidate inclusion. No candidate output can precede or remove it.
        if not raw.startswith(startup_marker):
            raise InfrastructureUnknown('trusted PHP startup was not observed')
        raw = raw[len(startup_marker):]
    return {'code': process.returncode, 'output': raw.decode('utf-8', 'replace'), 'limit': reason}


def nested(script, source, test, argument='relative', *, lint=False, shadow_source=None):
    """Fresh copy only. All mounted task inputs are read-only at generic paths."""
    with tempfile.TemporaryDirectory(prefix='b-oracle-nested-') as temporary:
        folder = Path(temporary)
        program = folder / 'program.php'
        source_file = folder / 'source.php'
        test_file = folder / 'test.php'
        program.write_bytes(script)
        source_file.write_bytes(source)
        test_file.write_bytes(test)
        source_file.chmod(0 if argument == 'unreadable' else 0o444)
        collision_mounts = []
        if argument == 'bare-collision':
            if shadow_source is None:
                raise InfrastructureUnknown('collision source absent')
            shadow_file = folder / 'shadow.php'
            shadow_file.write_bytes(shadow_source)
            collision_mounts = ['--ro-bind', str(source_file), '/work/EntryFetchThrottle.php',
                                '--ro-bind', str(shadow_file), '/shadow/EntryFetchThrottle.php']
        command = [BWRAP, '--die-with-parent', '--new-session', '--unshare-pid', '--as-pid-1',
                   '--unshare-net', '--unshare-ipc', '--unshare-uts', '--tmpfs', '/',
                   '--ro-bind', '/usr', '/usr', *system_links(),
                   '--proc', '/proc', '--dev', '/dev', '--tmpfs', '/tmp', '--dir', '/work',
                   '--ro-bind', str(program), '/work/verification/throttle_regression.php',
                   '--ro-bind', str(source_file), '/work/' + SOURCE_REL.as_posix(),
                   '--ro-bind', str(test_file), '/work/' + TEST_REL.as_posix(),
                   *collision_mounts, '--chdir', '/work', '--clearenv', '--setenv', 'PATH', '/usr/bin:/bin',
                   '--setenv', 'LANG', 'C.UTF-8', '--setenv', 'TZ', 'UTC', '--',
                   # The file it resolves to: Debian and Ubuntu link
                   # /usr/bin/php through /etc/alternatives, not mounted here.
                   os.path.realpath(PHP), '-n', '-d', 'zend.assertions=-1',
                   '-d', 'memory_limit=128M', '-d', 'max_execution_time=4',
                   '-d', 'disable_functions=' + DISABLED]
        if argument == 'bare-collision':
            command += ['-d', 'include_path=/shadow:.']
        startup_marker = None
        if lint:
            command += ['-l', '/work/' + SOURCE_REL.as_posix()]
        else:
            startup_marker = ('B-START:' + os.urandom(16).hex() + '\n').encode('ascii')
            # No extra mount or private path: bootstrap is fixed -r code.
            # Restore ordinary CLI globals before loading the supplied script.
            bootstrap = (
                'if (fwrite(STDOUT, ' + json.dumps(startup_marker.decode('ascii')) + ') !== '
                + str(len(startup_marker)) + ') { exit(125); } fflush(STDOUT); '
                'array_shift($argv); $argc = count($argv); '
                '$_SERVER["argv"] = $argv; $_SERVER["argc"] = $argc; '
                '$_SERVER["SCRIPT_FILENAME"] = $argv[0]; '
                '$_SERVER["PHP_SELF"] = $argv[0]; $_SERVER["SCRIPT_NAME"] = $argv[0]; '
                'require $argv[0];'
            )
            command += ['-r', bootstrap, '--', '/work/verification/throttle_regression.php']
            if argument == 'relative' or argument == 'unreadable':
                command += [SOURCE_REL.as_posix()]
            elif argument == 'bare-collision':
                command += ['EntryFetchThrottle.php']
            elif argument == 'absolute':
                command += ['/work/' + SOURCE_REL.as_posix()]
            elif argument == 'missing-file':
                command += ['missing/EntryFetchThrottle.php']
            elif argument != 'missing-argument':
                raise InfrastructureUnknown('invalid invocation mode')
        return execute(command, startup_marker=startup_marker)


def isolation_probe(source, test):
    sentinel = str(Path(__file__).resolve())
    probe = '''<?php
$source = %s;
$result = [
  'privateDenied' => !file_exists(%s),
  'homeDenied' => !file_exists('/home'),
  'referenceDenied' => !file_exists(%s),
  'outerArgsDenied' => !str_contains(file_get_contents('/proc/1/cmdline'), %s),
  'sourceReadonly' => @file_put_contents($source, 'tampered') === false,
  'testReadonly' => @file_put_contents(%s, 'tampered') === false,
  'pid' => getmypid(),
  'net' => readlink('/proc/self/ns/net'),
  'sourceDigest' => hash_file('sha256', $source),
];
echo json_encode($result);
''' % (json.dumps('/work/' + SOURCE_REL.as_posix()), json.dumps(sentinel), json.dumps(str(Path(__file__).resolve().with_name('reference.php'))), json.dumps(str(PRIVATE_ROOT)), json.dumps('/work/' + TEST_REL.as_posix()))
    result = nested(probe.encode(), source, test)
    if result['code'] != 0 or result['limit']:
        raise InfrastructureUnknown('nested isolation did not run: ' + result['output'][:1000])
    try:
        receipt = json.loads(result['output'])
    except ValueError as error:
        raise InfrastructureUnknown('nested isolation probe receipt invalid') from error
    if not (receipt.get('privateDenied') is True and receipt.get('homeDenied') is True
            and receipt.get('referenceDenied') is True and receipt.get('outerArgsDenied') is True
            and receipt.get('sourceReadonly') is True and receipt.get('testReadonly') is True
            and receipt.get('pid') == 1 and receipt.get('sourceDigest') == sha(source)
            and receipt.get('net') != os.readlink('/proc/self/ns/net')):
        raise InfrastructureUnknown('nested isolation invariants not established')
    return receipt


def meaningful_failure(result):
    # No prescribed wording or language. Reject silent exits, parser failures,
    # and a success printout reused for failed scenarios.
    return (result['code'] != 0 and not result['limit']
            and len(result['output'].strip()) >= 4
            and re.search(r'(?:^|\n)(?:PHP )?Parse error:', result['output']) is None)


def evaluate(candidate, source_file, test_file):
    source, test = public_inputs(source_file, test_file)
    probe = isolation_probe(source, test)  # infrastructure first, before any correctness result
    script_path = candidate / 'verification/throttle_regression.php'
    if not script_path.is_file() or script_path.is_symlink():
        return False, {'reason': 'missing ordinary verification/throttle_regression.php', 'isolation': probe}
    if script_path.stat().st_size > MAX_SCRIPT_BYTES:
        return False, {'reason': 'script exceeds bounded profile', 'isolation': probe}
    script = script_path.read_bytes()
    # Scope check compares only the disclosed original files; no Brain/Bank read.
    for relative, expected in ((SOURCE_REL, source), (TEST_REL, test)):
        path = candidate / relative
        if not path.is_file() or path.is_symlink() or path.read_bytes() != expected:
            return False, {'reason': 'public source/test changed or absent', 'isolation': probe}
    records = []
    success_output = None
    correct = True
    controls = variants(source)
    # Both directions matter: the argument must select the exact CWD source,
    # even when include_path offers an opposite-behavior file with the same name.
    broken = next(raw for name, raw, _ in controls if name == 'allow-all')
    for name, selected, shadow, expected in (
            ('exact-source-good-shadow-bad', source, broken, True),
            ('exact-source-bad-shadow-good', broken, source, False)):
        result = nested(script, selected, test, 'bare-collision', shadow_source=shadow)
        passed = (result['code'] == 0 and result['limit'] is None) if expected else meaningful_failure(result)
        records.append({'case': name, 'source_sha256': sha(selected),
                        'shadow_sha256': sha(shadow), 'expected_accept': expected,
                        'passed': passed, **result})
        if not passed:
            return False, {'isolation': probe, 'script_sha256': sha(script), 'records': records}
    for name, image, expected in controls:
        # Infrastructure sanity: every mutation must itself remain executable PHP.
        lint = nested(b'<?php', image, test, lint=True)
        if lint['code'] != 0 or lint['limit']:
            raise InfrastructureUnknown('invalid source control ' + name + ': ' + lint['output'][:500])
        result = nested(script, image, test)
        observed = result['code'] == 0 and result['limit'] is None
        passed = observed if expected else meaningful_failure(result)
        if expected and success_output is None:
            success_output = result['output']
        if not expected and result['output'] == success_output:
            passed = False
        records.append({'case': name, 'source_sha256': sha(image), 'expected_accept': expected,
                        'passed': passed, **result})
        correct = correct and passed
        if not passed:
            return False, {'isolation': probe, 'script_sha256': sha(script), 'records': records}
    for argument in ('absolute', 'missing-argument', 'missing-file', 'unreadable'):
        result = nested(script, source, test, argument)
        passed = (result['code'] == 0 and result['limit'] is None) if argument == 'absolute' else meaningful_failure(result)
        if argument != 'absolute' and result['output'] == success_output:
            passed = False
        records.append({'case': 'argument-' + argument, 'passed': passed, **result})
        correct = correct and passed
        if not passed:
            return False, {'isolation': probe, 'script_sha256': sha(script), 'records': records}
    return correct, {'isolation': probe, 'script_sha256': sha(script), 'records': records}


def main():
    global PHP
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--public-test', type=Path, required=True)
    # A trusted case names the PHP it sealed: in the verifier's namespace a
    # link through /etc/alternatives leads nowhere.
    parser.add_argument('--php', default=PHP)
    args = parser.parse_args()
    PHP = args.php
    try:
        correct, _ = evaluate(Path.cwd(), args.source, args.public_test)
        print(json.dumps({'schema': 'task-outcome-verifier-v1', 'completed': True, 'correct': correct}, separators=(',', ':')))
        return 0
    except (InfrastructureUnknown, OSError, ValueError, subprocess.SubprocessError) as error:
        # Nonzero gives verify_task unknown, never a false candidate label.
        print('B oracle infrastructure unavailable: ' + str(error), file=sys.stderr)
        return 70

if __name__ == '__main__':
    raise SystemExit(main())
