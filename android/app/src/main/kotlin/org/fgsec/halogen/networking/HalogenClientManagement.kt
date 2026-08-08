package org.fgsec.halogen.networking

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonObject
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.wire.DefaultDataType
import org.fgsec.halogen.wire.OpmlExportData
import org.fgsec.halogen.wire.OpmlImportData
import org.fgsec.halogen.wire.OpmlImportResultData
import org.fgsec.halogen.wire.PodcastAutoPlaylistData
import org.fgsec.halogen.wire.PodcastAutoPlaylistSetData
import org.fgsec.halogen.wire.PodcastConfigData
import org.fgsec.halogen.wire.PodcastConfigStoreData
import org.fgsec.halogen.wire.PodcastConfigUpdateData
import org.fgsec.halogen.wire.PodcastData
import org.fgsec.halogen.wire.PodcastUpdateData
import org.fgsec.halogen.wire.ResponseData
import org.fgsec.halogen.wire.UserData
import org.fgsec.halogen.wire.UserStoreData
import org.fgsec.halogen.wire.UserUpdateData

/// The writable config-overrides allowlist — wire `ConfigOverridesData`
/// (crates/wire/src/config.rs) field-for-field: every key optional, durations
/// as whole seconds. Unset fields encode as absent (serde reads that as None).
@Serializable
data class ConfigOverridesData(
    var subscription_fallback_poll_interval_secs: ULong? = null,
    var subscription_poll_wake_interval_secs: ULong? = null,
    var subscription_fallback_max_episodes: UInt? = null,
    var subscription_max_concurrent_downloads: UInt? = null,
    var subscription_max_poll_concurrent: UInt? = null,
    var subscription_poll_auto_download_enabled: Boolean? = null,
    var subscription_auto_playlist_add_to_start: Boolean? = null,
    var subscription_no_sync_before: String? = null,
    var subscription_sync_on_start: Boolean? = null,
    var auth_token_expiry_minutes: ULong? = null,
    var episode_playback_complete_percentage: UShort? = null,
    var opml_file: String? = null,
)

/// What `POST admin/db/import` did — wire `DbImportSummaryData`
/// (crates/wire/src/db_transfer.rs). `created_usernames` lists users the
/// import created with random passwords.
@Serializable
data class DbImportSummaryData(
    val users_merged: UInt,
    val users_created: UInt,
    val created_usernames: List<String>,
    val podcasts_merged: UInt,
    val podcasts_created: UInt,
    val subscriptions_created: UInt,
    val episodes_merged: UInt,
    val episodes_created: UInt,
    val chapters_created: UInt,
    val playbacks_upserted: UInt,
    val statuses_upserted: UInt,
    val playlists_merged: UInt,
    val playlists_created: UInt,
    val playlist_links_created: UInt,
    val auto_playlists_created: UInt,
)

// Podcast/config/user management + OPML + raw-config reads.

// MARK: - podcast management

suspend fun HalogenClient.updatePodcast(
    id: Int,
    title: String?,
    description: String?,
    feedUrl: String?,
) {
    put(
        "podcasts/$id",
        PodcastUpdateData(title = title, description = description, feed_url = feedUrl),
        PodcastUpdateData.serializer(),
        PodcastData.serializer(),
    )
}

suspend fun HalogenClient.deletePodcast(id: Int) {
    delete("podcasts/$id")
}

/// Create + link a download/poll config for a podcast (atomic).
suspend fun HalogenClient.createPodcastConfig(podcastId: Int, data: PodcastConfigStoreData) {
    post(
        "podcasts/$podcastId/config",
        data,
        PodcastConfigStoreData.serializer(),
        PodcastConfigData.serializer(),
    )
}

/// One config by id — the edit-form prefill (web `get_podcast_config`): a
/// cached podcast row can carry the config FK without the body, and editing
/// off missing values would overwrite the real config with defaults.
suspend fun HalogenClient.podcastConfig(id: Int): PodcastConfigData =
    get("podcast-configs/$id", emptyList(), PodcastConfigData.serializer())

/// Edit an existing config.
suspend fun HalogenClient.updatePodcastConfig(configId: Int, data: PodcastConfigUpdateData) {
    put(
        "podcast-configs/$configId",
        data,
        PodcastConfigUpdateData.serializer(),
        PodcastConfigData.serializer(),
    )
}

/// Unlink + delete a podcast's config.
suspend fun HalogenClient.deletePodcastConfig(podcastId: Int) {
    delete("podcasts/$podcastId/config")
}

suspend fun HalogenClient.autoPlaylists(podcastId: Int): List<PodcastAutoPlaylistData> =
    get(
        "podcasts/$podcastId/auto-playlists",
        emptyList(),
        ListSerializer(PodcastAutoPlaylistData.serializer()),
    )

/// Replace the podcast's auto-add playlist set (idempotent). `addToStart` is
/// the per-podcast insert-position override stamped on every link: true =
/// start, false = end, null = server default.
suspend fun HalogenClient.setAutoPlaylists(
    podcastId: Int,
    playlistIds: List<Int>,
    addToStart: Boolean?,
) {
    val request = Request.Builder()
        .url(url("podcasts/$podcastId/auto-playlists"))
        .put(
            envelopeBody(
                PodcastAutoPlaylistSetData(playlist_ids = playlistIds, add_to_start = addToStart),
                PodcastAutoPlaylistSetData.serializer(),
            )
        )
        .build()
    send(request, DefaultDataType.serializer())
}

// MARK: - user management

suspend fun HalogenClient.updateUsername(userId: Int, username: String) {
    updateUser(userId = userId, username = username, isAdmin = null)
}

