package app.cairn.notes

// Storage Access Framework bridge for Cairn vaults in user-picked folders.
//
// The Rust side (src-tauri/src/android.rs, `SafFs`) calls these commands with
// a tree URI and a vault-relative path ("folder/note.md"). Paths are resolved
// to document ids by walking the tree; results are cached per tree.
// Work runs on a background thread so large folders do not block the UI.

import android.app.Activity
import android.content.Intent
import android.database.Cursor
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.util.Base64
import androidx.activity.result.ActivityResult
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.FileNotFoundException
import java.io.FileOutputStream
import java.text.Normalizer
import java.util.concurrent.Executors

@InvokeArg
class PathArgs {
    var tree: String = ""
    var path: String = ""
}

@InvokeArg
class WriteArgs {
    var tree: String = ""
    var path: String = ""
    var data: String = ""
}

@InvokeArg
class MoveArgs {
    var tree: String = ""
    var from: String = ""
    var to: String = ""
}

private class Doc(val id: String, val name: String, val mime: String, val size: Long, val mtime: Long) {
    val isDir get() = mime == Document.MIME_TYPE_DIR
}

/**
 * The document found for a vault path: its id and its name on disk (NFC).
 * `exact` is false when a name on the way matched only ignoring case, so
 * the path is not how the document is named and must not be cached.
 */
private class Found(val id: String, val name: String, val exact: Boolean)

/** NFC form of a name: vault paths are NFC, names on disk may be NFD (made on macOS). */
private fun nfc(s: String): String = Normalizer.normalize(s, Normalizer.Form.NFC)

@TauriPlugin
class SafPlugin(private val activity: Activity) : Plugin(activity) {
    private val io = Executors.newSingleThreadExecutor()
    // "<tree>\u0000<path>" -> document id
    private val ids = HashMap<String, String>()
    private val resolver get() = activity.contentResolver

    private val projection = arrayOf(
        Document.COLUMN_DOCUMENT_ID,
        Document.COLUMN_DISPLAY_NAME,
        Document.COLUMN_MIME_TYPE,
        Document.COLUMN_SIZE,
        Document.COLUMN_LAST_MODIFIED,
    )

    private fun background(invoke: Invoke, work: () -> JSObject) {
        io.execute {
            try {
                invoke.resolve(work())
            } catch (e: FileNotFoundException) {
                invoke.reject("not found: ${e.message}")
            } catch (e: Exception) {
                invoke.reject(e.message ?: e.toString())
            }
        }
    }

    // ---------- helpers ----------

    private fun readDoc(c: Cursor) = Doc(
        c.getString(0),
        c.getString(1) ?: "",
        c.getString(2) ?: "",
        if (c.isNull(3)) 0 else c.getLong(3),
        if (c.isNull(4)) 0 else c.getLong(4),
    )

    private fun children(tree: Uri, parentId: String): List<Doc> {
        val uri = DocumentsContract.buildChildDocumentsUriUsingTree(tree, parentId)
        val out = ArrayList<Doc>()
        resolver.query(uri, projection, null, null, null)?.use { c ->
            while (c.moveToNext()) out.add(readDoc(c))
        }
        return out
    }

    private fun docInfo(tree: Uri, id: String): Doc? {
        val uri = DocumentsContract.buildDocumentUriUsingTree(tree, id)
        return try {
            resolver.query(uri, projection, null, null, null)?.use { c -> if (c.moveToFirst()) readDoc(c) else null }
        } catch (e: Exception) {
            null
        }
    }

    private fun key(tree: String, path: String) = "$tree\u0000$path"

    private fun parentOf(path: String) = path.substringBeforeLast('/', "")

    private fun nameOf(path: String) = path.substringAfterLast('/')

    private fun join(dir: String, name: String) = if (dir.isEmpty()) name else "$dir/$name"

