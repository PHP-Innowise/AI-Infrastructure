"""Private Harness state directories without path-based Windows ACL races."""
from __future__ import annotations

import os
from pathlib import Path
import stat
from contextlib import contextmanager

import portable_fs as fs


WINDOWS = os.name == "nt"
ERROR = "Harness state directory is unavailable or insecure."


class StateSecurityError(OSError):
    """A deliberately non-diagnostic state-storage failure."""


def _unsafe() -> StateSecurityError:
    return StateSecurityError(ERROR)


if WINDOWS:
    import ctypes
    from ctypes import wintypes as w

    TOKEN_QUERY = 0x0008
    TokenUser = 1
    TokenOwner = 4
    ERROR_INSUFFICIENT_BUFFER = 122
    SE_FILE_OBJECT = 1
    OWNER_SECURITY_INFORMATION = 0x00000001
    DACL_SECURITY_INFORMATION = 0x00000004
    PROTECTED_DACL_SECURITY_INFORMATION = 0x80000000
    READ_CONTROL = 0x00020000
    WRITE_DAC = 0x00040000
    FILE_READ_ATTRIBUTES = 0x00000080
    SYNCHRONIZE = 0x00100000
    FILE_SHARE_READ = 0x00000001
    FILE_SHARE_WRITE = 0x00000002
    FILE_FLAG_BACKUP_SEMANTICS = 0x02000000
    FILE_FLAG_OPEN_REPARSE_POINT = 0x00200000
    INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value

    class SID_AND_ATTRIBUTES(ctypes.Structure):
        _fields_ = [("Sid", ctypes.c_void_p), ("Attributes", w.DWORD)]

    class TOKEN_USER(ctypes.Structure):
        _fields_ = [("User", SID_AND_ATTRIBUTES)]

    class TOKEN_OWNER(ctypes.Structure):
        _fields_ = [("Owner", ctypes.c_void_p)]

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
    _kernel32.GetCurrentProcess.argtypes = []
    _kernel32.GetCurrentProcess.restype = w.HANDLE
    _kernel32.CloseHandle.argtypes = [w.HANDLE]
    _kernel32.CloseHandle.restype = w.BOOL
    _kernel32.LocalFree.argtypes = [ctypes.c_void_p]
    _kernel32.LocalFree.restype = ctypes.c_void_p
    _advapi32.OpenProcessToken.argtypes = [w.HANDLE, w.DWORD, ctypes.POINTER(w.HANDLE)]
    _advapi32.OpenProcessToken.restype = w.BOOL
    _advapi32.GetTokenInformation.argtypes = [w.HANDLE, w.DWORD, ctypes.c_void_p, w.DWORD,
                                               ctypes.POINTER(w.DWORD)]
    _advapi32.GetTokenInformation.restype = w.BOOL
    _advapi32.EqualSid.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
    _advapi32.EqualSid.restype = w.BOOL
    _advapi32.ConvertSidToStringSidW.argtypes = [ctypes.c_void_p, ctypes.POINTER(w.LPWSTR)]
    _advapi32.ConvertSidToStringSidW.restype = w.BOOL
    _advapi32.ConvertStringSecurityDescriptorToSecurityDescriptorW.argtypes = [w.LPCWSTR, w.DWORD,
                                                                                  ctypes.POINTER(ctypes.c_void_p),
                                                                                  ctypes.POINTER(w.DWORD)]
    _advapi32.ConvertStringSecurityDescriptorToSecurityDescriptorW.restype = w.BOOL
    _advapi32.GetSecurityDescriptorDacl.argtypes = [ctypes.c_void_p, ctypes.POINTER(w.BOOL),
                                                    ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(w.BOOL)]
    _advapi32.GetSecurityDescriptorDacl.restype = w.BOOL
    _advapi32.GetSecurityDescriptorControl.argtypes = [ctypes.c_void_p, ctypes.POINTER(w.WORD),
                                                       ctypes.POINTER(w.DWORD)]
    _advapi32.GetSecurityDescriptorControl.restype = w.BOOL
    _advapi32.GetSecurityInfo.argtypes = [w.HANDLE, w.DWORD, w.DWORD,
                                          ctypes.POINTER(ctypes.c_void_p), ctypes.c_void_p,
                                          ctypes.c_void_p, ctypes.c_void_p,
                                          ctypes.POINTER(ctypes.c_void_p)]
    _advapi32.GetSecurityInfo.restype = w.DWORD
    _advapi32.SetSecurityInfo.argtypes = [w.HANDLE, w.DWORD, w.DWORD,
                                          ctypes.c_void_p, ctypes.c_void_p,
                                          ctypes.c_void_p, ctypes.c_void_p]
    _advapi32.SetSecurityInfo.restype = w.DWORD


    def _close(handle):
        if handle and handle != INVALID_HANDLE_VALUE:
            _kernel32.CloseHandle(w.HANDLE(handle))


    class _CurrentUser:
        def __init__(self):
            token = w.HANDLE()
            if not _advapi32.OpenProcessToken(_kernel32.GetCurrentProcess(), TOKEN_QUERY, ctypes.byref(token)):
                raise _unsafe()
            try:
                size = w.DWORD()
                ok = _advapi32.GetTokenInformation(token, TokenUser, None, 0, ctypes.byref(size))
                if ok or ctypes.get_last_error() != ERROR_INSUFFICIENT_BUFFER or not size.value:
                    raise _unsafe()
                self.buffer = (ctypes.c_byte * size.value)()
                if not _advapi32.GetTokenInformation(token, TokenUser, self.buffer, size, ctypes.byref(size)):
                    raise _unsafe()
                self.sid = ctypes.cast(self.buffer, ctypes.POINTER(TOKEN_USER)).contents.User.Sid
                size = w.DWORD()
                ok = _advapi32.GetTokenInformation(token, TokenOwner, None, 0, ctypes.byref(size))
                if ok or ctypes.get_last_error() != ERROR_INSUFFICIENT_BUFFER or not size.value:
                    raise _unsafe()
                # SID pointers refer into these buffers; retain both for all
                # subsequent ownership checks.
                self.owner_buffer = (ctypes.c_byte * size.value)()
                if not _advapi32.GetTokenInformation(token, TokenOwner, self.owner_buffer, size, ctypes.byref(size)):
                    raise _unsafe()
                self.owner_sid = ctypes.cast(self.owner_buffer, ctypes.POINTER(TOKEN_OWNER)).contents.Owner
                if not self.owner_sid:
                    raise _unsafe()
                sid_text = w.LPWSTR()
                if not self.sid or not _advapi32.ConvertSidToStringSidW(self.sid, ctypes.byref(sid_text)):
                    raise _unsafe()
                try:
                    self.sid_text = sid_text.value
                    self.sddl = "O:" + self.sid_text + "D:P(A;OICI;FA;;;" + self.sid_text + ")(A;OICI;FA;;;SY)"
                finally:
                    _kernel32.LocalFree(ctypes.cast(sid_text, ctypes.c_void_p))
            finally:
                _close(token.value)


    @contextmanager
    def _descriptor(user):
        descriptor = ctypes.c_void_p()
        if not _advapi32.ConvertStringSecurityDescriptorToSecurityDescriptorW(
                user.sddl, 1, ctypes.byref(descriptor), None):
            raise _unsafe()
        try:
            yield descriptor
        finally:
            _kernel32.LocalFree(descriptor)


    def _owner(handle):
        owner, descriptor = ctypes.c_void_p(), ctypes.c_void_p()
        result = _advapi32.GetSecurityInfo(w.HANDLE(handle), SE_FILE_OBJECT,
                                           OWNER_SECURITY_INFORMATION, ctypes.byref(owner), None, None,
                                           None, ctypes.byref(descriptor))
        if result:
            raise _unsafe()
        return owner.value, descriptor.value


    def _protect(handle, user):
        owner, owner_descriptor = _owner(handle)
        try:
            # New objects use TokenOwner, which may be an enabled group for an
            # elevated process. Accept only this token's explicit default owner
            # or user, then make the user the owner and protect the DACL.
            # Leaving a group owner would retain that group's implicit WRITE_DAC.
            if not owner or not (_advapi32.EqualSid(owner, user.sid)
                                 or _advapi32.EqualSid(owner, user.owner_sid)):
                raise _unsafe()
        finally:
            _kernel32.LocalFree(owner_descriptor)
        with _descriptor(user) as descriptor:
            present, dacl, defaulted = w.BOOL(), ctypes.c_void_p(), w.BOOL()
            if not _advapi32.GetSecurityDescriptorDacl(descriptor, ctypes.byref(present),
                                                       ctypes.byref(dacl), ctypes.byref(defaulted)) or not present:
                raise _unsafe()
            result = _advapi32.SetSecurityInfo(
                w.HANDLE(handle), SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                user.sid, None, dacl, None,
            )
            if result:
                raise _unsafe()


    def _secure_fd(fd, user):
        # Security rights were requested at the rooted open. No second open of
        # a live exclusive admission lock is needed.
        _protect(fs.raw_handle(fd), user)


    def _acl_summary(fd):
        """Return protected-DACL state and allow ACE masks from a safe handle.

        It is intentionally private test support, never an HTTP response or
        diagnostic path.
        """
        handle = fs.raw_handle(fd)
        descriptor = ctypes.c_void_p()
        dacl = ctypes.c_void_p()
        try:
            result = _advapi32.GetSecurityInfo(w.HANDLE(handle), SE_FILE_OBJECT,
                                               DACL_SECURITY_INFORMATION, None, None,
                                               ctypes.byref(dacl), None, ctypes.byref(descriptor))
            if result or not dacl.value:
                raise _unsafe()
            control, revision = w.WORD(), w.DWORD()
            if not _advapi32.GetSecurityDescriptorControl(descriptor, ctypes.byref(control), ctypes.byref(revision)):
                raise _unsafe()
            count = ctypes.c_ushort.from_address(dacl.value + 4).value
            offset, entries = 8, []
            for _ in range(count):
                address = dacl.value + offset
                ace_type, ace_flags = ctypes.c_ubyte.from_address(address).value, ctypes.c_ubyte.from_address(address + 1).value
                size = ctypes.c_ushort.from_address(address + 2).value
                if ace_type != 0 or size < 12:
                    raise _unsafe()
                sid_text = w.LPWSTR()
                if not _advapi32.ConvertSidToStringSidW(ctypes.c_void_p(address + 8), ctypes.byref(sid_text)):
                    raise _unsafe()
                try:
                    mask = ctypes.c_ulong.from_address(address + 4).value
                    entries.append((ace_flags, mask, sid_text.value))
                finally:
                    _kernel32.LocalFree(ctypes.cast(sid_text, ctypes.c_void_p))
                offset += size
            return bool(control.value & 0x1000), entries
        finally:
            _kernel32.LocalFree(descriptor)



    def _is_private(fd, user):
        protected, entries = _acl_summary(fd)
        expected = {(0x03, 0x1f01ff, user.sid_text), (0x03, 0x1f01ff, 'S-1-5-18')}
        return protected and set(entries) == expected


