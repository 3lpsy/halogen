package org.fgsec.halogen.storage

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKey
import java.util.UUID
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

// One signed-in account — the Android analog of a web accounts-registry
// entry. Embedded non-admin users carry an app-managed password for silent
// re-login (the add-embedded-user flow generates it, same as the web).
@Serializable
data class Session(
    val id: String = UUID.randomUUID().toString(),
    val kind: Kind,
    // Remote only — the server base URL. Embedded resolves its loopback URL
    // at boot (the port is not stable identity).
    val serverUrl: String? = null,
    val username: String,
    // The API JWT from the last login. Remote sessions resume with it and
    // fall back to the landing page on expiry; embedded sessions re-login.
    val token: String,
    // Embedded non-admin users: app-managed password for silent re-login.
    val password: String? = null,
    val localUserId: Int? = null,
) {
    @Serializable
    enum class Kind {
        @SerialName("embedded") EMBEDDED,
        @SerialName("remote") REMOTE,
    }
}

// The device-global accounts registry (all saved sessions + which is
// active) — persisted as ONE encrypted-prefs entry; tokens/passwords never
// live in plain SharedPreferences.
@Serializable
data class StoredAccounts(
    val sessions: List<Session>,
    val activeId: String? = null,
) {
    val active: Session?
        get() = sessions.firstOrNull { it.id == activeId }
}

class SessionStore(private val context: Context) {
    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = true
    }

    private val prefs: SharedPreferences by lazy {
        val masterKey = MasterKey.Builder(context)
            .setKeyScheme(MasterKey.KeyScheme.AES256_GCM)
            .build()
        EncryptedSharedPreferences.create(
            context,
            "org.fgsec.halogen.session",
            masterKey,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    }

    fun load(): StoredAccounts {
        val data = prefs.getString(ENTRY, null)
            ?: return StoredAccounts(sessions = emptyList(), activeId = null)
        return runCatching { json.decodeFromString<StoredAccounts>(data) }
            .getOrNull() ?: StoredAccounts(sessions = emptyList(), activeId = null)
    }

    fun save(registry: StoredAccounts) {
        val data = runCatching { json.encodeToString(registry) }.getOrNull() ?: return
        prefs.edit().putString(ENTRY, data).apply()
    }

    // Insert-or-replace (matching kind + server + username) and make active.
    fun upsertActive(session: Session) {
        val registry = load()
        val sessions = registry.sessions.filterNot {
            it.kind == session.kind && it.serverUrl == session.serverUrl &&
                it.username == session.username
        } + session
        save(StoredAccounts(sessions = sessions, activeId = session.id))
    }

    fun switchTo(id: String) {
        val registry = load()
        if (registry.sessions.none { it.id == id }) return
        save(registry.copy(activeId = id))
    }

    // Remove a session; returns the next active session (if any).
    fun remove(id: String): Session? {
        val registry = load()
        val sessions = registry.sessions.filterNot { it.id == id }
        val activeId =
            if (registry.activeId == id) sessions.lastOrNull()?.id else registry.activeId
        val next = StoredAccounts(sessions = sessions, activeId = activeId)
        save(next)
        return next.active
    }

    fun renameActive(to: String) {
        val registry = load()
        val idx = registry.sessions.indexOfFirst { it.id == registry.activeId }
        if (idx < 0) return
        val sessions = registry.sessions.toMutableList()
        sessions[idx] = sessions[idx].copy(username = to)
        save(registry.copy(sessions = sessions))
    }

    fun clear() {
        prefs.edit().remove(ENTRY).apply()
    }

    private companion object {
        const val ENTRY = "active"
    }
}