    /**
     * The document at a vault path, or null if there is none. A name matches
     * a child with the same NFC form, or else one that differs only in case:
     * shared storage ignores case, so that child is the only one the path can
     * mean there (and a new one could not be created next to it).
     */
    private fun find(treeStr: String, path: String): Found? {
        val tree = Uri.parse(treeStr)
        if (path.isEmpty()) {
            // The picked folder keeps its tree URI after it is renamed, moved or
            // deleted, and ExternalStorageProvider then lists it as empty. Check
            // it is still there, or the vault would look emptied (and sync would
            // delete every note on the other devices).
            val root = DocumentsContract.getTreeDocumentId(tree)
            return if (docInfo(tree, root)?.isDir == true) Found(root, "", true) else null
        }
        val name = nameOf(path)
        ids[key(treeStr, path)]?.let { id ->
            if (docInfo(tree, id)?.name?.let(::nfc) == name) return Found(id, name, true)
            ids.remove(key(treeStr, path))
        }
        val parent = find(treeStr, parentOf(path)) ?: return null
        val kids = children(tree, parent.id)
        // Only paths spelled as on disk are cached.
        if (parent.exact) for (d in kids) ids[key(treeStr, join(parentOf(path), nfc(d.name)))] = d.id
        val d = kids.firstOrNull { nfc(it.name) == name }
            ?: kids.firstOrNull { nfc(it.name).equals(name, ignoreCase = true) }
            ?: return null
        return Found(d.id, nfc(d.name), parent.exact && nfc(d.name) == name)
    }

    /** Document id for a vault path, or null if it does not exist. */
    private fun resolve(treeStr: String, path: String): String? = find(treeStr, path)?.id

    /** Cached document id for `path` if it is still valid; never lists a folder. */
    private fun cached(treeStr: String, path: String): String? {
        val id = ids[key(treeStr, path)] ?: return null
        return id.takeIf { docInfo(Uri.parse(treeStr), it)?.name?.let(::nfc) == nameOf(path) }
    }

    private fun forget(treeStr: String, path: String) {
        val prefix = key(treeStr, path)
        ids.keys.removeAll { it == prefix || it.startsWith("$prefix/") }
    }

    private fun entry(path: String, d: Doc) = JSObject().apply {
        put("path", path)
        put("dir", d.isDir)
        put("size", d.size)
        put("mtime", d.mtime)
    }

    /**
     * The folder at `path`, created with any missing parents. A folder whose
     * name differs only in case counts: moving a note out of it into `path`
     * renames it (see renameSharedFolders); other writes and moves into it
     * are refused.
     */
    private fun mkdirs(treeStr: String, path: String): Found {
        val tree = Uri.parse(treeStr)
        find(treeStr, path)?.let { return it }
        if (path.isEmpty()) throw FileNotFoundException("notebook folder")
        val parent = mkdirs(treeStr, parentOf(path))
        val parentUri = DocumentsContract.buildDocumentUriUsingTree(tree, parent.id)
        val created = DocumentsContract.createDocument(resolver, parentUri, Document.MIME_TYPE_DIR, nameOf(path))
            ?: throw Exception("cannot create folder $path")
        requireName(tree, created, path, byCreate = true)
        val id = DocumentsContract.getDocumentId(created)
        if (parent.exact) ids[key(treeStr, path)] = id
        return Found(id, nameOf(path), parent.exact)
    }

    /**
     * Make sure a document this plugin has just created (or renamed into
     * place) got exactly the name asked for. Providers change names they
     * cannot store (characters FAT forbids, a trailing dot, more than 255
     * bytes) and add " (1)" to a name that is taken ignoring case; the vault
     * path would then not find the document. It is removed again, and the
     * write refused. `byCreate` is true for what createDocument handed back:
     * a provider might hand back the document already there under the name
     * it made of ours, so only an empty one not listed before is removed.
     */
    private fun requireName(tree: Uri, uri: Uri, path: String, byCreate: Boolean) {
        val real = docInfo(tree, DocumentsContract.getDocumentId(uri))
        if (real != null && nfc(real.name) == nfc(nameOf(path))) return
        val isNew = real == null || !ids.containsValue(real.id) &&
            (if (real.isDir) children(tree, real.id).isEmpty() else real.size == 0L)
        if (!byCreate || isNew) deleteQuietly(uri)
        throw Exception(if (real == null) "cannot create $path" else nameRefused(real.name))
    }

    private fun nameRefused(real: String) = "name not allowed on this storage (it would become \"$real\")"

