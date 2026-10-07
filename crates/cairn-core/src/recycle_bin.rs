//! The Windows Recycle Bin, for `TrashMode::System`.
//!
//! The shell deletes for good what its Recycle Bin cannot take: an entry on
//! a network share or on a drive without a bin, on a drive set to remove
//! files at once, or one larger than the bin. With the flags the trash crate
//! gives it, it does so without a prompt and reports success, so a delete
//! in Cairn, by sync or of the font file was lost (WIN-005). Cairn runs the
//! shell's delete itself, as Electron does, and keeps from it what it would
//! delete for good: an entry on a share, also one reached through a folder
//! link or junction in the vault, and one larger than the bin of its drive
//! takes (`bin_size`) never get to the shell, and a progress sink stops a
//! delete that the shell already knows it cannot recycle. `StdFs::remove`
//! then moves the entry to the vault's trash. What the shell still deletes
//! for good, the sink only learns of once it is gone: the delete then fails
//! with an error that says so.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{E_ABORT, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, MAX_PATH};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW};
use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RegGetValueW};
use windows::Win32::UI::Shell::{
    FOF_NO_UI, FOFX_ADDUNDORECORD, FOFX_RECYCLEONDELETE, FileOperation, IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem,
    SHCreateItemFromParsingName, TSF_DELETE_RECYCLE_IF_POSSIBLE,
};
use windows_core::{ComObject, HRESULT, PCWSTR, Ref, implement};

use crate::bin_size::{self, BinSettings, Seen};
use crate::error::os_text;
use crate::fs::SystemTrash;

/// Where Windows keeps the Recycle Bin settings of each drive, in the
/// user's registry, under the GUID of the drive's volume.
const BIT_BUCKET: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\BitBucket\Volume";
/// Where the "Maximum allowed Recycle Bin size" policy is.
const POLICIES: &str = r"Software\Microsoft\Windows\CurrentVersion\Policies\Explorer";

/// Move the entry at `abs`, a path in a vault as `StdFs` makes it, to the
/// Recycle Bin. What the bin cannot take never gets to the shell, or the
/// shell is stopped before it deletes it for good: either way the entry is
/// then where it was. An entry that the shell deletes for good all the
/// same is `Destroyed`.
pub(crate) fn recycle(abs: &Path) -> SystemTrash {
    let real = match real_location(abs) {
        Ok(real) => real,
        Err(why) => return SystemTrash::NotRecyclable(why),
    };
    let plain = match shell_path(&real) {
        Ok(plain) => plain.to_path_buf(),
        Err(why) => return SystemTrash::NotRecyclable(why.into()),
    };
    // The shell finds out only while it moves the entry that the bin is too
    // small for it, and then deletes it for good without a question. The
    // size is counted on `real`, whose \\?\ prefix keeps a name in a folder
    // that ends in a dot or space from naming another file.
    if let Err(why) = bin_size::fits(&real, bin_settings(plain.parent().unwrap_or(&plain))) {
        return SystemTrash::NotRecyclable(why);
    }
    // The shell wants COM in a single-threaded apartment. A thread of its
    // own leaves tokio's threads and sync's as they are, and a panic there
    // is a failed delete here, which the vault's trash then takes.
    let worker = std::thread::Builder::new().name("cairn-recycle-bin".into()).spawn(move || delete_in_apartment(&plain));
    let done = match worker.map(|t| t.join()) {
        Ok(Ok(done)) => done,
        Ok(Err(_)) => SystemTrash::Failed("the Recycle Bin thread panicked".into()),
        Err(e) => SystemTrash::Failed(format!("no thread for the Recycle Bin: {e}")),
    };
    // A shell that reports success but left the entry where it was took
    // nothing, or another entry: this one still needs a trash.
    if done == SystemTrash::Recycled && std::fs::symlink_metadata(abs).is_ok() {
        return SystemTrash::Failed("the shell left it in place".into());
    }
    done
}

/// Where the entry at `abs` really is: the folder it is in, with every
/// folder link and junction on the way resolved, and its own name, as the
/// trash crate does. `StdFs` builds `abs` name by name, so a note behind a
/// link to another drive or to a share would otherwise be judged by the
/// vault's drive, and recycled into that drive's bin, which cannot take it.
/// The name itself is not resolved: a link that is the entry is deleted as
/// a link.
fn real_location(abs: &Path) -> Result<PathBuf, String> {
    let (Some(dir), Some(name)) = (abs.parent(), abs.file_name()) else {
        return Err("its path has no folder".into());
    };
    dir.canonicalize().map(|dir| dir.join(name)).map_err(|e| format!("the folder it is in cannot be found: {e}"))
}

