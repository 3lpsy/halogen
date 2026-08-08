package org.fgsec.halogen.storage

import java.util.Base64
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.longOrNull

// The signed-in account and the storage namespace derived from it. Mirrors
// the web client's convention (crates/ui-platform namespace): embedded
// accounts are e{userId}, remote accounts u{userId}-{serverHash} — user ids
// are per-server, so the suffix keeps equal ids from sharing a namespace.
data class AccountContext(
    val kind: Kind,
    val userId: Int,
    val username: String,
) {
    sealed interface Kind {
        data object Embedded : Kind
        data class Remote(val serverUrl: String) : Kind
    }

    val namespace: String
        get() = when (kind) {
            is Kind.Embedded -> "e$userId"
            // 16-hex stable server hash, like the web's u{id}-{server:016x}.
            // (FNV-1a here vs the web's hasher — namespaces never cross the
            // device boundary, only stability within this app matters.)
            is Kind.Remote -> "u$userId-${fnv1a16hex(kind.serverUrl)}"
        }

    companion object {
        // Build a context from a login: the user id comes from the API JWT's
        // `sub` claim. Unverified decode by design — the token came from a
        // login WE performed, and this only names a cache directory.
        fun from(kind: Kind, username: String, jwt: String): AccountContext? {
            val id = jwtSub(jwt)?.toIntOrNull() ?: return null
            return AccountContext(kind, id, username)
        }

        private fun fnv1a16hex(s: String): String {
            var hash = 0xcbf29ce484222325UL
            for (byte in s.toByteArray(Charsets.UTF_8)) {
                hash = hash xor byte.toUByte().toULong()
                hash *= 0x00000100000001B3UL
            }
            return hash.toString(16).padStart(16, '0')
        }

        fun jwtSub(jwt: String): String? {
            val parts = jwt.split(".").filter { it.isNotEmpty() }
            if (parts.size != 3) return null
            var b64 = parts[1].replace("-", "+").replace("_", "/")
            while (b64.length % 4 != 0) b64 += "="
            val obj = runCatching {
                Json.parseToJsonElement(String(Base64.getDecoder().decode(b64))).jsonObject
            }.getOrNull() ?: return null
            val sub = obj["sub"] as? JsonPrimitive ?: return null
            return if (sub.isString) sub.content else sub.longOrNull?.toString()
        }
    }
}
