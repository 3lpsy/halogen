package org.fgsec.halogen.storage

import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import uniffi.halogen_mobile.QueuedOperation

/** Convert legacy native payloads without changing their durable operation IDs. */
fun OutboxOp.sharedOperation(): QueuedOperation {
    val json = Json { encodeDefaults = true }
    val fields = json.parseToJsonElement(json.encodeToString(OutboxOp.Kind.serializer(), kind)).jsonObject
    val names = mapOf(
        "setCursor" to "SetCursor", "setPlayed" to "MarkPlayed", "addToPlaylist" to "AddToPlaylist",
        "removeFromPlaylist" to "RemoveFromPlaylist", "moveInPlaylist" to "MoveInPlaylist",
        "reorderPlaylist" to "ReorderPlaylist", "updatePlaylist" to "UpdatePlaylist",
        "movePlaylist" to "MovePlaylist", "subscribe" to "Subscribe", "unsubscribe" to "Unsubscribe",
        "triggerDownload" to "TriggerDownload", "removeServerDownload" to "RemoveServerDownload",
        "updatePodcastConfig" to "UpdatePodcastConfig", "removePodcastConfig" to "RemovePodcastConfig",
        "setAutoPlaylists" to "SetPodcastAutoPlaylists",
    )
    val name = names[fields.getValue("type").jsonPrimitive.content] ?: error("unknown operation")
    val payload = fields.filterKeys { it != "type" }.flatMap { (key, value) ->
        if (key == "data") value.jsonObject.entries.map { it.key to it.value }
        else listOf(key.replace(Regex("[A-Z]")) { "_${it.value.lowercase()}" } to value)
    }.toMap().toMutableMap()
    if (name == "UpdatePlaylist") {
        payload.remove("delete_server_file")?.let { payload["on_remove_delete_file_server"] = it }
        payload.remove("delete_client_file")?.let { payload["on_remove_delete_file_client"] = it }
    }
    if (name in setOf("AddToPlaylist", "RemoveFromPlaylist", "TriggerDownload", "RemoveServerDownload")) {
        payload.remove("episode_id")?.let { payload["episode_ids"] = JsonArray(listOf(it)) }
    }
    return QueuedOperation(id, JsonObject(mapOf(name to JsonObject(payload))).toString())
}

/** Rebuild native metadata if the process stopped between journal commit and mirror persistence. */
fun restoreSharedQueue(raw: String): List<OutboxOp> {
    val json = Json { ignoreUnknownKeys = true }
    val names = mapOf(
        "SetCursor" to "setCursor", "MarkPlayed" to "setPlayed", "AddToPlaylist" to "addToPlaylist",
        "RemoveFromPlaylist" to "removeFromPlaylist", "MoveInPlaylist" to "moveInPlaylist",
        "ReorderPlaylist" to "reorderPlaylist", "UpdatePlaylist" to "updatePlaylist",
        "MovePlaylist" to "movePlaylist", "Subscribe" to "subscribe", "Unsubscribe" to "unsubscribe",
        "TriggerDownload" to "triggerDownload", "RemoveServerDownload" to "removeServerDownload",
        "UpdatePodcastConfig" to "updatePodcastConfig", "RemovePodcastConfig" to "removePodcastConfig",
        "SetPodcastAutoPlaylists" to "setAutoPlaylists",
    )
    return (json.parseToJsonElement(raw) as JsonArray).mapNotNull { entry ->
        val record = (entry as JsonArray)[1].jsonObject
        if (record["rejection"] != null && record["rejection"] != kotlinx.serialization.json.JsonNull) return@mapNotNull null
        val id = record.getValue("source_id").jsonPrimitive.content
        val operation = record.getValue("operation").jsonObject.entries.single()
        val name = names.getValue(operation.key)
        val source = operation.value.jsonObject
        val payload = mutableMapOf<String, kotlinx.serialization.json.JsonElement>()
        payload["type"] = kotlinx.serialization.json.JsonPrimitive(name)
        if (name == "updatePodcastConfig") {
            payload["configId"] = source.getValue("config_id")
            payload["data"] = JsonObject(source.filterKeys { it != "config_id" })
        } else {
            source.forEach { (key, value) ->
                val parts = key.split('_')
                val camel = parts.first() + parts.drop(1).joinToString("") { it.replaceFirstChar(Char::uppercase) }
                payload[camel] = value
            }
            (payload.remove("episodeIds") as? JsonArray)?.let {
                require(it.size == 1) { "unexpected native batch operation" }
                payload["episodeId"] = it.single()
            }
            if (name == "updatePlaylist") {
                payload.remove("onRemoveDeleteFileServer")?.let { payload["deleteServerFile"] = it }
                payload.remove("onRemoveDeleteFileClient")?.let { payload["deleteClientFile"] = it }
            }
        }
        OutboxOp(id, json.decodeFromJsonElement(OutboxOp.Kind.serializer(), JsonObject(payload)))
    }
}
