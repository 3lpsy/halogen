package org.fgsec.halogen.storage

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import java.util.UUID
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.json.*
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class JournalProjectionTests {
    @Test
    fun replayDoesNotOverwriteNewerAuthoritativeCache() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val namespace = "journal-test-${UUID.randomUUID()}"
        val store = LocalStore(context, namespace)
        try {
            val operation = OutboxOp("cursor-1", OutboxOp.Kind.SetCursor(7, 10uL))
            assertTrue(store.project(operation))
            val initial = store.load<JsonElement>(CacheKey.playbacks)!!.jsonObject
            assertEquals(10L, initial.getValue("7").jsonObject.getValue("cursor").jsonPrimitive.long)
            val newer = JsonObject(initial + ("7" to JsonObject(initial.getValue("7").jsonObject + ("cursor" to JsonPrimitive(30)))))
            store.saveDurably(newer, CacheKey.playbacks)

            val reopened = LocalStore(context, namespace)
            assertTrue(reopened.project(operation))
            val restored = reopened.load<JsonElement>(CacheKey.playbacks)!!.jsonObject
            assertEquals(30L, restored.getValue("7").jsonObject.getValue("cursor").jsonPrimitive.long)
        } finally { store.wipe() }
    }

    @Test
    fun replayingCompletedEpisodeClearsCompletionBeforeSync() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val store = LocalStore(context, "journal-test-${UUID.randomUUID()}")
        try {
            store.project(OutboxOp("finish", OutboxOp.Kind.SetPlayed(7, true)))
            store.project(OutboxOp("restart", OutboxOp.Kind.SetCursor(7, 10uL)))
            val playback = store.load<JsonElement>(CacheKey.playbacks)!!.jsonObject.getValue("7").jsonObject
            assertEquals(false, playback.getValue("completed").jsonPrimitive.boolean)
            assertEquals(10L, playback.getValue("cursor").jsonPrimitive.long)
        } finally { store.wipe() }
    }

    @Test
    fun membershipProjectsIntoLegacySnapshotsBeforeAcknowledgement() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val store = LocalStore(context, "journal-test-${UUID.randomUUID()}")
        try {
            val first = buildJsonObject { put("id", 7); put("title", "First") }
            val added = buildJsonObject { put("id", 8); put("title", "Second") }
            store.saveDurably(JsonArray(listOf(first)), CacheKey.playlistEpisodes(1))
            store.saveDurably(added, CacheKey.episode(8))
            val operation = OutboxOp("add-1", OutboxOp.Kind.AddToPlaylist(1, 8, 0))
            assertTrue(store.project(operation))
            assertTrue(store.project(operation))
            val rows = store.load<JsonElement>(CacheKey.playlistEpisodes(1))!!.jsonArray
            assertEquals(listOf(8, 7), rows.map { it.jsonObject.getValue("id").jsonPrimitive.int })
        } finally { store.wipe() }
    }
    @Test
    fun interruptedSnapshotReplaysBeforePendingPlayback() = runBlocking {
        val context = ApplicationProvider.getApplicationContext<Context>()
        val store = LocalStore(context, "snapshot-test-${UUID.randomUUID()}")
        try {
            val snapshot = buildJsonObject {
                put("sync_cursor", "epoch:1")
                put("podcasts", JsonArray(emptyList()))
                put("episodes", JsonArray(emptyList()))
                put("playlists", JsonArray(emptyList()))
                put("auto_playlists", JsonObject(emptyMap()))
                put("playbacks", JsonArray(listOf(buildJsonObject {
                    put("episode_id", 7); put("cursor", 10); put("completed", false)
                    put("updated_at", "2026-01-01T00:00:00Z")
                })))
            }.toString()
            val operation = OutboxOp("snapshot-cursor", OutboxOp.Kind.SetCursor(7, 23uL))
            store.projectSnapshot(snapshot)
            store.project(operation)
            store.projectSnapshot(snapshot)
            assertEquals(23L, store.load<JsonElement>(CacheKey.playbacks)!!.jsonObject.getValue("7").jsonObject.getValue("cursor").jsonPrimitive.long)
            // Missing final marker means an interrupted snapshot must replace stale cache receipts.
            store.remove("sync-projected-cursor")
            store.projectSnapshot(snapshot)
            store.project(operation)
            assertEquals(23L, store.load<JsonElement>(CacheKey.playbacks)!!.jsonObject.getValue("7").jsonObject.getValue("cursor").jsonPrimitive.long)
        } finally { store.wipe() }
    }

}
