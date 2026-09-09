package org.fgsec.halogen.storage

import kotlinx.serialization.json.*
import org.fgsec.halogen.networking.WireJson

/** Publish a committed Rust snapshot; the final cursor marker makes interrupted projection retryable. */
suspend fun LocalStore.projectSnapshot(raw: String) {
    val snapshot = WireJson.json.parseToJsonElement(raw).jsonObject
    val cursor = snapshot["sync_cursor"]?.jsonPrimitive?.contentOrNull ?: return
    if (load<String>(CURSOR_KEY) == cursor) return
    val podcasts = snapshot.getValue("podcasts").jsonArray
    val episodes = snapshot.getValue("episodes").jsonArray
    val playlists = snapshot.getValue("playlists").jsonArray
    val playbacks = snapshot.getValue("playbacks").jsonArray
    val autoPlaylists = snapshot.getValue("auto_playlists").jsonObject
    val episodeById = episodes.associateBy { it.jsonObject.id("id") }
    val playlistById = playlists.associateBy { it.jsonObject.id("id") }
    val descending = episodes.sortedByDescending { it.jsonObject["published_at"]?.jsonPrimitive?.contentOrNull.orEmpty() }

    for (key in listKeys()) {
        when {
            key.startsWith("episode-") -> {
                val row = episodeById[key.removePrefix("episode-").toIntOrNull()]
                if (row == null) remove(key) else replaceSnapshot(key, row)
            }
            key.startsWith("playlist-episodes-") -> {
                val playlist = playlistById[key.removePrefix("playlist-episodes-").toIntOrNull()]
                if (playlist == null) remove(key)
                else replaceSnapshot(key, playlistEpisodes(playlist.jsonObject, episodeById))
            }
            key.startsWith("podcast-episodes-") -> {
                val id = key.removePrefix("podcast-episodes-").toIntOrNull()
                replaceSnapshot(key, JsonArray(descending.filter { it.jsonObject.id("podcast_id") == id }))
            }
            key.startsWith("auto-playlists-") -> {
                if (key.removePrefix("auto-playlists-") !in autoPlaylists) remove(key)
            }
            key.startsWith("latest-") && key != CacheKey.latestScrollAnchor && key != "latest-all" -> {
                val previous = load<JsonElement>(key) as? JsonArray ?: continue
                replaceSnapshot(key, JsonArray(previous.mapNotNull { episodeById[it.jsonObject.id("id")] }))
            }
            key.startsWith("metadata-episodes-") || key.startsWith("metadata-podcasts-") -> remove(key)
        }
    }
    replaceSnapshot(CacheKey.podcasts, podcasts)
    replaceSnapshot(CacheKey.podcastTombstones, JsonArray(emptyList()))
    replaceSnapshot(CacheKey.playlists, playlists)
    replaceSnapshot(CacheKey.queueMeta, playlists.firstOrNull { it.jsonObject["is_default"]?.jsonPrimitive?.booleanOrNull == true } ?: JsonNull)
    replaceSnapshot("latest-all", JsonArray(descending))
    for (status in listOf("downloaded", "downloading")) {
        replaceSnapshot(CacheKey.downloads(status), JsonArray(descending.filter {
            it.jsonObject["download_status"]?.jsonPrimitive?.content == status.uppercase()
        }))
    }
    for (row in episodes) replaceSnapshot(CacheKey.episode(row.jsonObject.id("id")!!), row)
    for (podcast in podcasts) {
        val id = podcast.jsonObject.id("id")!!
        replaceSnapshot(CacheKey.podcastEpisodes(id), JsonArray(descending.filter { it.jsonObject.id("podcast_id") == id }))
    }
    for (playlist in playlists) {
        replaceSnapshot(CacheKey.playlistEpisodes(playlist.jsonObject.id("id")!!), playlistEpisodes(playlist.jsonObject, episodeById))
    }
    for ((id, rows) in autoPlaylists) {
        val links = rows.jsonArray
        replaceSnapshot(CacheKey.autoPlaylists(id.toInt()), buildJsonObject {
            put("playlistIds", JsonArray(links.map { it.jsonObject.getValue("playlist_id") }))
            put("addToStart", links.firstOrNull()?.jsonObject?.get("add_to_start") ?: JsonNull)
        })
    }
    replaceSnapshot(CacheKey.playbacks, buildJsonObject {
        for (playback in playbacks) {
            val row = playback.jsonObject
            val id = row.getValue("episode_id").jsonPrimitive.content
            put(id, buildJsonObject {
                put("cursor", row.getValue("cursor"))
                put("completed", row.getValue("completed"))
                put("updatedAt", WireJson.parseInstant(row.getValue("updated_at").jsonPrimitive.content).toEpochMilli())
            })
        }
    })
    val history = playbacks.sortedByDescending { it.jsonObject.getValue("updated_at").jsonPrimitive.content }
        .mapNotNull { episodeById[it.jsonObject.id("episode_id")] }
    replaceSnapshot(CacheKey.history, JsonArray(history))
    saveDurably(cursor, CURSOR_KEY)
}

private fun playlistEpisodes(playlist: JsonObject, episodes: Map<Int?, JsonElement>): JsonArray =
    JsonArray((playlist["episode_ids"] as? JsonArray).orEmpty().mapNotNull { episodes[it.jsonPrimitive.intOrNull] })

private const val CURSOR_KEY = "sync-projected-cursor"
