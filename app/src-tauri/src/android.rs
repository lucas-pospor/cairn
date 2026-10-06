//! Android-only code.
//!
//! Vaults on Android live either in app-specific storage (a normal folder,
//! handled by `StdFs`) or in a folder the user picked through the Storage
//! Access Framework. SAF folders have no file paths, only `content://` URIs,
//! so [`SafFs`] implements `VaultFs` by calling the Kotlin `SafPlugin`
//! (gen/android/app/src/main/java/app/cairn/notes/SafPlugin.kt).

use cairn_core::path as vpath;
use cairn_core::{CoreError, EntryKind, FileStat, VaultFs};
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime, Wry};

pub struct Saf(pub PluginHandle<Wry>);

pub fn init() -> TauriPlugin<Wry> {
    Builder::new("cairn-saf")
        .setup(|app, api| {
            STARTED.get_or_init(std::time::SystemTime::now);
            let handle = api.register_android_plugin("app.cairn.notes", "SafPlugin")?;
            app.manage(Saf(handle));
            Ok(())
        })
        .build()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TreeArgs<'a> {
    tree: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PathArgs<'a> {
    tree: &'a str,
    path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteArgs<'a> {
    tree: &'a str,
    path: &'a str,
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MoveArgs<'a> {
    tree: &'a str,
    from: &'a str,
    to: &'a str,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SafEntry {
    path: String,
    dir: bool,
    size: u64,
    mtime: i64,
}

#[derive(Deserialize)]
struct Listing {
    entries: Vec<SafEntry>,
}

#[derive(Deserialize)]
struct Data {
    data: String,
}

#[derive(Deserialize)]
struct MaybeEntry {
    entry: Option<SafEntry>,
}

#[derive(Deserialize)]
pub struct Picked {
    pub uri: Option<String>,
    pub name: Option<String>,
}

#[derive(Deserialize)]
struct Removed {
    removed: bool,
}

#[derive(Deserialize)]
struct Access {
    ok: bool,
}

fn to_stat(e: SafEntry) -> FileStat {
    FileStat { path: e.path, kind: if e.dir { EntryKind::Dir } else { EntryKind::File }, size: e.size, mtime: e.mtime }
}

fn err(path: &str, e: impl std::fmt::Display) -> CoreError {
    let msg = e.to_string();
    if msg.contains("not found") || msg.contains("NotFound") {
        CoreError::NotFound(path.to_string())
    } else if msg.contains("exists") {
        CoreError::AlreadyExists(path.to_string())
    } else {
        CoreError::Io(format!("{path}: {msg}"))
    }
}

fn b64(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn unb64(s: &str) -> Result<Vec<u8>, CoreError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).map_err(|e| CoreError::Io(e.to_string()))
}

/// A vault inside a Storage Access Framework tree.
pub struct SafFs {
    handle: PluginHandle<Wry>,
    tree: String,
    /// Display name of the picked folder.
    name: String,
}

impl SafFs {
    pub fn new(handle: PluginHandle<Wry>, tree: String, name: String) -> SafFs {
        SafFs { handle, tree, name }
    }
}

impl VaultFs for SafFs {
    fn list(&self, dir: &str) -> cairn_core::Result<Vec<FileStat>> {
        let r: Listing = self
            .handle
            .run_mobile_plugin("list", PathArgs { tree: &self.tree, path: dir })
            // A missing vault folder (renamed or moved elsewhere) is reported by its name.
            .map_err(|e| err(if dir.is_empty() { &self.name } else { dir }, e))?;
        Ok(r.entries.into_iter().filter(|e| crate::listing::shown(dir, &e.path)).map(to_stat).collect())
    }

    fn stat(&self, path: &str) -> cairn_core::Result<Option<FileStat>> {
        let r: MaybeEntry = self
            .handle
            .run_mobile_plugin("stat", PathArgs { tree: &self.tree, path })
            .map_err(|e| err(path, e))?;
        Ok(r.entry.map(to_stat))
    }

    fn read(&self, path: &str) -> cairn_core::Result<Vec<u8>> {
        let r: Data = self
            .handle
            .run_mobile_plugin("read", PathArgs { tree: &self.tree, path })
            .map_err(|e| err(path, e))?;
        unb64(&r.data)
    }

