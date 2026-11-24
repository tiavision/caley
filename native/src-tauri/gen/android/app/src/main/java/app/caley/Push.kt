package app.caley

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import android.util.Log
import android.webkit.CookieManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import org.json.JSONObject
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.UnifiedPush
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors

/**
 * Notifications while no page of the server's is open: UnifiedPush, through the distributor installed on the
 * phone (e.g. ntfy). One registration per server, its origin the connector's instance, while the server's
 * pages have mail notifications on (Caley's switch in the rail: `mail-notifications-enabled`, which
 * res/raw/shell_bridge.js reports) and Android allows notifications. The endpoint and the device's Web Push
 * keys (the connector makes them and decrypts what arrives) go to the server's
 * `/api/notifications/subscription` with the page's session cookie; the server sends new mail there
 * (backend/src/notifications.rs). No distributor on the phone: nothing changes, the pages notify while open.
 */
object Push {
  private const val TAG = "CaleyPush"
  private const val CHANNEL = "caley"
  const val EXTRA_OPEN = "app.caley.open"
  private val io = Executors.newSingleThreadExecutor()
  // the servers registered (or unregistered) in this run of the app, so each page does not ask again
  private val settled = mutableMapOf<String, Boolean>()

  /** The channel the app's notifications are on (the pages' and the pushed ones). */
  fun channel(context: Context): String {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      context.getSystemService(NotificationManager::class.java).createNotificationChannel(
        NotificationChannel(CHANNEL, context.getString(R.string.shell_channel), NotificationManager.IMPORTANCE_DEFAULT)
      )
    }
    return CHANNEL
  }

  /** A server's page says whether its mail notifications are on: register with the distributor, or not. */
  fun follow(activity: Activity, origin: String, on: Boolean) {
    val wanted = on && NotificationManagerCompat.from(activity).areNotificationsEnabled()
    if (settled[origin] == wanted) return
    settled[origin] = wanted
    if (!wanted) {
      // the connector calls onUnregistered only when the distributor unregisters, not for this
      if (endpoint(activity, origin) != null) {
        UnifiedPush.unregister(activity, origin)
        unsubscribe(activity, origin)
      }
      return
    }
    io.execute {
      // the server's VAPID key; none (an older server, or signed out): no notifications from it
      val vapid = runCatching { JSONObject(request(origin, "GET", "/api/notifications/vapid")).getString("public_key") }
        .onFailure { Log.i(TAG, "$origin: no notifications ($it)") }.getOrNull() ?: return@execute
      activity.runOnUiThread {
        UnifiedPush.tryUseCurrentOrDefaultDistributor(activity) { ok ->
          if (ok) UnifiedPush.register(activity, origin, activity.getString(R.string.push_registration, hostOf(origin)), vapid)
          else Log.i(TAG, "no UnifiedPush distributor")
        }
      }
    }
  }

  internal fun subscribe(context: Context, origin: String, endpoint: PushEndpoint) {
    val keys = endpoint.pubKeySet ?: run {
      Log.w(TAG, "$origin: endpoint without keys, not used")
      return
    }
    val previous = endpoint(context, origin)
    io.execute {
      runCatching {
        if (previous != null && previous != endpoint.url) unsubscribeAt(origin, previous)
        request(origin, "PUT", "/api/notifications/subscription",
          JSONObject().put("endpoint", endpoint.url).put("p256dh", keys.pubKey).put("auth", keys.auth))
        prefs(context).edit().putString(origin, endpoint.url).apply()
      }.onFailure { Log.w(TAG, "$origin: subscription not stored ($it)") }
    }
  }

  internal fun unsubscribe(context: Context, origin: String) {
    val endpoint = endpoint(context, origin) ?: return
    prefs(context).edit().remove(origin).apply()
    io.execute { runCatching { unsubscribeAt(origin, endpoint) } }
  }

  /** What the server sent (decrypted): a notification, a tap on which opens its page. */
  internal fun show(context: Context, origin: String, content: ByteArray) {
    val notifications = NotificationManagerCompat.from(context)
    if (!notifications.areNotificationsEnabled()) return
    val message = runCatching { JSONObject(String(content)) }.getOrNull() ?: return
    val (tag, title, body, path) = when (message.optString("kind")) {
      "new_email" -> listOf(
        // the tag the page's own notification of this message has (src/lib/mail-notifications.ts), so the
        // two replace each other
        "${message.optString("account_id")}|${message.optString("mailbox")}|${message.optInt("uid")}",
        context.getString(R.string.push_new_mail),
        "${message.optString("from").ifEmpty { context.getString(R.string.push_unknown_sender) }}\n" +
          message.optString("subject").ifEmpty { context.getString(R.string.push_no_subject) },
        message.optString("url"),
      )
      "test" -> listOf("test", "Caley", context.getString(R.string.push_test), "/")
      else -> return
    }
    val open = PendingIntent.getActivity(
      context, tag.hashCode(),
      Intent(context, MainActivity::class.java).putExtra(EXTRA_OPEN, origin + path)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
      PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    val notification = NotificationCompat.Builder(context, channel(context))
      .setSmallIcon(R.drawable.ic_stat_caley)
      .setColor(context.getColor(R.color.caley_accent))
      .setContentTitle(title)
      .setContentText(body)
      .setSubText(hostOf(origin))
      .setStyle(NotificationCompat.BigTextStyle().bigText(body))
      .setContentIntent(open)
      .setAutoCancel(true)
      .build()
    try {
      notifications.notify(tag, 0, notification)
    } catch (_: SecurityException) {
      // the permission was taken back meanwhile
    }
  }

  private fun unsubscribeAt(origin: String, endpoint: String) =
    request(origin, "DELETE", "/api/notifications/subscription", JSONObject().put("endpoint", endpoint))

  // a request to the server with the cookies the pages have (the session)
  private fun request(origin: String, method: String, path: String, body: JSONObject? = null): String {
    val connection = URL(origin + path).openConnection() as HttpURLConnection
    try {
      connection.requestMethod = method
      connection.connectTimeout = 15000
      connection.readTimeout = 15000
      CookieManager.getInstance().getCookie(origin)?.let { connection.setRequestProperty("Cookie", it) }
      if (body != null) {
        connection.doOutput = true
        connection.setRequestProperty("Content-Type", "application/json")
        connection.outputStream.use { it.write(body.toString().toByteArray()) }
      }
      val status = connection.responseCode
      if (status !in 200..299) error("$method $path: HTTP $status")
      return connection.inputStream.use { it.readBytes().toString(Charsets.UTF_8) }
    } finally {
      connection.disconnect()
    }
  }

  private fun prefs(context: Context) = context.getSharedPreferences("push", Context.MODE_PRIVATE)

  private fun endpoint(context: Context, origin: String) = prefs(context).getString(origin, null)

  private fun hostOf(origin: String) = origin.substringAfter("://")
}

/** The connector's service (AndroidManifest.xml): what the distributor delivers, per server (instance = origin). */
class CaleyPushService : PushService() {
  override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) = Push.subscribe(this, instance, endpoint)

  override fun onMessage(message: PushMessage, instance: String) {
    if (message.decrypted) Push.show(this, instance, message.content)
  }

  override fun onRegistrationFailed(reason: FailedReason, instance: String) {
    Log.w("CaleyPush", "$instance: registration failed ($reason)")
  }

  override fun onUnregistered(instance: String) = Push.unsubscribe(this, instance)
}
