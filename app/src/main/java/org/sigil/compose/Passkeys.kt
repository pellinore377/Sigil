package org.sigil.compose

import android.app.Activity
import android.content.Context
import android.os.Build
import android.util.Base64
import androidx.credentials.CreatePublicKeyCredentialRequest
import androidx.credentials.CreatePublicKeyCredentialResponse
import androidx.credentials.CredentialManager
import androidx.credentials.GetCredentialRequest
import androidx.credentials.GetPublicKeyCredentialOption
import androidx.credentials.PublicKeyCredential
import androidx.credentials.exceptions.CreateCredentialCancellationException
import androidx.credentials.exceptions.CreateCredentialException
import androidx.credentials.exceptions.GetCredentialCancellationException
import androidx.credentials.exceptions.GetCredentialException
import androidx.credentials.exceptions.NoCredentialException
import org.json.JSONArray
import org.json.JSONObject

/** WebAuthn ceremonies for account recovery; bytes are base64url without padding, as the core uses. */
internal object Passkeys {
    class Result(val credential: String, val prf: String)
    class Failure(message: String) : Exception(message)
    class Dismissed : Exception()
    private const val UNSUPPORTED = "This passkey provider can't protect Sigil recovery. Try another provider, or use a recovery code in Settings."

    fun available(context: Context) = Build.VERSION.SDK_INT >= 34 || (Build.VERSION.SDK_INT >= 28 &&
        runCatching { context.packageManager.getApplicationInfo("com.google.android.gms", 0).enabled }.getOrDefault(false))

    /** `options` is the core's `passkey_request` result. */
    suspend fun recover(activity: Activity, options: JSONObject): Result {
        val credentials = options.getJSONArray("credentials")
        if (credentials.length() == 0) throw Failure("This account has no passkey. Use a recovery code or link from another device.")
        val allow = JSONArray(); val salts = JSONObject()
        for (i in 0 until credentials.length()) credentials.getJSONObject(i).let { allow.put(descriptor(it.getString("id"))); salts.put(it.getString("id"), JSONObject().put("first", it.getString("salt"))) }
        val request = JSONObject().put("rpId", options.getString("rp_id")).put("challenge", options.getString("challenge")).put("allowCredentials", allow)
            .put("userVerification", "required").put("timeout", 120_000).put("extensions", JSONObject().put("prf", JSONObject().put("evalByCredential", salts)))
        return assertion(activity, request)
    }

    /** `options` is the core's `passkey_create_options` result. */
    suspend fun create(activity: Activity, options: JSONObject): Result {
        val salt = options.getString("salt")
        val exclude = JSONArray(); options.getJSONArray("exclude").let { for (i in 0 until it.length()) exclude.put(descriptor(it.getString(i))) }
        val request = JSONObject()
            .put("rp", JSONObject().put("id", options.getString("rp_id")).put("name", options.getString("rp_name")))
            .put("user", JSONObject().put("id", options.getString("user_id")).put("name", options.getString("user_name")).put("displayName", options.getString("user_display")))
            .put("challenge", options.getString("challenge"))
            .put("pubKeyCredParams", JSONArray().put(JSONObject().put("type", "public-key").put("alg", -7)).put(JSONObject().put("type", "public-key").put("alg", -257)))
            .put("authenticatorSelection", JSONObject().put("residentKey", "required").put("requireResidentKey", true).put("userVerification", "required"))
            .put("excludeCredentials", exclude).put("attestation", "none").put("timeout", 120_000)
            .put("extensions", JSONObject().put("prf", JSONObject().put("eval", JSONObject().put("first", salt))))
        val response = try { CredentialManager.create(activity).createCredential(activity, CreatePublicKeyCredentialRequest(request.toString())) }
            catch (_: CreateCredentialCancellationException) { throw Dismissed() }
            catch (error: CreateCredentialException) { throw Failure(error.errorMessage?.toString()?.takeIf { it.isNotBlank() } ?: "Could not create a passkey. Try again.") }
        val registration = JSONObject((response as? CreatePublicKeyCredentialResponse ?: throw Failure(UNSUPPORTED)).registrationResponseJson)
        val id = unpadded(registration.getString("rawId"))
        val prf = registration.optJSONObject("clientExtensionResults")?.optJSONObject("prf")
        prf?.optJSONObject("results")?.optString("first")?.takeIf { it.isNotEmpty() }?.let { return Result(id, unpadded(it)) }
        if (prf?.optBoolean("enabled") != true) throw Failure(UNSUPPORTED)
        // Some providers only evaluate PRF on assertion.
        return assertion(activity, JSONObject().put("rpId", options.getString("rp_id")).put("challenge", options.getString("challenge"))
            .put("allowCredentials", JSONArray().put(descriptor(id))).put("userVerification", "required").put("timeout", 120_000)
            .put("extensions", JSONObject().put("prf", JSONObject().put("eval", JSONObject().put("first", salt)))))
    }

    private suspend fun assertion(activity: Activity, request: JSONObject): Result {
        val response = try { CredentialManager.create(activity).getCredential(activity, GetCredentialRequest(listOf(GetPublicKeyCredentialOption(request.toString())))) }
            catch (_: GetCredentialCancellationException) { throw Dismissed() }
            catch (_: NoCredentialException) { throw Failure("No passkey for this account is available on this device.") }
            catch (error: GetCredentialException) { throw Failure(error.errorMessage?.toString()?.takeIf { it.isNotBlank() } ?: "Could not use this passkey. Try again.") }
        val credential = response.credential as? PublicKeyCredential ?: throw Failure(UNSUPPORTED)
        val json = JSONObject(credential.authenticationResponseJson)
        val prf = json.optJSONObject("clientExtensionResults")?.optJSONObject("prf")?.optJSONObject("results")?.optString("first")?.takeIf { it.isNotEmpty() } ?: throw Failure(UNSUPPORTED)
        return Result(unpadded(json.getString("rawId")), unpadded(prf))
    }

    private fun descriptor(id: String) = JSONObject().put("type", "public-key").put("id", id)
    // Providers differ on padding and alphabet; the core takes url-safe without padding.
    private fun unpadded(value: String) = Base64.encodeToString(Base64.decode(value.replace('+', '-').replace('/', '_'), Base64.URL_SAFE or Base64.NO_WRAP), Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING)
}