    fn write(&self, path: &str, data: &[u8]) -> cairn_core::Result<FileStat> {
        let r: MaybeEntry = self
            .handle
            .run_mobile_plugin("write", WriteArgs { tree: &self.tree, path, data: b64(data) })
            .map_err(|e| err(path, e))?;
        r.entry.map(to_stat).ok_or_else(|| CoreError::Io(format!("{path}: write returned nothing")))
    }

    fn create_dir(&self, path: &str) -> cairn_core::Result<()> {
        let _: MaybeEntry = self
            .handle
            .run_mobile_plugin("mkdirs", PathArgs { tree: &self.tree, path })
            .map_err(|e| err(path, e))?;
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> cairn_core::Result<()> {
        let _: MaybeEntry = self
            .handle
            .run_mobile_plugin("move", MoveArgs { tree: &self.tree, from, to })
            .map_err(|e| match err(from, e) {
                // The name in the way is the target's, as with StdFs.
                CoreError::AlreadyExists(_) => CoreError::AlreadyExists(to.to_string()),
                other => other,
            })?;
        Ok(())
    }

    fn remove(&self, path: &str) -> cairn_core::Result<()> {
        // Move to the vault's .trash folder (SAF has no system trash).
        let name = vpath::file_name(path);
        let mut target = format!(".trash/{name}");
        let mut n = 1;
        while self.stat(&target)?.is_some() {
            let (stem, ext) = match name.rfind('.') {
                Some(i) if i > 0 => (&name[..i], &name[i..]),
                _ => (name, ""),
            };
            target = format!(".trash/{stem} {n}{ext}");
            n += 1;
        }
        self.create_dir(".trash")?;
        self.rename(path, &target)
    }

    fn remove_empty_dir(&self, path: &str) -> cairn_core::Result<bool> {
        let r: Removed = self
            .handle
            .run_mobile_plugin("removeEmptyDir", PathArgs { tree: &self.tree, path })
            .map_err(|e| err(path, e))?;
        Ok(r.removed)
    }

    /// SafPlugin finds names ignoring case and refuses a case twin.
    fn refuses_case_twins(&self) -> bool {
        true
    }

    fn describe(&self) -> String {
        self.tree.clone()
    }
}

impl SafFs {
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// When this process set the app up; a save temp file older than that was
/// left by an earlier process.
static STARTED: std::sync::OnceLock<std::time::SystemTime> = std::sync::OnceLock::new();

/// Remove the temp files that killed processes left in a vault in the app's
/// own storage. `StdFs::write` writes the note to a hidden temp file named
/// after the process (`.<name>.cairn-tmp-<pid>`) and renames it into place; a
/// kill in between leaves the temp behind, and later runs never reuse it.
/// Only this app writes there, so a temp written before this process started
/// (and not named after it) is a leftover, whatever the exact naming.
pub fn remove_stale_temp_files(dir: &std::path::Path) {
    let Some(started) = STARTED.get() else { return };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            remove_stale_temp_files(&e.path());
        } else if t.is_file()
            && is_stale_temp(&e.file_name().to_string_lossy())
            && e.metadata().and_then(|m| m.modified()).is_ok_and(|m| m < *started)
        {
            match std::fs::remove_file(e.path()) {
                Ok(()) => log::info!("removed leftover temp file {}", e.path().display()),
                Err(err) => log::warn!("cannot remove leftover temp file {}: {err}", e.path().display()),
            }
        }
    }
}

/// A hidden `….cairn-tmp…` file not named after this process.
fn is_stale_temp(name: &str) -> bool {
    const MARK: &str = ".cairn-tmp";
    let Some(i) = name.rfind(MARK) else { return false };
    let rest = name[i + MARK.len()..].trim_start_matches('-');
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    name.starts_with('.') && digits.parse::<u32>().ok() != Some(std::process::id())
}

/// Whether a picked tree can be opened again: the app still holds the access
/// it kept when the folder was picked, and the folder is still there.
pub fn can_open<R: Runtime>(app: &tauri::AppHandle<R>, tree: &str) -> bool {
    let saf = app.state::<Saf>();
    saf.0.run_mobile_plugin::<Access>("canOpen", TreeArgs { tree }).is_ok_and(|r| r.ok)
}

/// Show the system folder picker; returns the tree URI (persisted).
pub fn pick_folder<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<Picked, String> {
    let saf = app.state::<Saf>();
    saf.0.run_mobile_plugin::<Picked>("pickFolder", TreeArgs { tree: "" }).map_err(|e| e.to_string())
}
