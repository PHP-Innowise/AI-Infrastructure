"""Race-safe rooted filesystem operations for Harness on POSIX and Windows.

Callers pass directory descriptors and a single literal child name.  The Windows
backend keeps this contract with ``NtCreateFile`` rooted handles, so a junction
or symlink cannot redirect an operation between validation and use.
"""
from __future__ import annotations

import errno
import os
import stat as stat_module

WINDOWS = os.name == 'nt'
# Deliberately high synthetic bits on Windows: do not overlap CRT flags such as
# O_BINARY/O_NOINHERIT, which are handed to ``msvcrt.open_osfhandle``.
O_DIRECTORY = getattr(os, 'O_DIRECTORY', 0x20000000)
O_NOFOLLOW = getattr(os, 'O_NOFOLLOW', 0x40000000)
O_NONBLOCK = getattr(os, 'O_NONBLOCK', 0x08000000)


def _component(name):
    if not isinstance(name, (str, bytes)):
        raise TypeError('A path component must be text or bytes.')
    text = os.fsdecode(name)
    if (not text or text in ('.', '..') or '/' in text or '\\' in text or ':' in text
            or '\x00' in text or text[-1:] in (' ', '.') or any(ord(c) < 32 for c in text)):
        raise ValueError('A filesystem component is unsafe.')
    if WINDOWS and text.rstrip(' .').upper().split('.')[0] in {
            'CON', 'PRN', 'AUX', 'NUL', *(f'COM{i}' for i in range(1, 10)), *(f'LPT{i}' for i in range(1, 10))}:
        raise ValueError('Windows device names are not filesystem components.')
    return text


if not WINDOWS:
    def open(path, flags, mode=0o777, *, dir_fd=None):
        if dir_fd is None:
            return os.open(path, flags, mode)
        return os.open(path, flags, mode, dir_fd=dir_fd)

    def open_metadata(name, *, dir_fd):
        return os.open(name, os.O_RDONLY | O_NOFOLLOW | O_NONBLOCK, dir_fd=dir_fd)

    close = os.close
    dup = os.dup
    fstat = os.fstat

    def stat(path, *, dir_fd=None, follow_symlinks=True):
        if dir_fd is None:
            return os.stat(path, follow_symlinks=follow_symlinks)
        return os.stat(path, dir_fd=dir_fd, follow_symlinks=follow_symlinks)

    def listdir(fd):
        return os.listdir(fd)

    def scandir(fd):
        return os.scandir(fd)

    def mkdir(name, mode=0o777, *, dir_fd=None):
        if dir_fd is None:
            return os.mkdir(name, mode)
        return os.mkdir(name, mode, dir_fd=dir_fd)

    def unlink(name, *, dir_fd=None):
        if dir_fd is None:
            return os.unlink(name)
        return os.unlink(name, dir_fd=dir_fd)

    def rmdir(name, *, dir_fd=None):
        if dir_fd is None:
            return os.rmdir(name)
        return os.rmdir(name, dir_fd=dir_fd)

    def replace(source, destination, *, src_dir_fd=None, dst_dir_fd=None):
        return os.replace(source, destination, src_dir_fd=src_dir_fd, dst_dir_fd=dst_dir_fd)

    def link(source, destination, *, src_dir_fd=None, dst_dir_fd=None, follow_symlinks=False):
        return os.link(source, destination, src_dir_fd=src_dir_fd, dst_dir_fd=dst_dir_fd,
                       follow_symlinks=follow_symlinks)

    def fchmod(fd, mode):
        return os.fchmod(fd, mode)

    def raw_handle(fd):
        return fd

    def open_target_directory(path, *, security=False):
        """Open an absolute directory component-by-component without following links."""
        text = os.fsdecode(os.fspath(path))
        if not os.path.isabs(text):
            raise ValueError('Use an absolute directory path.')
        fd = os.open(os.path.sep, os.O_RDONLY | O_DIRECTORY)
        try:
            for part in (part for part in text.split(os.path.sep) if part):
                child = os.open(part, os.O_RDONLY | O_DIRECTORY | O_NOFOLLOW, dir_fd=fd)
                os.close(fd); fd = child
            return fd
        except Exception:
            os.close(fd)
            raise

    def open_lock(path, mode=0o600):
        parent, name = os.path.split(os.fsdecode(os.fspath(path)))
        root = open_target_directory(parent)
        try:
            fd = os.open(_component(name), os.O_CREAT | os.O_RDWR | O_NOFOLLOW, mode, dir_fd=root)
        finally:
            os.close(root)
        import fcntl
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except Exception:
            os.close(fd)
            raise
        return fd

    def atomic_replace_open(source_fd, destination_dir_fd, destination):
        """Atomically replace a directory entry while retaining the source fd."""
        # POSIX rename works on names; callers should retain the parent fd.
        raise NotImplementedError('Use replace() on POSIX; source fd has no portable name.')

    def link_open(source_fd, destination_dir_fd, destination):
        raise NotImplementedError('Use link() on POSIX; source fd has no portable name.')

