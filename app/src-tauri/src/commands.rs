//! Tauri commands. Each one is a thin wrapper over `cairn_core::Vault`.
//! Mutating commands also emit `vault-changed` so every part of the UI
//! updates the same way whether a change came from Cairn or from outside.

use std::path::PathBuf;
use std::sync::Arc;

use cairn_core::index::{Backlinks, OutgoingLink, SearchHit};
use cairn_core::parse::LinkKind;
use cairn_core::{Change, CoreError, FileStat, NoteContent, StdFs, TrashMode, Vault, WriteResult};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::AppState;

type CmdResult<T> = Result<T, CoreError>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultInfo {
    pub root: String,
    pub name: String,
    pub entries: Vec<FileStat>,
}

fn vault(state: &State<'_, AppState>) -> CmdResult<Arc<Vault>> {
    state
        .vault
        .read()
        .clone()
        .ok_or_else(|| CoreError::NotFound("no vault is open".into()))
}

fn vault_root(state: &State<'_, AppState>) -> CmdResult<String> {
    state
        .vault_root
        .read()
        .clone()
        .ok_or_else(|| CoreError::NotFound("no vault is open".into()))
}

/// The open vault, once `path` is checked to stay inside it on disk when
/// `in_vault` is set: plugin calls. The app itself follows the symlinks the
/// user made.
fn vault_for(state: &State<'_, AppState>, path: &str, in_vault: Option<bool>) -> CmdResult<Arc<Vault>> {
    let v = vault(state)?;
    if in_vault == Some(true) {
        v.check_in_vault(path)?;
    }
    Ok(v)
}

fn emit(app: &AppHandle, changes: &[Change]) {
    if !changes.is_empty() {
        let _ = app.emit("vault-changed", changes);
        // Local change: let sync know.
        use tauri::Manager;
        if let Some(st) = app.try_state::<AppState>()
            && let Some(s) = st.sync.lock().as_ref()
        {
            s.poke();
        }
    }
}

/// Vault to open at start: a path given on the command line or in
/// `CAIRN_VAULT`, otherwise the most recently used one.
#[tauri::command]
pub async fn startup_vault(app: AppHandle) -> Option<String> {
    use tauri::Manager;
    let from_args = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    let from_env = std::env::var("CAIRN_VAULT").ok();
    from_args
        .or(from_env)
        .or_else(|| app.state::<AppState>().config.lock().recent_vaults.first().cloned())
        .filter(|p| vault_exists(&app, p))
}

/// Whether a vault location can still be opened: a folder, or on Android a
/// Storage Access Framework tree URI (not a path) the app still has access to.
#[allow(unused_variables)]
fn vault_exists(app: &AppHandle, path: &str) -> bool {
    #[cfg(target_os = "android")]
    if path.starts_with("content://") {
        return crate::android::can_open(app, path);
    }
    PathBuf::from(path).is_dir()
}

/// Build the file system for a vault location: a folder path, or on
/// Android a Storage Access Framework tree URI.
#[allow(unused_variables)]
fn vault_fs(app: &AppHandle, path: &str) -> CmdResult<(Arc<dyn cairn_core::VaultFs>, String, String)> {
    #[cfg(target_os = "android")]
    if path.starts_with("content://") {
        use tauri::Manager;
        let handle = app.state::<crate::android::Saf>().0.clone();
        let decoded = percent_encoding::percent_decode_str(path).decode_utf8_lossy().into_owned();
        let name = decoded.rsplit(['/', ':']).find(|s| !s.is_empty()).unwrap_or("Vault").to_string();
        let fs = crate::android::SafFs::new(handle, path.to_string(), name.clone());
        return Ok((Arc::new(fs), path.to_string(), name));
    }
    let fs = StdFs::new(path, TrashMode::System)?;
    let root = fs.root().to_path_buf();
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.to_string_lossy().into_owned());
    Ok((Arc::new(fs), root.to_string_lossy().into_owned(), name))
}

/// A typed "~/Notes" means Notes in the home folder, as in a shell.
fn expand_home(app: &AppHandle, path: String) -> String {
    use tauri::Manager;
    let rest = if path == "~" { Some("") } else { path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) };
    match (rest, app.path().home_dir()) {
        (Some(rest), Ok(home)) => home.join(rest).to_string_lossy().into_owned(),
        _ => path,
    }
}