def _walk_existing(fd, user, *, state_root=False):
    """Protect every retained child handle; unsupported enumeration fails closed."""
    metadata = fs.fstat(fd)
    if not stat.S_ISDIR(metadata.st_mode):
        raise _unsafe()
    if WINDOWS:
        _secure_fd(fd, user)
    with fs.scandir(fd) as entries:
        for entry in entries:
            name = entry.name
            metadata = entry.stat(follow_symlinks=False)
            child = fs.open_security(name, dir_fd=fd, directory=stat.S_ISDIR(metadata.st_mode))
            try:
                child_metadata = fs.fstat(child)
                if stat.S_ISDIR(child_metadata.st_mode):
                    if state_root and name == "worktrees":
                        _secure_fd(child, user)
                    else:
                        _walk_existing(child, user)
                elif not stat.S_ISREG(child_metadata.st_mode) or child_metadata.st_nlink != 1:
                    raise _unsafe()
                elif WINDOWS:
                    _secure_fd(child, user)
            finally:
                fs.close(child)


def secure_private_dir(path: Path, *, migrate=True) -> Path:
    """Return an existing private state directory or fail without diagnostics."""
    path = Path(path).expanduser().absolute()
    if not WINDOWS:
        path.mkdir(parents=True, exist_ok=True, mode=0o700)
        if path.is_symlink():
            raise _unsafe()
        os.chmod(path, 0o700)
        return path
    try:
        user = _CurrentUser()
        root = fs.open_target_directory(Path(path.anchor))
        try:
            parts = path.parts[1:]
            if not parts:
                raise _unsafe()
            for index, part in enumerate(parts):
                try:
                    if index == len(parts) - 1:
                        child = fs.open_security(part, dir_fd=root, directory=True)
                    else:
                        child = fs.open(part, os.O_RDONLY | fs.O_DIRECTORY | fs.O_NOFOLLOW, dir_fd=root)
                except FileNotFoundError:
                    # Creation receives the final protected DACL atomically; a
                    # custom state path must never briefly inherit a shared ACL.
                    with _descriptor(user) as descriptor:
                        child = fs.mkdir_private(root, part, descriptor)
                fs.close(root)
                root = child
                if index == len(parts) - 1:
                    if migrate:
                        _walk_existing(root, user, state_root=True)
                    else:
                        _secure_fd(root, user)
                        for filename in ('server.json', 'server.log'):
                            try:
                                item = fs.open_security(filename, dir_fd=root)
                            except FileNotFoundError:
                                continue
                            try:
                                if fs.fstat(item).st_nlink != 1:
                                    raise _unsafe()
                                _secure_fd(item, user)
                            finally:
                                fs.close(item)
        finally:
            fs.close(root)
        return path
    except (OSError, ValueError, TypeError):
        raise _unsafe() from None
