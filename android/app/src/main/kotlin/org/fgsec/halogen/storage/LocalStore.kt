package org.fgsec.halogen.storage

import android.content.Context
import java.io.File
import java.io.FileOutputStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.serializer
import org.fgsec.halogen.features.latest.EpisodeFilter
import org.fgsec.halogen.networking.WireJson

// Per-account local cache (stale-while-revalidate snapshots) — the native
// counterpart of the web client's IndexedDB LocalStore (webui/store).
// One JSON file per key under filesDir/halogen-client/<namespace>/<key>.json;
// the namespace (AccountContext) keeps accounts from sharing a cache.
class LocalStore(context: Context, namespace: String) {
    private val dir: File =
        File(File(context.filesDir, "halogen-client"), namespace).apply { mkdirs() }
    private val mutex = Mutex()

    val syncDatabasePath: String get() = File(dir, "sync.sqlite").absolutePath

    suspend fun prepareOutboxMigration(): String? = mutex.withLock {
        withContext(Dispatchers.IO) {
            val source = fileFor("outbox")
            if (!source.exists()) return@withContext null
            val original = source.readText()
            val backup = fileFor("outbox-v1-backup")
            if (!backup.exists()) backup.writeText(original)
            cachePayload(WireJson.json.parseToJsonElement(original)).toString()
        }
    }

    private fun readRaw(key: String): JsonElement? =
        fileFor(key).takeIf { it.exists() }?.readText()?.let(WireJson.json::parseToJsonElement)

    private fun writeRaw(key: String, data: JsonElement) {
        val tmp = File(dir, "$key.json.tmp")
        FileOutputStream(tmp).use { stream ->
            stream.write(data.toString().toByteArray(Charsets.UTF_8))
            stream.fd.sync()
        }
        check(tmp.renameTo(fileFor(key))) { "could not persist $key" }
    }

    suspend fun <T> saveDurably(serializer: KSerializer<T>, value: T, key: String) {
        mutex.withLock {
            withContext(Dispatchers.IO) {
                val payload = WireJson.json.encodeToJsonElement(serializer, value)
                writeRaw(key, cacheEnvelope(payload, cacheReceipts(readRaw(key))))
            }
        }
    }
    suspend inline fun <reified T> saveDurably(value: T, key: String) = saveDurably(serializer<T>(), value, key)

    suspend fun <T> load(serializer: KSerializer<T>, key: String): T? = mutex.withLock {
        withContext(Dispatchers.IO) {
            runCatching { readRaw(key)?.let { WireJson.json.decodeFromJsonElement(serializer, cachePayload(it)) } }.getOrNull()
        }
    }

    suspend fun <T> save(serializer: KSerializer<T>, value: T, key: String) {
        try { saveDurably(serializer, value, key) }
        catch (error: kotlinx.coroutines.CancellationException) { throw error }
        catch (_: Exception) { }
    }

    internal suspend fun replaceSnapshot(key: String, payload: JsonElement) {
        mutex.withLock { withContext(Dispatchers.IO) { writeRaw(key, payload) } }
    }

    /** Receipts can be pruned after the canonical journal no longer contains their payload. */
    internal suspend fun forgetJournalMarkers(ids: Set<String>) {
        if (ids.isEmpty()) return
        mutex.withLock {
            withContext(Dispatchers.IO) {
                for (file in dir.listFiles().orEmpty().filter { it.extension == "json" }) {
                    val key = file.nameWithoutExtension
                    val raw = readRaw(key) ?: continue
                    val receipts = cacheReceipts(raw)
                    if (receipts.any { it in ids }) {
                        writeRaw(key, cacheEnvelope(cachePayload(raw), receipts - ids))
                    }
                }
            }
        }
    }

    /** Apply payload and its replay receipt in the same atomic cache-file replacement. */
    internal suspend fun projectCache(key: String, operationId: String, transform: (JsonElement?) -> JsonElement?) {
        mutex.withLock {
            withContext(Dispatchers.IO) {
                val raw = readRaw(key)
                val receipts = cacheReceipts(raw)
                if (operationId in receipts) return@withContext
                val updated = transform(raw?.let(::cachePayload)) ?: return@withContext
                writeRaw(key, cacheEnvelope(updated, receipts + operationId))
            }
        }
    }

    suspend inline fun <reified T> load(key: String): T? = load(serializer<T>(), key)

