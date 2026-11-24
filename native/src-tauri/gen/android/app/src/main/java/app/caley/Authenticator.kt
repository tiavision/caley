package app.caley

import android.app.KeyguardManager
import android.content.Context
import android.hardware.biometrics.BiometricManager.Authenticators.BIOMETRIC_STRONG
import android.hardware.biometrics.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import android.hardware.biometrics.BiometricPrompt
import android.net.Uri
import android.os.Build
import android.os.CancellationSignal
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.util.Base64
import androidx.annotation.RequiresApi
import org.json.JSONArray
import org.json.JSONObject
import java.io.ByteArrayOutputStream
import java.math.BigInteger
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.MessageDigest
import java.security.PrivateKey
import java.security.SecureRandom
import java.security.Signature
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import javax.crypto.KeyGenerator
import javax.crypto.Mac
import javax.crypto.SecretKey

/**
 * A WebAuthn platform authenticator for the server's pages (res/raw/shell_bridge.js asks, over ShellBridge.kt):
 * the WebView offers no passkeys for a server the app cannot name in advance, and Caley's PGP vault is
 * unlocked with a passkey's PRF output. Each credential is two Android Keystore keys that never leave it
 * (StrongBox when the phone has one): an ECDSA P-256 key that signs assertions (ES256, what the server
 * verifies) and an HMAC-SHA256 key for PRF (HMAC over SHA-256("WebAuthn PRF" ‖ 0 ‖ input), as the WebAuthn
 * spec derives a PRF from hmac-secret). Both are usable only for a few seconds after the screen lock was
 * passed (fingerprint, face or the device PIN) in the prompt each ceremony shows. The authenticator, not
 * the page, writes the origin into clientDataJSON (the one WebView reports for the page) and checks the
 * rpId against it. Attestation "none", user present and verified, a signature counter per credential.
 * Android 11+ (prompts that allow the device PIN, time-limited key use).
 */
@RequiresApi(Build.VERSION_CODES.R)
class Authenticator(private val activity: MainActivity) {
  class Failure(val name: String, message: String) : Exception(message)

  private val store = KeyStore.getInstance(KEYSTORE).apply { load(null) }
  // credential id → { rpId, userId, counter }
  private val credentials = activity.getSharedPreferences("webauthn", Context.MODE_PRIVATE)

  /** navigator.credentials.create: a new credential for `origin`; its PRF output too when asked. */
  fun create(request: JSONObject, origin: Uri, done: (Result<JSONObject>) -> Unit) = guarded(done) {
    val rpId = rpIdFor(request.getString("rpId"), origin)
    val algorithms = request.optJSONArray("algorithms").ints()
    if (algorithms.isNotEmpty() && ES256 !in algorithms) throw Failure("NotSupportedError", "only ES256")
    val existing = request.optJSONArray("exclude").strings().filter { stored(it)?.optString("rpId") == rpId }
    if (existing.isNotEmpty()) throw Failure("InvalidStateError", "already registered on this device")
    if (!activity.getSystemService(KeyguardManager::class.java).isDeviceSecure) {
      throw Failure("NotAllowedError", activity.getString(R.string.auth_no_screen_lock))
    }
    verifyUser(rpId, done) {
      val id = ByteArray(32).also { SecureRandom().nextBytes(it) }
      val credentialId = b64u(id)
      val publicKey = generate(credentialId)
      val clientData = clientData("webauthn.create", request.getString("challenge"), origin)
      val authData = authenticatorData(rpId, 0) + attestedCredential(id, publicKey)
      val attestation = Cbor.map(
        Cbor.text("fmt") to Cbor.text("none"),
        Cbor.text("attStmt") to Cbor.map(),
        Cbor.text("authData") to Cbor.bytes(authData),
      )
      credentials.edit().putString(
        credentialId,
        JSONObject().put("rpId", rpId).put("userId", request.getString("userId"))
          .put("userName", request.optString("userName")).put("counter", 0).toString(),
      ).apply()
      JSONObject()
        .put("id", credentialId)
        .put("clientDataJSON", b64u(clientData))
        .put("attestationObject", b64u(attestation))
        .put("authenticatorData", b64u(authData))
        .put("prfEnabled", request.optBoolean("wantsPrf"))
        .apply { request.optString("prf").ifEmpty { null }?.let { put("prf", prf(credentialId, it)) } }
    }
  }

