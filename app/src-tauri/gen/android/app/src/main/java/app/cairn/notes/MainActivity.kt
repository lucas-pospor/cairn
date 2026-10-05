package app.cairn.notes

import android.content.res.Configuration
import android.os.Bundle
import android.os.SystemClock
import android.view.ContextThemeWrapper
import android.view.View
import android.view.ViewGroup
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  private var webView: WebView? = null
  private var nightMode = 0

  // Back first closes what the page has open on top (menu, dialog, settings,
  // quick switcher, drawer). The page answers window.__cairnBack() with true
  // when it took care of Back; with nothing open it saves pending edits and
  // then calls CairnAndroid.leave().
  private val backCallback = object : OnBackPressedCallback(true) {
    override fun handleOnBackPressed() {
      val view = webView ?: return leave()
      view.evaluateJavascript("window.__cairnBack ? window.__cairnBack() : false") { handled ->
        if (handled != "true") leave()
      }
    }
  }

  /** Back as the system handles it: leave the app. */
  private fun leave() {
    backCallback.isEnabled = false
    onBackPressedDispatcher.onBackPressed()
    backCallback.isEnabled = true
  }

  inner class BackBridge {
    @JavascriptInterface
    fun leave() = runOnUiThread { this@MainActivity.leave() }
  }

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    nightMode = resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK
    onBackPressedDispatcher.addCallback(this, backCallback)
    // The web view does not get system bar insets as CSS safe areas, so keep
    // it clear of the status bar, navigation bar and on-screen keyboard here.
    val root = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(root) { v, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      )
      v.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }

  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    // Called before the page loads, so the page sees it from the start.
    webView.addJavascriptInterface(BackBridge(), "CairnAndroid")
  }

  // A font size change does not recreate the activity (configChanges in the
  // manifest). The web view scales its text by the system font scale only when
  // it is created (text zoom 100 * fontScale), so apply the new scale here.
  override fun onConfigurationChanged(newConfig: Configuration) {
    super.onConfigurationChanged(newConfig)
    webView?.settings?.textZoom = (100 * newConfig.fontScale).toInt()
    // A switch between light and dark (uiMode) does not recreate the activity
    // either, so restyle the system bars and the window background behind
    // them as a start in the new mode would.
    val night = newConfig.uiMode and Configuration.UI_MODE_NIGHT_MASK
    if (night != nightMode) {
      nightMode = night
      enableEdgeToEdge()
      val theme = ContextThemeWrapper(createConfigurationContext(newConfig), R.style.Theme_cairn)
      val attrs = theme.obtainStyledAttributes(intArrayOf(android.R.attr.windowBackground))
      window.setBackgroundDrawable(attrs.getDrawable(0))
      attrs.recycle()
    }
  }

  /**
   * The web view's renderer crashed or was killed to free memory (called by
   * the generated RustWebViewClient, see buildSrc/.../BuildTask.kt).
   * Android would end the whole app. Instead drop the dead web view and
   * recreate the activity, which loads the page again and reopens the vault.
   * Edits younger than the autosave delay are lost with the renderer.
   */
  fun onRenderProcessGone(view: WebView): Boolean {
    // A renderer that dies again right after the reload would loop: let
    // Android end the app then.
    val now = SystemClock.elapsedRealtime()
    if (lastRenderGone != 0L && now - lastRenderGone < 10_000) return false
    lastRenderGone = now
    (view.parent as? ViewGroup)?.removeView(view)
    view.destroy()
    webView = null
    recreate()
    return true
  }

  companion object {
    /** When the renderer last died (elapsedRealtime); outlives a recreated activity. */
    private var lastRenderGone = 0L
  }
}
