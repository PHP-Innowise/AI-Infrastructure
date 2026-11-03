//! Windows desktop persistence through checked COM and registry APIs.
//! The parent executes only its separately admitted current engine. Shortcut
//! targets are passive data; the worker never launches them.
use crate::{HarnessError, Result};
use serde::{Deserialize, Serialize};

pub(super) const REQUEST_LIMIT: usize = 1024 * 1024;
#[cfg(windows)]
pub(super) const CUSTODY_EXIT: i32 = 74;
const PATH_UNITS: usize = 32767;
#[cfg(windows)]
const DESCRIPTION: &str =
    "Choose a project folder and work with Claude Code, Codex or Cursor Agent in your browser";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    target: Vec<u16>,
    icon: Vec<u16>,
    working_directory: Vec<u16>,
    staging_parent: Vec<u16>,
    staging_parent_identity: (u64, u64, u32),
}
fn invalid() -> HarnessError {
    HarnessError::invalid("Invalid native desktop shortcut request.")
}
fn absolute_units(value: &[u16]) -> bool {
    (value.len() >= 3
        && matches!(value[0], 65..=90 | 97..=122)
        && value[1] == 58
        && [47, 92].contains(&value[2]))
        || (value.len() >= 3 && value[0] == 92 && value[1] == 92)
}
fn checked_units(value: &[u16]) -> Result<()> {
    if value.is_empty() || value.len() > PATH_UNITS || value.contains(&0) || !absolute_units(value)
    {
        return Err(invalid());
    }
    Ok(())
}
fn decode(raw: &[u8]) -> Result<Request> {
    let value =
        infra_core::strict_json::parse(raw, REQUEST_LIMIT, 16, 1_000_000).map_err(|_| invalid())?;
    let request: Request = serde_json::from_value(value).map_err(|_| invalid())?;
    if request.version != 1 {
        return Err(invalid());
    }
    for value in [
        &request.target,
        &request.icon,
        &request.working_directory,
        &request.staging_parent,
    ] {
        checked_units(value)?;
    }
    Ok(request)
}
fn terminated(value: &[u16]) -> Result<Vec<u16>> {
    if value.contains(&0) {
        return Err(invalid());
    }
    let mut result = value.to_vec();
    result.push(0);
    Ok(result)
}
fn registry_bytes(value: &[u16]) -> Result<Vec<u8>> {
    Ok(terminated(value)?
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect())
}