  /** navigator.credentials.get: an assertion by one of this device's credentials for `origin`. */
  fun get(request: JSONObject, origin: Uri, done: (Result<JSONObject>) -> Unit) = guarded(done) {
    val rpId = rpIdFor(request.getString("rpId"), origin)
    val allowed = request.optJSONArray("allow").strings()
    val credentialId = (allowed.ifEmpty { credentials.all.keys.toList() })
      .firstOrNull { stored(it)?.optString("rpId") == rpId && store.containsAlias(alias(it, SIGN)) }
      ?: throw Failure("NotAllowedError", activity.getString(R.string.auth_no_credential))
    verifyUser(rpId, done) {
      val record = stored(credentialId)!!
      val counter = record.optInt("counter") + 1
      val clientData = clientData("webauthn.get", request.getString("challenge"), origin)
      val authData = authenticatorData(rpId, counter)
      val signature = Signature.getInstance("SHA256withECDSA").run {
        initSign(store.getKey(alias(credentialId, SIGN), null) as PrivateKey)
        update(authData)
        update(sha256(clientData))
        sign()
      }
      credentials.edit().putString(credentialId, record.put("counter", counter).toString()).apply()
      JSONObject()
        .put("id", credentialId)
        .put("clientDataJSON", b64u(clientData))
        .put("authenticatorData", b64u(authData))
        .put("signature", b64u(signature))
        .put("userHandle", record.optString("userId"))
        .apply { request.optString("prf").ifEmpty { null }?.let { put("prf", prf(credentialId, it)) } }
    }
  }

  // a ceremony's checks: a Failure (or any error) is the page's DOMException
  private inline fun guarded(done: (Result<JSONObject>) -> Unit, ceremony: () -> Unit) {
    try {
      ceremony()
    } catch (e: Exception) {
      done(Result.failure(failure(e)))
    }
  }

  private fun failure(e: Throwable) = e as? Failure ?: Failure("NotAllowedError", e.message ?: e.javaClass.simpleName)