/// The path to give the shell for `abs`, or why it gets none. `StdFs` paths
/// start with `\\?\`, as the vault folder is canonical, and the shell does
/// not take that prefix. Only a path on a drive letter is given to it, and
/// only one that means the same without the prefix: dunce keeps the prefix
/// of a path too long for the shell, or with a name that Windows would
/// change without it (a trailing dot or space, a device name such as CON),
/// anywhere on the path. A network share, also one behind a mapped drive
/// (the canonical path names the share), is never asked anything here.
fn shell_path(abs: &Path) -> Result<&Path, &'static str> {
    let Some(Component::Prefix(prefix)) = abs.components().next() else {
        return Err("its path is not a full path on a drive letter");
    };
    match prefix.kind() {
        Prefix::VerbatimDisk(_) => {}
        Prefix::VerbatimUNC(..) | Prefix::UNC(..) => return Err("it is on a network share"),
        _ => return Err("its path is not a full path on a drive letter"),
    }
    let plain = dunce::simplified(abs);
    if plain.as_os_str().as_encoded_bytes().starts_with(br"\\") {
        return Err("the shell cannot take its path: too long, or a name on it ends in a dot or space or is a device name");
    }
    Ok(plain)
}

/// The Recycle Bin settings of the drive that the folder `dir` is on, or
/// why they cannot be read.
fn bin_settings(dir: &Path) -> Result<BinSettings, String> {
    let root = volume_root(dir)?;
    let max_capacity = dword(HKEY_CURRENT_USER, &format!(r"{BIT_BUCKET}\{}", volume_guid(&root)?), "MaxCapacity")?;
    // Microsoft documents the policy for users only. One set for the whole
    // computer counts too, and of two, the smaller.
    let policies = [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE].map(|key| dword(key, POLICIES, "RecycleBinSize"));
    let size_policy = policies.into_iter().collect::<Result<Vec<_>, _>>()?.into_iter().flatten().min();
    Ok(BinSettings { max_capacity, size_policy, drive_size: drive_size(dir)? })
}

/// The root of the volume that `path` is on, such as `C:\`, or the folder
/// that the volume is mounted in: a wide string that ends in a NUL.
fn volume_root(path: &Path) -> Result<Vec<u16>, String> {
    let name = wide(path.as_os_str());
    // The root is never longer than the path.
    let mut root = vec![0u16; name.len().max(MAX_PATH as usize + 1)];
    // SAFETY: `name` ends in a NUL, and the call writes only within `root`.
    unsafe { GetVolumePathNameW(PCWSTR(name.as_ptr()), &mut root) }.map_err(|e| format!("its drive cannot be found: {e}"))?;
    Ok(root)
}

/// The GUID in the name of the volume at `root` (`\\?\Volume{GUID}\`), under
/// which Windows keeps the Recycle Bin settings of the drive.
fn volume_guid(root: &[u16]) -> Result<String, String> {
    // A volume name takes 49 characters and the NUL.
    let mut name = [0u16; 50];
    // SAFETY: `root` ends in a NUL, and the call writes only within `name`.
    unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(root.as_ptr()), &mut name) }.map_err(|e| format!("its drive has no volume name: {e}"))?;
    let name = String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]);
    match name.strip_prefix(r"\\?\Volume").and_then(|n| n.strip_suffix('\\')) {
        Some(guid) if guid.starts_with('{') && guid.ends_with('}') => Ok(guid.to_string()),
        _ => Err(format!("its drive has a volume name without a GUID: {name}")),
    }
}