#[cfg(windows)]
mod native {
    use super::*;
    use infra_core::bank::publication::{PublicationFiles, private::CreatedNativeDirectory};
    use std::{
        ffi::{OsStr, OsString, c_void},
        marker::PhantomData,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::{Path, PathBuf},
        ptr::{self, NonNull},
        rc::Rc,
        time::Duration,
    };
    use windows_sys::{
        Win32::{
            Foundation::{ERROR_SUCCESS, S_FALSE, S_OK},
            Storage::FileSystem::WIN32_FIND_DATAW,
            System::{
                Com::{
                    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance,
                    CoInitializeEx, CoUninitialize,
                },
                Registry::{
                    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
                    RegCloseKey, RegCreateKeyExW, RegSetValueExW,
                },
            },
        },
        core::{GUID, HRESULT, IUnknown_Vtbl},
    };
    const ICON_LIMIT: usize = 8 * 1024 * 1024;
    const SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
    const IID_SHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
    const IID_PERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);
    const LINK_HEADER: &[u8] = b"\x4c\0\0\0\x01\x14\x02\0\0\0\0\0\xc0\0\0\0\0\0\0\x46";

    type SetText = unsafe extern "system" fn(*mut c_void, *const u16) -> HRESULT;
    type GetText = unsafe extern "system" fn(*mut c_void, *mut u16, i32) -> HRESULT;
    // Layout copied from Microsoft's generated SDK ShObjIdl_core.h, IShellLinkW.
    // All 21 slots are declared, including unused inherited and getter methods.
    #[repr(C)]
    struct ShellLinkVTable {
        _base: IUnknown_Vtbl,
        _get_path: unsafe extern "system" fn(
            *mut c_void,
            *mut u16,
            i32,
            *mut WIN32_FIND_DATAW,
            u32,
        ) -> HRESULT,
        _get_id_list: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
        _set_id_list: unsafe extern "system" fn(*mut c_void, *const c_void) -> HRESULT,
        _get_description: GetText,
        set_description: SetText,
        _get_working_directory: GetText,
        set_working_directory: SetText,
        _get_arguments: GetText,
        set_arguments: SetText,
        _get_hotkey: unsafe extern "system" fn(*mut c_void, *mut u16) -> HRESULT,
        _set_hotkey: unsafe extern "system" fn(*mut c_void, u16) -> HRESULT,
        _get_show_command: unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT,
        _set_show_command: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
        _get_icon_location:
            unsafe extern "system" fn(*mut c_void, *mut u16, i32, *mut i32) -> HRESULT,
        set_icon_location: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
        _set_relative_path: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> HRESULT,
        _resolve: unsafe extern "system" fn(*mut c_void, *mut c_void, u32) -> HRESULT,
        set_path: SetText,
    }
    // IPersistFile inherits IPersist::GetClassID after the three IUnknown slots.
    #[repr(C)]
    struct PersistFileVTable {
        _base: IUnknown_Vtbl,
        _get_class_id: unsafe extern "system" fn(*mut c_void, *mut GUID) -> HRESULT,
        _is_dirty: unsafe extern "system" fn(*mut c_void) -> HRESULT,
        _load: unsafe extern "system" fn(*mut c_void, *const u16, u32) -> HRESULT,
        save: unsafe extern "system" fn(*mut c_void, *const u16, i32) -> HRESULT,
        _save_completed: SetText,
        _get_current_file: unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> HRESULT,
    }
    #[repr(C)]
    struct ComObject<T> {
        table: *const T,
    }
    struct ComPointer<T> {
        pointer: NonNull<ComObject<T>>,
        _thread: PhantomData<Rc<()>>,
    }
    impl<T> ComPointer<T> {
        fn raw(&self) -> *mut c_void {
            self.pointer.as_ptr().cast()
        }
        fn table(&self) -> &T {
            // SAFETY: pointers come only from successful CoCreateInstance or QI
            // using the matching SDK IID; both own a live reference.
            unsafe { &*self.pointer.as_ref().table }
        }
        fn unknown(&self) -> &IUnknown_Vtbl {
            // SAFETY: every declared interface starts with the IUnknown vtable.
            unsafe { &**self.pointer.as_ptr().cast::<*const IUnknown_Vtbl>() }
        }
        fn query<U>(&self, iid: &GUID) -> Result<ComPointer<U>> {
            let mut output = ptr::null_mut();
            // SAFETY: the matching IID is constant and output is writable.
            succeeded(unsafe { (self.unknown().QueryInterface)(self.raw(), iid, &mut output) })?;
            let pointer = NonNull::new(output.cast()).ok_or_else(failed)?;
            Ok(ComPointer {
                pointer,
                _thread: PhantomData,
            })
        }
    }
    impl<T> Drop for ComPointer<T> {
        fn drop(&mut self) {
            // SAFETY: this owner holds exactly the reference acquired at creation/QI.
            unsafe { (self.unknown().Release)(self.raw()) };
        }
    }
    struct Apartment(PhantomData<Rc<()>>);
    impl Apartment {
        fn new() -> Result<Self> {
            // A fresh worker thread has no inherited COM apartment state.
            let code = unsafe { CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) };
            if ![S_OK, S_FALSE].contains(&code) {
                return Err(failed());
            }
            Ok(Self(PhantomData))
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            // Both S_OK and S_FALSE initializations require one uninitialization.
            unsafe { CoUninitialize() };
        }
    }
    fn failed() -> HarnessError {
        HarnessError::unavailable("Windows shortcut creation failed.")
    }
    fn custody() -> HarnessError {
        HarnessError {
            status: 503,
            code: "desktop-staging-custody",
            message: "Desktop staging custody changed; temporary state was preserved.".into(),
        }
    }
    fn succeeded(code: HRESULT) -> Result<()> {
        if code < 0 { Err(failed()) } else { Ok(()) }
    }
    fn saved(code: HRESULT) -> Result<()> {
        if code == S_OK { Ok(()) } else { Err(failed()) }
    }
    fn shell_link() -> Result<ComPointer<ShellLinkVTable>> {
        let mut output = ptr::null_mut();
        // SAFETY: an initialized apartment, no aggregation, and matching IID.
        succeeded(unsafe {
            CoCreateInstance(
                &SHELL_LINK,
                ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_SHELL_LINK_W,
                &mut output,
            )
        })?;
        Ok(ComPointer {
            pointer: NonNull::new(output.cast()).ok_or_else(failed)?,
            _thread: PhantomData,
        })
    }
    fn wide(path: &Path) -> Result<Vec<u16>> {
        if !path.is_absolute() {
            return Err(invalid());
        }
        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        checked_units(&units)?;
        Ok(units)
    }
    fn path(units: &[u16]) -> Result<PathBuf> {
        checked_units(units)?;
        let path = PathBuf::from(OsString::from_wide(units));
        if !path.is_absolute() {
            return Err(invalid());
        }
        Ok(path)
    }
    fn packet(
        target: &Path,
        icon: &Path,
        directory: &Path,
        staging_parent: &Path,
        staging_parent_identity: (u64, u64, u32),
    ) -> Result<Vec<u8>> {
        let request = Request {
            version: 1,
            target: wide(target)?,
            icon: wide(icon)?,
            working_directory: wide(directory)?,
            staging_parent: wide(staging_parent)?,
            staging_parent_identity,
        };
        let raw = serde_json::to_vec(&request)?;
        decode(&raw)?;
        Ok(raw)
    }
    // No automatic pathname deletion: only explicit cleanup after process proof.
    struct Staging {
        path: PathBuf,
        parent: PublicationFiles,
        root: CreatedNativeDirectory,
        files: PublicationFiles,
    }
    impl Staging {
        fn create(parent: &Path, expected: Option<(u64, u64, u32)>) -> Result<Self> {
            let parent_path = infra_core::fs::absolute(parent)?;
            let mut parent = PublicationFiles::new(&parent_path).map_err(|_| custody())?;
            if expected.is_some_and(|identity| parent.root_identity() != identity) {
                return Err(custody());
            }
            let name = format!("desktop-shortcut-{}", uuid::Uuid::new_v4().simple());
            let root = parent
                .create_private_native_directory_capability(OsStr::new(&name))
                .map_err(|_| custody())?;
            let path = parent_path.join(name);
            let files = PublicationFiles::new(&path).map_err(|_| custody())?;
            let stage = Self {
                path,
                parent,
                root,
                files,
            };
            stage.check()?;
            Ok(stage)
        }
        fn check(&self) -> Result<()> {
            self.parent.check().map_err(|_| custody())?;
            self.root.check().map_err(|_| custody())?;
            self.files.check().map_err(|_| custody())
        }
        fn cleanup(self) -> Result<()> {
            if infra_host::process::is_stopping() {
                self.retain();
                return Err(custody());
            }
            if let Err(error) = self.check() {
                self.retain();
                return Err(error);
            }
            // Root ownership was checked immediately before removal. This is
            // bounded directory custody, not atomic whole-tree namespace CAS.
            if std::fs::remove_dir_all(&self.path).is_err() {
                self.retain();
                return Err(custody());
            }
            Ok(())
        }
        fn retain(self) {
            // A retained native worker or changed namespace must never trigger
            // a destructor that deletes possibly active/foreign temporary state.
            std::mem::forget(self);
        }
    }
    fn create_link(request: &Request, stage: &Staging) -> Result<Vec<u8>> {
        let target = terminated(&request.target)?;
        let icon = terminated(&request.icon)?;
        let directory = terminated(&request.working_directory)?;
        let arguments = terminated(&"app open".encode_utf16().collect::<Vec<_>>())?;
        let description = terminated(&DESCRIPTION.encode_utf16().collect::<Vec<_>>())?;
        let filename = stage.path.join("AI Accelerator.lnk");
        let output = terminated(&wide(&filename)?)?;
        // Pin the fresh child before COM writes anything into it.
        let mut files = PublicationFiles::new(&stage.path).map_err(|_| custody())?;
        stage.check()?;
        {
            let _apartment = Apartment::new()?;
            let link = shell_link()?;
            let table = link.table();
            // SAFETY: live matching COM interface and stable NUL-terminated buffers.
            unsafe {
                succeeded((table.set_path)(link.raw(), target.as_ptr()))?;
                succeeded((table.set_arguments)(link.raw(), arguments.as_ptr()))?;
                succeeded((table.set_icon_location)(link.raw(), icon.as_ptr(), 0))?;
                succeeded((table.set_working_directory)(
                    link.raw(),
                    directory.as_ptr(),
                ))?;
                succeeded((table.set_description)(link.raw(), description.as_ptr()))?;
            }
            let persist = link.query::<PersistFileVTable>(&IID_PERSIST_FILE)?;
            // Save S_FALSE means not saved, unlike CoInitializeEx S_FALSE.
            let result = unsafe { (persist.table().save)(persist.raw(), output.as_ptr(), 1) };
            saved(result)?;
            // Persist and link references are released before apartment shutdown
            // and before file capture/reply.
        }
        stage.check()?;
        let bytes = files
            .read("AI Accelerator.lnk", ICON_LIMIT)
            .map_err(|_| custody())?
            .ok_or_else(failed)?;
        files.check().map_err(|_| custody())?;
        stage.check()?;
        if bytes.len() < 76 || !bytes.starts_with(LINK_HEADER) {
            return Err(failed());
        }
        Ok(bytes)
    }
    pub(crate) fn worker(raw: &[u8]) -> Result<Vec<u8>> {
        let request = decode(raw)?;
        for units in [
            &request.target,
            &request.icon,
            &request.working_directory,
            &request.staging_parent,
        ] {
            path(units)?;
        }
        let stage = Staging::create(
            &path(&request.staging_parent)?,
            Some(request.staging_parent_identity),
        )?;
        let result = create_link(&request, &stage);
        if result
            .as_ref()
            .err()
            .is_some_and(|error| error.code == "desktop-staging-custody")
        {
            stage.retain();
            return result;
        }
        let cleanup = stage.cleanup();
        match (result, cleanup) {
            (_, Err(error)) => Err(error),
            (result, Ok(())) => result,
        }
    }
    pub(crate) fn stage(
        worker: &Path,
        target: &Path,
        source: &Path,
        icon: &Path,
    ) -> Result<Vec<u8>> {
        if infra_host::process::is_stopping() {
            return Err(custody());
        }
        // Validate passive fields before creating temporary state.
        wide(worker)?;
        wide(target)?;
        wide(source)?;
        wide(icon)?;
        let stage = Staging::create(&std::env::temp_dir(), None)?;
        let result = (|| {
            let raw = packet(
                target,
                icon,
                source,
                &stage.path,
                stage.files.root_identity(),
            )?;
            stage.check()?;
            let output = infra_tooling::process::run(
                worker.as_os_str(),
                &[OsString::from("__desktop-shortcut-worker")],
                source,
                Some(raw),
                Duration::from_secs(120),
                ICON_LIMIT,
            )?;
            stage.check()?;
            if output.code == CUSTODY_EXIT {
                return Err(custody());
            }
            if output.code != 0
                || output.stdout.len() < 76
                || !output.stdout.starts_with(LINK_HEADER)
            {
                return Err(failed());
            }
            Ok(output.stdout)
        })();
        let uncertain = result.as_ref().err().is_some_and(|error| {
            ["cleanup-unconfirmed", "desktop-staging-custody"].contains(&error.code)
        }) || infra_host::process::is_stopping();
        if uncertain {
            stage.retain();
            return match result {
                Err(error) => Err(error),
                Ok(_) => Err(custody()),
            };
        }
        let cleanup = stage.cleanup();
        match (result, cleanup) {
            (_, Err(error)) => Err(error),
            (result, Ok(())) => result,
        }
    }

    struct RegistryKey(HKEY);
    impl Drop for RegistryKey {
        fn drop(&mut self) {
            unsafe { RegCloseKey(self.0) };
        }
    }
    pub(crate) fn register(source: &Path, icon: &Path, executable: &Path) -> Result<()> {
        let executable = wide(executable)?;
        let mut uninstall = vec![34];
        uninstall.extend_from_slice(&executable);
        uninstall.extend("\" app uninstall".encode_utf16());
        let values = [
            (
                "DisplayName",
                "AI Accelerator".encode_utf16().collect::<Vec<_>>(),
            ),
            (
                "DisplayVersion",
                env!("CARGO_PKG_VERSION").encode_utf16().collect(),
            ),
            ("DisplayIcon", wide(icon)?),
            ("InstallLocation", wide(source)?),
            ("UninstallString", uninstall),
        ];
        // Prepare all encodings before creating the key or setting any values.
        let encoded = values
            .iter()
            .map(|(name, value)| {
                Ok((
                    terminated(&name.encode_utf16().collect::<Vec<_>>())?,
                    registry_bytes(value)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let name = terminated(
            &"Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\AIAccelerator"
                .encode_utf16()
                .collect::<Vec<_>>(),
        )?;
        let mut key = ptr::null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(HarnessError::unavailable(
                "Windows application registration failed.",
            ));
        }
        let key = RegistryKey(key);
        for (name, bytes) in encoded {
            let count = u32::try_from(bytes.len()).map_err(|_| invalid())?;
            let status =
                unsafe { RegSetValueExW(key.0, name.as_ptr(), 0, REG_SZ, bytes.as_ptr(), count) };
            if status != ERROR_SUCCESS {
                return Err(HarnessError::unavailable(
                    "Windows application registration failed.",
                ));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn windows_com_layout_matches_the_sdk_slot_contract() {
            use std::mem::{offset_of, size_of};
            assert!(saved(S_OK).is_ok());
            assert!(saved(S_FALSE).is_err());
            assert!(saved(-1).is_err());
            let word = size_of::<*const c_void>();
            assert_eq!(size_of::<ShellLinkVTable>(), 21 * word);
            assert_eq!(offset_of!(ShellLinkVTable, set_path), 20 * word);
            assert_eq!(offset_of!(ShellLinkVTable, set_arguments), 11 * word);
            assert_eq!(offset_of!(ShellLinkVTable, set_icon_location), 17 * word);
            assert_eq!(size_of::<PersistFileVTable>(), 9 * word);
            assert_eq!(offset_of!(PersistFileVTable, save), 6 * word);
        }
        #[test]
        fn windows_worker_refuses_a_replaced_parent_before_creating_a_child() {
            let temporary = tempfile::tempdir().unwrap();
            let source = temporary.path().join("source");
            std::fs::create_dir(&source).unwrap();
            let target = source.join("engine.exe");
            let icon = source.join("icon.ico");
            let outer = Staging::create(temporary.path(), None).unwrap();
            let raw = packet(
                &target,
                &icon,
                &source,
                &outer.path,
                outer.files.root_identity(),
            )
            .unwrap();
            let original = temporary.path().join("retained original staging");
            std::fs::rename(&outer.path, &original).unwrap();
            std::fs::create_dir(&outer.path).unwrap();
            let foreign = outer.path.join("foreign.txt");
            std::fs::write(&foreign, b"must stay untouched").unwrap();
            let error = worker(&raw).unwrap_err();
            assert_eq!(error.code, "desktop-staging-custody");
            assert_eq!(std::fs::read_dir(&outer.path).unwrap().count(), 1);
            assert_eq!(std::fs::read(&foreign).unwrap(), b"must stay untouched");
            assert_eq!(outer.cleanup().unwrap_err().code, "desktop-staging-custody");
            assert_eq!(std::fs::read(&foreign).unwrap(), b"must stay untouched");
            assert!(original.is_dir());
        }
        fn get_text(link: &ComPointer<ShellLinkVTable>, get: GetText) -> Vec<u16> {
            let mut text = vec![0; PATH_UNITS + 1];
            succeeded(unsafe { get(link.raw(), text.as_mut_ptr(), text.len() as i32) }).unwrap();
            let length = text.iter().position(|unit| *unit == 0).unwrap();
            text.truncate(length);
            text
        }
        #[test]
        fn windows_native_shortcut_roundtrip_preserves_launch_properties_without_execution() {
            let temporary = tempfile::tempdir().unwrap();
            let source = temporary.path().join("source O'Brien \u{1f680}");
            let target = source.join("engine O'Brien.exe");
            let icon = source.join("icon O'Brien.ico");
            std::fs::create_dir(&source).unwrap();
            std::fs::write(&target, b"passive target, never executed").unwrap();
            let outer = Staging::create(temporary.path(), None).unwrap();
            let raw = packet(
                &target,
                &icon,
                &source,
                &outer.path,
                outer.files.root_identity(),
            )
            .unwrap();
            let bytes = worker(&raw).unwrap();
            assert!(bytes.starts_with(LINK_HEADER));
            assert_eq!(std::fs::read_dir(&outer.path).unwrap().count(), 0);
            let saved = outer.path.join("readback.lnk");
            std::fs::write(&saved, &bytes).unwrap();
            {
                let _apartment = Apartment::new().unwrap();
                let link = shell_link().unwrap();
                let persist = link.query::<PersistFileVTable>(&IID_PERSIST_FILE).unwrap();
                let name = terminated(&wide(&saved).unwrap()).unwrap();
                succeeded(unsafe { (persist.table()._load)(persist.raw(), name.as_ptr(), 0) })
                    .unwrap();
                assert_eq!(
                    get_text(&link, link.table()._get_arguments),
                    "app open".encode_utf16().collect::<Vec<_>>()
                );
                assert_eq!(
                    get_text(&link, link.table()._get_description),
                    DESCRIPTION.encode_utf16().collect::<Vec<_>>()
                );
                let folder = path(&get_text(&link, link.table()._get_working_directory)).unwrap();
                assert_eq!(
                    folder.canonicalize().unwrap(),
                    source.canonicalize().unwrap()
                );
                let mut buffer = vec![0; PATH_UNITS + 1];
                let mut index = -1;
                succeeded(unsafe {
                    (link.table()._get_icon_location)(
                        link.raw(),
                        buffer.as_mut_ptr(),
                        buffer.len() as i32,
                        &mut index,
                    )
                })
                .unwrap();
                assert_eq!(index, 0);
                let length = buffer.iter().position(|unit| *unit == 0).unwrap();
                assert_eq!(&buffer[..length], wide(&icon).unwrap().as_slice());
                let mut target_buffer = vec![0; PATH_UNITS + 1];
                succeeded(unsafe {
                    (link.table()._get_path)(
                        link.raw(),
                        target_buffer.as_mut_ptr(),
                        target_buffer.len() as i32,
                        ptr::null_mut(),
                        4,
                    )
                })
                .unwrap();
                let length = target_buffer.iter().position(|unit| *unit == 0).unwrap();
                assert_eq!(
                    path(&target_buffer[..length])
                        .unwrap()
                        .canonicalize()
                        .unwrap(),
                    target.canonicalize().unwrap()
                );
            }
            outer.cleanup().unwrap();
            assert_eq!(
                std::fs::read(&target).unwrap(),
                b"passive target, never executed"
            );
        }
    }
}
#[cfg(windows)]
pub(super) use native::{register, stage, worker};

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> serde_json::Value {
        let path = "C:\\Users\\O'Brien\\desktop"
            .encode_utf16()
            .collect::<Vec<_>>();
        serde_json::json!({
            "version":1,"target":path,"icon":path,
            "working_directory":path,"staging_parent":path,"staging_parent_identity":[1,2,3]
        })
    }
    #[test]
    fn shortcut_packet_preserves_wide_units_and_rejects_unbounded_or_ambiguous_input() {
        let mut value = request();
        value["target"] = serde_json::json!([67, 58, 92, 0xd800, 0xdfff, 0x00e9]);
        let raw = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            decode(&raw).unwrap().target,
            [67, 58, 92, 0xd800, 0xdfff, 0x00e9]
        );
        for invalid_value in [
            serde_json::json!({"version":1}),
            {
                let mut v = request();
                v["version"] = 2.into();
                v
            },
            {
                let mut v = request();
                v["extra"] = true.into();
                v
            },
            {
                let mut v = request();
                v["target"] = serde_json::json!([67, 58, 92, 0]);
                v
            },
            {
                let mut v = request();
                v["target"] = serde_json::json!([67, 58, 92, 65536]);
                v
            },
            {
                let mut v = request();
                v["target"] = serde_json::json!([67, 58, 92, -1]);
                v
            },
            {
                let mut v = request();
                v["target"] = serde_json::json!([67, 58, 92, 1.5]);
                v
            },
            {
                let mut v = request();
                v["target"] = serde_json::json!([114, 101, 108]);
                v
            },
            {
                let mut v = request();
                let mut units = vec![67u16; PATH_UNITS + 1];
                units[..3].copy_from_slice(&[67, 58, 92]);
                v["target"] = serde_json::json!(units);
                v
            },
        ] {
            assert!(decode(&serde_json::to_vec(&invalid_value).unwrap()).is_err());
        }
        let valid = serde_json::to_string(&request()).unwrap();
        let duplicate = format!("{{\"version\":1,{}", &valid[1..]);
        assert!(decode(duplicate.as_bytes()).is_err());
        let escaped_duplicate = format!("{{\"vers\\u0069on\":1,{}", &valid[1..]);
        assert!(decode(escaped_duplicate.as_bytes()).is_err());
        let mut oversized = valid.into_bytes();
        oversized.resize(REQUEST_LIMIT + 1, b' ');
        assert!(decode(&oversized).is_err());
    }
    #[test]
    fn registry_sz_encoding_is_exact_little_endian_and_nul_terminated() {
        assert_eq!(registry_bytes(&[]).unwrap(), [0, 0]);
        assert_eq!(
            registry_bytes(&[0x0043, 0x003a, 0x005c, 0xd800, 0x00e9]).unwrap(),
            [0x43, 0, 0x3a, 0, 0x5c, 0, 0, 0xd8, 0xe9, 0, 0, 0]
        );
        assert!(registry_bytes(&[67, 0, 68]).is_err());
        let wide = "app open".encode_utf16().collect::<Vec<_>>();
        assert_eq!(terminated(&wide).unwrap().last(), Some(&0));
        assert_eq!(wide, "app open".encode_utf16().collect::<Vec<_>>());
    }
}