else:
    import ctypes
    import msvcrt
    import ntpath
    from ctypes import wintypes

    ULONG = wintypes.ULONG
    USHORT = wintypes.USHORT
    NTSTATUS = ctypes.c_long
    HANDLE = wintypes.HANDLE
    ACCESS_MASK = wintypes.DWORD

    class UNICODE_STRING(ctypes.Structure):
        _fields_ = [('Length', USHORT), ('MaximumLength', USHORT), ('Buffer', wintypes.LPWSTR)]

    class OBJECT_ATTRIBUTES(ctypes.Structure):
        _fields_ = [('Length', ULONG), ('RootDirectory', HANDLE),
                    ('ObjectName', ctypes.POINTER(UNICODE_STRING)), ('Attributes', ULONG),
                    ('SecurityDescriptor', wintypes.LPVOID), ('SecurityQualityOfService', wintypes.LPVOID)]

    class IO_STATUS_BLOCK(ctypes.Structure):
        _fields_ = [('Status', NTSTATUS), ('Information', ctypes.c_size_t)]

    class FILE_ATTRIBUTE_TAG_INFO(ctypes.Structure):
        _fields_ = [('FileAttributes', wintypes.DWORD), ('ReparseTag', wintypes.DWORD)]

    class BY_HANDLE_FILE_INFORMATION(ctypes.Structure):
        _fields_ = [('dwFileAttributes', wintypes.DWORD), ('ftCreationTime_dwLowDateTime', wintypes.DWORD),
                    ('ftCreationTime_dwHighDateTime', wintypes.DWORD), ('ftLastAccessTime_dwLowDateTime', wintypes.DWORD),
                    ('ftLastAccessTime_dwHighDateTime', wintypes.DWORD), ('ftLastWriteTime_dwLowDateTime', wintypes.DWORD),
                    ('ftLastWriteTime_dwHighDateTime', wintypes.DWORD), ('dwVolumeSerialNumber', wintypes.DWORD),
                    ('nFileSizeHigh', wintypes.DWORD), ('nFileSizeLow', wintypes.DWORD),
                    ('nNumberOfLinks', wintypes.DWORD), ('nFileIndexHigh', wintypes.DWORD),
                    ('nFileIndexLow', wintypes.DWORD)]

    _ntdll = ctypes.WinDLL('ntdll', use_last_error=True)
    _kernel32 = ctypes.WinDLL('kernel32', use_last_error=True)
    _nt_create = _ntdll.NtCreateFile
    _nt_create.argtypes = [ctypes.POINTER(HANDLE), ACCESS_MASK, ctypes.POINTER(OBJECT_ATTRIBUTES),
                            ctypes.POINTER(IO_STATUS_BLOCK), wintypes.LPVOID, ULONG, ULONG, ULONG,
                            ULONG, wintypes.LPVOID, ULONG]
    _nt_create.restype = NTSTATUS
    _nt_set = _ntdll.NtSetInformationFile
    _nt_set.argtypes = [HANDLE, ctypes.POINTER(IO_STATUS_BLOCK), wintypes.LPVOID, ULONG, ULONG]
    _nt_set.restype = NTSTATUS
    _nt_query_directory = _ntdll.NtQueryDirectoryFile
    _nt_query_directory.argtypes = [HANDLE, HANDLE, wintypes.LPVOID, wintypes.LPVOID,
                                    ctypes.POINTER(IO_STATUS_BLOCK), wintypes.LPVOID, ULONG,
                                    ULONG, wintypes.BOOL, ctypes.POINTER(UNICODE_STRING), wintypes.BOOL]
    _nt_query_directory.restype = NTSTATUS
    _status_to_error = _ntdll.RtlNtStatusToDosError
    _status_to_error.argtypes = [NTSTATUS]
    _status_to_error.restype = ULONG
    _get_info = _kernel32.GetFileInformationByHandle
    _get_info.argtypes = [HANDLE, ctypes.POINTER(BY_HANDLE_FILE_INFORMATION)]
    _get_info.restype = wintypes.BOOL
    _get_info_ex = _kernel32.GetFileInformationByHandleEx
    _get_info_ex.argtypes = [HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD]
    _get_info_ex.restype = wintypes.BOOL
    _close = _kernel32.CloseHandle
    _close.argtypes = [HANDLE]
    _close.restype = wintypes.BOOL

    FILE_READ_DATA = FILE_LIST_DIRECTORY = 0x0001
    FILE_WRITE_DATA = 0x0002
    FILE_APPEND_DATA = 0x0004
    FILE_READ_ATTRIBUTES = 0x0080
    FILE_WRITE_ATTRIBUTES = 0x0100
    FILE_TRAVERSE = 0x0020
    DELETE = 0x00010000
    SYNCHRONIZE = 0x00100000
    FILE_SHARE_READ, FILE_SHARE_WRITE = 0x1, 0x2
    FILE_OPEN, FILE_CREATE, FILE_OPEN_IF = 1, 2, 3
    FILE_DIRECTORY_FILE, FILE_NON_DIRECTORY_FILE = 0x1, 0x40
    FILE_SYNCHRONOUS_IO_NONALERT, FILE_OPEN_REPARSE_POINT = 0x20, 0x00200000
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT = 0x80, 0x400
    OBJ_CASE_INSENSITIVE = 0x40
    FileAttributeTagInfo = 9
    FileNamesInformation = 12
    FileRenameInformation, FileLinkInformation, FileDispositionInformation = 10, 11, 13

    def _raise(status):
        code = _status_to_error(NTSTATUS(status))
        raise ctypes.WinError(code or errno.EIO)

    def raw_handle(fd):
        return int(msvcrt.get_osfhandle(fd))

    def _unicode(text):
        buffer = ctypes.create_unicode_buffer(text)
        size = len(text.encode('utf-16-le'))
        if size > 0xffff:
            raise ValueError('A filesystem component is too long.')
        return buffer, UNICODE_STRING(size, size, ctypes.cast(buffer, wintypes.LPWSTR))

    def _metadata(handle):
        tag = FILE_ATTRIBUTE_TAG_INFO()
        if not _get_info_ex(HANDLE(handle), FileAttributeTagInfo, ctypes.byref(tag), ctypes.sizeof(tag)):
            raise ctypes.WinError(ctypes.get_last_error())
        if tag.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT:
            raise OSError('A reparse point is unsafe.')
        info = BY_HANDLE_FILE_INFORMATION()
        if not _get_info(HANDLE(handle), ctypes.byref(info)):
            raise ctypes.WinError(ctypes.get_last_error())
        return info

    def _stat_result(handle):
        info = _metadata(handle)
        is_dir = bool(info.dwFileAttributes & 0x10)
        permissions = 0o777 if is_dir else 0o444 if info.dwFileAttributes & 1 else 0o666
        mode = (stat_module.S_IFDIR if is_dir else stat_module.S_IFREG) | permissions
        size = (info.nFileSizeHigh << 32) | info.nFileSizeLow
        index = (info.nFileIndexHigh << 32) | info.nFileIndexLow
        def timestamp(high, low):
            value = (high << 32) | low
            return max(0, (value - 116444736000000000) * 100)
        ctime_ns = timestamp(info.ftCreationTime_dwHighDateTime, info.ftCreationTime_dwLowDateTime)
        atime_ns = timestamp(info.ftLastAccessTime_dwHighDateTime, info.ftLastAccessTime_dwLowDateTime)
        mtime_ns = timestamp(info.ftLastWriteTime_dwHighDateTime, info.ftLastWriteTime_dwLowDateTime)
        # FILETIME is 100ns since 1601.  The exact timestamp is less important than
        # stable identity and size checks; retain nanosecond resolution where possible.
        return os.stat_result((mode, index, info.dwVolumeSerialNumber, info.nNumberOfLinks,
                               0, 0, size, atime_ns // 1_000_000_000,
                               mtime_ns // 1_000_000_000, ctime_ns // 1_000_000_000),
                              {'st_atime_ns': atime_ns, 'st_mtime_ns': mtime_ns, 'st_ctime_ns': ctime_ns})

    def _open_relative(parent_fd, name, access, disposition=FILE_OPEN, options=0,
                       attributes=FILE_ATTRIBUTE_NORMAL, share=None, security_descriptor=None):
        text = _component(name)
        buffer, unicode = _unicode(text)
        attributes_object = OBJECT_ATTRIBUTES(ctypes.sizeof(OBJECT_ATTRIBUTES), HANDLE(raw_handle(parent_fd)),
                                               ctypes.pointer(unicode), OBJ_CASE_INSENSITIVE,
                                               security_descriptor, None)
        output, status = HANDLE(), IO_STATUS_BLOCK()
        result = _nt_create(ctypes.byref(output), access, ctypes.byref(attributes_object), ctypes.byref(status),
                            None, attributes, FILE_SHARE_READ | FILE_SHARE_WRITE if share is None else share, disposition,
                            options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT, None, 0)
        if result < 0:
            _raise(result)
        try:
            _metadata(output.value)
            crt_flags = os.O_BINARY
            if access & FILE_WRITE_DATA:
                crt_flags |= os.O_RDWR if access & FILE_READ_DATA else os.O_WRONLY
            if access & FILE_APPEND_DATA:
                crt_flags |= os.O_APPEND
            return msvcrt.open_osfhandle(output.value, crt_flags)
        except Exception:
            _close(output)
            raise

    def _open_root(path, security=False):
        text = os.fspath(path)
        if isinstance(text, bytes):
            text = os.fsdecode(text)
        drive, tail = ntpath.splitdrive(text)
        if not drive or drive.startswith('\\\\') or not ntpath.isabs(text):
            raise ValueError('Use an absolute local Windows path.')
        root = drive + '\\'
        # CreateFileW is used only for the volume root; untrusted components are
        # then opened one-by-one relative to that rooted handle.
        create = _kernel32.CreateFileW
        create.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
                           wintypes.DWORD, wintypes.DWORD, HANDLE]
        create.restype = HANDLE
        handle = create('\\\\?\\' + root, FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                        FILE_SHARE_READ | FILE_SHARE_WRITE, None, 3,
                        0x02000000 | 0x00200000, None)
        if handle in (None, ctypes.c_void_p(-1).value):
            raise ctypes.WinError(ctypes.get_last_error())
        fd = None
        try:
            _metadata(handle)
            fd = msvcrt.open_osfhandle(handle, os.O_BINARY)
            handle = None
            parts = [item for item in tail.replace('/', '\\').split('\\') if item]
            for index, part in enumerate(parts):
                access = FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE
                if security and index == len(parts) - 1:
                    access |= 0x00020000 | 0x00040000
                child = _open_relative(fd, part, access,
                                       options=FILE_DIRECTORY_FILE)
                os.close(fd); fd = child
            return fd
        except Exception:
            if fd is not None:
                os.close(fd)
            elif handle is not None:
                _close(HANDLE(handle))
            raise

    def open(path, flags, mode=0o777, *, dir_fd=None):
        if dir_fd is None:
            # Absolute paths are walked from the drive root.  A direct file open
            # uses the verified parent handle rather than a path-based fallback.
            text = os.fspath(path)
            parent, name = ntpath.split(text)
            root = _open_root(parent)
            try:
                return open(name, flags, mode, dir_fd=root)
            finally:
                os.close(root)
        want_dir = bool(flags & O_DIRECTORY)
        writing = bool(flags & (os.O_WRONLY | os.O_RDWR))
        creating = bool(flags & os.O_CREAT)
        exclusive = bool(flags & os.O_EXCL)
        if creating and exclusive:
            disposition = FILE_CREATE
        elif creating:
            disposition = FILE_OPEN_IF
        else:
            disposition = FILE_OPEN
        access = SYNCHRONIZE | FILE_READ_ATTRIBUTES
        if want_dir:
            access |= FILE_LIST_DIRECTORY | FILE_TRAVERSE
        elif writing:
            access |= FILE_WRITE_DATA | FILE_READ_DATA
        else:
            access |= FILE_READ_DATA
        if flags & os.O_APPEND:
            access |= FILE_APPEND_DATA
        options = FILE_DIRECTORY_FILE if want_dir else FILE_NON_DIRECTORY_FILE
        return _open_relative(dir_fd, path, access, disposition, options)

    def open_metadata(name, *, dir_fd):
        return _open_relative(dir_fd, name, FILE_READ_ATTRIBUTES | SYNCHRONIZE)

    def open_security(name, *, dir_fd, directory=False):
        access = FILE_READ_ATTRIBUTES | SYNCHRONIZE | 0x00020000 | 0x00040000
        if directory:
            access |= FILE_LIST_DIRECTORY | FILE_TRAVERSE
        return _open_relative(dir_fd, name, access,
                              options=FILE_DIRECTORY_FILE if directory else FILE_NON_DIRECTORY_FILE)

    close = os.close
    dup = os.dup

    def fstat(fd):
        return _stat_result(raw_handle(fd))

    def stat(path, *, dir_fd=None, follow_symlinks=True):
        if follow_symlinks:
            raise ValueError('Portable rooted stat never follows links.')
        if dir_fd is None:
            text = os.fspath(path)
            parent, name = ntpath.split(text)
            root = _open_root(parent)
            try:
                fd = _open_relative(root, name, FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            finally:
                os.close(root)
        else:
            fd = _open_relative(dir_fd, path, FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        try:
            return fstat(fd)
        finally:
            os.close(fd)

    def _names(fd):
        """List a trusted directory by HANDLE, never by reconstructed pathname."""
        result, restart = [], True
        no_more_files = ctypes.c_long(0x80000006).value
        while True:
            buffer = ctypes.create_string_buffer(65536)
            status = IO_STATUS_BLOCK()
            code = _nt_query_directory(HANDLE(raw_handle(fd)), None, None, None, ctypes.byref(status), buffer,
                                       len(buffer), FileNamesInformation, False, None, restart)
            restart = False
            if code == no_more_files:
                return result
            if code < 0:
                _raise(code)
            offset, end = 0, status.Information
            while offset < end:
                next_offset = ctypes.c_ulong.from_buffer(buffer, offset).value
                name_bytes = ctypes.c_ulong.from_buffer(buffer, offset + 8).value
                name = ctypes.string_at(ctypes.addressof(buffer) + offset + 12, name_bytes).decode('utf-16-le')
                if name not in ('.', '..'):
                    result.append(name)
                if not next_offset:
                    break
                offset += next_offset

    class _DirEntry:
        def __init__(self, parent_fd, name):
            self._parent_fd, self.name = parent_fd, name
            self.path = name

        def stat(self, *, follow_symlinks=False):
            return stat(self.name, dir_fd=self._parent_fd, follow_symlinks=follow_symlinks)

        def is_dir(self, *, follow_symlinks=False):
            return stat_module.S_ISDIR(self.stat(follow_symlinks=follow_symlinks).st_mode)

        def is_file(self, *, follow_symlinks=False):
            return stat_module.S_ISREG(self.stat(follow_symlinks=follow_symlinks).st_mode)

    class _Scandir:
        def __init__(self, fd):
            self._fd = os.dup(fd)
            self._entries = iter([_DirEntry(self._fd, name) for name in _names(self._fd)])

        def __iter__(self):
            return self

        def __next__(self):
            return next(self._entries)

        def close(self):
            if self._fd is not None:
                os.close(self._fd); self._fd = None

        def __enter__(self):
            return self

        def __exit__(self, *_):
            self.close()

    def listdir(fd):
        return _names(fd)

    def scandir(fd):
        return _Scandir(fd)

    def mkdir(name, mode=0o777, *, dir_fd=None):
        if dir_fd is None:
            raise ValueError('mkdir requires a trusted parent descriptor on Windows.')
        fd = _open_relative(dir_fd, name, FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                            FILE_CREATE, FILE_DIRECTORY_FILE)
        os.close(fd)

    def mkdir_private(parent_fd, name, security_descriptor):
        """Create a private directory with its DACL at creation time."""
        if not security_descriptor:
            raise ValueError('A private directory requires a security descriptor.')
        return _open_relative(parent_fd, name,
                              FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE | 0x00020000 | 0x00040000,
                              FILE_CREATE, FILE_DIRECTORY_FILE,
                              security_descriptor=security_descriptor)

    def _disposition(parent_fd, name, directory=False):
        access = DELETE | SYNCHRONIZE | FILE_READ_ATTRIBUTES
        options = FILE_DIRECTORY_FILE if directory else FILE_NON_DIRECTORY_FILE
        fd = _open_relative(parent_fd, name, access, FILE_OPEN, options)
        try:
            deleting = ctypes.c_ubyte(1)
            status = IO_STATUS_BLOCK()
            result = _nt_set(HANDLE(raw_handle(fd)), ctypes.byref(status), ctypes.byref(deleting), 1,
                             FileDispositionInformation)
            if result < 0:
                _raise(result)
        finally:
            os.close(fd)

    def unlink(name, *, dir_fd=None):
        if dir_fd is None:
            raise ValueError('unlink requires a trusted parent descriptor on Windows.')
        _disposition(dir_fd, name)

    def rmdir(name, *, dir_fd=None):
        if dir_fd is None:
            raise ValueError('rmdir requires a trusted parent descriptor on Windows.')
        _disposition(dir_fd, name, True)

    def _rename_information(parent_fd, name, replace):
        text = _component(name)
        encoded = text.encode('utf-16-le')
        # BOOLEAN ReplaceIfExists; padding; HANDLE RootDirectory; ULONG length; WCHAR[]
        root_offset = ctypes.sizeof(HANDLE)
        name_length_offset = root_offset + ctypes.sizeof(HANDLE)
        offset = name_length_offset + ctypes.sizeof(ULONG)
        body = ctypes.create_string_buffer(offset + len(encoded))
        body[0] = b'\x01' if replace else b'\x00'
        ctypes.c_void_p.from_address(ctypes.addressof(body) + root_offset).value = raw_handle(parent_fd)
        ctypes.c_ulong.from_address(ctypes.addressof(body) + name_length_offset).value = len(encoded)
        ctypes.memmove(ctypes.addressof(body) + offset, encoded, len(encoded))
        return body

    def atomic_replace_open(source_fd, destination_dir_fd, destination):
        body = _rename_information(destination_dir_fd, destination, True)
        status = IO_STATUS_BLOCK()
        result = _nt_set(HANDLE(raw_handle(source_fd)), ctypes.byref(status), body, len(body), FileRenameInformation)
        if result < 0:
            _raise(result)

    def link_open(source_fd, destination_dir_fd, destination):
        body = _rename_information(destination_dir_fd, destination, False)
        status = IO_STATUS_BLOCK()
        result = _nt_set(HANDLE(raw_handle(source_fd)), ctypes.byref(status), body, len(body), FileLinkInformation)
        if result < 0:
            _raise(result)

    def replace(source, destination, *, src_dir_fd=None, dst_dir_fd=None):
        if src_dir_fd is None or dst_dir_fd is None:
            raise ValueError('replace requires trusted source and destination directories on Windows.')
        fd = _open_relative(src_dir_fd, source, DELETE | SYNCHRONIZE | FILE_READ_ATTRIBUTES,
                            FILE_OPEN, FILE_NON_DIRECTORY_FILE)
        try:
            atomic_replace_open(fd, dst_dir_fd, destination)
        finally:
            os.close(fd)

    def link(source, destination, *, src_dir_fd=None, dst_dir_fd=None, follow_symlinks=False):
        if follow_symlinks or src_dir_fd is None or dst_dir_fd is None:
            raise ValueError('link requires rooted no-follow descriptors on Windows.')
        fd = _open_relative(src_dir_fd, source, SYNCHRONIZE | FILE_READ_ATTRIBUTES,
                            FILE_OPEN, FILE_NON_DIRECTORY_FILE)
        try:
            link_open(fd, dst_dir_fd, destination)
        finally:
            os.close(fd)

    def fchmod(fd, mode):
        # Windows has no POSIX execute-bit model.  The caller must not treat this
        # as a security boundary; ACL setup is a separate explicit operation.
        return None

    def open_target_directory(path, *, security=False):
        return _open_root(path, security=security)

    def open_lock(path, mode=0o600):
        parent, name = ntpath.split(os.fspath(path))
        root = _open_root(parent)
        try:
            text = _component(name); buffer, unicode = _unicode(text)
            attributes = OBJECT_ATTRIBUTES(ctypes.sizeof(OBJECT_ATTRIBUTES), HANDLE(raw_handle(root)),
                                            ctypes.pointer(unicode), OBJ_CASE_INSENSITIVE, None, None)
            output, status = HANDLE(), IO_STATUS_BLOCK()
            result = _nt_create(ctypes.byref(output), FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                                ctypes.byref(attributes), ctypes.byref(status), None, FILE_ATTRIBUTE_NORMAL,
                                0, FILE_OPEN_IF, FILE_NON_DIRECTORY_FILE | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
                                None, 0)
            if result < 0:
                _raise(result)
            try:
                _metadata(output.value)
                return msvcrt.open_osfhandle(output.value, os.O_BINARY)
            except Exception:
                _close(output); raise
        finally:
            os.close(root)