    /**
     * Write `bytes` over a document without ever making it shorter first. SAF
     * cannot replace a file in one step, and opening with "rwt" empties it
     * before the new bytes arrive: a process ended mid-write (Back ends it)
     * left an empty note. So write from the start, then cut off what is left
     * of the old content past the new end.
     */
    private fun overwrite(uri: Uri, bytes: ByteArray, path: String) {
        resolver.openFileDescriptor(uri, "rw")?.use { pfd ->
            FileOutputStream(pfd.fileDescriptor).use { out ->
                out.channel.position(0)
                out.write(bytes)
                out.channel.truncate(bytes.size.toLong())
                out.fd.sync()
            }
        } ?: throw Exception("cannot open $path for writing")
    }

    /**
     * Create the note at `path` with `bytes` in it; returns its document id.
     * The bytes go into a hidden temp document that is renamed into place
     * once complete, so a kill half-way leaves no empty note for the next
     * scan or sync to take for a real one. A temp left by such a kill is
     * reused by the next write of the same note.
     */
    private fun create(treeStr: String, parent: Found, path: String, bytes: ByteArray): String {
        val tree = Uri.parse(treeStr)
        val parentUri = DocumentsContract.buildDocumentUriUsingTree(tree, parent.id)
        val tmpName = ".${nameOf(path)}.cairn-tmp"
        val tmpPath = if (parentOf(path).isEmpty()) tmpName else "${parentOf(path)}/$tmpName"
        // Looking up `path` has just listed the folder, so a leftover temp is cached.
        val tmpUri = cached(treeStr, tmpPath)?.let { DocumentsContract.buildDocumentUriUsingTree(tree, it) }
            ?: try {
                // octet-stream keeps the file name exactly as given (no added extension)
                DocumentsContract.createDocument(resolver, parentUri, "application/octet-stream", tmpName)
            } catch (e: Exception) {
                null
            }
        if (tmpUri != null) {
            ids.remove(key(treeStr, tmpPath))
            try {
                overwrite(tmpUri, bytes, path)
            } catch (e: Exception) {
                deleteQuietly(tmpUri)
                throw e
            }
            val renamed = try {
                DocumentsContract.renameDocument(resolver, tmpUri, nameOf(path))
            } catch (e: Exception) {
                null
            }
            if (renamed != null) {
                requireName(tree, renamed, path, byCreate = false)
                val id = DocumentsContract.getDocumentId(renamed)
                if (parent.exact) ids[key(treeStr, path)] = id
                return id
            }
            // This storage cannot rename the temp (or not to this name): write the note directly.
            deleteQuietly(tmpUri)
        }
        val created = DocumentsContract.createDocument(resolver, parentUri, "application/octet-stream", nameOf(path))
            ?: throw Exception("cannot create $path")
        requireName(tree, created, path, byCreate = true)
        try {
            overwrite(created, bytes, path)
        } catch (e: Exception) {
            deleteQuietly(created) // never leave an empty note behind
            throw e
        }
        val id = DocumentsContract.getDocumentId(created)
        if (parent.exact) ids[key(treeStr, path)] = id
        return id
    }

    /**
     * `name`, or else the first free "stem (n).ext", such that no child of
     * these folders has it, ignoring case (shared storage is case-insensitive).
     */
    private fun freeName(tree: Uri, parentIds: List<String>, name: String): String {
        val taken = parentIds.flatMap { children(tree, it) }.map { nfc(it.name).lowercase() }.toSet()
        val dot = name.lastIndexOf('.').takeIf { it > 0 } ?: name.length
        var candidate = name
        var n = 1
        while (candidate.lowercase() in taken) {
            candidate = "${name.substring(0, dot)} ($n)${name.substring(dot)}"
            n++
        }
        return candidate
    }

    /**
     * Rename a document to exactly `name`. If the provider picks another name
     * (see requireName), the rename is undone and refused.
     */
    private fun rename(tree: Uri, uri: Uri, name: String, path: String): Uri {
        val old = docInfo(tree, DocumentsContract.getDocumentId(uri))?.name
        val out = DocumentsContract.renameDocument(resolver, uri, name) ?: throw Exception("cannot rename $path")
        val real = docInfo(tree, DocumentsContract.getDocumentId(out))?.name ?: throw Exception("cannot rename $path")
        if (nfc(real) == nfc(name)) return out
        if (old != null) {
            try {
                DocumentsContract.renameDocument(resolver, out, old)
            } catch (ignored: Exception) {
            }
        }
        throw Exception(nameRefused(real))
    }

