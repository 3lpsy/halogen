package org.fgsec.halogen.storage

import kotlinx.serialization.json.*
import org.fgsec.halogen.wire.OrderDirection
import org.fgsec.halogen.wire.PlaylistReorderField

internal fun playlistProjection(payload: JsonElement?, kind: OutboxOp.Kind): JsonElement? {
    if (payload is JsonArray) {
        var rows = payload.map { playlistProjection(it, kind) ?: it }
        if (kind is OutboxOp.Kind.MovePlaylist) {
            val moved = rows.firstOrNull { (it as? JsonObject)?.id("id") == kind.playlistId }
            if (moved != null) {
                val ordered = rows.sortedBy { (it as? JsonObject)?.id("position") ?: Int.MAX_VALUE }.toMutableList()
                ordered.remove(moved)
                ordered.add(kind.to.coerceIn(0, ordered.size), moved)
                rows = ordered.mapIndexed { index, row -> JsonObject(row.jsonObject + ("position" to JsonPrimitive(index))) }
            }
        }
        return JsonArray(rows)
    }
    val row = payload as? JsonObject ?: return payload
    val id = row.id("id") ?: return payload
    val changes = row.toMutableMap()
    if (kind is OutboxOp.Kind.UpdatePlaylist) {
        if (kind.isDefault == true) changes["is_default"] = JsonPrimitive(id == kind.playlistId)
        if (id == kind.playlistId) {
            kind.name?.let { changes["name"] = JsonPrimitive(it) }
            kind.description?.let { changes["description"] = JsonPrimitive(it) }
            kind.isDefault?.let { changes["is_default"] = JsonPrimitive(it) }
            kind.deleteServerFile?.let { changes["on_remove_delete_file_server"] = JsonPrimitive(it) }
            kind.deleteClientFile?.let { changes["on_remove_delete_file_client"] = JsonPrimitive(it) }
        }
    }
    val members = (row["episode_ids"] as? JsonArray)?.toMutableList()
    if (members != null) {
        when (kind) {
            is OutboxOp.Kind.AddToPlaylist -> if (id == kind.playlistId && JsonPrimitive(kind.episodeId) !in members)
                members.add((kind.position ?: members.size).coerceIn(0, members.size), JsonPrimitive(kind.episodeId))
            is OutboxOp.Kind.RemoveFromPlaylist -> if (id == kind.playlistId) members.remove(JsonPrimitive(kind.episodeId))
            is OutboxOp.Kind.MoveInPlaylist -> if (id == kind.playlistId && members.remove(JsonPrimitive(kind.episodeId)))
                members.add(kind.to.coerceIn(0, members.size), JsonPrimitive(kind.episodeId))
            else -> Unit
        }
        changes["episode_ids"] = JsonArray(members)
    }
    return JsonObject(changes)
}

internal fun episodeMembership(payload: JsonArray, playlistId: Int, kind: OutboxOp.Kind, added: JsonElement?): JsonArray {
    val rows = payload.toMutableList()
    when (kind) {
        is OutboxOp.Kind.AddToPlaylist -> if (kind.playlistId == playlistId && added != null && rows.none { it.jsonObject.id("id") == kind.episodeId })
            rows.add((kind.position ?: rows.size).coerceIn(0, rows.size), added)
        is OutboxOp.Kind.RemoveFromPlaylist -> if (kind.playlistId == playlistId)
            rows.removeAll { it.jsonObject.id("id") == kind.episodeId }
        is OutboxOp.Kind.MoveInPlaylist -> if (kind.playlistId == playlistId) {
            val moved = rows.firstOrNull { it.jsonObject.id("id") == kind.episodeId }
            if (moved != null) { rows.remove(moved); rows.add(kind.to.coerceIn(0, rows.size), moved) }
        }
        is OutboxOp.Kind.ReorderPlaylist -> if (kind.playlistId == playlistId) {
            val field = when (kind.field) {
                PlaylistReorderField.Published -> "published_at"
                PlaylistReorderField.Title -> "title"
                PlaylistReorderField.Duration -> "duration_secs"
                PlaylistReorderField.Added -> return payload
            }
            val comparator = if (kind.field == PlaylistReorderField.Duration)
                compareBy<JsonElement> { it.jsonObject.id(field) ?: 0 }
            else compareBy<JsonElement> { (it.jsonObject[field] as? JsonPrimitive)?.content.orEmpty().lowercase() }
            return JsonArray(rows.sortedWith(if (kind.direction == OrderDirection.Asc) comparator else comparator.reversed()))
        }
        else -> Unit
    }
    return JsonArray(rows)
}