/// The DWORD `value` in `key`\`subkey`, or `None` when it is not set.
fn dword(key: HKEY, subkey: &str, value: &str) -> Result<Option<u32>, String> {
    let (path, name) = (wide(OsStr::new(subkey)), wide(OsStr::new(value)));
    let (mut data, mut size) = (0u32, 4u32);
    // SAFETY: both names end in a NUL, and `data` has room for the `size`
    // bytes of a DWORD, the only type asked for.
    let e = unsafe { RegGetValueW(key, PCWSTR(path.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_DWORD, None, Some((&raw mut data).cast()), Some(&mut size)) };
    match e {
        ERROR_SUCCESS => Ok(Some(data)),
        ERROR_FILE_NOT_FOUND => Ok(None),
        e => Err(format!("{value} cannot be read: {}", os_text(&std::io::Error::from_raw_os_error(e.0 as i32)))),
    }
}

/// The bytes that the user may use on the drive that the folder `dir` is
/// on: its size, or the user's quota on it. The call needs a folder that
/// the user may list: the folder of a note Cairn shows is one, the root of
/// the drive may not be.
fn drive_size(dir: &Path) -> Result<u64, String> {
    let (name, mut total) = (wide(dir.as_os_str()), 0u64);
    // SAFETY: `name` ends in a NUL, and the call writes only `total`.
    unsafe { GetDiskFreeSpaceExW(PCWSTR(name.as_ptr()), None, Some(&raw mut total), None) }.map_err(|e| format!("the size of its drive cannot be read: {e}"))?;
    Ok(total)
}

/// `s` as a wide string that ends in a NUL, for Windows.
fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// `delete`, on a thread that has COM for it just for the call.
fn delete_in_apartment(plain: &Path) -> SystemTrash {
    // SAFETY: COM is set up on this new thread and taken down at the end,
    // after `delete` has released every COM object it made.
    if let Err(e) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.ok() {
        return SystemTrash::Failed(format!("cannot set up COM: {e}"));
    }
    let done = delete(plain);
    unsafe { CoUninitialize() };
    done
}

/// Delete `plain` through the shell, which may only recycle it.
fn delete(plain: &Path) -> SystemTrash {
    let sink = ComObject::new(RecycleOnly { refused: AtomicBool::new(false), destroyed: AtomicBool::new(false), kept: AtomicBool::new(false) });
    let aborted = perform(plain, &sink).map_err(|e| e.to_string());
    let [refused, destroyed, kept] = [&sink.refused, &sink.destroyed, &sink.kept].map(|f| f.load(Ordering::Relaxed));
    let gone = std::fs::symlink_metadata(plain).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound);
    bin_size::verdict(plain, Seen { refused, destroyed, kept }, gone, aborted)
}

/// Run the shell's delete of `plain`, with `sink` watching it: true if the
/// shell reports that it stopped some of it.
fn perform(plain: &Path, sink: &ComObject<RecycleOnly>) -> windows_core::Result<bool> {
    let name = wide(plain.as_os_str());
    // SAFETY: COM is set up on this thread, and `name` ends in a NUL and
    // outlives the calls that read it.
    unsafe {
        let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
        // Electron's flags (FOF_NO_UI includes FOF_SILENT and FOF_NOERRORUI),
        // less FOFX_SHOWELEVATIONPROMPT: a delete that needs an administrator
        // fails instead of showing a UAC prompt, also during a sync. The
        // delete goes on the undo stack, as one in File Explorer does.
        op.SetOperationFlags(FOF_NO_UI | FOFX_ADDUNDORECORD | FOFX_RECYCLEONDELETE)?;
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None)?;
        op.DeleteItem(&item, sink.as_interface::<IFileOperationProgressSink>())?;
        op.PerformOperations()?;
        Ok(op.GetAnyOperationsAborted()?.as_bool())
    }
}

/// Lets the shell delete an entry only by moving it to the Recycle Bin.
#[implement(IFileOperationProgressSink)]
struct RecycleOnly {
    /// The shell was about to delete an entry for good, and was stopped.
    refused: AtomicBool,
    /// The shell deleted an entry for good, past `PreDeleteItem`.
    destroyed: AtomicBool,
    /// The shell put an entry in the Recycle Bin.
    kept: AtomicBool,
}

impl IFileOperationProgressSink_Impl for RecycleOnly_Impl {
    /// The shell leaves the flag out for an entry it cannot recycle, and an
    /// error here cancels that delete and all that would follow it: for a
    /// folder, the first entry it would delete for good decides, and the
    /// folder stays with whatever is still in it.
    fn PreDeleteItem(&self, dwflags: u32, _item: Ref<'_, IShellItem>) -> windows_core::Result<()> {
        if dwflags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 != 0 {
            return Ok(());
        }
        self.refused.store(true, Ordering::Relaxed);
        Err(E_ABORT.into())
    }

