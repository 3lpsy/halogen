package org.fgsec.halogen.storage

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

// App-side mirror of the embedded server's silent-login secrets (username →
// app-generated password): one encrypted-prefs entry, independent of the session
// registry, so removing an account never destroys the only copy — sign-out stays recoverable.
class EmbeddedSecrets(private val context: Context) {
    private val json = Json { ignoreUnknownKeys = true }

    private val prefs: SharedPreferences by lazy {
        val masterKey = MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        EncryptedSharedPreferences.create(
            context,
            "org.fgsec.halogen.embedded-secrets",
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }

    fun password(forUsername: String): String? = loadMap()[forUsername]

    fun has(username: String): Boolean = loadMap()[username] != null

    fun remember(username: String, password: String) {
        val map = loadMap().toMutableMap()
        map[username] = password
        save(map)
    }

    // Local Data → destroy embedded library: the server-side users are gone,
    // so their credentials are dead weight.
    fun clear() {
        prefs.edit().remove(ENTRY).apply()
    }

    private fun loadMap(): Map<String, String> {
        val data = prefs.getString(ENTRY, null) ?: return emptyMap()
        return runCatching { json.decodeFromString<Map<String, String>>(data) }
            .getOrNull() ?: emptyMap()
    }

    private fun save(map: Map<String, String>) {
        val data = runCatching { json.encodeToString(map) }.getOrNull() ?: return
        prefs.edit().putString(ENTRY, data).apply()
    }

    private companion object {
        const val ENTRY = "users"
    }
}
