"""Non-mutating native sandbox checks, executed before each Creator phase."""
import ctypes
from ctypes import wintypes as w
from pathlib import Path
import sys
import uuid
import os


def main():
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    kernel.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, ctypes.c_void_p,
                                  w.DWORD, w.DWORD, w.HANDLE]
    kernel.CreateFileW.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    kernel.CloseHandle.restype = w.BOOL
    invalid = ctypes.c_void_p(-1).value
    # Open existing objects for rights without exercising those rights: no
    # probe file, ACL, content, or timestamp is changed in a protected root.
    workspace = os.path.normcase(os.path.abspath(sys.argv[1]))
    checked = set()
    for root in map(Path, sys.argv[2:]):
        pending = [iter((root,))]
        while pending:
            try:
                path = next(pending[-1])
            except StopIteration:
                pending.pop()
                continue
            key = os.path.normcase(os.path.abspath(path))
            if key in checked or key == workspace:
                continue
            checked.add(key)
            if len(checked) > 100000:
                raise RuntimeError('Protected Creator tree exceeds its verification limit.')
            rights = (0x2, 0x4, 0x10, 0x100, 0x10000, 0x40000, 0x80000)
            if path.is_dir():
                rights += (0x40,)  # FILE_DELETE_CHILD belongs to directories.
            for right in rights:
                handle = kernel.CreateFileW(str(path), right, 7, None, 3, 0x02200000, None)
                if handle != invalid:
                    kernel.CloseHandle(handle)
                    raise RuntimeError('Windows Creator sandbox did not enforce the read-only boundary.')
                if ctypes.get_last_error() != 5:
                    raise RuntimeError('Windows Creator sandbox boundary could not be verified.')
            if path.is_dir():
                pending.append(iter(path.iterdir()))
    path = Path.cwd() / '.harness-tmp' / ('probe-' + uuid.uuid4().hex)
    path.write_text('private workspace probe')
    path.unlink()
    print('Windows Creator read-only boundaries verified.', flush=True)


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('Windows Creator requires a functioning elevated Codex sandbox; boundary verification failed.', flush=True)
        raise SystemExit(1)
