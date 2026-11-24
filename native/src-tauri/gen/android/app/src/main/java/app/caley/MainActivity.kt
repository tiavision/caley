package app.caley

import android.content.Intent
import android.os.Bundle
import android.view.View
import android.webkit.WebView
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // started by a pushed notification's tap: its page, once a server page has loaded (pageOpened)
    pendingOpen = intent.getStringExtra(Push.EXTRA_OPEN)
  }

  private var webView: WebView? = null
  private var pendingOpen: String? = null

  /** A server's page loaded (the bridge's hello): the page a pushed notification asked for, if one waits. */
  fun pageOpened() {
    val url = pendingOpen ?: return
    pendingOpen = null
    webView?.loadUrl(url)
  }

  // Android draws apps edge to edge (enforced for targetSdk 35+), but Caley's pages do not keep clear of
  // the status bar, the navigation bar or the keyboard: the webview's container is padded by them instead,
  // its background (colors.xml, Caley's page background in light and dark) showing behind the bars.
  override fun onWebViewCreate(webView: WebView) {
    this.webView = webView
    val container = findViewById<View>(android.R.id.content)
    container.setBackgroundColor(getColor(R.color.caley_background))
    ViewCompat.setOnApplyWindowInsetsListener(container) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
      )
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      WindowInsetsCompat.CONSUMED
    }
    ViewCompat.requestApplyInsets(container)

    bridge = ShellBridge(this, webView).also { it.install() }

    backCallback = object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() = back(webView)
    }
  }

  private var backCallback: OnBackPressedCallback? = null
  private var bridge: ShellBridge? = null

  override fun onResume() {
    super.onResume()
    bridge?.resumed()
  }

  // a notification tapped while the app runs (singleTask: this activity gets the intent)
  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    bridge?.opened(intent)
    // a pushed notification's tap: its page now, or once the webview (made after the activity, when Android
    // recreated it) has a server page
    intent.getStringExtra(Push.EXTRA_OPEN)?.let { url ->
      val view = webView
      if (view != null && view.url?.startsWith("http://tauri.localhost") == false) view.loadUrl(url) else pendingOpen = url
    }
  }

  // Tauri's app plugin adds its own Back handler (only back in the webview, or leave) once the app is set up,
  // which is after onWebViewCreate; the last one added is asked first. So this one is added again whenever the
  // webview's history changes (every page), which keeps it first.
  override fun onWebViewHistoryChanged(webView: WebView) {
    backCallback?.let {
      it.remove()
      onBackPressedDispatcher.addCallback(this, it)
    }
  }

  // Back: from the picker out of the app; on a server's pages back in the webview while it can, then to the
  // picker (`?pick`: to choose, not to open the server used last, ui/picker.js). The webview's history cannot
  // be relied on to hold the picker: the picker opens the last server while it loads, without a tap, and
  // Chromium makes such a navigation replace the picker's entry. Leaving moves the app to the background, as
  // Back on an app's first screen does since Android 12 (not through Tauri's handler, which would go back
  // to the server behind the picker).
  private fun back(webView: WebView) {
    when {
      // the picker, even with the server's pages behind it in the history
      webView.url?.startsWith(PICKER) != false -> moveTaskToBack(true)
      webView.canGoBack() -> webView.goBack()
      else -> webView.loadUrl("$PICKER?pick")
    }
  }

  companion object {
    // where Tauri serves the app's own pages on Android (`useHttpsScheme` off, the default)
    private const val PICKER = "http://tauri.localhost/"
  }
}
