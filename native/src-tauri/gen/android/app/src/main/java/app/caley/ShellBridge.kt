package app.caley

import android.Manifest
import android.app.DownloadManager
import android.app.PendingIntent
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.util.Base64
import android.webkit.CookieManager
import android.webkit.URLUtil
import android.webkit.WebView
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.FileProvider
import androidx.webkit.JavaScriptReplyProxy
import androidx.webkit.ScriptHandler
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import org.json.JSONObject
import java.io.File

/**
 * What the server's pages get from the app (res/raw/shell_bridge.js, injected at document start): Android
 * notifications behind `Notification`, the share sheet behind `navigator.share`, saving downloads of data the
 * page made (blob:/data: links), Android's download manager for downloads from the server, with the page's
 * cookies, passkeys behind `navigator.credentials` (Authenticator.kt), and the phone's browser for links that
 * open a window on another site (every link in a mail). The page and the app talk over the `caleyShell` web
 * message channel, top frames only, and only the server the picker opened gets any of it (`server`: exactly
 * its origin; the script is injected there alone, every other origin's messages are dropped), so a page a link
 * led to gets no notifications, no files in Downloads and not the device's push endpoint. The picker (the
 * app's own page) says which server that is and gets nothing else.
 */
class ShellBridge(private val activity: MainActivity, private val webView: WebView) {
  private val prefs = activity.getSharedPreferences("shell", Context.MODE_PRIVATE)
  private val notifications = NotificationManagerCompat.from(activity)
  private var installed = false
  private var script: ScriptHandler? = null
  // the server the picker opened last (or a pushed notification's tap), kept across restarts of the app
  private var server: String? = prefs.getString(SERVER, null)
  // the page that said hello last: notification clicks and permission changes go there
  private var page: JavaScriptReplyProxy? = null
  // a page's notification ("<page>:<id>") → the Android one (tag, id)
  private val shown = mutableMapOf<String, Pair<String?, Int>>()
  private var nextId = 1
  // the server's passkeys (Authenticator.kt), Android 11+
  private val authenticator = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) Authenticator(activity) else null

  fun install() {
    if (!WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER) ||
      !WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)
    ) return
    installed = true
    WebViewCompat.addWebMessageListener(webView, "caleyShell", setOf("*")) { _, message, origin, isMainFrame, reply ->
      val data = message.data?.let { runCatching { JSONObject(it) }.getOrNull() }
      if (!isMainFrame || data == null) return@addWebMessageListener
      if (origin.host == PICKER_HOST) picked(data, reply)
      else if (server != null && originOf(origin) == server) handle(data, origin, reply)
    }
    inject()
    webView.setDownloadListener { url, userAgent, contentDisposition, mimeType, _ ->
      download(url, userAgent, contentDisposition, mimeType)
    }
    Push.channel(activity)
  }

  /** The server whose pages get the bridge from now on (the picker's choice, or a pushed notification's). */
  fun bind(origin: String) {
    if (origin == server) return
    server = origin
    page = null
    prefs.edit().putString(SERVER, origin).apply()
    if (installed) inject()
  }

  // the picker names the server it opens (ui/start.js) and waits for this answer before it goes there, so the
  // server's first page finds it bound
  private fun picked(message: JSONObject, reply: JavaScriptReplyProxy) {
    if (message.optString("type") != "server") return
    val origin = message.optString("origin")
    val uri = Uri.parse(origin)
    if ((uri.scheme == "http" || uri.scheme == "https") && !uri.host.isNullOrEmpty() && originOf(uri) == origin) bind(origin)
    answer(reply, message.optInt("id"), null)
  }

  /** Back in front: the permission may have been changed in the system settings meanwhile. */
  fun resumed() {
    if (!installed) return
    inject()
    page?.postMessage(JSONObject().put("permission", permission()).toString())
  }

  /** A notification tapped (the intent that brought the app to front): its click, on the page that sent it. */
  fun opened(intent: Intent) {
    val click = intent.getStringExtra(EXTRA_CLICK)?.split(":", limit = 2) ?: return
    val pageId = click[0]
    val id = click.getOrNull(1)?.toIntOrNull() ?: return
    page?.postMessage(JSONObject().put("click", id).put("page", pageId).toString())
  }

  // the script, with the permission as it is now written in (the page reads it at once), for the server's
  // pages only
  private fun inject() {
    script?.remove()
    script = null
    val server = server ?: return
    val source = activity.resources.openRawResource(R.raw.shell_bridge).bufferedReader().use { it.readText() }
      .replace("__PERMISSION__", permission())
      .replace("__WEBAUTHN__", (authenticator != null).toString())
    script = runCatching { WebViewCompat.addDocumentStartJavaScript(webView, source, setOf(server)) }.getOrNull()
  }

  private fun handle(message: JSONObject, origin: Uri, reply: JavaScriptReplyProxy) {
    val id = message.optInt("id")
    when (message.optString("type")) {
      "hello" -> {
        page = reply
        activity.pageOpened()
        Push.follow(activity, originOf(origin), message.optBoolean("mail"))
      }
      // Caley's mail notification switch changed (registers this server with UnifiedPush, or not)
      "mail" -> Push.follow(activity, originOf(origin), message.optBoolean("on"))
      "permission" -> requestPermission { answer(reply, id, it) }
      "notify" -> notify(message)
      "open" -> browse(message.optString("url"))
      "close" -> shown.remove("${message.optString("page")}:${message.optInt("id")}")?.let { (tag, n) -> notifications.cancel(tag, n) }
      "webauthn.create", "webauthn.get" -> {
        val authenticator = authenticator ?: return answer(reply, id, null, error = "not supported", name = "NotSupportedError")
        val done = { result: Result<JSONObject> ->
          result.fold(
            { answer(reply, id, it) },
            { answer(reply, id, null, error = it.message ?: "failed", name = (it as? Authenticator.Failure)?.name) },
          )
        }
        if (message.optString("type") == "webauthn.create") authenticator.create(message, origin, done)
        else authenticator.get(message, origin, done)
      }
      "share" -> {
        share(message)
        answer(reply, id, null)
      }
      "save" -> Thread {
        val saved = runCatching { save(message.getString("name"), message.getString("mime"), message.getString("data")) }
        activity.runOnUiThread {
          if (saved.isSuccess) {
            toast(R.string.shell_saved, message.getString("name"))
            answer(reply, id, null)
          } else {
            toast(R.string.shell_save_failed, message.getString("name"))
            answer(reply, id, null, error = saved.exceptionOrNull()?.message ?: "failed")
          }
        }
      }.start()
    }
  }

  private fun answer(reply: JavaScriptReplyProxy, id: Int, value: Any?, error: String? = null, name: String? = null) {
    val message = JSONObject().put("reply", id).put("value", value ?: JSONObject.NULL)
    if (error != null) message.put("error", error)
    if (name != null) message.put("name", name)
    reply.postMessage(message.toString())
  }

  // `Notification.permission`: denied once asked and refused, until turned on in the system settings
  private fun permission(): String = when {
    notifications.areNotificationsEnabled() -> {
      prefs.edit().remove("denied").apply()
      "granted"
    }
    prefs.getBoolean("denied", false) || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU -> "denied"
    else -> "default"
  }

  private fun requestPermission(done: (String) -> Unit) {
    if (permission() != "default") return done(permission())
    activity.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS)) { granted ->
      if (granted != true) prefs.edit().putBoolean("denied", true).apply()
      inject()
      done(permission())
    }
  }

  private fun notify(message: JSONObject) {
    if (!notifications.areNotificationsEnabled()) return
    val key = "${message.optString("page")}:${message.optInt("id")}"
    // a tag replaces the notification with the same tag, as in a browser
    val tag = message.optString("tag").ifEmpty { null }
    val id = if (tag != null) 0 else nextId++
    val requestCode = tag?.hashCode() ?: id
    val open = PendingIntent.getActivity(
      activity, requestCode,
      Intent(activity, MainActivity::class.java).putExtra(EXTRA_CLICK, key).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    val body = message.optString("body")
    val notification = NotificationCompat.Builder(activity, Push.channel(activity))
      .setSmallIcon(R.drawable.ic_stat_caley)
      .setColor(activity.getColor(R.color.caley_accent))
      .setContentTitle(message.optString("title"))
      .setContentText(body)
      .setStyle(NotificationCompat.BigTextStyle().bigText(body))
      .setContentIntent(open)
      .setAutoCancel(true)
      .build()
    try {
      notifications.notify(tag, id, notification)
      shown[key] = tag to id
    } catch (_: SecurityException) {
      // the permission was taken back meanwhile
    }
  }

  private fun share(message: JSONObject) {
    val title = message.optString("title")
    val intent = Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_SUBJECT, title).putExtra(Intent.EXTRA_TITLE, title)
    val file = message.optJSONObject("file")
    if (file != null) {
      // a file in the cache the FileProvider serves (res/xml/file_paths.xml), for the app shared to
      val dir = File(activity.cacheDir, "shared").apply { mkdirs() }
      val shared = File(dir, safeName(file.getString("name")))
      shared.writeBytes(bytes(file.getString("data")))
      val uri = FileProvider.getUriForFile(activity, "${activity.packageName}.fileprovider", shared)
      intent.setType(file.getString("type")).putExtra(Intent.EXTRA_STREAM, uri)
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      intent.clipData = ClipData.newUri(activity.contentResolver, shared.name, uri)
    } else {
      intent.setType("text/plain").putExtra(Intent.EXTRA_TEXT, message.optString("text"))
    }
    activity.startActivity(Intent.createChooser(intent, title.ifEmpty { null }))
  }

  // into Downloads (MediaStore: no storage permission since Android 10; the app's own folder before)
  private fun save(name: String, type: String, data: String) {
    val content = bytes(data)
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
      File(activity.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS), safeName(name)).writeBytes(content)
      return
    }
    val resolver = activity.contentResolver
    val values = ContentValues().apply {
      put(MediaStore.Downloads.DISPLAY_NAME, safeName(name))
      put(MediaStore.Downloads.MIME_TYPE, type)
      put(MediaStore.Downloads.IS_PENDING, 1)
    }
    val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) ?: error("no Downloads")
    resolver.openOutputStream(uri)!!.use { it.write(content) }
    resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
  }

  // a link on the server's page that opens a window on another site, or a window the page opens there
  // (shell_bridge.js): the phone's browser, not this webview
  private fun browse(url: String) {
    val uri = Uri.parse(url)
    if (uri.scheme != "http" && uri.scheme != "https") return
    try {
      activity.startActivity(Intent(Intent.ACTION_VIEW, uri).addCategory(Intent.CATEGORY_BROWSABLE))
    } catch (_: ActivityNotFoundException) {
      // no browser on the phone
    }
  }

  // a download from the server: Android's download manager, with the session cookie the page has; one from
  // another site: the phone's browser
  private fun download(url: String, userAgent: String, contentDisposition: String?, mimeType: String?) {
    if (!url.startsWith("http")) return
    if (originOf(Uri.parse(url)) != server) return browse(url)
    val name = URLUtil.guessFileName(url, contentDisposition, mimeType)
    val request = DownloadManager.Request(Uri.parse(url))
      .addRequestHeader("User-Agent", userAgent)
      .setTitle(name)
      .setMimeType(mimeType)
      .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
      .setDestinationInExternalPublicDir(Environment.DIRECTORY_DOWNLOADS, name)
    CookieManager.getInstance().getCookie(url)?.let { request.addRequestHeader("Cookie", it) }
    activity.getSystemService(DownloadManager::class.java).enqueue(request)
    toast(R.string.shell_downloading, name)
  }

  private fun toast(text: Int, name: String) =
    Toast.makeText(activity, activity.getString(text, name), Toast.LENGTH_SHORT).show()

  private fun bytes(dataUrl: String): ByteArray = Base64.decode(dataUrl.substringAfter(","), Base64.DEFAULT)

  private fun safeName(name: String) = name.replace(Regex("[/\\\\:*?\"<>|\\x00-\\x1f]"), "_").ifBlank { "download" }

  companion object {
    private const val EXTRA_CLICK = "app.caley.notification"
    private const val SERVER = "server"
    // where Tauri serves the picker (MainActivity.kt)
    private const val PICKER_HOST = "tauri.localhost"

    fun originOf(origin: Uri) = "${origin.scheme}://${origin.host}${if (origin.port != -1) ":${origin.port}" else ""}"

    /**
     * `url` if it is on a server the app knows (the one bound, or one registered for push), rebuilt from the
     * parts Android parsed, so the webview loads the host that was checked; else null.
     */
    fun serverUrl(context: Context, url: String): String? {
      val uri = Uri.parse(url)
      if ((uri.scheme != "http" && uri.scheme != "https") || uri.host.isNullOrEmpty()) return null
      val origin = originOf(uri)
      val bound = context.getSharedPreferences("shell", Context.MODE_PRIVATE).getString(SERVER, null)
      if (origin != bound && !Push.registered(context, origin)) return null
      return origin + (uri.encodedPath?.ifEmpty { null } ?: "/") +
        (uri.encodedQuery?.let { "?$it" } ?: "") + (uri.encodedFragment?.let { "#$it" } ?: "")
    }
  }
}
