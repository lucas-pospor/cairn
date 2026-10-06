//! The sync client.
//!
//! Local state (outside the vault, in the app's data folder):
//! * `config.json`: server, token, vault id, device name
//! * `key`: the unwrapped vault key (base64). The passphrase is not stored.
//! * `state.json`: for each file id, the last server revision this device
//!   has seen (path, seq, content hash), how the file here looked when a
//!   scan last found that content (to skip hashing it again), the server's
//!   path while the file is under a name here that is not uploaded yet, and
//!   the changes-feed cursor
//! * `state.journal`: the uploads made since `state.json` was last written,
//!   one JSON line each, after a line with the hash of that `state.json`.
//!   Each upload is recorded at once, and writing the whole state for each
//!   one would make the first sync of a large vault take quadratic time.
//!   Read on top of `state.json`; removed when it is written again (one
//!   left from before that, by a sync that stopped in between, is ignored)
//! * `bases/<file id>`: the content of that revision, the base for
//!   three-way merges of notes
//! * `pending/<file id>`: the encrypted server record of a remote change
//!   that could not be applied here yet, so it is not downloaded again
//! * `renames`: renames made in the app since the last scan (see
//!   [`rename`]), one JSON line each; the next scan takes them into
//!   `state.json`
//! * `lock`: locked while a sync runs, or `connect` sets the folder up.
//!   Another engine on the same folder (the vault opened again while a
//!   sync ran, or a second app instance) waits for it, then goes on from
//!   the state that sync saved, or stops if the folder's configuration is
//!   not its own any more (sync turned off, or connected again). Turning
//!   sync off keeps this file, so that it stays the one every engine locks.
//!
//! One sync round:
//! 1. Scan the vault and compare it with `state.json` to see what changed
//!    locally since the last sync.
//! 2. Pull every file head that changed on the server, in batches of about
//!    64 MB of records, each applied and recorded before the next is read.
//!    Apply a head where the local copy is untouched; otherwise merge (see
//!    below). Deletions are applied first in a batch, so the paths they free
//!    can be taken by the rest (a change held for want of such a path is
//!    tried again after the next batch), and a change that wants the path of
//!    a file with a later change in the batch waits for that one, which may
//!    move the file away (files that swapped names: one moves aside first).
//!    Afterwards `state.json` describes the server's current version of
//!    each file. The user may save files meanwhile: a remote change only
//!    replaces or deletes a file that still has the content the scan found,
//!    and a new file is only written where there is none.
//! 3. Scan again and push every difference between the vault and that
//!    state, with the known revision as parent, a rename or delete that
//!    frees a path before a rename that takes it. Each upload is recorded
//!    at once, and the cursor moves past it while the server has taken no
//!    other change since the last one this device has seen, so the next
//!    pull does not download it again. If the server says another
//!    device got there first, start a new round. A file the pull left under
//!    another name than the server's (a conflict copy name, or a rename made
//!    here) is pushed under that name, in a later sync if this one stops
//!    before.
//!
//! Merge rules, chosen so that nothing is lost silently:
//! * both edited a note: three-way merge; overlapping edits keep the local
//!   file and add the remote version as a conflict copy
//! * both edited an attachment: conflict copy
//! * edit vs delete: the edit wins
//! * rename on one side, edit on the other: both apply. A rename made in
//!   the app is known as one even when the file is edited before the next
//!   sync; one made outside the app is found by the file's unchanged
//!   content, and otherwise is a delete plus a new file (so is an empty
//!   file, or one of several with that content that kept neither its name
//!   nor its folder). Files that swapped
//!   names (through a temporary one) go up as edits of each other, as when
//!   done outside the app.
//! * rename vs delete: the rename wins, and the file keeps its id
//! * both renamed: the remote name wins
//! * a remote file wants a path that is taken locally: it gets a conflict
//!   copy name, unless both created the same file there, which is adopted.
//!   If the server has both files at that path (two devices uploaded one
//!   without seeing the other's), the one uploaded first gets the conflict
//!   copy name, on every device, even when that is the file here. With the
//!   same content they are one note: the one uploaded first is deleted on
//!   the server instead, unless either changed since, and a device that has
//!   it unchanged keeps it under the other's id.
//!   On a file system that ignores case (macOS, Windows) a name that
//!   differs only in case is taken too, and the local name is kept. Android
//!   shared storage (SAF) refuses such a name instead: the remote file waits
//!   as pending and is listed, so no device renames it. A remote rename of a
//!   file here to such a name waits the same way, with the changes made to
//!   the file here meanwhile, unless the user renamed the file here too. The
//!   same file here under a name that differs only in case takes the remote
//!   name there, unless a folder name differs (then the remote file waits).
//! * one file under two names here (a folder linked in twice, a symlink to
//!   a note) syncs under one of them (see `regroup`). A second copy that an
//!   older version uploaded under the other name is a duplicate: nothing is
//!   sent for it but its delete, with the note's when the note is deleted
//!   here. Its remote changes are not applied: a delete, or a version with
//!   the content the file has here, is recorded, and the rest waits and is
//!   listed. It syncs on in place of the note when another device deletes
//!   the note's copy
//! * remote deletions go to the trash, never straight to oblivion
//!
//! One file never stops the others. A remote change that cannot be applied
//! here (a name this file system refuses, a file where a folder is) stays
//! pending and is retried on every sync; that file is not pushed meanwhile.
//! So does any other change of a duplicate (see `Tracked::alias_of`), and
//! any change through a name of a file that is also synced here under
//! another name (see `refuse_alias`).
//! A local file that cannot be read is left alone. A record that does not
//! decrypt or has an unsafe path is dropped (or held as pending, if it
//! belongs to a file this device has). An upload that is too large, or that
//! stalls or is cut off while the server still answers, is retried on the
//! next sync. A file or folder whose name has a backslash is not part of
//! the vault (no vault path can name it). All of them are listed in
//! [`SyncReport::skipped`].
//!
//! A sync stops before changing anything when the server has fewer changes
//! than this device has seen: it was reset, and the user connects again,
//! which starts afresh. It also stops when the vault folder has no files at
//! all (hidden folders do not count) while this device has synced some: the
//! folder was more likely moved or unmounted than emptied by the user, who
//! can add a note to sync the deletions. A lost or damaged `state.json` is
//! rebuilt from the server: a file here with the path and content of a
//! server file, or only its content as an unambiguous rename, is taken for
//! that file rather than uploaded again (renames are found with the whole
//! feed: what does not fit in the first batch is downloaded twice then).
//! Likewise, when the app was killed during the pull, a remote version it
//! already wrote (into a file renamed here, or as a conflict copy) is taken
//! for that version rather than written again.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_core::index::{hash_bytes, hash_hex, Hash};
use cairn_core::path as vpath;
use cairn_core::{Change, CoreError, EntryKind, Vault};
use serde::{Deserialize, Serialize};

use crate::crypto::{random_bytes, FilePayload, VaultKey, DEFAULT_KDF};
use crate::protocol::*;
use crate::transport::{too_large, HttpTransport, PutOutcome, Transport};
use crate::SyncError;

const MAX_ROUNDS: usize = 6;
const PAGE: u32 = 500;
/// The pull reads the changes feed in batches of about this many bytes of
/// records (encrypted and base64-encoded, a third larger than the files),
/// and applies and records each batch before it reads the next: a first
/// sync can bring gigabytes. A batch can be larger by one page, which the
/// transport keeps to the size of one upload.
const PULL_BATCH: u64 = 64 << 20;
/// A file modified less than this long ago (ms) can be saved again without
/// its mtime changing, as file systems keep mtimes in steps (FAT in 2 s),
/// so the scan does not cache its hash yet.
const MTIME_SLACK: i64 = 3_000;
/// See the module documentation.
const JOURNAL: &str = "state.journal";
/// Notes are not merged when one side made more line edits than this:
/// the diff takes time in proportion to the square of the edits (about a
/// quarter of a second for this many in a release build; a 20,000-line
/// note rewritten on both sides took a minute in a debug build), and the
/// sync waits for it. Such a note gets a conflict copy instead.
const MERGE_MAX_EDITS: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncSettings {
    pub server: String,
    pub token: String,
    pub vault_id: String,
    pub device: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tracked {
    pub path: String,
    pub seq: u64,
    /// blake3 hex of the content at `seq` (empty when deleted).
    pub hash: String,
    pub deleted: bool,
    /// How the file looked when a scan last found it with this content, to
    /// skip hashing it again while it looks the same. Only the scan sets
    /// it; anything else that records a file leaves it empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<Seen>,
    /// The server's path at `seq`, while the file is under another name here
    /// that the server does not have yet: a conflict copy name, or a rename
    /// made here when a remote change of the file came in. For a file
    /// renamed here and deleted there, it is the name the server had before
    /// the delete. Every push uploads the rename until one gets through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_path: Option<String>,
    /// Set with `server_path` when the name here is one the user gave the
    /// file in the app (see [`rename`]), not one the sync chose: a remote
    /// delete then keeps the file, as it keeps one the scan finds renamed.
    #[serde(default, skip_serializing_if = "is_false")]
    pub renamed: bool,
    /// For a duplicate, the id of the file it duplicates: the vault reaches
    /// one file here under two names (a folder linked in twice, a symlink
    /// to a note), and an older version uploaded it under both, so other
    /// devices have two files (FINDING-224). Only the other one syncs from
    /// here. Nothing is sent for a duplicate but its delete, when the note
    /// is deleted here. A change of it from another device is not applied:
    /// a delete, or a version with the content the file has here, is
    /// recorded; anything else waits and is listed. See `regroup` and
    /// `heir`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias_of: Option<String>,
    /// For a duplicate that took the place of the note it duplicated (see
    /// `heir`), the content hash the note had here then. That content is
    /// synced already, under the note's id: until this file's next upload,
    /// a delete of it from another device applies while it still has it,
    /// though its own last version is older.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub took_over: Option<String>,
    /// For a duplicate: the note it duplicates was deleted here, and on
    /// the server too, so its delete goes up as well, as when the note's
    /// delete is pushed from here.
    #[serde(default, skip_serializing_if = "is_false")]
    pub delete_with_note: bool,
    /// For a duplicate: its name went here while the note it duplicates
    /// stayed (the link removed, or the note moved away from it), so it is
    /// no copy that a link here makes any more. A delete of the note here
    /// does not delete it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub detached: bool,
}

fn is_false(b: &bool) -> bool {
    !b
}

/// Size, mtime and change stamp (see `VaultFs::change_stamp`) of a local
/// file, from a stat taken before the file was read and still the same
/// after it, so that they describe the bytes that were hashed. Not kept for
/// a file modified less than [`MTIME_SLACK`] before the scan: a later save
/// could leave it looking the same.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Seen {
    pub size: u64,
    pub mtime: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    pub last_seq: u64,
    pub files: BTreeMap<String, Tracked>,
    /// Server revisions that are behind the cursor but not applied here yet
    /// (file id -> seq). Retried on every sync until a newer head replaces
    /// them.
    #[serde(default)]
    pub pending: BTreeMap<String, u64>,
}

/// An upload as `state.journal` records it: the file's entry in the state,
/// and the cursor after it.
#[derive(Serialize, Deserialize)]
struct Recorded {
    fid: String,
    file: Tracked,
    last_seq: u64,
}

/// `state.journal` as an engine last read or wrote it.
#[derive(Default)]
struct Journal {
    /// Its length, while it goes on top of `state.json` as the engine has
    /// it (0: there is none). `None` when it is from before that
    /// `state.json` was written, or cut off at the end, or the state is not
    /// from `state.json`: the next record writes the whole state instead.
    len: Option<u64>,
    /// Hash of all of its bytes, to see whether another engine has written
    /// to it since.
    hash: blake3::Hasher,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub pulled: usize,
    pub pushed: usize,
    pub rounds: usize,
    /// Conflict copies created in this sync.
    pub conflicts: Vec<String>,
    /// Files this sync had to leave out, and why.
    pub skipped: Vec<Skipped>,
    /// Vault changes made by the sync (for the UI).
    #[serde(skip)]
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    /// Vault path; empty for a server record that cannot be read at all.
    pub path: String,
    pub reason: String,
}

impl SyncReport {
    fn skip(&mut self, path: &str, reason: String) {
        let s = Skipped { path: path.to_string(), reason };
        if !self.skipped.contains(&s) {
            self.skipped.push(s);
        }
    }
}

#[derive(Debug, Clone)]
struct LocalFile {
    hash: String,
}

/// A file in the changes feed, as `relink` needs it.
struct ServerFile {
    fid: String,
    seq: u64,
    /// Path and content hash (`None` for an empty file); `None` when the
    /// head deletes the file or cannot be read.
    file: Option<(String, Option<String>)>,
}

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Unchanged,
    Modified,
    Deleted,
    /// The file is under this name now, with its content unchanged; or, on
    /// a file system that ignores case, under a name that differs only in
    /// case, and maybe edited too.
    Renamed(String),
    /// The file is there but cannot be read; it is left alone.
    Unreadable,
    /// On a file system that ignores case, another tracked file's name
    /// differs from this one's only in case, so both are one file on disk
    /// (a client without case-twin handling could leave that behind).
    /// Nothing is pushed for it; it is downloaded again as a new file.
    Shared,
}

pub struct SyncEngine {
    vault: Arc<Vault>,
    transport: Box<dyn Transport>,
    key: VaultKey,
    settings: SyncSettings,
    dir: PathBuf,
    state: SyncState,
    /// Hash of `state.json` as this engine last read or wrote it (`None`:
    /// there was none), to see whether another engine has saved it since.
    state_hash: Option<Hash>,
    journal: Journal,
    /// See [`PULL_BATCH`].
    pull_batch: u64,
    /// Where the last scan found tracked files that were renamed here
    /// since the last sync (`Status::Renamed`): their state still has the
    /// old path. For `tracked_aliases`.
    renamed_here: HashSet<String>,
    /// The names of files here that are not synced because the file syncs
    /// under another name, with that name, as the last scan found them
    /// (see `regroup`).
    kept: HashMap<String, String>,
    /// The files whose delete is in the batch of the pull being applied.
    deleting: HashSet<String>,
    /// New files reached through a link that wait for the last batch of
    /// the pull (see `apply_batch`).
    linked_waits: HashSet<String>,
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), SyncError> {
    write_bytes(path, &serde_json::to_vec_pretty(v).map_err(|e| SyncError::Local(e.to_string()))?)
}

/// Replace a file at once (through a temporary file and a rename).
fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), SyncError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// The configuration saved in a sync state folder, if any.
fn read_config(dir: &Path) -> Option<SyncSettings> {
    std::fs::read(dir.join("config.json")).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// Lock the sync state folder `dir`, waiting while another engine has it
/// locked (in this process or another). Unlocked when the file is dropped.
/// Without a folder there is no sync to lock: `NotConfigured`.
fn lock_dir(dir: &Path) -> Result<std::fs::File, SyncError> {
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("lock")).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { SyncError::NotConfigured } else { SyncError::from(e) }
    })?;
    lock_file(&file, dir);
    Ok(file)
}

/// Lock `file` (of `what`), waiting while another handle has it locked.
fn lock_file(file: &std::fs::File, what: &Path) {
    loop {
        match file.lock() {
            Ok(()) => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // A file system without locks: go on as before there were any.
            Err(e) => {
                log::warn!("sync: cannot lock {}: {e}", what.display());
                break;
            }
        }
    }
}

/// A rename made in the app, as [`rename`] records it.
#[derive(Debug, Serialize, Deserialize)]
struct Moved {
    from: String,
    to: String,
    /// A folder: the files in it moved.
    #[serde(default, skip_serializing_if = "is_false")]
    dir: bool,
}

/// Rename a file or folder in the vault for the user, as the app does, and
/// record the rename for the next sync from the sync state folder `dir`, if
/// sync is set up there. The scan then takes each file under its new name
/// for the file it was, even when it is edited before that sync: the rename
/// is uploaded as a change of the same file, which keeps its history and
/// merges with edits made elsewhere meanwhile. A rename made outside the
/// app is only found by the file's unchanged content. A rename that cannot
/// be recorded is still made.
pub fn rename(vault: &Vault, dir: &Path, from: &str, to: &str) -> cairn_core::Result<Vec<Change>> {
    if !dir.join("config.json").exists() {
        return vault.rename(from, to);
    }
    // Locked across the rename: the scan lists the vault either before the
    // rename or with it recorded (see `take_renames`).
    let path = dir.join("renames");
    let journal = std::fs::OpenOptions::new().create(true).append(true).open(&path);
    if let Ok(f) = &journal {
        lock_file(f, &path);
    }
    let changes = vault.rename(from, to)?;
    let lines: String = changes
        .iter()
        .filter_map(|c| match c {
            Change::Renamed { from, entry } => {
                let m = Moved { from: from.clone(), to: entry.path.clone(), dir: entry.kind == EntryKind::Dir };
                serde_json::to_string(&m).ok().map(|l| l + "\n")
            }
            _ => None,
        })
        .collect();
    if let Err(e) = journal.and_then(|mut f| std::io::Write::write_all(&mut f, lines.as_bytes())) {
        log::warn!("sync: cannot record the rename of {from} in {}: {e}", path.display());
    }
    Ok(changes)
}

/// A change to upload: file id, parent revision, path, deleted.
type Upload = (String, Option<u64>, String, bool);

/// What a remote change would do to a file here (see `refuse_alias`).
#[derive(Clone, Copy)]
enum Act {
    Delete,
    Move,
    Write,
}

/// The path and content of a remote change (see `open`).
type Opened = (String, Vec<u8>);

