package org.fgsec.halogen.storage

import kotlinx.serialization.json.*
import org.fgsec.halogen.networking.WireJson

/** Cache projections are replayable across a crash between any two file replacements. */
suspend fun LocalStore.project(operation: OutboxOp): Boolean {
    val kind = operation.kind
    val keys = listKeys().filterNot { it.startsWith("outbox") }.toMutableSet()
    when (kind) {
        is OutboxOp.Kind.SetCursor, is OutboxOp.Kind.SetPlayed -> keys.add(CacheKey.playbacks)
        is OutboxOp.Kind.Unsubscribe -> keys.add(CacheKey.podcastTombstones)
        is OutboxOp.Kind.SetAutoPlaylists -> keys.add(CacheKey.autoPlaylists(kind.podcastId))
        else -> Unit
    }
    if (kind is OutboxOp.Kind.AddToPlaylist) keys.add(CacheKey.playlistEpisodes(kind.playlistId))
    val addedEpisode = if (kind is OutboxOp.Kind.AddToPlaylist)
        load(JsonElement.serializer(), CacheKey.episode(kind.episodeId)) else null
    val removedEpisodes = if (kind is OutboxOp.Kind.Unsubscribe) {
        val ids = mutableSetOf<Int>()
        for (key in keys) {
            val value = load(JsonElement.serializer(), key)
            val rows = if (value is JsonArray) value else listOfNotNull(value)
            for (row in rows) {
                val episode = row as? JsonObject ?: continue
                if (episode.id("podcast_id") == kind.podcastId && "content_url" in episode) episode.id("id")?.let(ids::add)
            }
        }
        ids
    } else emptySet()
    val newDefault = if (kind is OutboxOp.Kind.UpdatePlaylist && kind.isDefault == true) {
        keys.add(CacheKey.queueMeta)
        (load(JsonElement.serializer(), CacheKey.playlists) as? JsonArray)?.firstOrNull {
            (it as? JsonObject)?.id("id") == kind.playlistId
        }
    } else null
    for (key in keys.filter { isAffectedKey(it, kind) }.sortedBy { if (it == CacheKey.playbacks || it == CacheKey.playlists || it == CacheKey.queueMeta) 0 else 1 }) {
        projectCache(key, operation.id) { payload ->
            projectPayload(key, if (key == CacheKey.queueMeta && newDefault != null) newDefault else payload, kind, addedEpisode, removedEpisodes)
        }
    }
    return true
}

private fun projectPayload(key: String, payload: JsonElement?, kind: OutboxOp.Kind, added: JsonElement?, removedEpisodes: Set<Int>): JsonElement? {
    if (kind is OutboxOp.Kind.Unsubscribe && key == CacheKey.playbacks && payload is JsonObject) {
        return JsonObject(payload.filterKeys { it.toIntOrNull() !in removedEpisodes })
    }
    if (kind is OutboxOp.Kind.Unsubscribe && (key == CacheKey.playlists || key == CacheKey.queueMeta)) {
        fun prune(row: JsonElement): JsonElement {
            val playlist = row as? JsonObject ?: return row
            val members = playlist["episode_ids"] as? JsonArray ?: return row
            return JsonObject(playlist + ("episode_ids" to JsonArray(members.filter { it.jsonPrimitive.intOrNull !in removedEpisodes })))
        }
        return if (payload is JsonArray) JsonArray(payload.map(::prune)) else payload?.let(::prune)
    }
    if (key == CacheKey.playbacks) return playbackProjection(payload, kind)
    if (kind is OutboxOp.Kind.Unsubscribe && key == CacheKey.podcastTombstones) {
        return JsonArray(((payload as? JsonArray).orEmpty() + JsonPrimitive(kind.podcastId)).distinct())
    }
    if (kind is OutboxOp.Kind.SetAutoPlaylists && key == CacheKey.autoPlaylists(kind.podcastId)) {
        return buildJsonObject {
            put("playlistIds", JsonArray(kind.playlistIds.map(::JsonPrimitive)))
            put("addToStart", kind.addToStart?.let(::JsonPrimitive) ?: JsonNull)
        }
    }
    if (key == CacheKey.playlists || key == CacheKey.queueMeta) return playlistProjection(payload, kind)
    if (kind is OutboxOp.Kind.RemoveServerDownload && key in setOf("downloads-downloaded", "downloads-downloading") && payload is JsonArray) {
        return JsonArray(payload.filterNot { (it as? JsonObject)?.id("id") == kind.episodeId })
    }
    if (key.startsWith("playlist-episodes-")) {
        val id = key.removePrefix("playlist-episodes-").toIntOrNull()
        val rows = (payload as? JsonArray) ?: if (kind is OutboxOp.Kind.AddToPlaylist) JsonArray(emptyList()) else null
        if (id != null && rows != null) return projectResources(episodeMembership(rows, id, kind, added), kind)
    }
    val isEpisodes = key.startsWith("episode-") || key.startsWith("podcast-episodes-") ||
        key.startsWith("latest-") || key.startsWith("downloads-") || key == CacheKey.history
    if (key == CacheKey.podcasts || isEpisodes) return projectResources(payload, kind)
    return payload
}