    suspend inline fun <reified T> save(value: T, key: String) =
        save(serializer<T>(), value, key)

    suspend fun remove(key: String) {
        mutex.withLock { withContext(Dispatchers.IO) { fileFor(key).delete() } }
    }

    // Every stored key (file names sans .json) — the purge screen's census.
    suspend fun listKeys(): List<String> =
        mutex.withLock {
            withContext(Dispatchers.IO) {
                (dir.list() ?: emptyArray())
                    .filter { it.endsWith(".json") }
                    .map { it.dropLast(5) }
            }
        }

    // Total bytes across a set of keys.
    suspend fun bytes(forKeys: List<String>): ULong =
        mutex.withLock {
            withContext(Dispatchers.IO) {
                forKeys.fold(0UL) { sum, key -> sum + fileFor(key).length().toULong() }
            }
        }

    suspend fun remove(keys: List<String>) {
        mutex.withLock {
            withContext(Dispatchers.IO) { keys.forEach { fileFor(it).delete() } }
        }
    }

    // Wipe the whole namespace (account sign-out / cache purge).
    suspend fun wipe() {
        mutex.withLock {
            withContext(Dispatchers.IO) {
                dir.deleteRecursively()
                dir.mkdirs()
            }
        }
    }

    private fun fileFor(key: String): File = File(dir, "$key.json")
}

// Element-tolerant array decoding: List<Lossy<T>> decodes every element
// independently, so one undecodable element (schema drift across builds)
// yields a null instead of failing the whole array. Callers mapNotNull.
@Serializable(with = LossySerializer::class)
class Lossy<T>(val value: T?)

class LossySerializer<T>(private val element: KSerializer<T>) : KSerializer<Lossy<T>> {
    @OptIn(ExperimentalSerializationApi::class)
    override val descriptor: SerialDescriptor =
        SerialDescriptor("org.fgsec.halogen.storage.Lossy", element.descriptor)

    override fun deserialize(decoder: Decoder): Lossy<T> {
        val input = decoder as JsonDecoder
        val raw = input.decodeJsonElement()
        return Lossy(runCatching { input.json.decodeFromJsonElement(element, raw) }.getOrNull())
    }

    override fun serialize(encoder: Encoder, value: Lossy<T>) {
        val v = value.value
        if (v != null) element.serialize(encoder, v)
        else (encoder as JsonEncoder).encodeJsonElement(JsonNull)
    }
}

// The cache-key vocabulary — one flat, stable list (the native analog of the
// web's store-key constants in ui-platform). Add entries as features land;
// never reuse a retired key for a different shape.
object CacheKey {
    const val podcasts = "podcasts"

    // Unsubscribe tombstones: podcast ids locally removed whose delete op
    // may still be queued (fetched pages filter against these).
    const val podcastTombstones = "podcast-tombstones"

    // One snapshot per chip-set (sorted tokens keep the key stable across
    // Set ordering); no chips = the canonical "all" snapshot.
    fun latest(filters: Set<EpisodeFilter>): String =
        if (filters.isEmpty()) "latest-all"
        else "latest-" + filters.map { it.rawValue }.sorted().joinToString("+")

    const val latestScrollAnchor = "latest-scroll-anchor"
    const val queueMeta = "queue-meta"
    const val playlists = "playlists"

    fun playlistEpisodes(id: Int): String = "playlist-episodes-$id"

    fun downloads(facet: String): String = "downloads-$facet"

    fun episode(id: Int): String = "episode-$id"

    // One podcast's episode list (EpisodesView's snapshot).
    fun podcastEpisodes(id: Int): String = "podcast-episodes-$id"

    // One podcast's auto-playlist selection (ids + insert-position).
    fun autoPlaylists(podcastId: Int): String = "auto-playlists-$podcastId"

    // A metadata (key→value) screen's rendered rows, by endpoint path.
    fun metadata(path: String): String = "metadata-" + path.replace("/", "-")

    const val history = "history"
    const val deviceDownloads = "device-downloads"

    // The optimistic playback overlay (PlaybackOverlayModel) — locally-known
    // cursors/played flags that outrank stale server rows until synced.
    const val playbacks = "playbacks-overlay"

    // The user-forced offline toggle, re-applied at boot (web
    // ClientConfig.manual_offline).
    const val manualOffline = "manual-offline"
}
