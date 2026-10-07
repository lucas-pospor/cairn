package app.cairn.notes

// Local network permission for sync.
//
// Android 17 blocks apps that target SDK 37 from addresses on the local
// network unless they hold ACCESS_LOCAL_NETWORK ("Nearby devices" in
// Android settings); a blocked connection just times out. The Rust side
// (src-tauri/src/android.rs, `allow_server`) calls `localNetwork` before
// sync connects to a server on the local network.

import android.app.Activity
import android.os.Build
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

private const val ALIAS = "localNetwork"

@InvokeArg
class LocalNetworkArgs {
    /** Show Android's prompt when the permission is not granted. */
    var ask: Boolean = false
}

@TauriPlugin(permissions = [Permission(strings = ["android.permission.ACCESS_LOCAL_NETWORK"], alias = ALIAS)])
class LocalNetworkPlugin(activity: Activity) : Plugin(activity) {
    /**
     * Answers {state}: "granted"; "prompt" when Android can still ask the
     * user; "denied" when the user said no and Android no longer asks (only
     * Android settings can allow it then). With `ask`, a permission that is
     * not granted is requested first, and the answer is the state after.
     */
    @Command
    fun localNetwork(invoke: Invoke) {
        val args = invoke.parseArgs(LocalNetworkArgs::class.java)
        // Without the @TauriPlugin permission (stripped from a build), a
        // request would never answer and sync would wait forever.
        val now = state() ?: return invoke.reject("the local network permission is not declared")
        if (now == "granted" || !args.ask) return answer(invoke, now)
        // Commands run on the main thread, which the permission request needs.
        requestPermissionForAlias(ALIAS, invoke, "localNetworkAnswered")
    }

    @PermissionCallback
    private fun localNetworkAnswered(invoke: Invoke) = answer(invoke, state() ?: "prompt")

    private fun answer(invoke: Invoke, state: String) {
        val ret = JSObject()
        ret.put("state", state)
        invoke.resolve(ret)
    }

    private fun state(): String? {
        // Before Android 17 the permission does not exist: INTERNET is enough.
        if (Build.VERSION.SDK_INT < 37) return "granted"
        return when (getPermissionState(ALIAS)) {
            null -> null
            PermissionState.GRANTED -> "granted"
            PermissionState.DENIED -> "denied"
            else -> "prompt"
        }
    }
}