    /**
     * Rename a document in folder `parentId` to `name`, which differs from its
     * name `old` only in case. Shared storage takes the new name for a taken
     * one and would add " (1)", so go through a free name.
     */
    private fun renameCase(tree: Uri, parentId: String, uri: Uri, old: String, name: String, path: String): Uri {
        val via = rename(tree, uri, freeName(tree, listOf(parentId), name), path)
        return try {
            rename(tree, via, name, path)
        } catch (e: Exception) {
            try {
                rename(tree, via, old, path)
            } catch (ignored: Exception) {
            }
            throw e
        }
    }

    /**
     * Folders that both paths go through but `to` spells in another case are
     * one folder on this storage: rename them to that case. A folder renamed
     * by case on another device arrives through sync like this, note by note
     * ("Notes/a.md" -> "notes/a.md"), and the new paths must name it. Each
     * folder renamed is added to `done` as (new path, old name).
     */
    private fun renameSharedFolders(treeStr: String, from: String, to: String, done: MutableList<Pair<String, String>>) {
        val tree = Uri.parse(treeStr)
        val a = parentOf(from).split('/')
        val b = parentOf(to).split('/')
        var parentId = resolve(treeStr, "") ?: return
        for (i in 0 until minOf(a.size, b.size)) {
            if (a[i].isEmpty() || !a[i].equals(b[i], ignoreCase = true)) return
            val path = a.subList(0, i + 1).joinToString("/")
            val newPath = b.subList(0, i + 1).joinToString("/")
            val dir = find(treeStr, path) ?: return
            // Storage that keeps case can have two folders here.
            if (resolve(treeStr, newPath) != dir.id) return
            var id = dir.id
            if (a[i] != b[i] && dir.name != b[i]) {
                val uri = renameCase(tree, parentId, DocumentsContract.buildDocumentUriUsingTree(tree, id), dir.name, b[i], path)
                id = DocumentsContract.getDocumentId(uri)
                forget(treeStr, path)
                forget(treeStr, newPath)
                done.add(newPath to dir.name)
            }
            parentId = id
        }
    }

    /** Give the folders renameSharedFolders renamed their old names back, deepest first. */
    private fun restoreFolders(treeStr: String, done: List<Pair<String, String>>) {
        val tree = Uri.parse(treeStr)
        for ((path, old) in done.asReversed()) {
            try {
                val id = resolve(treeStr, path) ?: continue
                val parentId = resolve(treeStr, parentOf(path)) ?: continue
                renameCase(tree, parentId, DocumentsContract.buildDocumentUriUsingTree(tree, id), nameOf(path), old, path)
            } catch (ignored: Exception) {
            }
            forget(treeStr, path)
        }
    }

    /** Remove a document this plugin has just created; a failure leaves it as it is. */
    private fun deleteQuietly(uri: Uri) {
        try {
            DocumentsContract.deleteDocument(resolver, uri)
        } catch (e: Exception) {
        }
    }

    // ---------- commands ----------