#[tauri::command]
pub async fn open_vault(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<VaultInfo> {
    let path = expand_home(&app, path);
    let (fs, root_str, name) = vault_fs(&app, &path)?;
    #[cfg(target_os = "android")]
    if app_vaults_root(&app).ok().and_then(|r| r.canonicalize().ok()).is_some_and(|r| PathBuf::from(&root_str).starts_with(r)) {
        // A kill during a save leaves a temp copy of the note behind.
        let root = PathBuf::from(&root_str);
        std::thread::spawn(move || crate::android::remove_stale_temp_files(&root));
    }
    let started = std::time::Instant::now();
    let v = Arc::new(Vault::open(fs)?);
    log::info!("opened vault {root_str} ({} notes) in {:?}", v.index().note_count(), started.elapsed());
    #[cfg(desktop)]
    {
        // Drop the old watcher before starting a new one.
        *state.watcher.lock() = None;
        match crate::watcher::start(app.clone(), v.clone(), PathBuf::from(&root_str)) {
            Ok(w) => *state.watcher.lock() = Some(w),
            Err(e) => log::warn!("file watcher unavailable: {e}"),
        }
    }
    // Sync for this vault (state lives in the app data folder). When this
    // vault was open already, a sync the old manager still runs ends before
    // the new one syncs (see `sync::run_lock`).
    if let Some(old) = state.sync.lock().take() {
        old.stop();
    }
    if let Ok(data) = tauri::Manager::path(&app).app_data_dir() {
        let dir = crate::sync::state_dir(&data, &root_str);
        *state.sync.lock() = Some(crate::sync::SyncManager::new(app.clone(), v.clone(), dir, root_str.clone()));
    }
    *state.vault.write() = Some(v.clone());
    *state.vault_root.write() = Some(root_str.clone());
    state.config.lock().touch_recent(&root_str);
    Ok(VaultInfo { name, root: root_str, entries: v.entries() })
}

/// Create the folder (and parents) and open it as a vault.
#[tauri::command]
pub async fn create_vault(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<VaultInfo> {
    // On mobile a bare name means a vault in the app's own storage.
    let path = if cfg!(mobile) && !path.contains('/') { app_vault_path(&app, &path)? } else { expand_home(&app, path) };
    std::fs::create_dir_all(&path).map_err(|e| CoreError::io(&path, e))?;
    open_vault(app, state, path).await
}

/// Folder holding vaults created inside the app's storage (mobile).
fn app_vaults_root(app: &AppHandle) -> CmdResult<PathBuf> {
    use tauri::Manager;
    let base = app.path().app_data_dir().map_err(|e| CoreError::Io(e.to_string()))?;
    Ok(base.join("vaults"))
}

fn app_vault_path(app: &AppHandle, name: &str) -> CmdResult<String> {
    cairn_core::path::validate_name(name)?;
    Ok(app_vaults_root(app)?.join(name).to_string_lossy().into_owned())
}

/// "android", "ios", "linux", "macos" or "windows".
#[tauri::command]
pub fn platform() -> &'static str {
    std::env::consts::OS
}

/// Vaults stored inside the app (mobile): (name, path) pairs.
#[tauri::command]
pub fn app_vaults(app: AppHandle) -> Vec<(String, String)> {
    let Ok(root) = app_vaults_root(&app) else { return Vec::new() };
    let mut out: Vec<(String, String)> = std::fs::read_dir(&root)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[derive(Serialize)]
pub struct PickedFolder {
    uri: Option<String>,
    name: Option<String>,
}

/// Android: let the user pick a folder with the system picker (Storage
/// Access Framework). Returns the persisted tree URI, or nothing if cancelled.
#[tauri::command]
pub async fn pick_folder(app: AppHandle) -> Result<PickedFolder, String> {
    #[cfg(target_os = "android")]
    {
        let p = tauri::async_runtime::spawn_blocking(move || crate::android::pick_folder(&app)).await.map_err(|e| e.to_string())??;
        return Ok(PickedFolder { uri: p.uri, name: p.name });
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err("the system folder picker command is only used on Android".into())
    }
}

#[tauri::command]
pub fn recent_vaults(state: State<'_, AppState>) -> Vec<String> {
    state.config.lock().recent_vaults.clone()
}

#[tauri::command]
pub fn forget_vault(state: State<'_, AppState>, path: String) {
    state.config.lock().forget(&path);
}

#[tauri::command]
pub async fn list_entries(state: State<'_, AppState>) -> CmdResult<Vec<FileStat>> {
    Ok(vault(&state)?.entries())
}

#[tauri::command]
pub async fn read_note(state: State<'_, AppState>, path: String, in_vault: Option<bool>) -> CmdResult<NoteContent> {
    vault_for(&state, &path, in_vault)?.read_note(&path)
}

#[tauri::command]
pub async fn write_note(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    content: String,
    base_hash: Option<String>,
    in_vault: Option<bool>,
) -> CmdResult<WriteResult> {
    let r = vault_for(&state, &path, in_vault)?.write_note(&path, &content, base_hash.as_deref())?;
    emit(&app, &r.changes);
    Ok(r)
}

/// Save a note whose file is gone; fails with a conflict if a file is there again.
#[tauri::command]
pub async fn recreate_note(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> CmdResult<WriteResult> {
    let r = vault(&state)?.recreate_note(&path, &content)?;
    emit(&app, &r.changes);
    Ok(r)
}

/// Merge a note's unsaved text (`ours`) with a change made on disk since it
/// was loaded (`base` to `theirs`), as sync merges notes. `None` when the
/// changes overlap, or when a side changed too many lines to merge quickly
/// (a 2 MB note took about a minute in a debug build). Async, and the
/// merge runs on a blocking thread: the main thread never waits for it.
#[tauri::command]
pub async fn merge_text(base: String, ours: String, theirs: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || cairn_sync::merge_text(&base, &ours, &theirs))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_note(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    content: Option<String>,
    in_vault: Option<bool>,
) -> CmdResult<WriteResult> {
    let r = vault_for(&state, &path, in_vault)?.create_note(&path, content.as_deref().unwrap_or(""))?;
    emit(&app, &r.changes);
    Ok(r)
}

#[tauri::command]
pub async fn create_folder(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<Vec<Change>> {
    let c = vault(&state)?.create_folder(&path)?;
    emit(&app, &c);
    Ok(c)
}

#[tauri::command]
pub async fn rename_entry(
    app: AppHandle,
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> CmdResult<Vec<Change>> {
    let v = vault(&state)?;
    // Not to a name that differs only in case from another entry there, as
    // with create (`Vault::rename` takes it, for sync).
    v.check_rename(&from, &to)?;
    // Through sync, which records the rename for its next run.
    let sync = state.sync.lock().clone();
    let c = match sync {
        Some(s) => s.rename(&v, &from, &to)?,
        None => v.rename(&from, &to)?,
    };
    emit(&app, &c);
    Ok(c)
}

#[tauri::command]
pub async fn delete_entry(app: AppHandle, state: State<'_, AppState>, path: String) -> CmdResult<Vec<Change>> {
    let c = vault(&state)?.delete(&path)?;
    emit(&app, &c);
    Ok(c)
}

#[tauri::command]
pub async fn unique_path(state: State<'_, AppState>, dir: String, base: String, ext: String) -> CmdResult<String> {
    vault(&state)?.unique_path(&dir, &base, &ext)
}

#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String, limit: Option<usize>) -> CmdResult<Vec<SearchHit>> {
    Ok(vault(&state)?.search(&query, limit.unwrap_or(100)))
}

#[tauri::command]
pub async fn backlinks(state: State<'_, AppState>, path: String) -> CmdResult<Vec<Backlinks>> {
    Ok(vault(&state)?.backlinks(&path))
}

#[tauri::command]
pub async fn outgoing_links(state: State<'_, AppState>, path: String) -> CmdResult<Vec<OutgoingLink>> {
    Ok(vault(&state)?.outgoing(&path))
}

/// What a click on a link in `source` opens. `kind` is "wiki" (the default)
/// or "markdown", which resolves relative to the source note first.
#[tauri::command]
pub async fn resolve_link(
    state: State<'_, AppState>,
    target: String,
    source: String,
    kind: Option<LinkKind>,
) -> CmdResult<Option<String>> {
    Ok(vault(&state)?.resolve_target(&target, kind.unwrap_or(LinkKind::Wiki), &source))
}

#[tauri::command]
pub async fn rescan(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Vec<Change>> {
    let c = vault(&state)?.rescan()?;
    emit(&app, &c);
    Ok(c)
}

/// Called by the UI once the first screen is painted; logs start-up time.
#[tauri::command]
pub fn ui_ready() -> u64 {
    let ms = crate::STARTED.get().map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
    log::info!("UI ready {ms} ms after process start");
    ms
}

/// Read a file from the vault's `.cairn/` config folder (`None` if missing).
#[tauri::command]
pub async fn read_config(state: State<'_, AppState>, name: String) -> CmdResult<Option<String>> {
    vault(&state)?.read_config(&name)
}

#[tauri::command]
pub async fn write_config(state: State<'_, AppState>, name: String, content: String) -> CmdResult<()> {
    vault(&state)?.write_config(&name, &content)
}

/// File names in a `.cairn/` subfolder (e.g. "snippets").
#[tauri::command]
pub async fn list_config(state: State<'_, AppState>, dir: String) -> CmdResult<Vec<String>> {
    vault(&state)?.list_config(&dir)
}

/// Plugins turned on on this device for the open vault, by file name (kept in
/// the app config folder, never in the vault).
#[tauri::command]
pub async fn plugin_approvals(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<std::collections::BTreeMap<String, crate::config::PluginApproval>> {
    let root = vault_root(&state)?;
    crate::config::plugin_approvals(&app, &root).map_err(|e| CoreError::io("plugin approvals", e))
}

/// Record that the user turned a plugin on (`approval`) or off (`None`) on this device.
#[tauri::command]
pub async fn set_plugin_approval(
    app: AppHandle,
    state: State<'_, AppState>,
    file: String,
    approval: Option<crate::config::PluginApproval>,
) -> CmdResult<()> {
    let root = vault_root(&state)?;
    crate::config::set_plugin_approval(&app, &root, &file, approval).map_err(|e| CoreError::io("plugin approvals", e))
}

/// Largest file an embed shows as text; bigger ones get a file card.
const MAX_EMBED_TEXT: u64 = 256 * 1024;

/// Read a vault file as text, for embeds of non-note files. `None` when it
/// is too big to show in a note or is not UTF-8 text (a PDF, an archive):
/// the embed then shows a file card instead of the raw bytes.
#[tauri::command]
pub async fn read_text_file(state: State<'_, AppState>, path: String) -> CmdResult<Option<String>> {
    let v = vault(&state)?;
    let rel = cairn_core::path::normalize(&path)?;
    // Check the size first: a video or a disk image is never read at all.
    if v.fs().stat(&rel)?.is_some_and(|st| st.size > MAX_EMBED_TEXT) {
        return Ok(None);
    }
    let bytes = v.read_file(&rel)?;
    if bytes.contains(&0) {
        return Ok(None);
    }
    Ok(String::from_utf8(bytes).ok())
}

/// Store pasted or dropped bytes as a new file. The body is the raw bytes;
/// the `x-dir` and `x-name` headers (percent-encoded) say where. A free name
/// is chosen if the file exists. Returns the vault path.
#[tauri::command]
pub async fn save_attachment(app: AppHandle, state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> CmdResult<String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(CoreError::InvalidPath("expected raw bytes".into()));
    };
    let header = |k: &str| -> String {
        request
            .headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(|v| percent_encoding::percent_decode_str(v).decode_utf8_lossy().into_owned())
            .unwrap_or_default()
    };
    let dir = header("x-dir");
    let name = header("x-name");
    let v = vault(&state)?;
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_string(), name[i + 1..].to_string()),
        _ => (name.clone(), String::new()),
    };
    let path = v.unique_path(&dir, &stem, &ext)?;
    let r = v.create_file(&path, bytes)?;
    emit(&app, &r.changes);
    Ok(path)
}

#[tauri::command]
pub async fn tags(state: State<'_, AppState>) -> CmdResult<Vec<cairn_core::index::TagCount>> {
    Ok(vault(&state)?.tags())
}

#[tauri::command]
pub async fn graph(state: State<'_, AppState>, include_unresolved: bool) -> CmdResult<cairn_core::index::Graph> {
    Ok(vault(&state)?.graph(include_unresolved))
}

#[tauri::command]
pub async fn note_info(state: State<'_, AppState>, path: String) -> CmdResult<Option<cairn_core::index::NoteInfo>> {
    Ok(vault(&state)?.note_info(&path))
}

/// Absolute OS path of a vault entry (desktop only; vaults there are folders).
fn os_path(state: &State<'_, AppState>, path: &str) -> CmdResult<PathBuf> {
    let v = vault(state)?;
    let rel = cairn_core::path::normalize(path)?;
    // The file system knows the real name (it can differ from the NFC one).
    if let Some(p) = v.fs().os_path(&rel) {
        return Ok(p);
    }
    let mut p = PathBuf::from(v.fs().describe());
    for c in rel.split('/').filter(|c| !c.is_empty()) {
        p.push(c);
    }
    Ok(p)
}

/// File types `open_externally` hands to the system's default app:
/// documents, images, media and archives. The system opener also runs
/// programs, scripts and launchers (`.desktop`, `.bat`, `.lnk`, `.js` on
/// Windows, an extensionless executable on macOS), so every other type is
/// refused, as is a file with no extension. Web pages and XML (`html`,
/// `htm`, `xml`) and Office files with macros (`docm`, `xlsm`, `pptm`) stay
/// out on purpose: their app runs the scripts or macros inside them.
const OPEN_EXTERNALLY: &[&str] = &[
    "pdf", "epub", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "odg", "pages", "numbers",
    "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico", "tif", "tiff", "heic", "heif",
    "mp3", "ogg", "oga", "opus", "wav", "m4a", "flac", "aac", "mp4", "m4v", "webm", "mov", "mkv", "avi",
    "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz",
];

/// Text types `open_externally` also opens, unless the file has an
/// executable bit: that makes a text file a script, and some desktops offer
/// to run it. (The bit is not checked for the binary types above: their app
/// is picked by type, and on exFAT or NTFS drives every file has it.)
const OPEN_EXTERNALLY_TEXT: &[&str] =
    &["txt", "log", "csv", "tsv", "rtf", "svg", "json", "yaml", "yml", "ics", "vcf"];

#[derive(PartialEq)]
enum OpenKind {
    Binary,
    Text,
}

fn open_kind(p: &std::path::Path) -> Option<OpenKind> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if OPEN_EXTERNALLY.contains(&ext.as_str()) {
        Some(OpenKind::Binary)
    } else if OPEN_EXTERNALLY_TEXT.contains(&ext.as_str()) {
        Some(OpenKind::Text)
    } else {
        None
    }
}

/// Open a file with the system's default application. Only the types in
/// `OPEN_EXTERNALLY` and `OPEN_EXTERNALLY_TEXT`; anything else would let a
/// disguised attachment (`report.pdf.desktop`) run code on a click.
#[tauri::command]
pub async fn open_externally(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let refused = || {
        CoreError::Io(
            "Cairn does not open programs, scripts or unknown file types. Use Reveal in file manager to open it yourself."
                .into(),
        )
    };
    let p = os_path(&state, &path)?;
    let kind = open_kind(&p).ok_or_else(refused)?;
    // A symlink must point to an allowed type too: the system may open the
    // file it points to by that file's own name.
    let real = std::fs::canonicalize(&p).map_err(|e| CoreError::io(&path, e))?;
    let real_kind = open_kind(&real).ok_or_else(refused)?;
    let meta = std::fs::metadata(&real).map_err(|e| CoreError::io(&path, e))?;
    if !meta.is_file() {
        return Err(CoreError::Io("it is not a file.".into()));
    }
    if (kind == OpenKind::Text || real_kind == OpenKind::Text) && is_executable(&meta) {
        return Err(CoreError::Io(
            "it is marked as executable. Use Reveal in file manager to open it yourself.".into(),
        ));
    }
    tauri_plugin_opener::open_path(&p, None::<&str>).map_err(|e| CoreError::Io(e.to_string()))
}

#[cfg(unix)]
fn is_executable(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    false
}

/// Show a file or folder in the system file manager.
#[tauri::command]
pub async fn reveal_in_file_manager(state: State<'_, AppState>, path: String) -> CmdResult<()> {
    let p = os_path(&state, &path)?;
    tauri_plugin_opener::reveal_item_in_dir(&p).map_err(|e| CoreError::Io(e.to_string()))
}

// ---------- sync ----------

fn sync_mgr(state: &State<'_, AppState>) -> Result<Arc<crate::sync::SyncManager>, String> {
    state.sync.lock().clone().ok_or_else(|| "no vault is open".to_string())
}

#[tauri::command]
pub async fn sync_status(state: State<'_, AppState>) -> Result<crate::sync::SyncStatus, String> {
    Ok(sync_mgr(&state)?.status())
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSetupArgs {
    server: String,
    token: String,
    vault_id: String,
    device: String,
    passphrase: String,
}

fn sync_settings(server: &str, token: &str, vault_id: &str, device: &str) -> cairn_sync::engine::SyncSettings {
    cairn_sync::engine::SyncSettings {
        server: server.trim().trim_end_matches('/').to_string(),
        token: token.trim().to_string(),
        vault_id: vault_id.trim().to_string(),
        device: device.trim().to_string(),
    }
}

#[tauri::command]
pub async fn sync_setup(state: State<'_, AppState>, args: SyncSetupArgs) -> Result<crate::sync::SyncStatus, String> {
    let m = sync_mgr(&state)?;
    let settings = sync_settings(&args.server, &args.token, &args.vault_id, &args.device);
    tauri::async_runtime::spawn_blocking(move || m.setup(settings, &args.passphrase))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncVaultArgs {
    server: String,
    token: String,
    vault_id: String,
}

/// Whether the server has the vault named in the setup form (it is not
/// created; see `SyncManager::vault_exists`).
#[tauri::command]
pub async fn sync_vault_exists(state: State<'_, AppState>, args: SyncVaultArgs) -> Result<bool, String> {
    let m = sync_mgr(&state)?;
    let settings = sync_settings(&args.server, &args.token, &args.vault_id, "");
    tauri::async_runtime::spawn_blocking(move || m.vault_exists(&settings))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>) -> Result<crate::sync::SyncStatus, String> {
    let m = sync_mgr(&state)?;
    tauri::async_runtime::spawn_blocking(move || m.sync_now()).await.map_err(|e| e.to_string())
}

/// Give up the sync setup in progress.
#[tauri::command]
pub async fn sync_cancel(state: State<'_, AppState>) -> Result<(), String> {
    sync_mgr(&state)?.cancel();
    Ok(())
}

#[tauri::command]
pub async fn sync_disconnect(state: State<'_, AppState>) -> Result<crate::sync::SyncStatus, String> {
    // Cancels a running sync of this manager and waits for it to stop;
    // also waits for a sync the vault's previous manager still runs.
    let m = sync_mgr(&state)?;
    tauri::async_runtime::spawn_blocking(move || m.disconnect()).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn sync_history(state: State<'_, AppState>, path: String) -> Result<Vec<cairn_sync::protocol::HistoryEntry>, String> {
    let m = sync_mgr(&state)?;
    tauri::async_runtime::spawn_blocking(move || m.read(|e| e.history(&path))).await.map_err(|e| e.to_string())?
}

#[derive(serde::Serialize)]
pub struct RevisionText {
    path: String,
    text: String,
}

#[tauri::command]
pub async fn sync_revision(state: State<'_, AppState>, seq: u64) -> Result<RevisionText, String> {
    let m = sync_mgr(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        m.read(|e| e.revision_content(seq)).map(|p| RevisionText { path: p.path, text: String::from_utf8_lossy(&p.data).into_owned() })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn sync_restore(app: AppHandle, state: State<'_, AppState>, path: String, seq: u64) -> Result<(), String> {
    let m = sync_mgr(&state)?;
    let changes = tauri::async_runtime::spawn_blocking(move || m.restore(&path, seq))
        .await
        .map_err(|e| e.to_string())??;
    emit(&app, &changes);
    Ok(())
}

/// A sensible default device name (host name, or the platform).
#[tauri::command]
pub fn default_device_name() -> String {
    if cfg!(target_os = "android") {
        return "Android".into();
    }
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}