/// `PUT /users/{id}` — self-edit sends `is_admin: null` (the server forbids
/// changing your own flag); the admin flow sends the toggle's value.
suspend fun HalogenClient.updateUser(userId: Int, username: String?, isAdmin: Boolean?) {
    put(
        "users/$userId",
        UserUpdateData(username = username, is_admin = isAdmin),
        UserUpdateData.serializer(),
        UserData.serializer(),
    )
}

/// Every server user, one max-size page — admin user counts are tiny, so no
/// lazy paging (web: AdminUsers' MAX_USERS request).
suspend fun HalogenClient.listUsers(): List<UserData> {
    val envelope = getEnvelope(
        "admin/users",
        listOf(
            "pagination[page]" to "0",
            "pagination[size]" to "65536",
        ),
        ListSerializer(UserData.serializer()),
    )
    return envelope.data ?: throw HalogenClient.ClientError.EmptyData
}

suspend fun HalogenClient.getUser(id: Int): UserData =
    get("users/$id", emptyList(), UserData.serializer())

/// Admin: delete a user (the server refuses self-deletes independently).
/// List + delete are admin-nested; get/update stay at /users/{id}.
suspend fun HalogenClient.deleteUser(id: Int) {
    delete("admin/users/$id")
}

/// Admin: create a user (the embedded add-account flow passes
/// `isAdmin: true` — every embedded user is an admin by web policy).
suspend fun HalogenClient.createUser(
    username: String,
    password: String,
    isAdmin: Boolean?,
): UserData =
    post(
        "admin/users",
        UserStoreData(
            username = username,
            password = password,
            password_confirm = password,
            is_admin = isAdmin,
        ),
        UserStoreData.serializer(),
        UserData.serializer(),
    )

// MARK: - OPML

suspend fun HalogenClient.opmlExport(): String =
    get("admin/opml/export", emptyList(), OpmlExportData.serializer()).opml

suspend fun HalogenClient.opmlImport(opml: String): OpmlImportResultData =
    post(
        "admin/opml/import",
        OpmlImportData(opml = opml),
        OpmlImportData.serializer(),
        OpmlImportResultData.serializer(),
    )

// MARK: - raw JSON reads (admin config surfaces — no typed DTO needed)

/// Any envelope endpoint's `data` as a raw JSON object (config + metadata
/// surfaces).
suspend fun HalogenClient.rawJson(path: String): JsonObject {
    val builder = Request.Builder().url(url(path)).get()
    token?.let { builder.header("Authorization", "Bearer $it") }
    val response = Http.client.newCall(builder.build()).await()
    val status = response.code
    val body = response.readBody()
    if (status !in 200..299) throw HalogenClient.ClientError.Http(status)
    val payload = try {
        (WireJson.json.parseToJsonElement(body) as? JsonObject)?.get("data") as? JsonObject
    } catch (e: SerializationException) {
        DeviceLog.warn("management: $path decode failed — ${e::class.simpleName}: ${e.message}")
        null
    }
    return payload ?: throw HalogenClient.ClientError.EmptyData
}

/// The current overrides set, typed (unset keys decode as null).
suspend fun HalogenClient.configOverrides(): ConfigOverridesData =
    get("admin/config-overrides", emptyList(), ConfigOverridesData.serializer())

/// Replace the config-overrides set wholesale with TYPED values (the server
/// deserializes `ConfigOverridesData`; strings would 422). An empty struct
/// clears none — use DELETE for clear-all.
suspend fun HalogenClient.setConfigOverrides(overrides: ConfigOverridesData) {
    postEmpty("admin/config-overrides", overrides, ConfigOverridesData.serializer())
}

suspend fun HalogenClient.clearConfigOverrides() {
    delete("admin/config-overrides")
}

// MARK: - database transfer (raw bytes)

suspend fun HalogenClient.dbExport(): Pair<ByteArray, String> {
    val builder = Request.Builder().url(url("admin/db/export")).get()
    token?.let { builder.header("Authorization", "Bearer $it") }
    Http.client.newCall(builder.build()).await().use { response ->
        if (response.code !in 200..299) throw HalogenClient.ClientError.Http(response.code)
        val disposition = response.header("Content-Disposition") ?: ""
        val filename = disposition.split("\"").getOrNull(1) ?: "halogen-export.db.gz"
        // Multi-MB body: drain off the caller's dispatcher (readBody() rule).
        val bytes = withContext(Dispatchers.IO) { response.body?.bytes() ?: ByteArray(0) }
        return Pair(bytes, filename)
    }
}

/// Upload an export (gzipped or raw SQLite); returns the typed merge summary
/// (the embedded flow needs `created_usernames`).
suspend fun HalogenClient.dbImport(payload: ByteArray): DbImportSummaryData {
    val builder = Request.Builder()
        .url(url("admin/db/import"))
        .post(payload.toRequestBody("application/octet-stream".toMediaType()))
    token?.let { builder.header("Authorization", "Bearer $it") }
    // Mutation: never transparently re-POSTed on a dropped connection.
    val response = Http.mutationClient.newCall(builder.build()).await()
    val status = response.code
    val body = response.readBody()
    if (status !in 200..299) throw HalogenClient.ClientError.Http(status)
    val envelope = WireJson.json.decodeFromString(
        ResponseData.serializer(DbImportSummaryData.serializer()), body)
    envelope.errors?.let { throw HalogenClient.ClientError.Api(it) }
    return envelope.data ?: throw HalogenClient.ClientError.EmptyData
}