    /// A delete that worked left the entry in the Recycle Bin, or, with no
    /// item there, destroyed it. `PreDeleteItem` stops what the shell knows
    /// in time that it cannot recycle; that an entry is too large for the
    /// bin, the shell finds out only while it moves it, and it then deletes
    /// it for good without asking (FOF_NO_UI includes FOF_NOCONFIRMATION).
    fn PostDeleteItem(&self, _dwflags: u32, _item: Ref<'_, IShellItem>, hrdelete: HRESULT, created: Ref<'_, IShellItem>) -> windows_core::Result<()> {
        if hrdelete.is_ok() {
            let seen = if created.is_null() { &self.destroyed } else { &self.kept };
            seen.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    // Nothing else the shell reports matters here.

    fn StartOperations(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn FinishOperations(&self, _hrresult: HRESULT) -> windows_core::Result<()> {
        Ok(())
    }

    fn PreRenameItem(&self, _dwflags: u32, _item: Ref<'_, IShellItem>, _newname: &PCWSTR) -> windows_core::Result<()> {
        Ok(())
    }

    fn PostRenameItem(&self, _dwflags: u32, _item: Ref<'_, IShellItem>, _newname: &PCWSTR, _hrrename: HRESULT, _created: Ref<'_, IShellItem>) -> windows_core::Result<()> {
        Ok(())
    }

    fn PreMoveItem(&self, _dwflags: u32, _item: Ref<'_, IShellItem>, _folder: Ref<'_, IShellItem>, _newname: &PCWSTR) -> windows_core::Result<()> {
        Ok(())
    }

    fn PostMoveItem(&self, _dwflags: u32, _item: Ref<'_, IShellItem>, _folder: Ref<'_, IShellItem>, _newname: &PCWSTR, _hrmove: HRESULT, _created: Ref<'_, IShellItem>) -> windows_core::Result<()> {
        Ok(())
    }

    fn PreCopyItem(&self, _dwflags: u32, _item: Ref<'_, IShellItem>, _folder: Ref<'_, IShellItem>, _newname: &PCWSTR) -> windows_core::Result<()> {
        Ok(())
    }

    fn PostCopyItem(&self, _dwflags: u32, _item: Ref<'_, IShellItem>, _folder: Ref<'_, IShellItem>, _newname: &PCWSTR, _hrcopy: HRESULT, _created: Ref<'_, IShellItem>) -> windows_core::Result<()> {
        Ok(())
    }

    fn PreNewItem(&self, _dwflags: u32, _folder: Ref<'_, IShellItem>, _newname: &PCWSTR) -> windows_core::Result<()> {
        Ok(())
    }

    fn PostNewItem(
        &self,
        _dwflags: u32,
        _folder: Ref<'_, IShellItem>,
        _newname: &PCWSTR,
        _template: &PCWSTR,
        _attributes: u32,
        _hrnew: HRESULT,
        _item: Ref<'_, IShellItem>,
    ) -> windows_core::Result<()> {
        Ok(())
    }

    fn UpdateProgress(&self, _worktotal: u32, _worksofar: u32) -> windows_core::Result<()> {
        Ok(())
    }

    fn ResetTimer(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn PauseTimer(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn ResumeTimer(&self) -> windows_core::Result<()> {
        Ok(())
    }
}

// The module is Windows only, and so are its tests: only Windows parses
// \\?\ and \\server\share prefixes, and the shell and its Recycle Bin are
// Windows'.
#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::System::Registry::{REG_DWORD, RegDeleteKeyValueW, RegSetKeyValueW};

    use super::*;
    use crate::fs::{StdFs, TrashMode, VaultFs};

    /// Only a path on a drive letter that means the same without the \\?\
    /// prefix goes to the shell; a path on a share never does.
    #[test]
    fn only_plain_paths_on_a_drive_go_to_the_shell() {
        assert_eq!(shell_path(Path::new(r"\\?\C:\Users\me\Notes\a.md")), Ok(Path::new(r"C:\Users\me\Notes\a.md")));
        assert_eq!(shell_path(Path::new(r"\\?\D:\Notes\sub\b.md")), Ok(Path::new(r"D:\Notes\sub\b.md")));
        let long = format!(r"\\?\C:\{}\a.md", ["a folder with a long name"; 12].join(r"\"));
        let refused = [
            r"\\?\UNC\server\share\Notes\a.md",
            r"\\server\share\Notes\a.md",
            r"\\?\Volume{8a3c33e2-1b5f-4c7e-9a3d-5e6f7a8b9c0d}\Notes\a.md",
            r"\\.\C:\Notes\a.md",
            r"C:\Notes\a.md",
            r"Notes\a.md",
            r"\\?\C:\Notes.\a.md",
            r"\\?\C:\Notes \a.md",
            r"\\?\C:\Notes\con\a.md",
            r"\\?\C:\Notes\aux.md",
            r"\\?\C:\Notes\a.md.",
            &long,
        ];
        for p in refused {
            assert!(shell_path(Path::new(p)).is_err(), "{p}");
        }
        for p in [r"\\?\UNC\server\share\Notes\a.md", r"\\server\share\a.md"] {
            assert_eq!(shell_path(Path::new(p)), Err("it is on a network share"), "{p}");
        }
    }

    /// The shell is given the folder a note is really in: a junction on the
    /// way is resolved, a junction that is the entry stays as it is.
    #[test]
    fn the_shell_is_given_where_a_linked_folder_leads() {
        let d = tempfile::tempdir().unwrap();
        let local = d.path().canonicalize().unwrap();
        std::fs::create_dir(local.join("real")).unwrap();
        junction(&local.join("linked"), &local.join("real"));
        assert_eq!(real_location(&local.join("linked").join("a.md")), Ok(local.join("real").join("a.md")));
        assert_eq!(real_location(&local.join("linked")), Ok(local.join("linked")));
        assert!(real_location(&local.join("gone").join("a.md")).is_err());
    }

    /// A junction at `link` to the folder `target`. Junctions need no
    /// administrator, unlike symlinks, and only cmd makes them.
    fn junction(link: &Path, target: &Path) {
        let out = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(dunce::simplified(link))
            .arg(dunce::simplified(target))
            .output()
            .unwrap();
        assert!(out.status.success(), "mklink /J: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// The drive of the temp folder has a volume GUID, under which Windows
    /// keeps the settings of its Recycle Bin, and they can be read. A value
    /// that is not there is not set, which is no error.
    #[test]
    fn the_bin_settings_of_a_drive_can_be_read() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().canonicalize().unwrap();
        let guid = volume_guid(&volume_root(dunce::simplified(&dir)).unwrap()).unwrap();
        let hex = &guid[1..guid.len() - 1];
        let groups: Vec<usize> = hex.split('-').map(str::len).collect();
        assert!(groups == [8, 4, 4, 4, 12] && hex.chars().all(|c| c == '-' || c.is_ascii_hexdigit()), "{guid}");
        let bin = bin_settings(dunce::simplified(&dir)).unwrap();
        assert!(bin.drive_size > 0, "{bin:?}");
        assert_eq!(dword(HKEY_CURRENT_USER, r"Software\Cairn test\no such key", "MaxCapacity"), Ok(None));
        assert_eq!(dword(HKEY_CURRENT_USER, BIT_BUCKET, "no such value"), Ok(None));
    }

    /// A folder counts the files in it, also those whose names end in a
    /// dot, and nothing behind a junction in it: neither a folder with a
    /// big file nor the folder itself again.
    #[test]
    fn a_folder_counts_nothing_behind_a_junction() {
        let d = tempfile::tempdir().unwrap();
        let local = d.path().canonicalize().unwrap();
        let (f, other) = (local.join("f"), local.join("other"));
        std::fs::create_dir_all(f.join("sub")).unwrap();
        std::fs::create_dir_all(f.join("d.")).unwrap();
        std::fs::create_dir(&other).unwrap();
        std::fs::write(f.join("a.md"), [b'a'; 600]).unwrap();
        std::fs::write(f.join("sub").join("b.md"), [b'b'; 400]).unwrap();
        std::fs::write(f.join("x."), [b'x'; 10]).unwrap();
        std::fs::write(f.join("d.").join("c.md"), [b'c'; 5]).unwrap();
        std::fs::write(other.join("big.bin"), vec![0u8; 100_000]).unwrap();
        junction(&f.join("big"), &other);
        junction(&f.join("sub").join("loop"), &f);
        assert_eq!(bin_size::size_up_to(&f, u64::MAX).unwrap(), 1015);
        assert_eq!(bin_size::size_up_to(&f.join("big"), u64::MAX).unwrap(), 0);
    }

    /// MaxCapacity of a drive's Recycle Bin, set for as long as the guard
    /// lives. Dropped, also when a test fails, it puts back what was there
    /// before: the old value, or none. One guard at a time: a second one
    /// would take the first one's value for the old one, and put it back.
    struct BinCapacity {
        key: String,
        old: Option<u32>,
        _one: parking_lot::MutexGuard<'static, ()>,
    }

    impl BinCapacity {
        fn set(guid: &str, megabytes: u32) -> BinCapacity {
            static ONE: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
            let one = ONE.lock();
            let key = format!(r"{BIT_BUCKET}\{guid}");
            let guard = BinCapacity { old: dword(HKEY_CURRENT_USER, &key, "MaxCapacity").unwrap(), key, _one: one };
            assert_eq!(guard.write(Some(megabytes)), ERROR_SUCCESS, "HKCU\\{}", guard.key);
            guard
        }

        /// Set the value, making the key if it is missing, or delete it.
        fn write(&self, value: Option<u32>) -> WIN32_ERROR {
            let (key, name) = (wide(OsStr::new(&self.key)), wide(OsStr::new("MaxCapacity")));
            // SAFETY: both names end in a NUL, and the data is the 4 bytes of
            // a DWORD.
            unsafe {
                match value {
                    Some(v) => RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), REG_DWORD.0, Some((&raw const v).cast()), 4),
                    None => RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr())),
                }
            }
        }
    }

    impl Drop for BinCapacity {
        fn drop(&mut self) {
            let e = self.write(self.old);
            if e != ERROR_SUCCESS && !(self.old.is_none() && e == ERROR_FILE_NOT_FOUND) {
                eprintln!("MaxCapacity in HKCU\\{} could not be put back to {:?}: {e:?}", self.key, self.old);
            }
        }
    }

    /// With the Recycle Bin of the temp folder's drive set to 1 MB, what is
    /// larger never gets to the shell, which would delete it for good
    /// without a word: a 5 MB file and a folder of three 600 KB files go to
    /// the vault's trash with all their bytes, and a 100 KB file still goes
    /// to the Recycle Bin. It changes the user's Recycle Bin settings for a
    /// moment and uses the bin, so it runs on CI only (CI is set); the old
    /// setting is put back, also when the test fails.
    #[test]
    fn what_is_larger_than_the_bin_goes_to_the_vault_trash() {
        if std::env::var_os("CI").is_none() {
            eprintln!("skipped: this test changes the size of the Recycle Bin and uses it, so it runs on CI only");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let vault = d.path().canonicalize().unwrap();
        let big: Vec<u8> = (0..5u32 << 20).map(|i| (i * 31 + 7) as u8).collect();
        let part: Vec<u8> = (0..600u32 << 10).map(|i| (i % 251) as u8).collect();
        std::fs::write(vault.join("big.bin"), &big).unwrap();
        std::fs::create_dir(vault.join("parts")).unwrap();
        for n in 1..=3 {
            std::fs::write(vault.join("parts").join(format!("{n}.bin")), &part).unwrap();
        }
        std::fs::write(vault.join("small.bin"), vec![b's'; 100 << 10]).unwrap();
        let plain = dunce::simplified(&vault).to_path_buf();
        let limit = BinCapacity::set(&volume_guid(&volume_root(&plain).unwrap()).unwrap(), 1);
        assert_eq!(bin_settings(&plain).map(|b| b.max_capacity), Ok(Some(1)));
        let fs = StdFs::new(&vault, TrashMode::System).unwrap();
        let done = ["big.bin", "parts", "small.bin"].map(|p| fs.remove(p));
        let from = take_from_the_bin("small.bin", &[&vault]);
        drop(limit);
        assert!(done.iter().all(Result::is_ok), "{done:?}");
        let trash = vault.join(".trash");
        assert!(std::fs::read(trash.join("big.bin")).is_ok_and(|b| b == big), "big.bin is not in .trash as it was");
        for n in 1..=3 {
            assert!(std::fs::read(trash.join("parts").join(format!("{n}.bin"))).is_ok_and(|b| b == part), "parts/{n}.bin is not in .trash as it was");
        }
        assert_eq!(from, [plain.to_string_lossy().to_lowercase()], "small.bin is not in the Recycle Bin");
        assert!(!trash.join("small.bin").exists());
        for p in ["big.bin", "parts", "small.bin"] {
            assert!(!vault.join(p).exists(), "{p} is still in the vault");
        }
    }

    /// What the size check would have kept from the shell, handed to it
    /// directly: with the Recycle Bin of the temp folder's drive set to
    /// 1 MB, the shell deletes a 5 MB file for good, and the sink's report
    /// of it makes the delete `Destroyed`, not `Recycled` (WIN-005). The
    /// shell reads the bin's settings once in a process, so the delete runs
    /// in a child test process, started after the setting is made; the
    /// child is this test, with the file to delete in CAIRN_DESTROY_FILE.
    /// It changes the user's Recycle Bin settings for a moment and hands a
    /// delete to the shell, so it runs on CI only (CI is set).
    #[test]
    fn the_sink_reports_a_delete_the_shell_did_for_good() {
        const CHILD: &str = "CAIRN_DESTROY_FILE";
        if let Some(file) = std::env::var_os(CHILD) {
            let file = PathBuf::from(file);
            let done = std::thread::spawn(move || delete_in_apartment(&file)).join().unwrap();
            assert_eq!(done, SystemTrash::Destroyed);
            return;
        }
        if std::env::var_os("CI").is_none() {
            eprintln!("skipped: this test changes the size of the Recycle Bin and hands a delete to the shell, so it runs on CI only");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let plain = dunce::simplified(&d.path().canonicalize().unwrap()).to_path_buf();
        let big = plain.join("destroyed.bin");
        std::fs::write(&big, vec![b'd'; 5 << 20]).unwrap();
        let limit = BinCapacity::set(&volume_guid(&volume_root(&plain).unwrap()).unwrap(), 1);
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "recycle_bin::tests::the_sink_reports_a_delete_the_shell_did_for_good", "--nocapture", "--test-threads=1"])
            .env(CHILD, &big)
            .output()
            .unwrap();
        let from = take_from_the_bin("destroyed.bin", &[&plain]);
        drop(limit);
        let out = format!("{}{}", String::from_utf8_lossy(&child.stdout), String::from_utf8_lossy(&child.stderr));
        assert!(child.status.success() && out.contains("1 passed"), "the child test failed:\n{out}");
        assert!(from.is_empty(), "the shell recycled it: {from:?}");
        assert!(!big.exists(), "the shell left it in place");
    }

    /// The bin items named `name` from `parents`, taken out of the Recycle
    /// Bin again: their parent folders.
    fn take_from_the_bin(name: &str, parents: &[&Path]) -> Vec<String> {
        let parents: Vec<String> = parents.iter().map(|p| dunce::simplified(p).to_string_lossy().to_lowercase()).collect();
        let ours: Vec<trash::TrashItem> = trash::os_limited::list()
            .unwrap()
            .into_iter()
            .filter(|i| i.name == name && parents.contains(&i.original_parent.to_string_lossy().to_lowercase()))
            .collect();
        let from = ours.iter().map(|i| i.original_parent.to_string_lossy().to_lowercase()).collect();
        trash::os_limited::purge_all(ours).unwrap();
        from
    }

    /// A note in a vault on the temp folder's drive, behind a junction to a
    /// folder on another drive, goes to that drive's Recycle Bin, or stays
    /// with an error: never gone. It uses the Recycle Bin, so it runs on CI
    /// only (CI is set), and only where the runner has a second drive.
    #[test]
    fn a_note_behind_a_junction_to_another_drive_is_never_lost() {
        if std::env::var_os("CI").is_none() {
            eprintln!("skipped: this test uses the Recycle Bin, so it runs on CI only");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let vault = d.path().canonicalize().unwrap();
        let Some(Component::Prefix(p)) = vault.components().next() else { panic!("{}", vault.display()) };
        let Prefix::VerbatimDisk(here) = p.kind() else { panic!("{}", vault.display()) };
        let other = (b'C'..=b'Z')
            .filter(|&l| l != here.to_ascii_uppercase())
            .find_map(|l| tempfile::Builder::new().prefix("cairn-recycle-").tempdir_in(format!(r"{}:\", l as char)).ok());
        let Some(other) = other else {
            eprintln!("skipped: this runner has no second drive to write to");
            return;
        };
        let target = other.path().canonicalize().unwrap();
        let linked = vault.join("linked");
        junction(&linked, &target);
        std::fs::write(target.join("a.md"), "behind a junction").unwrap();
        assert_eq!(real_location(&linked.join("a.md")), Ok(target.join("a.md")));
        let done = StdFs::new(&vault, TrashMode::System).unwrap().remove("linked/a.md");
        let from = take_from_the_bin("a.md", &[&target, &linked]);
        let kept = std::fs::read_to_string(target.join("a.md")).ok();
        let trashed = vault.join(".trash").join("a.md").exists();
        assert!(kept.is_some() || trashed || !from.is_empty(), "the note is gone: {done:?}");
        assert_eq!(done.is_ok(), kept.is_none(), "{done:?}");
        let target = dunce::simplified(&target).to_string_lossy().to_lowercase();
        assert!(from.iter().all(|p| *p == target), "recycled from {from:?}, not from {target}");
    }

    /// A note in a vault on a local drive, behind a folder symlink to a
    /// share, never reaches the shell: it goes to the vault's trash, or it
    /// stays with an error (a move from the share to the local drive cannot
    /// be a rename). It makes a symlink, which needs an administrator, and
    /// would use the Recycle Bin if it failed, so it runs on CI only.
    #[test]
    fn a_note_behind_a_link_to_a_share_never_reaches_the_shell() {
        if std::env::var_os("CI").is_none() {
            eprintln!("skipped: this test makes a symlink and could use the Recycle Bin, so it runs on CI only");
            return;
        }
        let (d, s) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (vault, local) = (d.path().canonicalize().unwrap(), s.path().canonicalize().unwrap());
        let Some(share) = on_admin_share(&local) else { return };
        let nas = vault.join("nas");
        std::os::windows::fs::symlink_dir(&share, &nas).unwrap();
        std::fs::write(local.join("a.md"), "on the share").unwrap();
        let real = real_location(&nas.join("a.md")).unwrap();
        assert_eq!(shell_path(&real), Err("it is on a network share"), "{}", real.display());
        let done = StdFs::new(&vault, TrashMode::System).unwrap().remove("nas/a.md");
        let from = take_from_the_bin("a.md", &[&local, &nas, &share]);
        let kept = std::fs::read_to_string(local.join("a.md")).ok();
        let trashed = std::fs::read_to_string(vault.join(".trash").join("a.md")).ok();
        assert!(from.is_empty(), "the shell was given it: {from:?}");
        assert!(kept.is_some() || trashed.is_some(), "the note is gone: {done:?}");
        assert_eq!(done.is_ok(), kept.is_none(), "{done:?}");
    }

    /// `local`, a canonical path on a drive letter, through the admin share
    /// of its drive (\\localhost\C$\...), if that can be reached. The admin
    /// share needs the Server service and an administrator, which the CI
    /// runner has: there a test fails without it, elsewhere it is skipped.
    fn on_admin_share(local: &Path) -> Option<PathBuf> {
        let Some(Component::Prefix(p)) = local.components().next() else { panic!("{}", local.display()) };
        let Prefix::VerbatimDisk(drive) = p.kind() else { panic!("{}", local.display()) };
        let rest: PathBuf = local.components().skip(2).collect();
        let share = PathBuf::from(format!(r"\\localhost\{}$", drive as char)).join(rest);
        if let Err(e) = std::fs::metadata(&share) {
            assert!(std::env::var_os("CI").is_none(), "{} cannot be reached: {e}", share.display());
            eprintln!("skipped: {} cannot be reached: {e}", share.display());
            return None;
        }
        Some(share)
    }

    /// A vault on a share (here the admin share of the drive the temp folder
    /// is on) never hands a delete to the shell, which would delete it for
    /// good: a note and a folder go to the vault's trash.
    #[test]
    fn deletes_on_a_share_go_to_the_vault_trash() {
        let d = tempfile::tempdir().unwrap();
        let local = d.path().canonicalize().unwrap();
        let Some(share) = on_admin_share(&local) else { return };
        std::fs::write(local.join("a.md"), "on the share").unwrap();
        std::fs::create_dir(local.join("f")).unwrap();
        std::fs::write(local.join("f").join("b.md"), "in a folder").unwrap();
        let fs = StdFs::new(&share, TrashMode::System).unwrap();
        assert!(fs.root().as_os_str().as_encoded_bytes().starts_with(br"\\?\UNC\"), "{}", fs.root().display());
        fs.remove("a.md").unwrap();
        fs.remove("f").unwrap();
        let trash = local.join(".trash");
        assert_eq!(std::fs::read_to_string(trash.join("a.md")).unwrap(), "on the share");
        assert_eq!(std::fs::read_to_string(trash.join("f").join("b.md")).unwrap(), "in a folder");
        assert!(!local.join("a.md").exists() && !local.join("f").exists());
    }

    /// The sink on its own: the shell has no Recycle Bin for a share, and
    /// would delete a note there for good (WIN-005). Handed one anyway,
    /// which `recycle` never does, the sink stops it and the note stays. It
    /// hands a delete to the shell, so it runs on CI only (CI is set).
    #[test]
    fn the_sink_stops_a_delete_the_shell_would_not_recycle() {
        if std::env::var_os("CI").is_none() {
            eprintln!("skipped: this test hands a delete to the shell, so it runs on CI only");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let local = d.path().canonicalize().unwrap();
        let Some(share) = on_admin_share(&local) else { return };
        std::fs::write(local.join("kept.md"), "kept").unwrap();
        let note = share.join("kept.md");
        let done = std::thread::spawn(move || delete_in_apartment(&note)).join().unwrap();
        assert_eq!(std::fs::read_to_string(local.join("kept.md")).ok().as_deref(), Some("kept"), "{done:?}");
        assert!(matches!(done, SystemTrash::NotRecyclable(_)), "{done:?}");
    }

    /// A note and a folder on a local drive go to the Recycle Bin through
    /// the shell, past the sink. This puts them in the user's Recycle Bin
    /// and takes them out again, so it runs on CI only (CI is set), never in
    /// a developer's Recycle Bin.
    #[test]
    fn local_deletes_go_to_the_recycle_bin() {
        if std::env::var_os("CI").is_none() {
            eprintln!("skipped: this test uses the Recycle Bin, so it runs on CI only");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let local = d.path().canonicalize().unwrap();
        let (note, folder) = (local.join("cairn recycle test.md"), local.join("cairn recycle test"));
        std::fs::write(&note, "recycled").unwrap();
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("inner.md"), "inner").unwrap();
        let done = [recycle(&note), recycle(&folder)];
        // Out of the Recycle Bin again before anything is checked: only what
        // was in this test's folder.
        let parent = dunce::simplified(&local).to_string_lossy().to_lowercase();
        let (ours, others): (Vec<trash::TrashItem>, Vec<trash::TrashItem>) =
            trash::os_limited::list().unwrap().into_iter().partition(|i| i.original_parent.to_string_lossy().to_lowercase() == parent);
        let mut names: Vec<String> = ours.iter().map(|i| i.name.to_string_lossy().into_owned()).collect();
        trash::os_limited::purge_all(ours).unwrap();
        assert_eq!(done, [SystemTrash::Recycled, SystemTrash::Recycled]);
        assert!(!note.exists() && !folder.exists());
        names.sort();
        let others: Vec<_> = others.iter().map(|i| i.original_path()).collect();
        assert_eq!(names, ["cairn recycle test", "cairn recycle test.md"], "from {parent}; the bin also has {others:?}");
    }
}