    @Command
    fun pickFolder(invoke: Invoke) {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or Intent.FLAG_GRANT_PREFIX_URI_PERMISSION,
            )
        }
        startActivityForResult(invoke, intent, "folderPicked")
    }

    @ActivityCallback
    fun folderPicked(invoke: Invoke, result: ActivityResult) {
        val uri = result.data?.data
        if (result.resultCode != Activity.RESULT_OK || uri == null) {
            invoke.resolve(JSObject())
            return
        }
        try {
            resolver.takePersistableUriPermission(
                uri,
                Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION,
            )
        } catch (e: SecurityException) {
            invoke.reject("cannot keep access to this folder: ${e.message}")
            return
        }
        val name = docInfo(uri, DocumentsContract.getTreeDocumentId(uri))?.name ?: "Notebook"
        invoke.resolve(JSObject().apply {
            put("uri", uri.toString())
            put("name", name)
        })
    }

    /** Whether a picked tree can be opened again (after a restart): the access
     *  taken in folderPicked is still held and the folder is still there. */
    @Command
    fun canOpen(invoke: Invoke) {
        val a = invoke.parseArgs(PathArgs::class.java)
        background(invoke) {
            val tree = Uri.parse(a.tree)
            val kept = resolver.persistedUriPermissions.any { it.uri == tree && it.isReadPermission && it.isWritePermission }
            JSObject().apply { put("ok", kept && find(a.tree, "") != null) }
        }
    }

    @Command
    fun list(invoke: Invoke) {
        val a = invoke.parseArgs(PathArgs::class.java)
        background(invoke) {
            val tree = Uri.parse(a.tree)
            val out = JSArray()
            // ExternalStorageProvider's ids are paths. On storage that ignores
            // case, an id spelled in an old case still finds the document and
            // reports that spelling as its name, so a cached id outlives a
            // case-only rename made by another app. A scan of the whole vault
            // (every sync starts with one) caches every id afresh.
            if (a.path.isEmpty()) ids.keys.removeAll { it.startsWith(key(a.tree, "")) }
            val start = find(a.tree, a.path) ?: throw FileNotFoundException(a.path)
            fun walk(id: String, path: String) {
                for (d in children(tree, id)) {
                    if (d.name.startsWith(".")) continue
                    val p = join(path, nfc(d.name))
                    if (start.exact) ids[key(a.tree, p)] = d.id
                    out.put(entry(p, d))
                    if (d.isDir) walk(d.id, p)
                }
            }
            walk(start.id, a.path)
            JSObject().apply { put("entries", out) }
        }
    }

    @Command
    fun stat(invoke: Invoke) {
        val a = invoke.parseArgs(PathArgs::class.java)
        background(invoke) {
            val id = resolve(a.tree, a.path)
            val d = id?.let { docInfo(Uri.parse(a.tree), it) }
            JSObject().apply { put("entry", if (d == null) null else entry(a.path, d)) }
        }
    }

    @Command
    fun read(invoke: Invoke) {
        val a = invoke.parseArgs(PathArgs::class.java)
        background(invoke) {
            val id = resolve(a.tree, a.path) ?: throw FileNotFoundException(a.path)
            val uri = DocumentsContract.buildDocumentUriUsingTree(Uri.parse(a.tree), id)
            val bytes = resolver.openInputStream(uri)?.use { it.readBytes() } ?: throw FileNotFoundException(a.path)
            JSObject().apply { put("data", Base64.encodeToString(bytes, Base64.NO_WRAP)) }
        }
    }

    @Command
    fun write(invoke: Invoke) {
        val a = invoke.parseArgs(WriteArgs::class.java)
        background(invoke) {
            val tree = Uri.parse(a.tree)
            val bytes = Base64.decode(a.data, Base64.NO_WRAP)
            val parent = find(a.tree, parentOf(a.path)) ?: throw FileNotFoundException(parentOf(a.path))
            // A folder whose name differs only in case is another folder (a
            // device that keeps case can have both): never write into it.
            if (!parent.exact) throw Exception("already exists: ${parentOf(a.path)}")
            val found = find(a.tree, a.path)
            val id = when {
                found == null -> create(a.tree, parent, a.path, bytes)
                found.name == nameOf(a.path) -> found.id.also { overwrite(DocumentsContract.buildDocumentUriUsingTree(tree, it), bytes, a.path) }
                // Another file whose name differs only in case: this storage cannot hold both.
                else -> throw Exception("already exists: ${a.path}")
            }
            val d = docInfo(tree, id) ?: throw FileNotFoundException(a.path)
            JSObject().apply { put("entry", entry(a.path, d)) }
        }
    }

    @Command
    fun mkdirs(invoke: Invoke) {
        val a = invoke.parseArgs(PathArgs::class.java)
        background(invoke) {
            val id = mkdirs(a.tree, a.path).id
            val d = docInfo(Uri.parse(a.tree), id)
            JSObject().apply { put("entry", if (d == null) null else entry(a.path, d)) }
        }
    }

    @Command
    fun move(invoke: Invoke) {
        val a = invoke.parseArgs(MoveArgs::class.java)
        background(invoke) {
            val tree = Uri.parse(a.tree)
            val first = resolve(a.tree, a.from) ?: throw FileNotFoundException(a.from)
            // Shared storage ignores case, so `to` may find this same document
            // (a case-only rename). Any other document there is in the way.
            val taken = resolve(a.tree, a.to)
            if (taken != null && taken != first) throw Exception("already exists: ${a.to}")
            var dir = ""
            for (part in parentOf(a.to).split('/').filter { it.isNotEmpty() }) {
                dir = join(dir, part)
                if (resolve(a.tree, dir) == first) throw Exception("cannot move a folder into itself")
            }
            val renamed = ArrayList<Pair<String, String>>()
            try {
                renameSharedFolders(a.tree, a.from, a.to, renamed)
                // Renaming a folder can change the ids of the documents in it.
                val id = resolve(a.tree, a.from) ?: throw FileNotFoundException(a.from)
                var uri = DocumentsContract.buildDocumentUriUsingTree(tree, id)
                val oldName = docInfo(tree, id)?.name ?: throw FileNotFoundException(a.from)
                val newName = nameOf(a.to)
                val srcParentId = resolve(a.tree, parentOf(a.from)) ?: throw FileNotFoundException(parentOf(a.from))
                val dst = find(a.tree, parentOf(a.to)) ?: throw FileNotFoundException(parentOf(a.to))
                // A folder that matched only ignoring case, and is not one the
                // document is in (those were renamed above), is another folder:
                // never move into it (see write).
                if (!dst.exact) throw Exception("already exists: ${parentOf(a.to)}")
                if (srcParentId != dst.id) {
                    val srcParent = DocumentsContract.buildDocumentUriUsingTree(tree, srcParentId)
                    val dstParent = DocumentsContract.buildDocumentUriUsingTree(tree, dst.id)
                    // moveDocument keeps the name, and the destination may already
                    // hold the old one (a note deleted into .trash a second time).
                    // So rename first, in place: to the new name when neither folder
                    // has it, else to a free "name (n)"; then move, then rename to
                    // the new name. On failure, put the document back as it was.
                    val via = if (nfc(oldName) == newName) oldName else freeName(tree, listOf(srcParentId, dst.id), newName)
                    if (via != oldName) uri = rename(tree, uri, via, a.from)
                    var moved = false
                    try {
                        uri = DocumentsContract.moveDocument(resolver, uri, srcParent, dstParent)
                            ?: throw Exception("this storage provider cannot move ${a.from}")
                        moved = true
                        if (nfc(via) != newName) uri = rename(tree, uri, newName, a.from)
                    } catch (e: Exception) {
                        try {
                            val back = if (moved) DocumentsContract.moveDocument(resolver, uri, dstParent, srcParent) else uri
                            if (back != null && via != oldName) rename(tree, back, oldName, a.from)
                        } catch (ignored: Exception) {
                        }
                        forget(a.tree, a.from)
                        throw e
                    }
                } else if (nfc(oldName) != newName) {
                    uri = if (nfc(oldName).equals(newName, ignoreCase = true)) {
                        renameCase(tree, srcParentId, uri, oldName, newName, a.from)
                    } else {
                        rename(tree, uri, newName, a.from)
                    }
                }
                forget(a.tree, a.from)
                forget(a.tree, a.to)
                ids[key(a.tree, a.to)] = DocumentsContract.getDocumentId(uri)
            } catch (e: Exception) {
                restoreFolders(a.tree, renamed)
                throw e
            }
            JSObject()
        }
    }

    @Command
    fun removeEmptyDir(invoke: Invoke) {
        val a = invoke.parseArgs(PathArgs::class.java)
        background(invoke) {
            val tree = Uri.parse(a.tree)
            val id = resolve(a.tree, a.path)
            var removed = false
            if (id != null && a.path.isNotEmpty() && children(tree, id).isEmpty()) {
                removed = DocumentsContract.deleteDocument(resolver, DocumentsContract.buildDocumentUriUsingTree(tree, id))
                forget(a.tree, a.path)
            }
            JSObject().apply { put("removed", removed) }
        }
    }
}