  // the screen lock (fingerprint, face or PIN), then `then` on the UI thread, within the keys' KEY_SECONDS
  private fun verifyUser(rpId: String, done: (Result<JSONObject>) -> Unit, then: () -> JSONObject) {
    BiometricPrompt.Builder(activity)
      .setTitle(activity.getString(R.string.auth_title))
      .setSubtitle(rpId)
      .setDescription(activity.getString(R.string.auth_description))
      .setAllowedAuthenticators(BIOMETRIC_STRONG or DEVICE_CREDENTIAL)
      .build()
      .authenticate(CancellationSignal(), activity.mainExecutor, object : BiometricPrompt.AuthenticationCallback() {
        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
          done(runCatching(then).recoverCatching { throw failure(it) })
        }

        override fun onAuthenticationError(code: Int, message: CharSequence) {
          done(Result.failure(Failure("NotAllowedError", message.toString())))
        }
      })
  }

  // the rpId the page asks for, if the page's origin may use it: its host or a parent domain of it (not a
  // bare top-level one), https or localhost
  private fun rpIdFor(rpId: String, origin: Uri): String {
    val host = origin.host ?: throw Failure("SecurityError", "no origin")
    val secure = origin.scheme == "https" || host == "localhost"
    val ours = host == rpId || (host.endsWith(".$rpId") && rpId.contains('.'))
    if (!secure || !ours) throw Failure("SecurityError", "rpId $rpId is not allowed for $host")
    return rpId
  }

  private fun clientData(type: String, challenge: String, origin: Uri): ByteArray {
    val port = if (origin.port != -1) ":${origin.port}" else ""
    val json = JSONObject().put("type", type).put("challenge", challenge)
      .put("origin", "${origin.scheme}://${origin.host}$port").put("crossOrigin", false)
    return json.toString().toByteArray()
  }

  // rpIdHash ‖ flags (user present, user verified; attested credential data for a new one) ‖ counter
  private fun authenticatorData(rpId: String, counter: Int): ByteArray {
    val flags = if (counter == 0) 0x45 else 0x05
    return sha256(rpId.toByteArray()) + byteArrayOf(flags.toByte()) +
      byteArrayOf((counter ushr 24).toByte(), (counter ushr 16).toByte(), (counter ushr 8).toByte(), counter.toByte())
  }

  // AAGUID (none) ‖ credential id length ‖ credential id ‖ the public key (COSE, EC2 P-256 ES256)
  private fun attestedCredential(id: ByteArray, key: ECPublicKey): ByteArray {
    val cose = Cbor.map(
      Cbor.int(1) to Cbor.int(2),
      Cbor.int(3) to Cbor.int(ES256),
      Cbor.int(-1) to Cbor.int(1),
      Cbor.int(-2) to Cbor.bytes(coordinate(key.w.affineX)),
      Cbor.int(-3) to Cbor.bytes(coordinate(key.w.affineY)),
    )
    return ByteArray(16) + byteArrayOf((id.size ushr 8).toByte(), id.size.toByte()) + id + cose
  }

  private fun generate(credentialId: String): ECPublicKey {
    fun <T> strongBox(make: (Boolean) -> T): T =
      try { make(true) } catch (_: StrongBoxUnavailableException) { make(false) }

    val pair = strongBox { strong ->
      KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, KEYSTORE).run {
        initialize(
          KeyGenParameterSpec.Builder(alias(credentialId, SIGN), KeyProperties.PURPOSE_SIGN)
            .setAlgorithmParameterSpec(ECGenParameterSpec("secp256r1"))
            .setDigests(KeyProperties.DIGEST_SHA256)
            .locked(strong)
            .build()
        )
        generateKeyPair()
      }
    }
    strongBox { strong ->
      KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_HMAC_SHA256, KEYSTORE).run {
        init(KeyGenParameterSpec.Builder(alias(credentialId, PRF), KeyProperties.PURPOSE_SIGN).locked(strong).build())
        generateKey()
      }
    }
    return pair.public as ECPublicKey
  }

  private fun KeyGenParameterSpec.Builder.locked(strongBox: Boolean) = apply {
    setUserAuthenticationRequired(true)
    setUserAuthenticationParameters(KEY_SECONDS, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
    setInvalidatedByBiometricEnrollment(false)
    if (strongBox) setIsStrongBoxBacked(true)
  }

  private fun prf(credentialId: String, input: String): String {
    val salt = sha256("WebAuthn PRF".toByteArray() + byteArrayOf(0) + Base64.decode(input, B64U))
    val mac = Mac.getInstance("HmacSHA256").apply { init(store.getKey(alias(credentialId, PRF), null) as SecretKey) }
    return b64u(mac.doFinal(salt))
  }

  private fun stored(credentialId: String): JSONObject? = credentials.getString(credentialId, null)?.let(::JSONObject)

  private fun alias(credentialId: String, use: String) = "caley-webauthn-$credentialId-$use"

  private fun coordinate(value: BigInteger): ByteArray {
    val bytes = value.toByteArray().dropWhile { it == 0.toByte() }.toByteArray()
    return ByteArray(32 - bytes.size) + bytes
  }

  private fun sha256(bytes: ByteArray) = MessageDigest.getInstance("SHA-256").digest(bytes)

  private fun b64u(bytes: ByteArray) = Base64.encodeToString(bytes, B64U)

  private fun JSONArray?.strings() = (0 until (this?.length() ?: 0)).map { this!!.getString(it) }

  private fun JSONArray?.ints() = (0 until (this?.length() ?: 0)).map { this!!.getInt(it) }

  companion object {
    private const val KEYSTORE = "AndroidKeyStore"
    private const val ES256 = -7
    private const val SIGN = "sign"
    private const val PRF = "prf"
    private const val KEY_SECONDS = 10
    private const val B64U = Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP
  }
}

/** The CBOR WebAuthn needs: integers, byte and text strings, maps (in the order given, the canonical one). */
private object Cbor {
  fun int(value: Int): ByteArray = if (value >= 0) head(0, value.toLong()) else head(1, (-1L - value))
  fun bytes(value: ByteArray) = head(2, value.size.toLong()) + value
  fun text(value: String) = value.toByteArray().let { head(3, it.size.toLong()) + it }
  fun map(vararg entries: Pair<ByteArray, ByteArray>): ByteArray {
    val out = ByteArrayOutputStream()
    out.write(head(5, entries.size.toLong()))
    for ((key, value) in entries) {
      out.write(key)
      out.write(value)
    }
    return out.toByteArray()
  }

  private fun head(major: Int, length: Long): ByteArray {
    val type = major shl 5
    return when {
      length < 24 -> byteArrayOf((type or length.toInt()).toByte())
      length < 0x100 -> byteArrayOf((type or 24).toByte(), length.toByte())
      length < 0x10000 -> byteArrayOf((type or 25).toByte(), (length ushr 8).toByte(), length.toByte())
      else -> byteArrayOf(
        (type or 26).toByte(), (length ushr 24).toByte(), (length ushr 16).toByte(), (length ushr 8).toByte(), length.toByte(),
      )
    }
  }
}