/// The uploads `ops` in an order that other devices can apply one at a
/// time, as they get them in the order of the server: one that takes a path
/// that another frees on the server (`frees`, by a rename or a delete;
/// case does not count) goes after it, so that the path is free there when
/// it comes, as when a.md was renamed to z.md and then b.md to a.md.
/// Otherwise they stay in the given order, and so do uploads that wait for
/// each other in a ring.
fn upload_order(ops: Vec<Upload>, frees: &[Option<String>]) -> Vec<Upload> {
    let mut freeing: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, f) in frees.iter().enumerate() {
        if let Some(f) = f {
            freeing.entry(f.to_lowercase()).or_default().push(i);
        }
    }
    if freeing.is_empty() {
        return ops;
    }
    let mut after: Vec<Vec<usize>> = vec![Vec::new(); ops.len()];
    let mut waits = vec![0usize; ops.len()];
    for (i, (_, _, path, deleted)) in ops.iter().enumerate() {
        for &j in freeing.get(&path.to_lowercase()).into_iter().flatten().filter(|&&j| j != i && !deleted) {
            after[j].push(i);
            waits[i] += 1;
        }
    }
    let mut ready: BTreeSet<usize> = (0..ops.len()).filter(|&i| waits[i] == 0).collect();
    let mut order = Vec::with_capacity(ops.len());
    while let Some(i) = ready.pop_first() {
        order.push(i);
        for &k in &after[i] {
            waits[k] -= 1;
            if waits[k] == 0 {
                ready.insert(k);
            }
        }
    }
    order.extend((0..ops.len()).filter(|&i| waits[i] > 0));
    let mut ops: Vec<Option<Upload>> = ops.into_iter().map(Some).collect();
    order.into_iter().filter_map(|i| ops[i].take()).collect()
}

fn hex_hash(data: &[u8]) -> String {
    hash_hex(&hash_bytes(data))
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn new_file_id() -> String {
    random_bytes::<16>().iter().map(|b| format!("{b:02x}")).collect()
}

/// Civil date from a Unix timestamp (UTC), for conflict copy names.
fn stamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's days-to-civil algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}{:02}", rem / 3600, (rem % 3600) / 60)
}

/// Longest conflict copy name in bytes: below the file system limit by
/// room for the counter `Vault::unique_path` adds when the name is taken
/// (" 2") and for the temp file `StdFs::write` names after the target
/// (".<name>.cairn-tmp-<pid>", up to 19 bytes more), so that every device
/// can write the copy.
const CONFLICT_NAME_MAX: usize = vpath::NAME_MAX - 32;
/// Longest device name part of a conflict copy name, in bytes.
const CONFLICT_DEVICE_MAX: usize = 64;
/// Longest extension a conflict copy keeps, in bytes. Longer text after the
/// last dot, as in "Mr. <a long title>", is part of the name.
const CONFLICT_EXT_MAX: usize = 32;

/// Base name and extension of a conflict copy of `path`:
/// "<stem> (conflict <stamp> <device>)". The device name is cleaned of
/// characters that some file systems or links refuse, and the stem is
/// shortened (on a char boundary) to keep the name within
/// [`CONFLICT_NAME_MAX`]. The tag is at most 92 bytes and the extension 33
/// with its dot, so at least 98 bytes are left for the stem.
fn conflict_name(path: &str, stamp: &str, device: &str) -> (String, String) {
    let name = vpath::file_name(path);
    let ext = if vpath::extension(path).is_some() { name.rsplit('.').next().unwrap() } else { "" };
    let (stem, ext) = if ext.len() > CONFLICT_EXT_MAX { (name, "") } else { (vpath::stem(path), ext) };
    let device = vpath::sanitize_name_part(device);
    let device = vpath::truncate(device.trim(), CONFLICT_DEVICE_MAX).trim_end();
    let tag = if device.is_empty() { format!(" (conflict {stamp})") } else { format!(" (conflict {stamp} {device})") };
    let dot_ext = if ext.is_empty() { 0 } else { ext.len() + 1 };
    let room = CONFLICT_NAME_MAX.saturating_sub(tag.len() + dot_ext);
    (format!("{}{tag}", vpath::truncate(stem, room)), ext.to_string())
}

fn is_mergeable(path: &str) -> bool {
    vpath::is_markdown(path) || vpath::extension(path).as_deref() == Some("txt")
}

/// Three-way merge of two versions of a note; None for a conflict, or for
/// a side with too many edits to diff quickly (`MERGE_MAX_EDITS`).
pub(crate) fn merge_text(base: &str, ours: &str, theirs: &str) -> Option<String> {
    if theirs == base {
        return Some(ours.to_string());
    }
    if ours == base {
        return Some(theirs.to_string());
    }
    if edits_at_least(base, ours) > MERGE_MAX_EDITS || edits_at_least(base, theirs) > MERGE_MAX_EDITS {
        return None;
    }
    diffy::merge(base, ours, theirs).ok()
}

/// At least how many lines a diff from `base` to `side` adds or removes,
/// as far as the cost of the diff goes: lines between the common start and
/// end that one has more often than the other. Lines only added or only
/// removed there cost next to nothing, so they count as none. (A reordering
/// is not counted.)
fn edits_at_least(base: &str, side: &str) -> usize {
    let a: Vec<&str> = base.split_inclusive('\n').collect();
    let b: Vec<&str> = side.split_inclusive('\n').collect();
    let start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let end = a[start..].iter().rev().zip(b[start..].iter().rev()).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[start..a.len() - end], &b[start..b.len() - end]);
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut count: HashMap<&str, isize> = HashMap::new();
    for l in a {
        *count.entry(l).or_default() += 1;
    }
    for l in b {
        *count.entry(l).or_default() -= 1;
    }
    count.values().map(|c| c.unsigned_abs()).sum()
}

/// The user edited a file while sync was writing it (or see [`MOVED_ON`]):
/// the whole round is retried rather than the file being skipped.
fn is_race(e: &SyncError) -> bool {
    matches!(e, SyncError::Changed(_)) || matches!(e, SyncError::Local(msg) if msg == MOVED_ON)
}

/// Another device changed a file on the server while this sync was deciding
/// about it: the round is retried too, and its pull shows the change.
const MOVED_ON: &str = "a file changed on the server during sync";

/// A pending change that a newer one on the server replaces: it waits for
/// that one (in a later batch of the pull, or the next sync). Not a race:
/// a new round would try the same pending change again.
const SUPERSEDED: &str = "a newer change to this file is on the server and is synced next";

/// The server does not have the revision asked for: HTTP 404, which
/// `transport` words as "no such revision (HTTP 404)" when the server says
/// why, and "HTTP 404" when it does not.
fn is_gone(e: &SyncError) -> bool {
    matches!(e, SyncError::Server(msg) if msg.starts_with("HTTP 404") || msg.ends_with("(HTTP 404)"))
}

/// An error as a reason in [`SyncReport::skipped`].
fn reason(e: &SyncError) -> String {
    match e {
        SyncError::Local(m) | SyncError::Server(m) | SyncError::Upload(m) => m.clone(),
        SyncError::Changed(p) => format!("file changed on disk: {p}"),
        e => e.to_string(),
    }
}

impl SyncEngine {
    /// Set up sync for a vault: unlock the vault on the server (or create it
    /// there) with the passphrase, and store the configuration in `dir`.
    pub fn connect(vault: Arc<Vault>, dir: &Path, settings: SyncSettings, passphrase: &str) -> Result<SyncEngine, SyncError> {
        let transport = Box::new(HttpTransport::new(&settings.server, &settings.token));
        Self::connect_with(vault, dir, settings, passphrase, transport, DEFAULT_KDF)
    }

    pub fn connect_with(
        vault: Arc<Vault>,
        dir: &Path,
        settings: SyncSettings,
        passphrase: &str,
        transport: Box<dyn Transport>,
        kdf: KdfParams,
    ) -> Result<SyncEngine, SyncError> {
        if !valid_id(&settings.vault_id) {
            return Err(SyncError::Local("the notebook name may only contain letters, digits, - and _ (up to 64 characters)".into()));
        }
        if passphrase.chars().count() < 8 {
            return Err(SyncError::Local("use a passphrase of at least 8 characters".into()));
        }
        let (key, head_seq) = match transport.get_vault(&settings.vault_id)? {
            Some(info) => (VaultKey::unwrap(&info.keys, passphrase)?, Some(info.head_seq)),
            None => {
                let key = VaultKey::generate();
                transport.create_vault(&settings.vault_id, &key.wrap(passphrase, kdf)?)?;
                (key, None)
            }
        };
        std::fs::create_dir_all(dir)?;
        // Not while another engine syncs from this folder.
        let _lock = lock_dir(dir)?;
        // A different vault on the server means a fresh start, and so does a
        // vault the server has lost, or lost changes of (see `check_server`).
        let same = read_config(dir).is_some_and(|o| o.server == settings.server && o.vault_id == settings.vault_id);
        let (mut state, mut state_hash, mut journal) = Self::read_state(dir);
        if !same || head_seq.is_none_or(|h| h < state.last_seq) {
            let _ = std::fs::remove_file(dir.join("state.json"));
            let _ = std::fs::remove_file(dir.join(JOURNAL));
            let _ = std::fs::remove_dir_all(dir.join("bases"));
            let _ = std::fs::remove_dir_all(dir.join("pending"));
            (state, state_hash, journal) = (SyncState::default(), None, Journal::default());
        }
        std::fs::create_dir_all(dir.join("bases"))?;
        write_json(&dir.join("config.json"), &settings)?;
        write_bytes(&dir.join("key"), b64(key.as_bytes()).as_bytes())?;
        Ok(SyncEngine {
            vault,
            transport,
            key,
            settings,
            dir: dir.to_path_buf(),
            state,
            state_hash,
            journal,
            pull_batch: PULL_BATCH,
            renamed_here: HashSet::new(),
            kept: HashMap::new(),
            deleting: HashSet::new(),
            linked_waits: HashSet::new(),
        })
    }

    /// Load a configuration saved by `connect`. `None` if sync is not set up.
    pub fn load(vault: Arc<Vault>, dir: &Path) -> Result<Option<SyncEngine>, SyncError> {
        let Ok(cfg) = std::fs::read(dir.join("config.json")) else { return Ok(None) };
        let settings: SyncSettings = serde_json::from_slice(&cfg).map_err(|e| SyncError::Local(e.to_string()))?;
        let transport = Box::new(HttpTransport::new(&settings.server, &settings.token));
        Self::load_with(vault, dir, settings, transport).map(Some)
    }

    pub fn load_with(vault: Arc<Vault>, dir: &Path, settings: SyncSettings, transport: Box<dyn Transport>) -> Result<SyncEngine, SyncError> {
        let raw = std::fs::read_to_string(dir.join("key")).map_err(|_| SyncError::NotConfigured)?;
        let bytes = unb64(raw.trim()).ok_or_else(|| SyncError::Local("damaged key file".into()))?;
        let key: [u8; 32] = bytes.try_into().map_err(|_| SyncError::Local("damaged key file".into()))?;
        let (state, state_hash, journal) = Self::read_state(dir);
        Ok(SyncEngine {
            vault,
            transport,
            key: VaultKey::from_bytes(key),
            settings,
            dir: dir.to_path_buf(),
            state,
            state_hash,
            journal,
            pull_batch: PULL_BATCH,
            renamed_here: HashSet::new(),
            kept: HashMap::new(),
            deleting: HashSet::new(),
            linked_waits: HashSet::new(),
        })
    }