private fun playbackProjection(payload: JsonElement?, kind: OutboxOp.Kind): JsonElement? {
    val id = when (kind) {
        is OutboxOp.Kind.SetCursor -> kind.episodeId
        is OutboxOp.Kind.SetPlayed -> kind.episodeId
        else -> return payload
    }.toString()
    val rows = (payload as? JsonObject).orEmpty().toMutableMap()
    rows[id] = buildJsonObject {
        put("cursor", if (kind is OutboxOp.Kind.SetCursor) JsonPrimitive(kind.cursor.toLong()) else JsonPrimitive(0))
        put("completed", if (kind is OutboxOp.Kind.SetPlayed) JsonPrimitive(kind.played) else JsonPrimitive(false))
        put("updatedAt", System.currentTimeMillis())
    }
    return JsonObject(rows)
}

private fun projectResources(payload: JsonElement?, kind: OutboxOp.Kind): JsonElement? {
    if (payload is JsonArray) return JsonArray(payload.mapNotNull { row ->
        projectResources(row, kind)?.takeUnless { it == JsonNull }
    })
    val row = payload as? JsonObject ?: return payload
    if (kind is OutboxOp.Kind.Unsubscribe &&
        (row.id("podcast_id") == kind.podcastId || ("feed_url" in row && row.id("id") == kind.podcastId))) return JsonNull
    val changes = row.toMutableMap()
    when (kind) {
        is OutboxOp.Kind.TriggerDownload -> if (row.id("id") == kind.episodeId && "content_url" in row)
            changes["download_status"] = JsonPrimitive("DOWNLOADING")
        is OutboxOp.Kind.RemoveServerDownload -> if (row.id("id") == kind.episodeId && "content_url" in row)
            changes["download_status"] = JsonPrimitive("NOT_DOWNLOADED")
        is OutboxOp.Kind.UpdatePodcastConfig -> if (row.id("podcast_config_id") == kind.configId) {
            val fields = WireJson.json.encodeToJsonElement(org.fgsec.halogen.wire.PodcastConfigUpdateData.serializer(), kind.data).jsonObject.filterValues { it != JsonNull }
            changes["podcast_config"] = JsonObject((row["podcast_config"] as? JsonObject).orEmpty() + fields)
        }
        is OutboxOp.Kind.RemovePodcastConfig -> if ("feed_url" in row && row.id("id") == kind.podcastId) {
            changes["podcast_config_id"] = JsonNull
            changes["podcast_config"] = JsonNull
        }
        else -> Unit
    }
    return JsonObject(changes)
}

internal fun JsonObject.id(key: String): Int? = (get(key) as? JsonPrimitive)?.intOrNull

private fun isAffectedKey(key: String, kind: OutboxOp.Kind): Boolean = when (kind) {
    is OutboxOp.Kind.SetCursor, is OutboxOp.Kind.SetPlayed -> key == CacheKey.playbacks
    is OutboxOp.Kind.SetAutoPlaylists -> key == CacheKey.autoPlaylists(kind.podcastId)
    is OutboxOp.Kind.Subscribe -> false
    is OutboxOp.Kind.UpdatePodcastConfig, is OutboxOp.Kind.RemovePodcastConfig -> key == CacheKey.podcasts
    is OutboxOp.Kind.UpdatePlaylist, is OutboxOp.Kind.MovePlaylist -> key == CacheKey.playlists || key == CacheKey.queueMeta
    is OutboxOp.Kind.AddToPlaylist -> key == CacheKey.playlists || key == CacheKey.queueMeta || key == CacheKey.playlistEpisodes(kind.playlistId)
    is OutboxOp.Kind.RemoveFromPlaylist -> key == CacheKey.playlists || key == CacheKey.queueMeta || key == CacheKey.playlistEpisodes(kind.playlistId)
    is OutboxOp.Kind.MoveInPlaylist -> key == CacheKey.playlists || key == CacheKey.queueMeta || key == CacheKey.playlistEpisodes(kind.playlistId)
    is OutboxOp.Kind.ReorderPlaylist -> key == CacheKey.playlistEpisodes(kind.playlistId)
    is OutboxOp.Kind.TriggerDownload, is OutboxOp.Kind.RemoveServerDownload -> key.startsWith("episode-") || key.startsWith("podcast-episodes-") || key.startsWith("playlist-episodes-") || key.startsWith("latest-") || key.startsWith("downloads-") || key == CacheKey.history
    is OutboxOp.Kind.Unsubscribe -> key == CacheKey.playbacks || key == CacheKey.playlists || key == CacheKey.queueMeta || key == CacheKey.podcasts || key == CacheKey.podcastTombstones || key.startsWith("episode-") || key.startsWith("podcast-episodes-") || key.startsWith("playlist-episodes-") || key.startsWith("latest-") || key.startsWith("downloads-") || key == CacheKey.history
}
