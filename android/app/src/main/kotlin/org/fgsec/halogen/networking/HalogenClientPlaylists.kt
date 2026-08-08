package org.fgsec.halogen.networking

import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import org.fgsec.halogen.wire.DefaultPlaylistData
import org.fgsec.halogen.wire.EpisodeData
import org.fgsec.halogen.wire.EpisodePlaylistData
import org.fgsec.halogen.wire.EpisodePlaylistMoveData
import org.fgsec.halogen.wire.EpisodePlaylistStoreData
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaylistData
import org.fgsec.halogen.wire.PlaylistReorderData
import org.fgsec.halogen.wire.PlaylistReorderField
import org.fgsec.halogen.wire.PlaylistStoreData
import org.fgsec.halogen.wire.PlaylistUpdateData

/// Body for `POST /playlists/{id}/move` (the wire `PlaylistMoveData` isn't in
/// the generated set yet — same shape, target index only).
@Serializable
private data class PlaylistMoveBody(val to: Int)

// Playlist slice of the API client. The queue is not a separate concept —
// it's the user's `is_default` playlist (`GET /playlists/default`).

suspend fun HalogenClient.playlists(): List<PlaylistData> =
    get(
        "playlists",
        listOf(
            "includes[0]" to "EpisodeIds",
            // Server default page size is 10 — a curated playlist list fits
            // one big page.
            "pagination[page]" to "0",
            "pagination[size]" to "500",
        ),
        ListSerializer(PlaylistData.serializer()),
    )

/// One playlist by id — the deep-link fetch-through, episode ids included so
/// the detail can render its list (web `get_playlist`).
suspend fun HalogenClient.playlist(id: Int): PlaylistData =
    get("playlists/$id", listOf("includes[0]" to "EpisodeIds"), PlaylistData.serializer())

/// The queue, or null when the user has no default playlist yet. The wrapper
/// (`DefaultPlaylistData`) keeps "no queue" distinct from the envelope's
/// missing-data error.
suspend fun HalogenClient.defaultPlaylist(): PlaylistData? =
    get("playlists/default", emptyList(), DefaultPlaylistData.serializer()).playlist

/// A playlist's episodes in pivot (position) order, podcast embedded. The order
/// MUST be requested explicitly: the param-less default is id ASC, which landed
/// the queue's add-to-front rows at the END on iOS.
suspend fun HalogenClient.playlistEpisodes(
    playlistId: Int,
    pageSize: Int = 500,
): List<EpisodeData> =
    get(
        "playlists/$playlistId/episodes",
        listOf(
            "pagination[page]" to "0",
            "pagination[size]" to pageSize.toString(),
            "includes[0]" to "Podcast",
            "order[order_by]" to "position",
            "order[direction]" to "Asc",
        ),
        ListSerializer(EpisodeData.serializer()),
    )

/// Create a playlist with the web form's full field set (name + description +
/// make-default + the two delete-on-remove cleanup flags).
suspend fun HalogenClient.createPlaylist(
    name: String,
    description: String? = null,
    isDefault: Boolean = false,
    deleteServerFile: Boolean = false,
    deleteClientFile: Boolean = false,
): PlaylistData =
    post(
        "playlists",
        PlaylistStoreData(
            name = name,
            description = description,
            is_default = isDefault,
            on_remove_delete_file_server = deleteServerFile,
            on_remove_delete_file_client = deleteClientFile,
        ),
        PlaylistStoreData.serializer(),
        PlaylistData.serializer(),
    )

suspend fun HalogenClient.deletePlaylist(id: Int) {
    delete("playlists/$id")
}

/// Partial playlist update (`PUT /playlists/{id}`; null = leave unchanged) —
/// the edit form's direct path and the outbox drain path for offline-queued
/// edits (web: `OutboxOp::UpdatePlaylist`).
suspend fun HalogenClient.updatePlaylist(
    id: Int,
    name: String?,
    isDefault: Boolean?,
    description: String? = null,
    deleteServerFile: Boolean? = null,
    deleteClientFile: Boolean? = null,
) {
    put(
        "playlists/$id",
        PlaylistUpdateData(
            name = name,
            description = description,
            is_default = isDefault,
            on_remove_delete_file_server = deleteServerFile,
            on_remove_delete_file_client = deleteClientFile,
        ),
        PlaylistUpdateData.serializer(),
        PlaylistData.serializer(),
    )
}

/// Move a playlist within the user's manual (`position`) order —
/// `POST /playlists/{id}/move` (web: `commands::move_playlist`).
suspend fun HalogenClient.movePlaylist(id: Int, to: Int) {
    postEmpty("playlists/$id/move", PlaylistMoveBody(to = to), PlaylistMoveBody.serializer())
}

/// Promote a playlist to be the queue (server demotes the old default).
suspend fun HalogenClient.makeQueuePlaylist(id: Int) {
    put(
        "playlists/$id",
        PlaylistUpdateData(
            name = null,
            description = null,
            is_default = true,
            on_remove_delete_file_server = null,
            on_remove_delete_file_client = null,
        ),
        PlaylistUpdateData.serializer(),
        PlaylistData.serializer(),
    )
}

/// Smart-reorder the playlist's episodes by field/direction (bakes into the
/// position order).
suspend fun HalogenClient.reorderPlaylist(
    id: Int,
    field: PlaylistReorderField,
    direction: OrderDirection,
) {
    postEmpty(
        "playlists/$id/reorder-by",
        PlaylistReorderData(field = field, direction = direction),
        PlaylistReorderData.serializer(),
    )
}

suspend fun HalogenClient.addEpisode(playlistId: Int, episodeId: Int, position: Int?) {
    post(
        "playlists/$playlistId/episodes/$episodeId",
        EpisodePlaylistStoreData(position = position),
        EpisodePlaylistStoreData.serializer(),
        EpisodePlaylistData.serializer(),
    )
}

suspend fun HalogenClient.removeEpisode(playlistId: Int, episodeId: Int) {
    delete("playlists/$playlistId/episodes/$episodeId")
}

/// Move an episode to index `to` within the playlist's position order.
suspend fun HalogenClient.moveEpisode(playlistId: Int, episodeId: Int, to: Int) {
    postEmpty(
        "playlists/$playlistId/episodes/$episodeId/move",
        EpisodePlaylistMoveData(to = to),
        EpisodePlaylistMoveData.serializer(),
    )
}