    /// Forget the sync configuration for this vault (the notes stay).
    /// Everything but the `lock` file goes, the configuration first: a sync
    /// that another engine still runs from this folder keeps it locked, so
    /// a `connect` waits for that sync to end instead of locking a new file.
    pub fn disconnect(dir: &Path) -> Result<(), SyncError> {
        if !dir.exists() {
            return Ok(());
        }
        let _ = std::fs::remove_file(dir.join("config.json"));
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_name() == "lock" {
                continue;
            }
            let r = if entry.file_type()?.is_dir() { std::fs::remove_dir_all(entry.path()) } else { std::fs::remove_file(entry.path()) };
            // Gone already: that sync renamed it meanwhile.
            if let Err(e) = r
                && e.kind() != std::io::ErrorKind::NotFound
            {
                return Err(e.into());
            }
        }
        Ok(())
    }

    /// The saved state, the hash of `state.json` and what an engine needs
    /// to know of the journal (see `parse_state`); a fresh state if there
    /// is none yet, or if it cannot be read (the first sync then matches
    /// the vault with the server, see `relink`).
    fn read_state(dir: &Path) -> (SyncState, Option<Hash>, Journal) {
        let path = dir.join("state.json");
        let bytes = match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("sync: cannot read {}: {e}; starting from a fresh sync state", path.display());
                }
                None
            }
        };
        let journal = std::fs::read(dir.join(JOURNAL)).unwrap_or_default();
        Self::parse_state(dir, bytes.as_deref(), &journal)
    }

    /// The state in `bytes`, the content of `state.json` in `dir` (`None`:
    /// there is none), with the uploads that `journal`, the content of
    /// `state.journal`, records on top of it; the hash of `state.json`; and
    /// the journal as read.
    fn parse_state(dir: &Path, bytes: Option<&[u8]>, journal: &[u8]) -> (SyncState, Option<Hash>, Journal) {
        let hash = bytes.map(hash_bytes);
        let mut read = Journal::default();
        read.hash.update(journal);
        let mut state = match bytes.map(serde_json::from_slice::<SyncState>) {
            Some(Ok(state)) => state,
            None => return (SyncState::default(), hash, read),
            Some(Err(e)) => {
                log::warn!("sync: {} is damaged ({e}); starting from a fresh sync state", dir.join("state.json").display());
                return (SyncState::default(), hash, read);
            }
        };
        let mut lines = journal.split(|&b| b == b'\n');
        // A journal from before state.json was last written (the sync
        // stopped before removing it) has its uploads in there.
        if !journal.is_empty() && lines.next() != hash.map(|h| hash_hex(&h)).as_ref().map(|h| h.as_bytes()) {
            return (state, hash, read);
        }
        let mut whole = journal.is_empty() || journal.ends_with(b"\n");
        for line in lines.filter(|l| !l.is_empty()) {
            match serde_json::from_slice::<Recorded>(line) {
                Ok(r) => {
                    state.files.insert(r.fid, r.file);
                    state.last_seq = r.last_seq;
                }
                // Cut off: the app was killed while it was written.
                Err(e) => {
                    log::warn!("sync: ignoring the rest of {} ({e})", dir.join(JOURNAL).display());
                    whole = false;
                    break;
                }
            }
        }
        read.len = whole.then_some(journal.len() as u64);
        (state, hash, read)
    }

    /// Take up the state that another engine on this folder saved since
    /// this one last read or wrote it, so that its uploads are not made
    /// again (`sync` does this first, under the folder's lock). Use it
    /// before `history` too, which finds files in that state.
    ///
    /// `NotConfigured` when the folder's configuration is not this engine's
    /// any more: sync was turned off meanwhile, or connected again (to
    /// another vault, with another token or device name, or to this vault
    /// created again on a reset server, with a new key). That state is not
    /// this engine's to go on from; load the engine again.
    pub fn refresh(&mut self) -> Result<(), SyncError> {
        let key = std::fs::read_to_string(self.dir.join("key")).ok().and_then(|k| unb64(k.trim()));
        if read_config(&self.dir).as_ref() != Some(&self.settings) || key.as_deref() != Some(&self.key.as_bytes()[..]) {
            return Err(SyncError::NotConfigured);
        }
        let path = self.dir.join("state.json");
        let bytes = match std::fs::read(&path) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            // Unreadable now: keep this engine's state, the save replaces it.
            Err(e) => {
                log::warn!("sync: cannot read {}: {e}", path.display());
                return Ok(());
            }
        };
        let journal = std::fs::read(self.dir.join(JOURNAL)).unwrap_or_default();
        if bytes.as_deref().map(hash_bytes) != self.state_hash || blake3::hash(&journal) != self.journal.hash.finalize() {
            log::info!("sync: {} changed since this engine read or saved it; continuing from it", path.display());
            (self.state, self.state_hash, self.journal) = Self::parse_state(&self.dir, bytes.as_deref(), &journal);
        }
        Ok(())
    }

    fn save_state(&mut self) -> Result<(), SyncError> {
        // Sync was turned off for this folder while this one ran (from
        // another app instance): the state of this sync is not kept.
        if !self.dir.join("config.json").exists() {
            return Err(SyncError::NotConfigured);
        }
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(|e| SyncError::Local(e.to_string()))?;
        write_bytes(&self.dir.join("state.json"), &bytes)?;
        self.state_hash = Some(hash_bytes(&bytes));
        // Its uploads are in state.json now. One that stays is not read on
        // top of it any more, and the next record replaces it.
        if let Err(e) = std::fs::remove_file(self.dir.join(JOURNAL))
            && e.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!("sync: cannot remove {}: {e}", self.dir.join(JOURNAL).display());
        }
        self.journal = Journal { len: Some(0), hash: blake3::Hasher::new() };
        Ok(())
    }

    /// Save the state after an upload of `fid`, by adding the file's entry
    /// and the cursor to `state.journal` (see the module documentation):
    /// writing the whole state after each upload would take quadratic time.
    fn record(&mut self, fid: &str) -> Result<(), SyncError> {
        let (Some(len), Some(base), Some(file)) = (self.journal.len, self.state_hash, self.state.files.get(fid)) else {
            return self.save_state();
        };
        // As in `save_state`.
        if !self.dir.join("config.json").exists() {
            return Err(SyncError::NotConfigured);
        }
        let r = Recorded { fid: fid.to_string(), file: file.clone(), last_seq: self.state.last_seq };
        let mut line = serde_json::to_vec(&r).map_err(|e| SyncError::Local(e.to_string()))?;
        line.push(b'\n');
        let path = self.dir.join(JOURNAL);
        let written = if len == 0 {
            line.splice(0..0, format!("{}\n", hash_hex(&base)).into_bytes());
            self.journal.hash = blake3::Hasher::new();
            std::fs::write(&path, &line)
        } else {
            std::fs::OpenOptions::new().append(true).open(&path).and_then(|mut f| std::io::Write::write_all(&mut f, &line))
        };
        if let Err(e) = written {
            self.journal.len = None;
            return Err(e.into());
        }
        self.journal.hash.update(&line);
        self.journal.len = Some(len + line.len() as u64);
        Ok(())
    }

    pub fn settings(&self) -> &SyncSettings {
        &self.settings
    }

    pub fn state(&self) -> &SyncState {
        &self.state
    }

    /// Pull in batches of about `bytes` of records instead of [`PULL_BATCH`].
    pub fn set_pull_batch(&mut self, bytes: u64) {
        self.pull_batch = bytes;
    }

    fn base_path(&self, fid: &str) -> PathBuf {
        self.dir.join("bases").join(fid)
    }

    fn save_base(&self, fid: &str, path: &str, data: &[u8]) {
        if is_mergeable(path) {
            let _ = std::fs::write(self.base_path(fid), data);
        } else {
            let _ = std::fs::remove_file(self.base_path(fid));
        }
    }

    fn load_base(&self, fid: &str) -> Option<String> {
        std::fs::read_to_string(self.base_path(fid)).ok()
    }

    /// Where the record of a pending change is kept (`None` for a file id
    /// that is not safe as a file name).
    fn held_path(&self, fid: &str) -> Option<PathBuf> {
        valid_id(fid).then(|| self.dir.join("pending").join(fid))
    }

    /// Keep a remote change that cannot be applied yet, with its record, so
    /// that later syncs retry it without downloading it again.
    fn hold(&mut self, head: &RemoteHead) {
        if let Some(p) = self.held_path(&head.file_id) {
            let saved = self.state.pending.get(&head.file_id) == Some(&head.seq) && p.exists();
            if !saved {
                let _ = std::fs::create_dir_all(self.dir.join("pending"));
                let _ = write_json(&p, head);
            }
        }
        self.state.pending.insert(head.file_id.clone(), head.seq);
    }

    fn unhold(&mut self, fid: &str) {
        if self.state.pending.remove(fid).is_some()
            && let Some(p) = self.held_path(fid)
        {
            let _ = std::fs::remove_file(p);
        }
    }

    /// The saved record of a pending change, if there is one.
    fn held(&self, fid: &str, seq: u64) -> Option<RemoteHead> {
        let head: RemoteHead = serde_json::from_slice(&std::fs::read(self.held_path(fid)?).ok()?).ok()?;
        (head.file_id == fid && head.seq == seq).then_some(head)
    }

    // ---------- local side ----------

    /// Every file in the vault with its content hash. A tracked file's hash
    /// is reused while the file looks as it did when a scan found it with
    /// that content (see [`Seen`]). Files that cannot be read are returned
    /// separately (and reported). A vault with no files at all, while this
    /// device has synced some, is an error.
    fn scan(&mut self, report: &mut SyncReport) -> Result<(HashMap<String, LocalFile>, HashSet<String>), SyncError> {
        // Taken before anything is listed: a file saved after this has a
        // newer mtime than one modified MTIME_SLACK before it.
        let now = now_ms();
        // Only a vault folder that cannot be listed fails the rescan. One
        // that is gone (moved, renamed, unmounted) is taken as below for one
        // that lists as empty.
        let rescanned = match self.vault.rescan() {
            Err(CoreError::NotFound(_)) if self.state.files.values().any(|t| !t.deleted) => return Err(SyncError::VaultEmpty),
            r => r?,
        };
        report.changes.extend(rescanned);
        // The journal of renames stays locked until the vault is listed: a
        // rename that the listing has is in the state too.
        let entries = {
            let _renames = self.take_renames()?;
            self.vault.entries()
        };
        // Left out by the listing (FINDING-011): the user
        // renames them to sync them.
        for name in self.vault.skipped_backslash_names() {
            report.skip(&name, cairn_core::vault::BACKSLASH_REASON.into());
        }
        let fs = self.vault.fs().clone();
        let known: HashMap<&str, (&String, &Tracked)> =
            self.state.files.iter().filter(|(_, t)| !t.deleted).map(|(f, t)| (t.path.as_str(), (f, t))).collect();
        let mut out = HashMap::new();
        let mut unreadable = HashSet::new();
        let mut seen = Vec::new();
        for e in entries {
            if e.kind != EntryKind::File {
                continue;
            }
            let tracked = known.get(e.path.as_str()).copied();
            // The listing is from before the read below.
            let before = tracked.map(|_| Seen { size: e.size, mtime: e.mtime, stamp: fs.change_stamp(&e.path) });
            if let Some((_, t)) = tracked
                && t.seen == before
                && !t.hash.is_empty()
            {
                out.insert(e.path.clone(), LocalFile { hash: t.hash.clone() });
                continue;
            }
            let hash = match self.vault.read_file(&e.path) {
                Ok(b) => hex_hash(&b),
                Err(CoreError::NotFound(_)) => continue,
                Err(err) => {
                    log::warn!("sync: cannot read {}: {err}", e.path);
                    report.skip(&e.path, reason(&err.into()));
                    unreadable.insert(e.path.clone());
                    continue;
                }
            };
            // A file that still has the tracked content is cached, if it
            // looks the same after the read as before it.
            if let (Some((fid, t)), Some(before)) = (tracked, before)
                && hash == t.hash
                && e.mtime < now - MTIME_SLACK
                && fs.stat(&e.path).ok().flatten().is_some_and(|s| (s.size, s.mtime) == (before.size, before.mtime))
                && fs.change_stamp(&e.path) == before.stamp
            {
                seen.push((fid.clone(), before));
            }
            out.insert(e.path.clone(), LocalFile { hash });
        }
        for (fid, s) in seen {
            if let Some(t) = self.state.files.get_mut(&fid) {
                t.seen = Some(s);
            }
        }
        // A vault folder that was moved, renamed or unmounted lists as empty
        // (shared storage on Android does not say it is gone). Taken at its
        // word, the sync would delete every note on every device. A vault
        // whose files are all in folders it cannot read is not empty.
        if out.is_empty() && unreadable.is_empty() && self.vault.unreadable_folders().is_empty() && self.state.files.values().any(|t| !t.deleted) {
            return Err(SyncError::VaultEmpty);
        }
        Ok((out, unreadable))
    }

    /// Take the renames that [`rename`] recorded into the state, and return
    /// their journal, emptied and still locked. Each tracked file goes under
    /// its new name, and the rename is pushed with the file's next change,
    /// unless it cannot go up as a rename (see `settle_renames`).
    /// A journal that cannot be read or emptied is left for the next scan;
    /// the scan finds those renames by content meanwhile.
    fn take_renames(&mut self) -> Result<Option<std::fs::File>, SyncError> {
        let path = self.dir.join("renames");
        let journal = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path);
        let mut file = match journal {
            Ok(f) => f,
            Err(e) => {
                log::warn!("sync: cannot open {}: {e}", path.display());
                return Ok(None);
            }
        };
        lock_file(&file, &path);
        let mut bytes = Vec::new();
        // Emptied before the state is saved: a sync that stops in between
        // does not apply these renames again, to files that may have taken
        // the names meanwhile.
        if let Err(e) = std::io::Read::read_to_end(&mut file, &mut bytes).and_then(|_| file.set_len(0)) {
            log::warn!("sync: cannot read {}: {e}", path.display());
            return Ok(Some(file));
        }
        for line in String::from_utf8_lossy(&bytes).lines() {
            match serde_json::from_str::<Moved>(line) {
                Ok(m) => self.moved(&m),
                Err(e) => log::warn!("sync: skipping a damaged line of {}: {e}", path.display()),
            }
        }
        if self.settle_renames() || !bytes.is_empty() {
            self.save_state()?;
        }
        Ok(Some(file))
    }

    /// Move the tracked files that a rename in the app moved. A sync that
    /// ran meanwhile may have written another tracked file under the old
    /// name, after the rename: the one whose content is there now stayed.
    /// When that leaves not exactly one file, the rename is left to the scan.
    fn moved(&mut self, m: &Moved) {
        let mut at: HashMap<String, Vec<String>> = HashMap::new();
        for (fid, t) in self.state.files.iter().filter(|(_, t)| !t.deleted) {
            if if m.dir { vpath::is_inside(&t.path, &m.from) } else { t.path == m.from } {
                at.entry(t.path.clone()).or_default().push(fid.clone());
            }
        }
        for (path, mut fids) in at {
            if self.exists(&path) {
                let here = self.vault.read_file(&path).map(|d| hex_hash(&d)).unwrap_or_default();
                fids.retain(|f| self.state.files[f].hash != here);
            }
            let [fid] = fids.as_slice() else {
                log::info!("sync: cannot tell which file was renamed from {path}; the scan finds it");
                continue;
            };
            let to = vpath::rebase(&path, &m.from, &m.to);
            let Some(t) = self.state.files.get_mut(fid) else { continue };
            let server = t.server_path.take().unwrap_or(path);
            t.renamed = server != to;
            t.server_path = t.renamed.then_some(server);
            t.path = to;
            t.seen = None;
        }
    }

    /// Give up the renames made in the app that cannot go up as renames, in
    /// the state; the scan then finds those files by content, as if the
    /// renames had not been recorded:
    /// * the file is under its server name again (renamed back outside the
    ///   app), and nothing is under the name it was given: an edit of it is
    ///   an edit, not a delete plus a new file;
    /// * files that took each other's server names in a ring (a swap through
    ///   a temporary name): in any order of uploads, a device that syncs
    ///   between them (or runs a client without this) finds the first one's
    ///   name taken, so each goes up as an edit of the file that had its
    ///   name, and the names come out right.
    ///
    /// Returns whether it changed anything.
    fn settle_renames(&mut self) -> bool {
        let renamed: Vec<(String, String, String)> = self
            .state
            .files
            .iter()
            .filter(|(_, t)| t.renamed && !t.deleted)
            .filter_map(|(f, t)| Some((f.clone(), t.path.clone(), t.server_path.clone()?)))
            .collect();
        let mut undo = Vec::new();
        for (fid, path, server) in &renamed {
            if !self.exists(path)
                && self.exists(server)
                && !self.state.files.iter().any(|(f, t)| f != fid && !t.deleted && t.path == *server)
            {
                undo.push(fid.clone());
            }
        }
        // The file that has a renamed file's name on the server must go
        // first; following that from each file finds the rings.
        let by_server: HashMap<&str, &str> = renamed.iter().map(|(f, _, s)| (s.as_str(), f.as_str())).collect();
        let next: HashMap<&str, &str> = renamed.iter().filter_map(|(f, p, _)| Some((f.as_str(), *by_server.get(p.as_str())?))).collect();
        let mut done: HashSet<&str> = HashSet::new();
        for (start, _, _) in &renamed {
            let mut walk: Vec<&str> = Vec::new();
            let mut at = Some(start.as_str());
            while let Some(f) = at.filter(|f| !done.contains(f)) {
                if let Some(i) = walk.iter().position(|w| *w == f) {
                    undo.extend(walk[i..].iter().map(|w| w.to_string()));
                    break;
                }
                walk.push(f);
                at = next.get(f).copied();
            }
            done.extend(walk);
        }
        let mut changed = false;
        for fid in undo {
            if let Some(t) = self.state.files.get_mut(&fid)
                && let Some(server) = t.server_path.take()
            {
                log::info!("sync: {} goes up as a change of {server}, not as a rename", t.path);
                t.path = server;
                t.renamed = false;
                t.seen = None;
                changed = true;
            }
        }
        changed
    }

    /// What happened locally to each tracked file since the last sync, plus
    /// the paths of new, untracked files.
    fn classify(&self, local: &HashMap<String, LocalFile>, unreadable: &HashSet<String>) -> (HashMap<String, Status>, Vec<String>) {
        let mut status = HashMap::new();
        // The tracked files at each path: one, unless a pull stopped between
        // two batches (see below).
        let mut owners: HashMap<&str, Vec<(&String, &Tracked)>> = HashMap::new();
        for (fid, t) in self.state.files.iter().filter(|(_, t)| !t.deleted) {
            owners.entry(t.path.as_str()).or_default().push((fid, t));
        }
        let mut created: Vec<String> = local.keys().filter(|p| !owners.contains_key(p.as_str())).cloned().collect();
        created.sort();
        let mut missing = Vec::new();
        let unreadable_dirs = self.vault.unreadable_folders();
        for (fid, t) in &self.state.files {
            if t.deleted {
                continue;
            }
            if unreadable.contains(&t.path) {
                status.insert(fid.clone(), Status::Unreadable);
                continue;
            }
            match local.get(&t.path) {
                // Another tracked file, which a pull wrote here after this
                // one was moved or deleted here: both have this path until
                // a later batch of that pull brings this one's next change,
                // and the pull stopped before it; or the user deleted a file
                // and renamed another one to its name in the app. The file
                // here is the one renamed in the app, or else the one with
                // its content (the newest, if both have it), or else the one
                // the pull wrote, edited since: the newest.
                Some(lf)
                    if owners[t.path.as_str()]
                        .iter()
                        .max_by_key(|(_, o)| (o.renamed, o.hash == lf.hash, o.seq))
                        .is_some_and(|(f, _)| *f != fid) =>
                {
                    missing.push(fid.clone())
                }
                Some(lf) if lf.hash == t.hash => {
                    status.insert(fid.clone(), Status::Unchanged);
                }
                Some(_) => {
                    status.insert(fid.clone(), Status::Modified);
                }
                // In a folder the scan could not read, or a link it could not
                // follow: unknown, not deleted.
                None if unreadable_dirs.iter().any(|d| vpath::is_same_or_inside(&t.path, d)) => {
                    status.insert(fid.clone(), Status::Unchanged);
                }
                None => missing.push(fid.clone()),
            }
        }
        let mut gone = Vec::new();
        for fid in missing {
            let t = &self.state.files[&fid];
            // A missing file that the file system still finds is there under
            // a name that differs only in case. If that name is not tracked,
            // the file was renamed (and maybe edited); otherwise two tracked
            // files are one file here. Not on Android shared storage, which
            // cannot hold both names: there the other tracked file was stored
            // under its name after this one went, so this one was deleted or
            // renamed here, as when the phone deletes the one of two case
            // twins that it stores (FINDING-172).
            if let Some(twin) = self.same_file(&t.path) {
                match created.iter().position(|p| *p == twin) {
                    Some(i) => {
                        status.insert(fid, Status::Renamed(created.remove(i)));
                    }
                    None if self.vault.fs().refuses_case_twins() && owners.contains_key(twin.as_str()) => gone.push(fid),
                    None => {
                        status.insert(fid, Status::Shared);
                    }
                }
                continue;
            }
            gone.push(fid);
        }
        // A missing file whose exact content appeared elsewhere was renamed,
        // paired as the vault's scan pairs them (`pair_renames`): when more
        // files have that content, a pair needs the same name or folder. An
        // empty file says nothing about where it went. The rest were deleted.
        let empty = hex_hash(b"");
        // Nor does a duplicate (see `Tracked::alias_of`): a new file is
        // never taken for one, and `regroup` finds where it went.
        let dels: Vec<(&String, &Tracked)> =
            gone.iter().map(|f| (f, &self.state.files[f])).filter(|(_, t)| t.hash != empty && t.alias_of.is_none()).collect();
        let del_keys: Vec<(&str, &str)> = dels.iter().map(|(_, t)| (t.path.as_str(), t.hash.as_str())).collect();
        let cres: Vec<(&str, &str)> = created.iter().map(|p| (p.as_str(), local[p].hash.as_str())).filter(|(_, h)| *h != empty).collect();
        let mut renamed_to = HashSet::new();
        for (d, c) in cairn_core::vault::pair_renames(&del_keys, &cres) {
            renamed_to.insert(cres[c].0.to_string());
            status.insert(dels[d].0.clone(), Status::Renamed(cres[c].0.to_string()));
        }
        created.retain(|p| !renamed_to.contains(p));
        for fid in gone {
            status.entry(fid).or_insert(Status::Deleted);
        }
        // The tracked file that has the disk name may hold the other one's
        // content; if it changed, it is downloaded again too.
        let shared: HashSet<String> =
            status.iter().filter(|(_, s)| **s == Status::Shared).map(|(f, _)| self.state.files[f].path.to_lowercase()).collect();
        for (fid, s) in status.iter_mut() {
            if *s == Status::Modified && shared.contains(&self.state.files[fid].path.to_lowercase()) {
                *s = Status::Shared;
            }
        }
        (status, created)
    }

    /// Sync one name of each file that the vault reaches under several (see
    /// `VaultFs::other_names`: a folder linked in twice, a symlink to a
    /// note), so that other devices get one copy of it (FINDING-224). The
    /// other names stay in the vault but leave `created`, so they are not
    /// uploaded, and go into `self.kept` with the name that syncs. That is
    /// the name of the tracked file there, so it stays while that file is
    /// there; otherwise the name with no link on the way, then the one with
    /// the fewest folders, then the first in byte order. A name in a folder
    /// that cannot be read now counts as there, as the listing before
    /// found it.
    ///
    /// Older versions uploaded every name. Of several tracked files at one
    /// file's names, the one at the name chosen so syncs on, and the others
    /// become its duplicates (`Tracked::alias_of`); other devices keep their
    /// copies. A duplicate is tracked where it is now, a name of the file
    /// it duplicates, so that it can sync on in its place (see `heir`). If
    /// only duplicates are left there (the file they duplicate went), one
    /// of them syncs on. A duplicate that is a file of its own here (its
    /// link replaced by a copy) syncs as one again. One that is gone here,
    /// as the note it duplicates is: if another device changed it
    /// meanwhile, it syncs again and the change applies (edit beats
    /// delete); if the note was deleted here, its delete goes up with the
    /// note's; if another device deleted the note, it is downloaded again
    /// as a file of its own, as the other devices have it.
    ///
    /// A new file with the content of a tracked file in a folder that
    /// cannot be read now may be another name of it, which only the listing
    /// that could read the folder knows: it is not uploaded meanwhile, and
    /// is listed. Also notes where tracked files were renamed
    /// (`self.renamed_here`). Returns whether the state changed.
    fn regroup(&mut self, local: &HashMap<String, LocalFile>, status: &mut HashMap<String, Status>, created: &mut HashSet<String>, report: &mut SyncReport) -> bool {
        let fs = self.vault.fs().clone();
        let unreadable = self.vault.unreadable_folders();
        let unknown = |p: &str| unreadable.iter().any(|d| vpath::is_same_or_inside(p, d));
        let there = |p: &str| local.contains_key(p) || unknown(p);
        // Where each live tracked file is now.
        let mut at: HashMap<String, String> = HashMap::new();
        self.renamed_here.clear();
        for (fid, t) in self.state.files.iter().filter(|(_, t)| !t.deleted) {
            let here = match status.get(fid) {
                Some(Status::Renamed(to)) => {
                    if t.alias_of.is_none() {
                        self.renamed_here.insert(to.clone());
                    }
                    to
                }
                Some(Status::Deleted) => continue,
                _ => &t.path,
            };
            if there(here) {
                at.insert(here.clone(), fid.clone());
            }
        }
        let mut paths: Vec<&String> = local.keys().collect();
        paths.sort();
        let mut grouped: HashSet<String> = HashSet::new();
        let mut groups: Vec<Vec<String>> = Vec::new();
        for p in paths {
            if grouped.contains(p) {
                continue;
            }
            let mut names: Vec<String> = fs.other_names(p).into_iter().filter(|o| there(o)).collect();
            if names.is_empty() {
                continue;
            }
            names.push(p.clone());
            names.sort();
            names.dedup();
            grouped.extend(names.iter().cloned());
            groups.push(names);
        }
        let best = |names: Vec<&String>| names.into_iter().min_by_key(|p| (fs.via_link(p), p.matches('/').count(), p.as_str())).cloned();
        let mut changed = false;
        let mut in_group: HashSet<String> = HashSet::new();
        self.kept.clear();
        for names in groups {
            let ids: Vec<(&String, String)> = names.iter().filter_map(|p| Some((p, at.get(p)?.clone()))).collect();
            let synced: Vec<&String> = ids.iter().filter(|(_, f)| self.state.files[f].alias_of.is_none()).map(|(p, _)| *p).collect();
            let dups: Vec<&String> = ids.iter().filter(|(_, f)| self.state.files[f].alias_of.is_some()).map(|(p, _)| *p).collect();
            let kept = match synced.as_slice() {
                [one] => Some((*one).clone()),
                [] if dups.is_empty() => best(names.iter().filter(|p| local.contains_key(*p)).collect()),
                [] => best(dups),
                _ => best(synced),
            };
            let Some(kept) = kept else { continue };
            let kept_fid = at.get(&kept).cloned();
            let mut placed: Vec<(String, String)> = ids.iter().map(|(p, f)| ((*p).clone(), f.clone())).collect();
            // Duplicates of this file that are no longer at the name they
            // had (their link renamed or moved; `classify` never takes a
            // duplicate for renamed): at a name of the file that no tracked
            // file has, the same file name first.
            if let Some(k) = &kept_fid {
                let mut free: Vec<&String> = names.iter().filter(|p| **p != kept && !at.contains_key(*p) && local.contains_key(*p)).collect();
                let lost: Vec<String> = self
                    .state
                    .files
                    .iter()
                    .filter(|(f, t)| !t.deleted && t.alias_of.as_ref() == Some(k) && !ids.iter().any(|(_, g)| g == *f))
                    .filter(|(f, t)| matches!(status.get(*f), Some(Status::Deleted)) && !unknown(&t.path))
                    .map(|(f, _)| f.clone())
                    .collect();
                for fid in lost {
                    let name = vpath::file_name(&self.state.files[&fid].path).to_string();
                    let Some(i) = free.iter().position(|p| vpath::file_name(p) == name).or((!free.is_empty()).then_some(0)) else { break };
                    placed.push((free.remove(i).clone(), fid));
                }
            }
            for (name, fid) in &placed {
                in_group.insert(fid.clone());
                let want = kept_fid.clone().filter(|k| k != fid);
                let t = self.state.files.get_mut(fid).expect("tracked");
                if t.alias_of != want {
                    log::info!("sync: {} syncs as {kept} only, the same file here", t.path);
                    t.alias_of = want;
                    changed = true;
                }
                // A duplicate is tracked where it is, with the server's name
                // kept for a rename should it sync on (nothing is sent
                // for it before), and has the status of that name.
                if t.alias_of.is_some() && t.path != *name {
                    let server = t.server_path.take().unwrap_or_else(|| t.path.clone());
                    t.server_path = (server != *name).then_some(server);
                    t.path = name.clone();
                    t.seen = None;
                    let same = local.get(name).is_some_and(|lf| lf.hash == t.hash);
                    status.insert(fid.clone(), if same { Status::Unchanged } else { Status::Modified });
                    changed = true;
                }
                if t.detached {
                    t.detached = false;
                    changed = true;
                }
            }
            for p in names.into_iter().filter(|p| *p != kept) {
                created.remove(&p);
                self.kept.insert(p, kept.clone());
            }
        }
        // Duplicates outside every group.
        let lone: Vec<String> =
            self.state.files.iter().filter(|(f, t)| !t.deleted && t.alias_of.is_some() && !in_group.contains(*f)).map(|(f, _)| f.clone()).collect();
        for fid in lone {
            let t = &self.state.files[&fid];
            let here = matches!(status.get(&fid), Some(Status::Unchanged | Status::Modified)) && local.contains_key(&t.path);
            let original = t.alias_of.as_ref().and_then(|o| self.state.files.get(o)).is_some_and(|o| !o.deleted);
            if here {
                log::info!("sync: {} is a file of its own here now; it syncs again", t.path);
                self.state.files.get_mut(&fid).expect("tracked").alias_of = None;
                changed = true;
            } else if original || unknown(&t.path) {
                // Still a name of a file that syncs, or may be. One gone here
                // while that file stays is a copy no link here makes now.
                let note_here = t.alias_of.as_ref().is_some_and(|o| status.get(o) != Some(&Status::Deleted));
                if original && !unknown(&t.path) && status.get(&fid) == Some(&Status::Deleted) && note_here && !t.detached {
                    self.state.files.get_mut(&fid).expect("tracked").detached = true;
                    changed = true;
                }
            } else if self.state.pending.contains_key(&fid) {
                // The note it duplicates is gone, and another device changed
                // this copy meanwhile: that change is applied as to any file
                // deleted here (edit beats delete).
                self.state.files.get_mut(&fid).expect("tracked").alias_of = None;
                changed = true;
            } else if !t.delete_with_note || t.detached {
                // Another device deleted the note, and this name is gone
                // here, or the note was deleted here after this copy was
                // detached from it: the copy that the other devices keep is
                // downloaded again, as a file of its own.
                log::info!("sync: {} is a copy that other devices keep of a note gone here; downloading it", t.path);
                let seq = t.seq;
                self.state.files.remove(&fid);
                let _ = std::fs::remove_file(self.base_path(&fid));
                self.state.pending.insert(fid, seq);
                changed = true;
            }
            // Otherwise the note was deleted here: the push deletes this
            // copy too.
        }
        // New files that may be another name of a tracked file that cannot
        // be read now.
        if !unreadable.is_empty() {
            let empty = hex_hash(b"");
            let mut waiting: Vec<(String, String)> = Vec::new();
            for p in created.iter() {
                let hash = &local[p].hash;
                let other = self.state.files.values().find(|t| !t.deleted && t.hash == *hash && *hash != empty && unknown(&t.path));
                if let Some(t) = other {
                    waiting.push((p.clone(), t.path.clone()));
                }
            }
            for (p, other) in waiting {
                created.remove(&p);
                report.skip(&p, format!("this may be another name of {other}, which is in a folder this device cannot read now, so it is not uploaded. It syncs when that folder can be read again"));
            }
        }
        changed
    }

    /// The duplicate of file `fid` that syncs on when another device deletes
    /// `fid`, unchanged here (see `regroup`): one at a name of the same file
    /// here, the one `regroup` would choose. Not one whose own delete comes
    /// in the same batch of the pull: then the note was deleted under both
    /// names. (When that delete comes later, it applies through
    /// `Tracked::took_over`.) When the only duplicates are in a folder that
    /// cannot be read now, they may still be names of the file, and the
    /// delete waits; so it does while another device's file at another name
    /// of this one waits to sync here.
    fn heir(&self, fid: &str, path: &str, local: &HashMap<String, LocalFile>) -> Result<Option<String>, SyncError> {
        let unreadable = self.vault.unreadable_folders();
        let fs = self.vault.fs();
        let dups: Vec<(&String, &Tracked)> =
            self.state.files.iter().filter(|(f, t)| !t.deleted && t.alias_of.as_deref() == Some(fid) && !self.deleting.contains(*f)).collect();
        let heir = dups
            .iter()
            .filter(|(_, t)| local.contains_key(&t.path))
            .min_by_key(|(_, t)| (fs.via_link(&t.path), t.path.matches('/').count(), t.path.clone()))
            .map(|(f, _)| (*f).clone());
        if heir.is_some() {
            return Ok(heir);
        }
        let maybe: Vec<&str> = dups.iter().filter(|(_, t)| unreadable.iter().any(|d| vpath::is_same_or_inside(&t.path, d))).map(|(_, t)| t.path.as_str()).collect();
        if !maybe.is_empty() {
            return Err(SyncError::Local(format!(
                "another device deleted this file, but {path} may be one file with {} on this device, which is in a folder it cannot read now, so the delete is not applied here. To sync it, make that folder readable again",
                maybe.join(", ")
            )));
        }
        // A copy that another device keeps under another name of the file
        // here, which waits to sync (see `apply_remote`): deleting the file
        // would delete that name too.
        let mut names = fs.other_names(path);
        names.extend(self.kept.iter().filter(|(_, k)| *k == path).map(|(n, _)| n.clone()));
        if !names.is_empty() {
            for (f, seq) in &self.state.pending {
                if self.state.files.get(f).is_some_and(|t| !t.deleted) {
                    continue;
                }
                let Some((held, _)) = self.held(f, *seq).and_then(|h| self.open(&h).ok()) else { continue };
                if names.contains(&held) {
                    return Err(SyncError::Local(format!(
                        "another device deleted this file, but a different file waits to sync at {held}, which on this device is another name of {path}, so the delete is not applied here: it would delete {held} too. To sync it, rename or delete {held} on the device that has it"
                    )));
                }
            }
        }
        Ok(None)
    }

    /// Why a remote change of duplicate `t` (see `Tracked::alias_of`) is
    /// not applied here; `rpath` is its name there.
    fn duplicate_reason(&self, t: &Tracked, rpath: &str) -> String {
        let kept = t.alias_of.as_ref().and_then(|o| self.state.files.get(o)).filter(|o| !o.deleted).map(|o| o.path.as_str());
        let copy = match kept {
            Some(k) if self.exists(&t.path) => format!("an older copy of {k}: on this device both names are one file, through a symlink, which syncs as {k} only"),
            Some(k) => format!("an older copy of {k}, which this device does not keep (it syncs {k} only)"),
            None => "an older copy of a note that this device syncs under another name".into(),
        };
        let keep = match kept {
            Some(k) => format!("copy what you need from it into {k}, then delete this copy on a device where it is a separate file, not another name of {k} through a link"),
            None => "copy what you need from it into the note, then delete this copy on a device where it is a separate file".into(),
        };
        if rpath != t.server_path.as_deref().unwrap_or(&t.path) {
            format!("another device renamed this file ({} here), {copy}. The rename is not applied here. To sync it, {keep}", t.path)
        } else {
            format!("another device changed this file, {copy}. The change is not applied here. To keep it, {keep}")
        }
    }

    fn exists(&self, path: &str) -> bool {
        self.vault.index().entry(path).is_some_and(|e| e.kind == EntryKind::File)
    }

    /// A file here other than `own` whose name differs from `path` only in
    /// case, if the file system finds a file under `path` too, as one that
    /// ignores case does (macOS, Windows, Android shared storage). The
    /// variants are looked up in the index first, so the file system is
    /// asked only when there are some.
    fn case_twin(&self, path: &str, own: Option<&str>) -> Option<String> {
        let twin = self.vault.index().case_variants(path).into_iter().find(|p| Some(p.as_str()) != own)?;
        matches!(self.vault.fs().stat(path), Ok(Some(s)) if s.kind == EntryKind::File).then_some(twin)
    }

    /// Like `case_twin`, but the file system must show the same size and
    /// mtime under both names: a file that a case-sensitive file system has
    /// at `path` again, after the scan missed it, is not taken for the twin.
    fn same_file(&self, path: &str) -> Option<String> {
        let variants = self.vault.index().case_variants(path);
        if variants.is_empty() {
            return None;
        }
        let fs = self.vault.fs();
        let s = fs.stat(path).ok().flatten().filter(|s| s.kind == EntryKind::File)?;
        variants.into_iter().find(|v| matches!(fs.stat(v), Ok(Some(o)) if (o.size, o.mtime) == (s.size, s.mtime)))
    }

    /// Whether writing or moving a remote file to `path` would replace
    /// another local file. `own` is where that file is here, if anywhere:
    /// it may take a name that differs from it only in case.
    fn taken(&self, path: &str, own: Option<&str>) -> bool {
        self.exists(path) || self.case_twin(path, own).is_some()
    }

    /// On a file system that refuses case twins (Android shared storage), a
    /// remote file that `path` would make a case twin of another file here
    /// (other than `own`) is not stored under a conflict copy name, which
    /// would rename it on every device: the change stays pending and is
    /// listed (FINDING-172). `own` is where the file is here,
    /// for a remote rename of it: it keeps that name, and its changes wait
    /// too, so the reason names it.
    fn refuse_case_twin(&self, path: &str, own: Option<&str>) -> Result<(), SyncError> {
        if !self.vault.fs().refuses_case_twins() || self.exists(path) || self.case_twin(path, own).is_none() {
            return Ok(());
        }
        Err(SyncError::Local(match own {
            Some(own) => format!(
                "another device renamed this note ({own} here) to this name, but this storage cannot hold that name next to another file here: the two names differ only in case. Until one of them is renamed or deleted, {own} keeps its name and its changes do not sync"
            ),
            None => "another file on this device has the same name in a different case, and this storage cannot hold two names that differ only in case".into(),
        }))
    }

    fn conflict_path(&self, path: &str) -> Result<String, SyncError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        let (base, ext) = conflict_name(path, &stamp(now), &self.settings.device);
        Ok(self.vault.unique_path(vpath::parent(path), &base, &ext)?)
    }

    /// Write a file. The scan is from before the pull, so nothing the user
    /// saved since is overwritten: with `expect`, only that content is
    /// replaced, and without, the file is only created where there is none.
    fn write(&self, path: &str, data: &[u8], expect: Option<&str>, report: &mut SyncReport) -> Result<(), SyncError> {
        let r = match expect {
            Some(_) => {
                self.refuse_alias(path, Act::Write)?;
                self.vault.write_file(path, data, expect)?
            }
            None => self.vault.write_new_file(path, data)?,
        };
        report.changes.extend(r.changes);
        Ok(())
    }

    /// Write a remote file to `path`, which was free when last looked at. A
    /// file put there since is kept: if it has the same content
    /// (and is not another tracked file), it is taken for the remote one;
    /// otherwise the remote file gets a conflict copy name. Returns where
    /// the file is.
    fn place(&self, fid: &str, path: &str, data: &[u8], created: &HashSet<String>, report: &mut SyncReport) -> Result<String, SyncError> {
        match self.vault.write_new_file(path, data) {
            Ok(r) => {
                report.changes.extend(r.changes);
                return Ok(path.to_string());
            }
            Err(CoreError::AlreadyExists(_)) => {}
            Err(e) => return Err(e.into()),
        }
        // A folder there cannot be read: the change stays pending.
        let ours = self.vault.read_file(path)?;
        let other = self.state.files.iter().any(|(f, t)| f != fid && !t.deleted && t.path == path);
        if ours == data && !other {
            return Ok(path.to_string());
        }
        self.conflict_copy(path, data, created, report)
    }

    /// Write a remote file under a conflict copy name of `path`. A copy that
    /// is there already, new since the last sync and with that content, is
    /// taken instead: a sync that stopped right after writing it could not
    /// record it.
    fn conflict_copy(&self, path: &str, data: &[u8], created: &HashSet<String>, report: &mut SyncReport) -> Result<String, SyncError> {
        let c = match self.own_copy(path, data, created) {
            Some(c) => c,
            None => {
                let c = self.conflict_path(path)?;
                self.write(&c, data, None, report)?;
                c
            }
        };
        report.conflicts.push(c.clone());
        Ok(c)
    }

    /// A file here with a conflict copy name of `path` and the content
    /// `data`. Only a new file of the scan (`created`) that no other file
    /// took in this pull yet will do: a new file that the scan took for a
    /// tracked file renamed here gets that file's remote change written into
    /// it, and so does one taken earlier in the pull.
    fn own_copy(&self, path: &str, data: &[u8], created: &HashSet<String>) -> Option<String> {
        let at = stamp(0);
        let (base, ext) = conflict_name(path, &at, &self.settings.device);
        let prefix = &base[..base.rfind(&format!(" (conflict {at}"))?];
        let (prefix, dot_ext) = (format!("{prefix} (conflict "), if ext.is_empty() { ext } else { format!(".{ext}") });
        let mut names: Vec<&String> = created
            .iter()
            .filter(|p| vpath::parent(p) == vpath::parent(path))
            .filter(|p| vpath::file_name(p).starts_with(&prefix) && p.ends_with(&dot_ext))
            .collect();
        names.sort();
        names
            .into_iter()
            .find(|p| !self.state.files.values().any(|t| !t.deleted && t.path == **p) && self.vault.read_file(p).is_ok_and(|d| d == data))
            .cloned()
    }

    /// Two devices that had not seen each other's file each uploaded one at
    /// `path`, and the pull brings the other one (`other`). As PLAN says, the
    /// file uploaded first gets a conflict copy name and the other keeps the
    /// name, and every device must choose the same one: if the file here was
    /// uploaded first, it moves to a conflict copy name, and the rename is
    /// pushed. Only a file that is here as the server has it, maybe edited,
    /// moves, and not one with a pending remote change. Returns whether it
    /// moved.
    fn make_way(
        &mut self,
        path: &str,
        other: &str,
        local: &mut HashMap<String, LocalFile>,
        status: &HashMap<String, Status>,
        report: &mut SyncReport,
    ) -> Result<bool, SyncError> {
        let Some(fid) = self.movable(path, status) else { return Ok(false) };
        if self.first_seq(&fid)? > self.first_seq(other)? {
            return Ok(false);
        }
        let copy = self.set_aside(&fid, path, local, report)?;
        report.conflicts.push(copy);
        Ok(true)
    }

    /// The tracked file at `path`, if it can move aside: it is here as the
    /// server has it, maybe edited, has no pending remote change, and is
    /// not synced under another name too (see `refuse_alias`).
    fn movable(&self, path: &str, status: &HashMap<String, Status>) -> Option<String> {
        let (fid, t) = self.state.files.iter().find(|(_, t)| !t.deleted && t.path == path)?;
        let here = matches!(status.get(fid).unwrap_or(&Status::Unchanged), Status::Unchanged | Status::Modified);
        let free = t.server_path.is_none() && !self.state.pending.contains_key(fid) && self.refuse_alias(path, Act::Move).is_ok();
        (here && free).then(|| fid.clone())
    }

    /// The other vault paths of the file at `path` here that are where a
    /// tracked file that is not a duplicate is now: one file on disk under
    /// two names (see `VaultFs::other_names`). A tracked file that the scan
    /// found renamed here counts under its new name.
    fn tracked_aliases(&self, path: &str) -> Vec<String> {
        let mut others = self.vault.fs().other_names(path);
        others.retain(|p| self.renamed_here.contains(p) || self.state.files.values().any(|t| !t.deleted && t.alias_of.is_none() && t.path == *p));
        others
    }

    /// The name of a tracked file that `path`, reached through a link here,
    /// is another name of now: one with the same file name. For a name the
    /// last scan did not see (written through the link in this pull).
    fn linked_name_of(&self, path: &str) -> Option<String> {
        let fs = self.vault.fs();
        if !fs.via_link(path) {
            return None;
        }
        let name = vpath::file_name(path);
        self.state
            .files
            .values()
            .filter(|t| !t.deleted && t.alias_of.is_none() && t.path != path && vpath::file_name(&t.path) == name)
            .find(|t| fs.same_file(&t.path, path))
            .map(|t| t.path.clone())
    }

    /// No remote change is applied through `path` while the file there is
    /// also synced under another vault path here (a tracked file there that
    /// is not a duplicate; a duplicate's own changes wait in `apply_remote`):
    /// one file on disk reached
    /// under two names, through a folder linked in twice or a symlink to
    /// another note, which the other devices have as two files. Writing it would change the other name too, and deleting or
    /// moving it through a folder link would delete or move the other one:
    /// when another device deleted its duplicate under the link's name,
    /// this device moved the real note to the trash, and its next push
    /// deleted that note on every device (FINDING-224). The change stays
    /// pending and is listed instead, as for a name this storage cannot
    /// hold. Replacing the link with a copy of what it leads to makes the
    /// names separate files here too, and the change applies to its own.
    /// A tracked file in a folder this device cannot read now, which may be
    /// another name of the file (see `unknown_aliases`), holds it too.
    fn refuse_alias(&self, path: &str, act: Act) -> Result<(), SyncError> {
        let others = self.tracked_aliases(path);
        let (same, effect, fix) = if !others.is_empty() {
            let others = others.join(", ");
            (
                format!("{path} is one file with {others} on this device, through a symlink, while other devices have them as separate files"),
                format!(": it would change {others} too"),
                "To sync it, replace the link on this device with a copy of what it leads to",
            )
        } else {
            let maybe = self.unknown_aliases(path, matches!(act, Act::Delete));
            if maybe.is_empty() {
                return Ok(());
            }
            let maybe = maybe.join(", ");
            (
                format!("{path} may be one file with {maybe} on this device, which is in a folder it cannot read now"),
                String::new(),
                "To sync it, make that folder readable again",
            )
        };
        Err(SyncError::Local(match act {
            Act::Delete => format!("another device deleted this file, but {same}, so the delete is not applied here. {fix}"),
            Act::Move => format!("another device renamed this file ({path} here) to this name, but {same}, so the rename is not applied here. {fix}"),
            Act::Write => format!("another device changed this file, but {same}, so the change is not applied here{effect}. {fix}"),
        }))
    }

    /// Tracked files, not duplicates, in folders that this device cannot
    /// read now, last synced with the content of the file at `path`, or for
    /// a delete (`by_name`) with its file name too, as a folder link keeps
    /// names: the listing could not see whether they are other names of it,
    /// as through a link in such a folder (which only a listing made by
    /// this app while the folder was readable knows). An empty file says
    /// nothing by its content.
    fn unknown_aliases(&self, path: &str, by_name: bool) -> Vec<String> {
        let unreadable = self.vault.unreadable_folders();
        if unreadable.is_empty() {
            return Vec::new();
        }
        let hash = self.state.files.values().find(|t| !t.deleted && t.path == path).map(|t| t.hash.as_str());
        let hash = hash.filter(|h| !h.is_empty() && *h != hex_hash(b""));
        let name = vpath::file_name(path);
        let mut found: Vec<String> = self
            .state
            .files
            .values()
            .filter(|t| !t.deleted && t.alias_of.is_none() && t.path != path && unreadable.iter().any(|d| vpath::is_same_or_inside(&t.path, d)))
            .filter(|t| Some(t.hash.as_str()) == hash || (by_name && vpath::file_name(&t.path) == name))
            .map(|t| t.path.clone())
            .collect();
        found.sort();
        found
    }

    /// Move the file `fid` from `path` to a conflict copy name, and return
    /// that. The rename is pushed, unless a remote change of the file moves
    /// it on first (the remote name wins).
    fn set_aside(&mut self, fid: &str, path: &str, local: &mut HashMap<String, LocalFile>, report: &mut SyncReport) -> Result<String, SyncError> {
        let copy = self.conflict_path(path)?;
        self.rename(path, &copy, report)?;
        // The scan's view of the file goes with it, for a change of the file
        // later in this pull.
        if let Some(lf) = local.remove(path) {
            local.insert(copy.clone(), lf);
        }
        if let Some(t) = self.state.files.get_mut(fid) {
            t.path = copy.clone();
            t.server_path = Some(path.to_string());
        }
        // Saved at once: if the app is killed before the pull ends, an edited
        // file is still known under its new name, not taken for a new one.
        self.save_state()?;
        Ok(copy)
    }

    /// The seq of a file's first revision: when it was first uploaded. Unlike
    /// its head, a later change does not move it, so devices that have seen
    /// different heads still agree which of two files came first.
    fn first_seq(&self, fid: &str) -> Result<u64, SyncError> {
        let revisions = self.transport.history(&self.settings.vault_id, fid)?;
        revisions.iter().map(|r| r.seq).min().ok_or_else(|| SyncError::Server("the server has no history for this file".into()))
    }

    /// Two devices that had not seen each other's file each uploaded one at
    /// `path` with the same content, and the pull brings the other one
    /// (`head`, with `data`): the file here, unchanged and at that path on
    /// the server too, is the same note. The file uploaded later keeps its
    /// id (the one that keeps the name in `make_way`), and the earlier one
    /// is deleted on the server, which keeps its history; no file here
    /// changes. If the file here is the earlier one, it is tracked under the
    /// other's id from now on, as on the devices that have it (see
    /// `keep_retired_copies`). Returns whether the pulled file is dealt with
    /// this way (no file here is written for it).
    ///
    /// Not if either file changed on the server since this pull saw it: then
    /// they are two files (see `make_way`). If the pulled file changed, or
    /// the server refuses the delete because the earlier file changed just
    /// now, the round starts again, and its pull shows the change; a pulled
    /// change that waited as pending waits for the newer one instead (a new
    /// round would try the same pending change again). If the file here was
    /// deleted on the server since this device's pull, most likely by
    /// another device retiring it, the pulled copy is the note: the file
    /// here is tracked under its id, as `keep_retired_copies` does when the
    /// delete comes in the same batch of the pull. That delete may be in a
    /// later batch, which a new round would not reach.
    fn retire(&mut self, head: &RemoteHead, path: &str, data: &[u8], status: &HashMap<String, Status>) -> Result<bool, SyncError> {
        let hash = hex_hash(data);
        let Some((here, t)) = self.state.files.iter().find(|(_, t)| !t.deleted && t.alias_of.is_none() && t.path == path) else { return Ok(false) };
        let unchanged = *status.get(here).unwrap_or(&Status::Unchanged) == Status::Unchanged;
        if !unchanged || t.hash != hash || t.server_path.is_some() || self.state.pending.contains_key(here) {
            return Ok(false);
        }
        let (here, here_seq, seen) = (here.clone(), t.seq, t.seen);
        let (ours, theirs) = (self.revisions(&here)?, self.revisions(&head.file_id)?);
        if theirs.1.seq != head.seq {
            let retried = self.state.pending.get(&head.file_id) == Some(&head.seq);
            return Err(SyncError::Local(if retried { SUPERSEDED } else { MOVED_ON }.into()));
        }
        if ours.1.seq != here_seq && ours.1.deleted {
            log::info!("sync: {path} was uploaded twice with the same content, and the copy here was deleted; keeping it as the other");
            let t = self.state.files[&here].clone();
            let _ = std::fs::remove_file(self.base_path(&here));
            self.state.files.insert(here, Tracked { seq: ours.1.seq, hash: String::new(), deleted: true, seen: None, ..t.clone() });
            self.state.files.insert(head.file_id.clone(), Tracked { seq: head.seq, hash, seen, ..t });
            self.save_base(&head.file_id, path, data);
            return Ok(true);
        }
        if ours.1.seq != here_seq {
            return Ok(false);
        }
        let here_first = ours.0 < theirs.0;
        let (old, parent) = if here_first { (&here, here_seq) } else { (&head.file_id, head.seq) };
        let payload = FilePayload { path: path.to_string(), mtime: 0, data: Vec::new() };
        let rev = PutRevision {
            parent_seq: Some(parent),
            device: self.settings.device.clone(),
            deleted: true,
            blob: b64(&self.key.encrypt(old, &payload.encode())),
        };
        let seq = match self.transport.put(&self.settings.vault_id, old, &rev)? {
            PutOutcome::Stored(seq) => seq,
            PutOutcome::Conflict(_) => return Err(SyncError::Local(MOVED_ON.into())),
        };
        log::info!("sync: {path} was uploaded twice with the same content; keeping one");
        let _ = std::fs::remove_file(self.base_path(old));
        let gone = Tracked { path: path.to_string(), seq, hash: String::new(), deleted: true, seen: None, server_path: None, renamed: false, alias_of: None, took_over: None, delete_with_note: false, detached: false };
        self.state.files.insert(old.clone(), gone);
        if here_first {
            let kept = Tracked { path: path.to_string(), seq: head.seq, hash, deleted: false, seen, server_path: None, renamed: false, alias_of: None, took_over: None, delete_with_note: false, detached: false };
            self.state.files.insert(head.file_id.clone(), kept);
            self.save_base(&head.file_id, path, data);
        }
        Ok(true)
    }

    /// The seq of a file's first revision on the server, and its newest
    /// revision.
    fn revisions(&self, fid: &str) -> Result<(u64, HistoryEntry), SyncError> {
        let revisions = self.transport.history(&self.settings.vault_id, fid)?;
        let first = revisions.iter().map(|r| r.seq).min();
        let newest = revisions.into_iter().max_by_key(|r| r.seq);
        first.zip(newest).ok_or_else(|| SyncError::Server("the server has no history for this file".into()))
    }

    /// The files here that another device deleted on the server as the
    /// earlier of two uploads of one note (see `retire`): a remote delete of
    /// a file that is unchanged here, at the path it has here, while a file
    /// that is new here comes in the same pull and had that path and content
    /// when the delete was made. That is the copy that was kept: the file
    /// here is tracked under its id from now on, at that version, and the
    /// delete is not applied, so nothing is moved to the trash. If the kept
    /// copy was edited or renamed since (a client without the retire renames
    /// it to a conflict copy name), the pull applies that as usual. Only a
    /// kept copy that is at that path now, or has that content, is looked
    /// for.
    fn keep_retired_copies(&mut self, heads: &[RemoteHead], status: &HashMap<String, Status>) -> Result<(), SyncError> {
        // path -> the file here and the seq of its delete
        let mut retired: HashMap<String, (String, u64)> = HashMap::new();
        for h in heads.iter().filter(|h| h.deleted) {
            let Some(t) = self.state.files.get(&h.file_id) else { continue };
            let unchanged = status.get(&h.file_id) == Some(&Status::Unchanged);
            if t.deleted || !unchanged || t.server_path.is_some() || t.alias_of.is_some() || self.state.pending.contains_key(&h.file_id) {
                continue;
            }
            if let Ok((path, _)) = self.open(h)
                && path == t.path
            {
                retired.insert(path, (h.file_id.clone(), h.seq));
            }
        }
        if retired.is_empty() {
            return Ok(());
        }
        for h in heads.iter().filter(|h| !h.deleted) {
            if self.state.files.get(&h.file_id).is_some_and(|t| !t.deleted) {
                continue;
            }
            let Ok((path, data)) = self.open(h) else { continue };
            let hash = hex_hash(&data);
            // Where it may have been: at its path, or else where a file with
            // its content was deleted (empty files say nothing).
            let mut places: Vec<&String> = retired.keys().filter(|at| **at == path).collect();
            if places.is_empty() && !data.is_empty() {
                places = retired.iter().filter(|(_, (fid, _))| self.state.files[fid].hash == hash).map(|(at, _)| at).collect();
                places.sort();
            }
            let mut found = None;
            for at in places {
                let (fid, seq) = &retired[at];
                let t = &self.state.files[fid];
                let kept = if path == *at && hash == t.hash { Some((h.seq, data.clone())) } else { self.version_before(&h.file_id, *seq, at, &t.hash)? };
                if let Some(kept) = kept {
                    found = Some((at.clone(), kept));
                    break;
                }
            }
            let Some((at, (kept_seq, kept_data))) = found else { continue };
            let (fid, seq) = retired.remove(&at).unwrap();
            let t = self.state.files[&fid].clone();
            log::info!("sync: {at} was uploaded twice with the same content; keeping the copy here as the other");
            let _ = std::fs::remove_file(self.base_path(&fid));
            self.state.files.insert(fid, Tracked { seq, hash: String::new(), deleted: true, seen: None, ..t.clone() });
            self.state.files.insert(h.file_id.clone(), Tracked { seq: kept_seq, ..t });
            self.save_base(&h.file_id, &at, &kept_data);
        }
        Ok(())
    }

    /// The revision of file `fid` that was its head just before seq
    /// `before`, with its content, if it had the path `path` and content
    /// with the hash `hash`. `None` too if the server cannot serve it (only a
    /// lost connection or token is an error).
    fn version_before(&self, fid: &str, before: u64, path: &str, hash: &str) -> Result<Option<(u64, Vec<u8>)>, SyncError> {
        let fetched = self.transport.history(&self.settings.vault_id, fid).and_then(|history| {
            let Some(r) = history.into_iter().filter(|r| r.seq < before).max_by_key(|r| r.seq) else { return Ok(None) };
            let r = self.transport.revision(&self.settings.vault_id, r.seq)?;
            Ok(Some(r))
        });
        let r = match fetched {
            Ok(Some(r)) if r.file_id == fid && !r.deleted => r,
            Ok(_) => return Ok(None),
            Err(e @ (SyncError::Network(_) | SyncError::Unauthorized)) => return Err(e),
            Err(e) => {
                log::warn!("sync: cannot fetch an earlier version of file {fid}: {e}");
                return Ok(None);
            }
        };
        let Some(blob) = unb64(&r.blob) else { return Ok(None) };
        let Ok(payload) = self.key.decrypt(fid, &blob).and_then(|b| FilePayload::decode(&b)) else { return Ok(None) };
        let same = vpath::normalize(&payload.path).is_ok_and(|p| p == path) && hex_hash(&payload.data) == hash;
        Ok(same.then_some((r.seq, payload.data)))
    }

    fn rename(&self, from: &str, to: &str, report: &mut SyncReport) -> Result<(), SyncError> {
        self.refuse_alias(from, Act::Move)?;
        self.refuse_link_move(from, to)?;
        report.changes.extend(self.vault.ensure_folder(vpath::parent(to))?);
        report.changes.extend(self.vault.rename(from, to)?);
        self.prune(vpath::parent(from), report);
        Ok(())
    }

    /// Apply a remote deletion to a file that was unchanged here (`hash`),
    /// if it still is. One saved since the scan is kept: the edit beats the
    /// delete, and the file is revived on push.
    fn delete(&self, path: &str, hash: &str, report: &mut SyncReport) -> Result<(), SyncError> {
        self.refuse_alias(path, Act::Delete)?;
        match self.vault.delete_file(path, hash) {
            Ok(c) => report.changes.extend(c),
            Err(CoreError::NotFound(_)) => {}
            Err(CoreError::Conflict(_)) => {
                log::info!("sync: {path} was edited during sync; keeping it");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }
        self.prune(vpath::parent(path), report);
        Ok(())
    }

    /// A symlink to a note with a relative target leads somewhere else once
    /// it is moved into another folder: such a rename waits, listed, and
    /// nothing changes on disk.
    fn refuse_link_move(&self, from: &str, to: &str) -> Result<(), SyncError> {
        if vpath::parent(from) == vpath::parent(to) {
            return Ok(());
        }
        let Some(abs) = self.vault.fs().os_path(from) else { return Ok(()) };
        match std::fs::read_link(&abs) {
            Ok(target) if target.is_relative() => Err(SyncError::Local(format!(
                "another device moved this note ({from} here) to this name, but on this device {from} is a link to another file by a relative path, which the move would break. To sync it, replace the link with a copy of the note"
            ))),
            _ => Ok(()),
        }
    }

    /// Remove folder `dir` and the folders above it while they are empty,
    /// after a remote change moved or deleted the last file in it. The
    /// change is made by then: a folder that cannot be removed (a mount
    /// point, no permission) stays, and does not hold the change back.
    fn prune(&self, dir: &str, report: &mut SyncReport) {
        match self.vault.prune_empty_folders(dir) {
            Ok(c) => report.changes.extend(c),
            Err(e) => log::warn!("sync: cannot remove the emptied folder {dir}: {e}"),
        }
    }

    fn stat(&self, path: &str) -> (u64, i64) {
        self.vault.index().entry(path).map(|e| (e.size, e.mtime)).unwrap_or((0, 0))
    }

    /// Merge a remote edit into the local file at `path`, edited here too
    /// (`lf_hash` is the hash of the local content), from the last synced
    /// version.
    fn merge(
        &self,
        fid: &str,
        path: &str,
        lf_hash: &str,
        rdata: &[u8],
        created: &HashSet<String>,
        report: &mut SyncReport,
    ) -> Result<(), SyncError> {
        let local_data = self.vault.read_file(path)?;
        let merged = if is_mergeable(path) {
            match (self.load_base(fid), std::str::from_utf8(&local_data), std::str::from_utf8(rdata)) {
                (Some(base), Ok(ours), Ok(theirs)) => merge_text(&base, ours, theirs),
                _ => None,
            }
        } else {
            None
        };
        match merged {
            Some(m) => {
                if m.as_bytes() != local_data.as_slice() {
                    self.write(path, m.as_bytes(), Some(lf_hash), report)?;
                }
            }
            None => {
                // Keep ours in place; theirs becomes a conflict copy.
                self.conflict_copy(path, rdata, created, report)?;
            }
        }
        Ok(())
    }

    // ---------- the round ----------

    /// Sync until the vault and the server agree (or give up after a few
    /// rounds of other devices racing us).
    pub fn sync(&mut self) -> Result<SyncReport, SyncError> {
        // One sync at a time per state folder, each from the state the last
        // one saved.
        let _lock = lock_dir(&self.dir)?;
        self.refresh()?;
        let mut report = SyncReport::default();
        self.check_server()?;
        for round in 1..=MAX_ROUNDS {
            report.rounds = round;
            match self.round(&mut report) {
                Ok(true) => {
                    self.save_state()?;
                    return Ok(report);
                }
                Ok(false) => {
                    self.save_state()?;
                    continue;
                }
                Err(e) if is_race(&e) => {
                    // The user edited a file while we were writing it; try again.
                    log::info!("sync: {e}; retrying");
                    self.save_state()?;
                    continue;
                }
                Err(e) => {
                    let _ = self.save_state();
                    return Err(e);
                }
            }
        }
        Err(SyncError::Server("the server kept changing during sync; will retry later".into()))
    }

    /// The server must have every change this device has seen. One with
    /// fewer was reset or restored from an older backup: pulling from the
    /// cursor here would skip its new changes, and it would refuse edits to
    /// the files it lost. Nothing is synced until the user connects again,
    /// which starts afresh (see `connect_with`). A server without the vault
    /// gets its own message: it may have been reset, but another Cairn
    /// server answering at its address looks the same. Anything else there
    /// answering 404 is not a Cairn server, and `get_vault` says so.
    fn check_server(&self) -> Result<(), SyncError> {
        if self.state.last_seq == 0 {
            return Ok(());
        }
        match self.transport.get_vault(&self.settings.vault_id)? {
            Some(info) if info.head_seq >= self.state.last_seq => Ok(()),
            Some(_) => Err(SyncError::ServerReset),
            None => Err(SyncError::VaultGone),
        }
    }

    /// Decrypt a head and check its path.
    fn open(&self, head: &RemoteHead) -> Result<Opened, SyncError> {
        let blob = unb64(&head.blob).ok_or_else(|| SyncError::Server("bad blob encoding".into()))?;
        let payload = FilePayload::decode(&self.key.decrypt(&head.file_id, &blob)?)?;
        let path = vpath::normalize(&payload.path).map_err(|_| SyncError::Server(format!("unsafe path {:?}", payload.path)))?;
        if path.is_empty() || vpath::is_hidden(&path) {
            return Err(SyncError::Server(format!("unsafe path {path:?}")));
        }
        Ok((path, payload.data))
    }

    /// On a fresh state (a first sync, or after the state was lost), find
    /// the files renamed here meanwhile: a live server file that is missing
    /// here and whose content is that of exactly one new file here, at a
    /// path no server file has, with no other such server file having that
    /// content and no other server file at its path (empty files say
    /// nothing). It is tracked as last synced, under the server's path, so
    /// the push finds it renamed, and keeps finding it until the rename is
    /// uploaded, instead of the server file being written next to it. Files
    /// at the server's path with the server's content are adopted in
    /// `apply_remote`. Nothing here changes or deletes a file. `files` are
    /// those of the whole feed (see `server_files`).
    fn relink(&mut self, mut files: Vec<ServerFile>, local: &HashMap<String, LocalFile>, created: &HashSet<String>) -> Vec<String> {
        // Only the newest head of a file counts: a file shows up twice if
        // its head moved while the feed was read.
        files.sort_by_key(|f| std::cmp::Reverse(f.seq));
        let mut ids = HashSet::new();
        files.retain(|f| ids.insert(f.fid.clone()));
        let remote: Vec<(&ServerFile, &str, Option<&str>)> =
            files.iter().filter_map(|f| f.file.as_ref().map(|(p, h)| (f, p.as_str(), h.as_deref()))).collect();
        let mut server_paths: HashMap<String, usize> = HashMap::new();
        for (_, p, _) in &remote {
            *server_paths.entry(p.to_lowercase()).or_default() += 1;
        }
        let mut missing: HashMap<&str, Vec<(&ServerFile, &str)>> = HashMap::new();
        for &(f, path, hash) in &remote {
            if let Some(hash) = hash
                && server_paths[&path.to_lowercase()] == 1
                && !local.contains_key(path)
                && !self.taken(path, None)
            {
                missing.entry(hash).or_default().push((f, path));
            }
        }
        let mut new_here: HashMap<&str, Vec<&str>> = HashMap::new();
        for p in created.iter().filter(|p| !server_paths.contains_key(&p.to_lowercase())) {
            new_here.entry(&local[p].hash).or_default().push(p);
        }
        let mut linked = Vec::new();
        for (hash, found) in missing {
            let ([(f, path)], Some([to])) = (found.as_slice(), new_here.get(hash).map(Vec::as_slice)) else { continue };
            log::info!("sync: {path} on the server is {to} here; pushing the rename");
            let t = Tracked {
                path: path.to_string(),
                seq: f.seq,
                hash: hash.to_string(),
                deleted: false,
                seen: None,
                server_path: None,
                renamed: false,
                alias_of: None,
                took_over: None,
                delete_with_note: false,
                detached: false,
            };
            self.state.files.insert(f.fid.clone(), t);
            linked.push(f.fid.clone());
        }
        linked
    }

    /// The files that `heads` have, for `relink`. The base of each one with
    /// the content of a new file here (`wanted`, content hashes) is saved
    /// now, while its content is at hand: `relink` may take the new file for
    /// it.
    fn server_files(&self, heads: &[RemoteHead], wanted: &HashSet<&str>) -> Vec<ServerFile> {
        let file = |h: &RemoteHead| {
            if h.deleted {
                return None;
            }
            let (path, data) = self.open(h).ok()?;
            let hash = (!data.is_empty()).then(|| hex_hash(&data));
            if hash.as_deref().is_some_and(|x| wanted.contains(x)) {
                self.save_base(&h.file_id, &path, &data);
            }
            Some((path, hash))
        };
        heads.iter().map(|h| ServerFile { fid: h.file_id.clone(), seq: h.seq, file: file(h) }).collect()
    }

    /// The files in the feed after `cursor`, for `relink`.
    fn read_ahead(&self, mut cursor: u64, wanted: &HashSet<&str>) -> Result<Vec<ServerFile>, SyncError> {
        let mut files = Vec::new();
        loop {
            let page = self.changes(cursor)?;
            files.extend(self.server_files(&page.heads, wanted));
            cursor = page.cursor;
            if !page.more {
                return Ok(files);
            }
        }
    }

    /// One pull-merge-push pass. `Ok(false)` means "go again".
    fn round(&mut self, report: &mut SyncReport) -> Result<bool, SyncError> {
        let fresh = self.state.files.is_empty() && self.state.last_seq == 0;
        let (mut local, unreadable) = self.scan(report)?;
        let (mut status, created) = self.classify(&local, &unreadable);
        let mut created: HashSet<String> = created.into_iter().collect();
        if self.regroup(&local, &mut status, &mut created, report) {
            self.save_state()?;
        }
        // Files that are one file on disk are forgotten here and downloaded
        // again below, as new files: the path is taken, so they get conflict
        // copy names, and whatever the disk file holds is pushed as a new file.
        for (fid, st) in &status {
            if *st == Status::Shared
                && let Some(t) = self.state.files.remove(fid)
            {
                log::warn!("sync: {} is the same file as another one here; downloading it again", t.path);
                let _ = std::fs::remove_file(self.base_path(fid));
                self.state.pending.entry(fid.clone()).or_insert(t.seq);
            }
        }

        // ----- pull -----
        // The feed comes in batches (see PULL_BATCH), each applied and
        // recorded before the next is read. Changes that wait as pending are
        // retried with the first one.
        let mut cursor = self.state.last_seq;
        let (mut heads, mut more) = self.read_batch(&mut cursor)?;
        let mut retry: Vec<(String, u64)> = self.state.pending.iter().map(|(f, s)| (f.clone(), *s)).collect();
        self.linked_waits.clear();
        let mut rebuild = fresh && !created.is_empty();
        let mut failed = HashMap::new();
        // The files that this pull has changed or tried to (and those relink
        // took on): what the scan found is not true of them any more.
        let mut touched: HashSet<String> = HashSet::new();
        loop {
            let held = self.state.pending.clone();
            let batch = self.with_pending(heads, &retry, report)?;
            // One of them comes again: its head moved while the feed was
            // read, or a change of it that waits (pending or held) was tried
            // first. Its newer change is applied to the vault as it is now,
            // as in a new round: the vault is scanned again.
            if batch.iter().any(|h| touched.contains(&h.file_id) && !self.applied(h)) {
                let (l, unreadable) = self.scan(report)?;
                let (mut s, c) = self.classify(&l, &unreadable);
                let mut c: HashSet<String> = c.into_iter().collect();
                self.regroup(&l, &mut s, &mut c, report);
                (local, status, created) = (l, s, c);
            }
            // A lost state is rebuilt from the whole feed: what is not in
            // this batch is read ahead for it (and read again to apply it).
            if std::mem::take(&mut rebuild) {
                let wanted: HashSet<&str> = created.iter().map(|p| local[p].hash.as_str()).collect();
                let mut files = self.server_files(&batch, &wanted);
                if more {
                    files.extend(self.read_ahead(cursor, &wanted)?);
                }
                touched.extend(self.relink(files, &local, &created));
            }
            // A delete of a file here that another device retired as the
            // earlier of two identical uploads is not applied when the kept
            // copy is in this batch too (see `keep_retired_copies`).
            self.keep_retired_copies(&batch, &status)?;
            touched.extend(batch.iter().map(|h| h.file_id.clone()));
            self.apply_batch(batch, more, &mut local, &status, &created, &mut failed, report)?;
            self.state.last_seq = cursor;
            self.save_state()?;
            if !more {
                break;
            }
            // Deletions go first only within a batch: a change held in this
            // one may need a path that a deletion in the next one frees. It
            // is tried once more with the next batch.
            retry = self
                .state
                .pending
                .iter()
                .filter(|(f, s)| held.get(*f) != Some(*s) || self.linked_waits.contains(*f))
                .map(|(f, s)| (f.clone(), *s))
                .collect();
            (heads, more) = self.read_batch(&mut cursor)?;
        }

        // ----- push -----
        // Files with a pending remote change wait until it is applied; their
        // parent revision is not the server's head. An unchanged file goes up
        // if the server has it under another name.
        let (local, unreadable) = self.scan(report)?;
        let (mut status, created) = self.classify(&local, &unreadable);
        let mut created: HashSet<String> = created.into_iter().collect();
        self.regroup(&local, &mut status, &mut created, report);
        let mut ops: Vec<Upload> = Vec::new();
        for (fid, st) in &status {
            // `regroup` may have forgotten it.
            let Some(t) = self.state.files.get(fid) else { continue };
            if self.state.pending.contains_key(fid) {
                continue;
            }
            // Nothing is sent for a duplicate but its delete, when the note
            // was deleted here: its delete goes up now, or went up already
            // (or another device deleted the note too, see
            // `delete_with_note`).
            if let Some(original) = &t.alias_of {
                let gone = status.get(original) == Some(&Status::Deleted) || t.delete_with_note;
                if *st == Status::Deleted && gone && !t.detached && !self.state.pending.contains_key(original) {
                    ops.push((fid.clone(), Some(t.seq), t.server_path.clone().unwrap_or_else(|| t.path.clone()), true));
                }
                continue;
            }
            match st {
                Status::Unchanged if t.server_path.is_some() => ops.push((fid.clone(), Some(t.seq), t.path.clone(), false)),
                Status::Unchanged | Status::Unreadable | Status::Shared => {}
                Status::Modified => ops.push((fid.clone(), Some(t.seq), t.path.clone(), false)),
                Status::Renamed(to) => ops.push((fid.clone(), Some(t.seq), to.clone(), false)),
                Status::Deleted => ops.push((fid.clone(), Some(t.seq), t.path.clone(), true)),
            }
        }
        let mut created: Vec<String> = created.into_iter().collect();
        created.sort();
        for path in created {
            // Re-creating a file at the path of a deleted one revives its id.
            let revive = self
                .state
                .files
                .iter()
                .find(|(f, t)| t.deleted && t.path == path && !self.state.pending.contains_key(*f))
                .map(|(f, t)| (f.clone(), t.seq));
            match revive {
                Some((fid, seq)) => ops.push((fid, Some(seq), path, false)),
                None => ops.push((new_file_id(), None, path, false)),
            }
        }
        ops.sort_by(|a, b| a.2.cmp(&b.2));
        // The server path each upload frees, by a rename or a delete.
        let frees: Vec<Option<String>> = ops
            .iter()
            .map(|(fid, _, path, deleted)| {
                let t = self.state.files.get(fid).filter(|t| !t.deleted)?;
                let server = t.server_path.as_ref().unwrap_or(&t.path);
                (*deleted || server != path).then(|| server.clone())
            })
            .collect();
        let mut ops = upload_order(ops, &frees);
        // The deletes of duplicates go first: a note whose duplicate's delete
        // did not go up stays, and both go up in a later sync (once the
        // note's delete is recorded, nothing tells that it was made here).
        let is_dup = |op: &Upload| op.3 && self.state.files.get(&op.0).is_some_and(|t| !t.deleted && t.alias_of.is_some());
        ops.sort_by_key(|op| !is_dup(op));
        let mut blocked: HashSet<String> = HashSet::new();
        let max_file = self.transport.max_file_size();
        for (fid, parent, path, deleted) in ops {
            if blocked.contains(&fid) {
                continue;
            }
            // Not read and encrypted on every sync only to be refused.
            if !deleted && self.stat(&path).0 > max_file {
                report.skip(&path, too_large(max_file));
                continue;
            }
            let (data, mtime) = if deleted {
                (Vec::new(), 0)
            } else {
                match self.vault.read_file(&path) {
                    Ok(d) => (d, self.stat(&path).1),
                    Err(CoreError::NotFound(_)) => continue, // gone meanwhile; next sync
                    Err(e) => {
                        report.skip(&path, reason(&e.into()));
                        continue;
                    }
                }
            };
            let payload = FilePayload { path: path.clone(), mtime, data };
            let rev = PutRevision {
                parent_seq: parent,
                device: self.settings.device.clone(),
                deleted,
                blob: b64(&self.key.encrypt(&fid, &payload.encode())),
            };
            let outcome = match self.transport.put(&self.settings.vault_id, &fid, &rev) {
                Ok(o) => o,
                Err(e @ SyncError::Upload(_)) => {
                    // Too large, stalled or cut off: the other files still
                    // go up, and this one is tried again on the next sync
                    // (with the note it duplicates, if it is a duplicate).
                    log::warn!("sync: cannot upload {path}: {e}");
                    report.skip(&path, reason(&e));
                    blocked.extend(self.state.files.get(&fid).and_then(|t| t.alias_of.clone()));
                    continue;
                }
                Err(e) => return Err(e),
            };
            match outcome {
                PutOutcome::Stored(seq) => {
                    // A note deleted here: its duplicates' deletes follow,
                    // also those that cannot be seen now (see `regroup`).
                    if deleted && self.state.files.get(&fid).is_some_and(|t| t.alias_of.is_none()) {
                        for t in self.state.files.values_mut().filter(|t| !t.deleted && !t.detached && t.alias_of.as_deref() == Some(fid.as_str())) {
                            t.delete_with_note = true;
                        }
                    }
                    let hash = if deleted { String::new() } else { hex_hash(&payload.data) };
                    if !deleted {
                        self.save_base(&fid, &path, &payload.data);
                    } else {
                        let _ = std::fs::remove_file(self.base_path(&fid));
                    }
                    // Not `seen`: the next scan hashes the file again, so a
                    // save made since the read above is pushed then.
                    self.state.files.insert(fid.clone(), Tracked { path, seq, hash, deleted, seen: None, server_path: None, renamed: false, alias_of: None, took_over: None, delete_with_note: false, detached: false });
                    // The server has taken no other change (of any vault)
                    // since the last one this device has seen: the next
                    // pull starts after this one, rather than download it.
                    if seq == self.state.last_seq + 1 {
                        self.state.last_seq = seq;
                    }
                    report.pushed += 1;
                    self.record(&fid)?;
                }
                PutOutcome::Conflict(_) => return Ok(false),
            }
        }
        Ok(true)
    }

    /// One page of the changes feed after `cursor`. A page that says there
    /// is more must move the cursor on, or paging would never end (a buggy
    /// server, or a proxy that answers every request from its cache).
    fn changes(&self, cursor: u64) -> Result<ChangesResponse, SyncError> {
        let page = self.transport.changes(&self.settings.vault_id, cursor, PAGE)?;
        if page.more && page.cursor <= cursor {
            return Err(SyncError::Server("the server's list of changes did not move on".into()));
        }
        Ok(page)
    }

    /// Read the changes feed from `cursor` until about `pull_batch` bytes of
    /// records are in, or the feed ends; `cursor` moves past them. Returns
    /// the heads, and whether the feed has more.
    fn read_batch(&self, cursor: &mut u64) -> Result<(Vec<RemoteHead>, bool), SyncError> {
        let mut heads = Vec::new();
        let mut size = 0;
        loop {
            let page = self.changes(*cursor)?;
            size += page.heads.iter().map(|h| h.blob.len() as u64).sum::<u64>();
            heads.extend(page.heads);
            *cursor = page.cursor;
            if !page.more || size >= self.pull_batch {
                return Ok((heads, page.more));
            }
        }
    }

    /// A batch of heads with the pending changes `retry` (file id, seq), in
    /// the order to apply them.
    fn with_pending(&mut self, mut heads: Vec<RemoteHead>, retry: &[(String, u64)], report: &mut SyncReport) -> Result<Vec<RemoteHead>, SyncError> {
        // Retry what could not be applied before, unless the file has a
        // newer head by now (in this batch; one in a later batch is applied
        // after it). The file id is the blob's associated data, so a server
        // cannot pass off another file's revision here.
        let newer: HashSet<&str> = heads.iter().map(|h| h.file_id.as_str()).collect();
        let mut again = Vec::new();
        let mut gone = Vec::new();
        for &(ref fid, seq) in retry.iter().filter(|(f, _)| !newer.contains(f.as_str())) {
            if let Some(head) = self.held(fid, seq) {
                again.push(head);
                continue;
            }
            match self.transport.revision(&self.settings.vault_id, seq) {
                Ok(r) => again.push(RemoteHead { file_id: fid.clone(), seq, parent_seq: None, device: String::new(), deleted: r.deleted, blob: r.blob }),
                Err(e @ (SyncError::Network(_) | SyncError::Unauthorized)) => return Err(e),
                Err(e) => {
                    // The server cannot serve it (for example after a restore
                    // from an older backup). As with a head that does not
                    // decrypt, a file this device has stays on hold; one it
                    // does not have is given up once the server says the
                    // revision is gone, and retried after any other error.
                    log::warn!("sync: cannot fetch revision {seq} of file {fid}: {e}");
                    let tracked = self.state.files.get(fid).map(|t| t.path.as_str());
                    let lost = is_gone(&e);
                    report.skip(tracked.unwrap_or(""), if lost { "the server no longer has this change".into() } else { reason(&e) });
                    if tracked.is_none() && lost {
                        gone.push(fid.clone());
                    }
                }
            }
        }
        for fid in gone {
            self.unhold(&fid);
        }
        again.sort_by_key(|h| h.seq);
        heads.splice(0..0, again);
        // A file shows up twice if its head moved while the feed was paged;
        // only the newest head counts. Deletions go first, otherwise server
        // order: they free paths that other changes may need, as when a
        // folder is replaced by a file of the same name (deleting "Old/x.md"
        // removes the emptied folder before the file "Old" is written).
        let mut newest: HashMap<String, u64> = HashMap::new();
        for h in &heads {
            let seq = newest.entry(h.file_id.clone()).or_default();
            *seq = (*seq).max(h.seq);
        }
        heads.retain(|h| newest[&h.file_id] == h.seq);
        heads.sort_by_key(|h| !h.deleted);
        Ok(heads)
    }

    /// Whether this device has a head's revision of the file, or a newer
    /// one: it is our own upload, or applied already. A pending change, or
    /// a head that moved while the feed was read, can be older than what a
    /// later batch (or relink) brought.
    fn applied(&self, head: &RemoteHead) -> bool {
        self.state.files.get(&head.file_id).is_some_and(|t| t.seq >= head.seq)
    }

    /// Apply a batch of heads from `with_pending`. `failed` has the reason
    /// in `report` for each file whose change could not be applied in an
    /// earlier batch, to take back if it is applied now.
    #[allow(clippy::too_many_arguments)]
    fn apply_batch(
        &mut self,
        heads: Vec<RemoteHead>,
        more: bool,
        local: &mut HashMap<String, LocalFile>,
        status: &HashMap<String, Status>,
        created: &HashSet<String>,
        failed: &mut HashMap<String, Skipped>,
        report: &mut SyncReport,
    ) -> Result<(), SyncError> {
        // A change that wants a path where another file is here, which has a
        // change still to come in this batch (`ahead`), waits for that one:
        // it may move the file away. The server sends the newest change of
        // each file in the order they were made, so a rename onto a name that
        // another rename freed comes first once the other file has changed
        // again. Waiting changes are tried again after the rest, as long as
        // some of them can go, or a ring of them can be broken; then they go
        // as they are.
        let mut ahead: HashSet<String> = heads.iter().map(|h| h.file_id.clone()).collect();
        self.deleting = heads.iter().filter(|h| h.deleted).map(|h| h.file_id.clone()).collect();
        let mut queue: VecDeque<(RemoteHead, Option<Opened>)> = heads.into_iter().map(|h| (h, None)).collect();
        let mut waiting: Vec<(RemoteHead, Opened)> = Vec::new();
        let mut deferred: HashSet<String> = HashSet::new();
        let (mut wait, mut went) = (true, false);
        loop {
            let Some((head, opened)) = queue.pop_front() else {
                if waiting.is_empty() {
                    break;
                }
                // The waiting changes go round again. If none of them could
                // go, they wait for each other in a ring, as when two files
                // swapped names: one file of the ring moves aside, and its
                // own change moves it on.
                wait = went || self.break_ring(&waiting, &ahead, local, status, report)?;
                went = false;
                queue.extend(waiting.drain(..).map(|(h, x)| (h, Some(x))));
                continue;
            };
            let fid = head.file_id.clone();
            if self.applied(&head) {
                self.unhold(&fid);
                ahead.remove(&fid);
                continue;
            }
            let tracked_path = self.state.files.get(&fid).map(|t| t.path.clone());
            let (path, data) = match opened.map_or_else(|| self.open(&head), Ok) {
                Ok(x) => x,
                Err(e) => {
                    log::warn!("sync: skipping revision {} of file {fid}: {e}", head.seq);
                    report.skip(tracked_path.as_deref().unwrap_or(""), reason(&e));
                    // A file this device has stays on hold until a newer
                    // head replaces this one: pushing a local edit on top
                    // could overwrite a remote version it never saw. A file
                    // it does not have is simply not pulled.
                    if tracked_path.is_some() {
                        self.hold(&head);
                    } else {
                        self.unhold(&fid);
                    }
                    ahead.remove(&fid);
                    continue;
                }
            };
            // Once, after the rest of the batch: a change of a duplicate
            // whose note has a change in it too (it may bring the content
            // the duplicate has), and a new file reached through a link
            // here, for a file at its own name (see `regroup`). One through
            // a link to a folder of the vault waits for the last batch of
            // the pull, unlisted: the file at its own name may come later.
            if !head.deleted && !deferred.contains(&fid) {
                let tracked = self.state.files.get(&fid).filter(|t| !t.deleted);
                let dup = tracked.and_then(|t| t.alias_of.as_ref()).is_some_and(|o| ahead.contains(o));
                let linked = tracked.is_none() && self.vault.fs().via_link(&path);
                if linked && more && !self.vault.fs().leads_outside(&path) {
                    self.hold(&head);
                    ahead.remove(&fid);
                    self.linked_waits.insert(fid);
                    continue;
                }
                if (dup || linked) && !queue.is_empty() {
                    deferred.insert(fid);
                    queue.push_back((head, Some((path, data))));
                    continue;
                }
            }
            if wait && !head.deleted && self.blocker(&fid, &path, &ahead).is_some() {
                waiting.push((head, (path, data)));
                continue;
            }
            ahead.remove(&fid);
            went = true;
            let conflicts = report.conflicts.len();
            match self.apply_remote(&head, &path, &data, local, status, created, report) {
                Ok(()) => {
                    // Its entry in the report goes, unless another file
                    // still waits for the same reason at the same path.
                    if let Some(s) = failed.remove(&fid)
                        && !failed.values().any(|x| *x == s)
                    {
                        report.skipped.retain(|x| *x != s);
                    }
                    self.unhold(&fid);
                    report.pulled += 1;
                }
                Err(e) if is_race(&e) => {
                    // The file does not have the content the scan found: the
                    // next round hashes it again, rather than trust a cached
                    // hash and fail the same way every time.
                    if let Some(t) = self.state.files.get_mut(&fid) {
                        t.seen = None;
                    }
                    return Err(e);
                }
                Err(e) => {
                    log::warn!("sync: cannot apply {path}: {e}");
                    report.conflicts.truncate(conflicts);
                    let s = Skipped { path, reason: reason(&e) };
                    report.skip(&s.path, s.reason.clone());
                    failed.insert(fid, s);
                    self.hold(&head);
                }
            }
        }
        Ok(())
    }

    /// The file here at `path`, which a remote change of `fid` wants, if it
    /// has a change in `ahead`.
    fn blocker(&self, fid: &str, path: &str, ahead: &HashSet<String>) -> Option<String> {
        let own = self.state.files.get(fid).filter(|t| !t.deleted).map(|t| t.path.as_str());
        if own == Some(path) || !self.taken(path, own) {
            return None;
        }
        let path = path.to_lowercase();
        let (f, _) = self.state.files.iter().find(|(f, t)| *f != fid && !t.deleted && ahead.contains(*f) && t.path.to_lowercase() == path)?;
        Some(f.clone())
    }

    /// Move aside a file in a ring of `waiting` changes, each of which wants
    /// the path of a file whose change waits too (see `apply_batch`): the
    /// change that wants its path can go then. Returns whether one moved.
    fn break_ring(
        &mut self,
        waiting: &[(RemoteHead, Opened)],
        ahead: &HashSet<String>,
        local: &mut HashMap<String, LocalFile>,
        status: &HashMap<String, Status>,
        report: &mut SyncReport,
    ) -> Result<bool, SyncError> {
        let wants: HashMap<&str, String> =
            waiting.iter().filter_map(|(h, (p, _))| Some((h.file_id.as_str(), self.blocker(&h.file_id, p, ahead)?))).collect();
        let mut walk: Vec<String> = Vec::new();
        let mut at = waiting.first().map(|(h, _)| h.file_id.clone());
        while let Some(f) = at {
            if let Some(i) = walk.iter().position(|w| *w == f) {
                for fid in &walk[i..] {
                    let Some(path) = self.state.files.get(fid).map(|t| t.path.clone()) else { continue };
                    if self.movable(&path, status).as_ref() == Some(fid) {
                        log::info!("sync: moving {path} aside while the files of a swap change places");
                        self.set_aside(fid, &path, local, report)?;
                        return Ok(true);
                    }
                }
                break;
            }
            at = wants.get(f.as_str()).cloned();
            walk.push(f);
        }
        Ok(false)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_remote(
        &mut self,
        head: &RemoteHead,
        rpath: &str,
        rdata: &[u8],
        local: &mut HashMap<String, LocalFile>,
        status: &HashMap<String, Status>,
        created: &HashSet<String>,
        report: &mut SyncReport,
    ) -> Result<(), SyncError> {
        let fid = head.file_id.clone();
        let rhash = hex_hash(rdata);
        // The server's revision, at `path` here: if that is not the server's
        // path, the rename is pushed.
        let remote_tracked = |path: &str| Tracked {
            path: path.to_string(),
            seq: head.seq,
            hash: if head.deleted { String::new() } else { rhash.clone() },
            deleted: head.deleted,
            seen: None,
            server_path: (path != rpath && !head.deleted).then(|| rpath.to_string()),
            renamed: false,
            alias_of: None,
            took_over: None,
            delete_with_note: false,
            detached: false,
        };
        let old = self.state.files.get(&fid).cloned();
        let st = old.as_ref().filter(|t| !t.deleted).map(|_| status.get(&fid).cloned().unwrap_or(Status::Unchanged));

        // A duplicate of a file that syncs here under another id (see
        // `regroup`): its delete is recorded, and nothing else of it is
        // applied, as that would change the file it duplicates.
        if let Some(t) = old.as_ref().filter(|t| !t.deleted && t.alias_of.is_some()) {
            // A version of it that is what the file here holds, under its
            // server name or a name of the file here (an older version with
            // the same link uploads one whenever the note changes or is
            // renamed), is recorded, under that name.
            let fs = self.vault.fs();
            let original = t.alias_of.as_ref().and_then(|o| self.state.files.get(o)).filter(|o| !o.deleted).map(|o| o.path.as_str());
            let name_here = rpath == t.path || fs.same_file(rpath, &t.path) || original.is_some_and(|o| fs.same_file(rpath, o));
            let at = if name_here { rpath } else { t.path.as_str() };
            let named = at == rpath || rpath == t.server_path.as_deref().unwrap_or(&t.path);
            if !head.deleted && named && self.vault.read_file(at).is_ok_and(|d| hex_hash(&d) == rhash) {
                let server_path = (at != rpath).then(|| rpath.to_string());
                self.state.files.insert(fid.clone(), Tracked { path: at.to_string(), seq: head.seq, hash: rhash, seen: None, server_path, ..t.clone() });
                self.save_base(&fid, at, rdata);
                return Ok(());
            }
            if !head.deleted {
                return Err(SyncError::Local(self.duplicate_reason(t, rpath)));
            }
            self.state.files.insert(fid.clone(), remote_tracked(&t.path));
            let _ = std::fs::remove_file(self.base_path(&fid));
            return Ok(());
        }

        // A remote file that is new to us (or was deleted when we last saw it).
        let Some(st) = st else {
            if head.deleted {
                self.state.files.insert(fid.clone(), remote_tracked(rpath));
                let _ = std::fs::remove_file(self.base_path(&fid));
                return Ok(());
            }
            // A name that is not synced here, as the file syncs under another
            // one (see `regroup`): a copy that another device has of it, with
            // the same content, is that file's duplicate. Nothing is written
            // here: another file there would replace that file.
            if let Some(kept) = self.kept.get(rpath).cloned().filter(|_| self.exists(rpath)).or_else(|| self.linked_name_of(rpath)) {
                if !self.vault.read_file(rpath).is_ok_and(|d| hex_hash(&d) == rhash) {
                    return Err(SyncError::Local(format!(
                        "another device has a different file at this name, but on this device this name is another name of {kept}, through a link, so it is not written here: it would replace {kept}. If it is an older copy of {kept}, copy what you need from it into {kept}, then delete it on a device where it is a separate file; otherwise rename it there"
                    )));
                }
                let original = self.state.files.iter().find(|(_, t)| !t.deleted && t.alias_of.is_none() && t.path == kept).map(|(f, _)| f.clone());
                self.state.files.insert(fid.clone(), Tracked { alias_of: original, ..remote_tracked(rpath) });
                self.save_base(&fid, rpath, rdata);
                return Ok(());
            }
            // Whether the path is free is decided with the vault as it is now,
            // not as scanned: a change earlier in this pull may have moved a
            // file away from it.
            let free = match local.get(rpath) {
                _ if !self.taken(rpath, None) => true,
                Some(lf)
                    if lf.hash == rhash
                        && created.contains(rpath)
                        && !self.state.files.values().any(|t| !t.deleted && t.path == rpath) =>
                {
                    // Same file created on both sides (and not taken for
                    // another file in this pull): adopt it.
                    self.state.files.insert(fid.clone(), remote_tracked(rpath));
                    self.save_base(&fid, rpath, rdata);
                    return Ok(());
                }
                _ => {
                    // The same file, on a file system that ignores case under
                    // a name that differs only in case: ours is kept, and the
                    // rename is pushed. Android shared storage renames ours
                    // instead, and pushes nothing: the phone does not rename
                    // a note on every device (FINDING-172). Not a file in a
                    // folder whose name differs in case, as that would rename
                    // the folder and all of its files (FINDING-034): the
                    // remote file waits below, as a case twin.
                    if !local.contains_key(rpath)
                        && let Some(p) = self.case_twin(rpath, None)
                        && created.contains(&p)
                        && local.get(&p).is_some_and(|lf| lf.hash == rhash)
                        && !self.state.files.values().any(|t| !t.deleted && t.path == p)
                    {
                        if !self.vault.fs().refuses_case_twins() {
                            self.state.files.insert(fid.clone(), remote_tracked(&p));
                            self.save_base(&fid, &p, rdata);
                            return Ok(());
                        }
                        if vpath::parent(&p) == vpath::parent(rpath) {
                            // If the rename fails, the change waits, listed.
                            self.rename(&p, rpath, report)?;
                            // The scan's view of the file goes with it, so
                            // that no later change in this pull takes it.
                            if let Some(lf) = local.remove(&p) {
                                local.insert(rpath.to_string(), lf);
                            }
                            self.state.files.insert(fid.clone(), remote_tracked(rpath));
                            self.save_base(&fid, rpath, rdata);
                            return Ok(());
                        }
                    }
                    false
                }
            };
            // A path taken by other content, at the scan or since: the remote
            // file gets a conflict name, and we tell the server about the
            // rename. Unless the server has the file here at that path too,
            // and got it first: then that one does (see `make_way`). With the
            // same content, they are one file (see `retire`). A case twin on
            // a file system that refuses it waits instead.
            if !free {
                self.refuse_case_twin(rpath, None)?;
            }
            if !free && self.retire(head, rpath, rdata, status)? {
                return Ok(());
            }
            let free = free || self.make_way(rpath, &fid, local, status, report)?;
            let target = if free { self.place(&fid, rpath, rdata, created, report)? } else { self.conflict_copy(rpath, rdata, created, report)? };
            self.state.files.insert(fid.clone(), remote_tracked(&target));
            self.save_base(&fid, &target, rdata);
            return Ok(());
        };
        let old = old.unwrap();
        // A duplicate that took the note's place is behind on the server,
        // not edited: a delete applies while it has the note's content
        // (also under a name it was moved to, see `regroup`).
        let (old, st) = match (&old.took_over, st) {
            (Some(h), Status::Modified) if head.deleted && local.get(&old.path).is_some_and(|lf| lf.hash == *h) => {
                (Tracked { hash: h.clone(), ..old }, Status::Unchanged)
            }
            (_, st) => (old, st),
        };
        // A rename here that the server does not have yet: the file is
        // handled as renamed here from the server's path since the last sync.
        // A remote delete of it, unchanged here, is applied where it is, as
        // when the rename has been uploaded: the name may be one the sync
        // chose (a conflict copy name). One the user gave it in the app keeps
        // it, as a rename the scan finds does. `mine`: the name here is one
        // the user gave it, in the app or found by the scan.
        let mine = old.renamed || matches!(st, Status::Renamed(_));
        let (old, st) = match (old.server_path.clone(), st) {
            (Some(_), Status::Unchanged) if head.deleted && !old.renamed => (old, Status::Unchanged),
            (Some(sp), Status::Unchanged | Status::Modified) => {
                let to = old.path.clone();
                (Tracked { path: sp, server_path: None, ..old }, Status::Renamed(to))
            }
            (Some(sp), Status::Renamed(to)) => (Tracked { path: sp, server_path: None, ..old }, Status::Renamed(to)),
            (_, st) => (old, st),
        };

        // The remote name is another name of this file here, one no other
        // tracked file has (another device renamed its copy to the name of
        // its second copy, after deleting that): nothing moves on disk.
        let same_file = rpath != old.path
            && !head.deleted
            && self.vault.fs().other_names(&old.path).iter().any(|p| p == rpath)
            && !self.state.files.iter().any(|(f, t)| *f != fid && !t.deleted && t.alias_of.is_none() && t.path == rpath);
        match st {
            Status::Unreadable => return Err(SyncError::Local(format!("cannot read {} on this device", old.path))),
            Status::Shared => return Err(SyncError::Local(format!("{} is the same file as another one on this device: their names differ only in case", old.path))),
            Status::Unchanged => {
                if head.deleted
                    && let Some(heir) = self.heir(&fid, &old.path, local)?
                {
                    // Another device deleted this copy of the note but has
                    // a duplicate of it, which is this file here too: the
                    // note stays, and syncs on as that one (FINDING-224).
                    log::info!("sync: {} was deleted elsewhere; it syncs on as its other copy", old.path);
                    self.state.files.insert(fid.clone(), remote_tracked(&old.path));
                    let _ = std::fs::remove_file(self.base_path(&fid));
                    for t in self.state.files.values_mut().filter(|t| t.alias_of.as_deref() == Some(fid.as_str())) {
                        t.alias_of = Some(heir.clone());
                    }
                    if let Some(t) = self.state.files.get_mut(&heir) {
                        t.alias_of = None;
                        t.took_over = Some(old.hash.clone());
                    }
                    return Ok(());
                }
                if same_file {
                    if rhash != old.hash {
                        self.write(&old.path, rdata, Some(&old.hash), report)?;
                    }
                    self.state.files.insert(fid.clone(), remote_tracked(rpath));
                    self.save_base(&fid, rpath, rdata);
                    return Ok(());
                }
                if head.deleted {
                    let others = self.vault.fs().other_names(&old.path);
                    self.delete(&old.path, &old.hash, report)?;
                    self.state.files.insert(fid.clone(), remote_tracked(&old.path));
                    let _ = std::fs::remove_file(self.base_path(&fid));
                    // Deleting a symlink to a note leaves the note: the file
                    // goes under its other names too, as the note was
                    // deleted (through a folder link it is gone already).
                    // (A duplicate there whose own delete comes in this batch
                    // is no reason to keep it.)
                    for o in others {
                        let kept = |(f, t): (&String, &Tracked)| {
                            !t.deleted && t.path == o && !(t.alias_of.as_deref() == Some(fid.as_str()) && self.deleting.contains(f))
                        };
                        if matches!(self.vault.fs().stat(&o), Ok(Some(st)) if st.kind == EntryKind::File) && !self.state.files.iter().any(kept) {
                            self.delete(&o, &old.hash, report)?;
                        }
                    }
                    return Ok(());
                }
                let mut path = old.path.clone();
                if rpath != old.path {
                    let target = if self.taken(rpath, Some(&old.path)) {
                        self.refuse_case_twin(rpath, Some(&old.path))?;
                        let c = self.conflict_path(rpath)?;
                        report.conflicts.push(c.clone());
                        c
                    } else {
                        rpath.to_string()
                    };
                    self.rename(&old.path, &target, report)?;
                    path = target;
                }
                if rhash != old.hash {
                    self.write(&path, rdata, Some(&old.hash), report)?;
                }
                self.state.files.insert(fid.clone(), remote_tracked(&path));
                self.save_base(&fid, &path, rdata);
            }
            Status::Modified => {
                if head.deleted {
                    // Edit beats delete: keep ours; it is revived on push.
                    self.state.files.insert(fid.clone(), remote_tracked(&old.path));
                    let _ = std::fs::remove_file(self.base_path(&fid));
                    return Ok(());
                }
                if same_file {
                    let lf_hash = local.get(&old.path).map(|l| l.hash.clone()).unwrap_or_default();
                    if rhash != lf_hash {
                        self.merge(&fid, &old.path, &lf_hash, rdata, created, report)?;
                    }
                    self.state.files.insert(fid.clone(), remote_tracked(rpath));
                    self.save_base(&fid, rpath, rdata);
                    return Ok(());
                }
                let mut path = old.path.clone();
                if rpath != old.path && !self.taken(rpath, Some(&old.path)) {
                    self.rename(&old.path, rpath, report)?;
                    path = rpath.to_string();
                    // If the merge below fails, the change stays pending: the
                    // edited file must still be known under its new name, or
                    // it would be pushed as a new file.
                    if let Some(t) = self.state.files.get_mut(&fid) {
                        t.path = path.clone();
                        t.server_path = Some(old.path.clone());
                    }
                } else if rpath != old.path {
                    // A name this storage cannot hold here waits, with the
                    // edit: pushing the old name would undo the rename on
                    // every device (FINDING-172).
                    self.refuse_case_twin(rpath, Some(&old.path))?;
                }
                let lf_hash = local.get(&old.path).map(|l| l.hash.clone()).unwrap_or_default();
                if rhash != lf_hash {
                    self.merge(&fid, &path, &lf_hash, rdata, created, report)?;
                }
                // Record the server's version; the local difference is pushed next.
                self.state.files.insert(fid.clone(), remote_tracked(&path));
                self.save_base(&fid, &path, rdata);
            }
            Status::Deleted => {
                // Under a name this device did not know it by: renamed there.
                let renamed = rpath != old.path && old.server_path.as_deref() != Some(rpath);
                if head.deleted || (rhash == old.hash && !renamed) {
                    // Deleted remotely too, or not changed there: our delete
                    // stands (pushed next unless already deleted).
                    let path = if head.deleted { old.path.clone() } else { rpath.to_string() };
                    self.state.files.insert(fid.clone(), remote_tracked(&path));
                    let _ = std::fs::remove_file(self.base_path(&fid));
                    // Its duplicates go with it, as when its delete is pushed
                    // from here (see `regroup`).
                    if head.deleted {
                        for t in self.state.files.values_mut().filter(|t| !t.deleted && !t.detached && t.alias_of.as_deref() == Some(fid.as_str())) {
                            t.delete_with_note = true;
                        }
                    }
                    return Ok(());
                }
                // A new file here with the remote content is that version: a
                // stale state missed the change, or a sync that stopped right
                // after writing it (into the file renamed here, or under a
                // conflict name) could not record it. It is adopted at the
                // remote path, or else where the only such file is (not an
                // empty one), and that name is pushed.
                let same: Vec<&String> = created
                    .iter()
                    .filter(|p| local.get(*p).is_some_and(|lf| lf.hash == rhash))
                    .filter(|p| !self.state.files.iter().any(|(f, t)| *f != fid && !t.deleted && t.path == **p))
                    .collect();
                let found = match same.as_slice() {
                    s if s.iter().any(|p| p.as_str() == rpath) => Some(rpath.to_string()),
                    [p] if !rdata.is_empty() => Some(p.to_string()),
                    _ => None,
                };
                // Only renamed there, and a new file here has the new name: it
                // is this file, moved here too (by the user, or by a sync that
                // stopped before recording it) and edited since. It is taken
                // for the remote version, and the edit is pushed.
                let found = found.or_else(|| {
                    let free = !self.state.files.iter().any(|(f, t)| *f != fid && !t.deleted && t.path == rpath);
                    (rhash == old.hash && created.contains(rpath) && free).then(|| rpath.to_string())
                });
                if let Some(p) = found {
                    self.state.files.insert(fid.clone(), remote_tracked(&p));
                    self.save_base(&fid, &p, rdata);
                    return Ok(());
                }
                // Edited or renamed remotely after we deleted it: the edit or
                // the rename wins, whichever device synced first (see
                // `Status::Renamed`).
                let target = if self.taken(rpath, None) {
                    self.refuse_case_twin(rpath, None)?;
                    self.conflict_copy(rpath, rdata, created, report)?
                } else {
                    self.place(&fid, rpath, rdata, created, report)?
                };
                self.state.files.insert(fid.clone(), remote_tracked(&target));
                self.save_base(&fid, &target, rdata);
            }
            Status::Renamed(to) => {
                if head.deleted {
                    // We renamed it, they deleted it: the rename wins, as
                    // when it reaches the server first (see `Status::Deleted`).
                    // The file keeps its id: the push revives it under its new
                    // name, on top of the delete. Until then it is a rename
                    // made here (`renamed`), which a later remote delete does
                    // not override either.
                    let t = Tracked { path: to, seq: head.seq, server_path: Some(old.path.clone()), seen: None, renamed: true, ..old };
                    self.state.files.insert(fid.clone(), t);
                    return Ok(());
                }
                // Tracked where it is now; if that is not the server's path,
                // the rename is pushed, and until then a name the user gave
                // it stays theirs (`renamed`): a later remote delete does not
                // override it either.
                let tracked = |path: &str| {
                    let t = remote_tracked(path);
                    Tracked { renamed: mine && t.server_path.is_some(), ..t }
                };
                // Rename before writing: if the write fails, the unchanged
                // content still shows where the file went, and it is not
                // taken for a new file.
                let mut path = to.clone();
                if rpath != old.path && rpath != to && !self.taken(rpath, Some(&to)) {
                    // Both renamed: the remote name wins.
                    self.rename(&to, rpath, report)?;
                    path = rpath.to_string();
                } else if rpath != old.path && rpath != to && !mine {
                    // The name here is one the sync chose: it is not pushed
                    // over a remote name this storage cannot hold here, which
                    // waits instead (FINDING-172). One the user gave the file
                    // here is kept and pushed, as when another file has the
                    // remote name.
                    self.refuse_case_twin(rpath, Some(&to))?;
                }
                // A case-only rename on a file system that ignores case is
                // recognised even when the file was edited too; then the
                // edits are merged, as for a modified file.
                let lf_hash = local.get(&to).map(|l| l.hash.clone()).unwrap_or_default();
                if lf_hash != old.hash {
                    // As for a modified file: if the merge fails after the
                    // remote name was taken, the file is known under it, a
                    // name the user did not give it.
                    if path != to
                        && let Some(t) = self.state.files.get_mut(&fid)
                    {
                        t.path = path.clone();
                        t.server_path = Some(old.path.clone());
                        t.renamed = false;
                    }
                    if rhash != lf_hash {
                        self.merge(&fid, &path, &lf_hash, rdata, created, report)?;
                    }
                    self.state.files.insert(fid.clone(), tracked(&path));
                    self.save_base(&fid, &path, rdata);
                    return Ok(());
                }
                if rhash != old.hash {
                    self.write(&path, rdata, Some(&old.hash), report)?;
                }
                self.state.files.insert(fid.clone(), tracked(&path));
                self.save_base(&fid, &path, rdata);
            }
        }
        Ok(())
    }

    // ---------- history ----------

    /// The file at `path` now; deleted files that had this name before are
    /// not it.
    fn file_id_for(&self, path: &str) -> Option<String> {
        self.synced_at(path).map(|(f, _)| f).or_else(|| self.renamed_to(path))
    }

    /// The tracked file that syncs the file at `path` now, with its id: the
    /// one at `path`, or what that one duplicates (see `Tracked::alias_of`),
    /// or the one at another name of the same file here.
    fn synced_at(&self, path: &str) -> Option<(String, Tracked)> {
        let live = |p: &str| self.state.files.iter().find(|(_, t)| !t.deleted && t.path == p).map(|(f, t)| (f.clone(), t.clone()));
        let original = |o: &String| self.state.files.get(o).filter(|t| !t.deleted).map(|t| (o.clone(), t.clone()));
        match live(path) {
            Some((_, t)) if t.alias_of.is_some() => t.alias_of.as_ref().and_then(original),
            Some(found) => Some(found),
            None => self.vault.fs().other_names(path).iter().filter_map(|p| live(p)).find(|(_, t)| t.alias_of.is_none()),
        }
    }

    /// A tracked file renamed to `path` since the last sync: its old path is
    /// gone and `path` holds exactly its synced content (the test the next
    /// sync uses to push it as a rename).
    fn renamed_to(&self, path: &str) -> Option<String> {
        let hash = hex_hash(&self.vault.read_file(path).ok()?);
        let gone = |p: &str| matches!(self.vault.fs().stat(p), Ok(None));
        self.state.files.iter().find(|(_, t)| !t.deleted && t.hash == hash && gone(&t.path)).map(|(f, _)| f.clone())
    }

    /// Server-side revisions of a file, newest first.
    pub fn history(&self, path: &str) -> Result<Vec<HistoryEntry>, SyncError> {
        let fid = self.file_id_for(path).ok_or_else(|| SyncError::Local(format!("{path} has not been synced yet")))?;
        self.transport.history(&self.settings.vault_id, &fid)
    }

    /// Content of one revision (decrypted).
    pub fn revision_content(&self, seq: u64) -> Result<FilePayload, SyncError> {
        let r = self.transport.revision(&self.settings.vault_id, seq)?;
        self.decode_revision(&r)
    }

    fn decode_revision(&self, r: &RevisionBlob) -> Result<FilePayload, SyncError> {
        let blob = unb64(&r.blob).ok_or_else(|| SyncError::Server("bad blob".into()))?;
        FilePayload::decode(&self.key.decrypt(&r.file_id, &blob)?)
    }

    /// Put an old revision's content back into the file (the next sync
    /// uploads it as a new revision). Only text the server already has is
    /// replaced: if the file changed since this device last synced it, this
    /// refuses, so sync first and the current text stays in the history.
    pub fn restore(&self, path: &str, seq: u64) -> Result<Vec<Change>, SyncError> {
        let not_synced = || SyncError::Local(format!("{path} has changes that are not synced yet, so nothing was restored"));
        let (_, synced) = self.synced_at(path).filter(|(_, t)| !t.hash.is_empty()).ok_or_else(not_synced)?;
        let r = self.transport.revision(&self.settings.vault_id, seq)?;
        if r.deleted {
            return Err(SyncError::Local("that version is a deletion; it has no text to restore".into()));
        }
        let p = self.decode_revision(&r)?;
        match self.vault.write_file(path, &p.data, Some(&synced.hash)) {
            Ok(w) => Ok(w.changes),
            Err(CoreError::Conflict(_)) => Err(not_synced()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{conflict_name, edits_at_least, merge_text, stamp, SyncState, CONFLICT_NAME_MAX};

    #[test]
    fn state_from_older_builds_loads() {
        let s: SyncState = serde_json::from_str(r#"{"last_seq": 7, "files": {}}"#).unwrap();
        assert_eq!(s.last_seq, 7);
        assert!(s.pending.is_empty());
        let s: SyncState =
            serde_json::from_str(r#"{"last_seq": 7, "files": {"f": {"path": "a.md", "seq": 3, "hash": "ab", "deleted": false}}}"#).unwrap();
        assert_eq!(s.files["f"].server_path, None);
        // Their size and mtime may describe newer bytes than the hash: the
        // file is hashed again.
        let s: SyncState = serde_json::from_str(
            r#"{"last_seq": 7, "files": {"f": {"path": "a.md", "seq": 3, "hash": "ab", "deleted": false, "size": 2, "mtime": 9}}}"#,
        )
        .unwrap();
        assert_eq!(s.files["f"].seen, None);
    }

    #[test]
    fn large_rewrites_are_not_merged() {
        assert_eq!(edits_at_least("a\nb\nc\nd\n", "a\nx\ny\nd\n"), 4);
        assert_eq!(edits_at_least("a\nb\n", "a\nnew\nb\n"), 0);
        let lines = |n: usize, line: &dyn Fn(usize) -> String| (0..n).map(line).collect::<String>();
        let base = lines(20_000, &|i| format!("base line {i}\n"));
        let crlf = lines(20_000, &|i| format!("base line {i}\r\n"));
        let theirs = lines(20_000, &|i| format!("theirs line {i}\n"));
        assert_eq!(merge_text(&base, &crlf, &theirs), None);
        // one side unchanged: the other side, however much it changed
        assert_eq!(merge_text(&base, &crlf, &base).as_deref(), Some(crlf.as_str()));
        assert_eq!(merge_text(&base, &base, &theirs).as_deref(), Some(theirs.as_str()));
        // many lines added on one side, one edited on the other: quick to
        // diff, so merged
        let pasted = format!("{base}{}", lines(30_000, &|i| format!("pasted {i}\n")));
        let edited = base.replacen("base line 5\n", "edited\n", 1);
        assert_eq!(merge_text(&base, &pasted, &edited), Some(pasted.replacen("base line 5\n", "edited\n", 1)));
    }

    #[test]
    fn stamps() {
        assert_eq!(stamp(0), "1970-01-01 0000");
        assert_eq!(stamp(1_790_985_600 + 3_600 * 15 + 60 * 30), "2026-10-03 1530");
        assert_eq!(stamp(951_782_400), "2000-02-29 0000");
    }

    #[test]
    fn conflict_names() {
        let at = "2026-10-03 1530";
        let n = |path: &str, device: &str| conflict_name(path, at, device);
        assert_eq!(n("a/n.md", "phone"), ("n (conflict 2026-10-03 1530 phone)".into(), "md".into()));
        assert_eq!(n("Makefile", "phone").1, "");
        assert_eq!(n("x.tar.GZ", "phone"), ("x.tar (conflict 2026-10-03 1530 phone)".into(), "GZ".into()));
        // characters some file systems or links refuse are replaced
        assert_eq!(n("n.md", "Sam's Pixel? [old]").0, "n (conflict 2026-10-03 1530 Sam's Pixel- -old-)");
        assert_eq!(n("n.md", "  ").0, "n (conflict 2026-10-03 1530)");
        // long names are shortened on a char boundary (this note's name is
        // 228 bytes, its conflict copy would be 261 unshortened)
        let long = format!("{}.md", "日本語".repeat(25));
        for device in ["phone", &"d".repeat(300), &"機".repeat(100)] {
            let (base, ext) = n(&long, device);
            let name = format!("{base}.{ext}");
            assert!(name.len() <= CONFLICT_NAME_MAX, "{} bytes: {name}", name.len());
            assert!(base.starts_with("日本語日本語") && base.contains(" (conflict 2026-10-03 1530 "), "{base}");
            assert!(cairn_core::path::validate_name(&name).is_ok(), "{name}");
        }
        // long text after the last dot is not an extension: it is shortened
        // with the rest of the name
        let tail = format!("x.{}", "a".repeat(200));
        for path in [format!("Mr. {}", "a".repeat(215)), tail.clone(), format!("{tail}.md")] {
            for device in ["phone", &"d".repeat(300)] {
                let (base, ext) = n(&path, device);
                let name = if ext.is_empty() { base.clone() } else { format!("{base}.{ext}") };
                assert!(name.len() <= CONFLICT_NAME_MAX, "{} bytes: {name}", name.len());
                assert!(base.starts_with(&path[..2]), "{base}");
                assert!(cairn_core::path::validate_name(&name).is_ok(), "{name}");
            }
        }
        assert_eq!(n(&format!("x.{}", "a".repeat(32)), "phone").1, "a".repeat(32));
        assert_eq!(n(&format!("x.{}", "a".repeat(33)), "phone").1, "");
    }
}
