package org.fgsec.halogen.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.launch
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import org.fgsec.halogen.storage.LocalStore

/// Tolerant enum coding: an unknown stored token falls back to the default
/// (iOS decodes per-field with try?; the web's from_str_or_default).
internal open class TokenEnumSerializer<T : Enum<T>>(
    name: String,
    private val entries: List<T>,
    private val token: (T) -> String,
    private val fallback: T,
) : KSerializer<T> {
    override val descriptor: SerialDescriptor =
        PrimitiveSerialDescriptor(name, PrimitiveKind.STRING)

    override fun deserialize(decoder: Decoder): T {
        val raw = decoder.decodeString()
        return entries.firstOrNull { token(it) == raw } ?: fallback
    }

    override fun serialize(encoder: Encoder, value: T) {
        encoder.encodeString(token(value))
    }
}

internal object UISizeSerializer : TokenEnumSerializer<ClientPrefs.UISize>(
    "UISize", ClientPrefs.UISize.entries, { it.token }, ClientPrefs.UISize.Medium)

internal object AppThemeSerializer : TokenEnumSerializer<ClientPrefs.AppTheme>(
    "AppTheme", ClientPrefs.AppTheme.entries, { it.token }, ClientPrefs.AppTheme.Dark)

internal object PlaybackStrategySerializer : TokenEnumSerializer<ClientPrefs.PlaybackStrategy>(
    "PlaybackStrategy", ClientPrefs.PlaybackStrategy.entries, { it.token },
    ClientPrefs.PlaybackStrategy.DownloadOnly)

/// Per-account client preferences — the native ClientConfig: UI size/theme,
/// playback behavior, and device-download chunking. Persisted per account.
@Serializable
data class ClientPrefs(
    val uiSize: UISize = UISize.Medium,
    val theme: AppTheme = AppTheme.Dark,
    val playbackStrategy: PlaybackStrategy = PlaybackStrategy.DownloadOnly,
    /// Seconds for the player's skip buttons + media-session commands.
    val skipForwardSecs: Int = 30,
    val skipBackSecs: Int = 15,
    val defaultRate: Float = 1.0f,
    /// Auto-play the next queue/playlist item when an episode ends — the
    /// web's PlaybackPrefs.auto_advance (default true).
    val autoAdvance: Boolean = true,
    /// Single "Add to Queue" inserts at the FRONT of the queue (newest first)
    /// — the web's add_to_queue_front (default true).
    val addToQueueFront: Boolean = true,
    /// Minutes the sleep timer arms with (tap or auto-arm) — the web's
    /// default_sleep_minutes.
    val defaultSleepMinutes: Int = 30,
    /// Auto-arm the sleep timer once per listening session — the web's
    /// sleep_by_default.
    val sleepByDefault: Boolean = false,
    /// Device-download chunk size (KiB) — each chunk is an independent
    /// resumable unit. `0` = no chunking (one streamed whole-file request).
    /// Mirrors the web's DownloadChunkSize (2–32 MB + NoChunking, default 4 MB).
    val downloadChunkKiB: Int = 4096,
    /// Concurrent chunk fetches WITHIN one download (writes still land in
    /// byte order) — the web's DownloadPrefs.parallelism (default 1 =
    /// sequential). Ignored under no-chunking (a single request).
    val downloadParallelism: Int = 1,
    /// Discover providers the user toggled OFF (raw wire tokens) — the web's
    /// ClientConfig.disabled_discover_providers.
    val disabledDiscoverProviders: List<String> = emptyList(),
    /// Bluetooth/media-session next/previous-track actions SEEK within the
    /// episode instead of switching episodes — the web's
    /// PlaybackPrefs.media_next_prev_seek (default false).
    val mediaNextPrevSeek: Boolean = false,
) {
    @Serializable(with = UISizeSerializer::class)
    enum class UISize(val token: String) {
        Small("small"),
        Medium("medium"),
        Large("large"),
        Xlarge("xlarge");

        val label: String
            get() = when (this) {
                Small -> "Small (100%)"
                Medium -> "Medium (125%, default)"
                Large -> "Large (135%)"
                Xlarge -> "XLarge (165%)"
            }

        /// Compose fontScale override — the web's ladder (Small 100% /
        /// Medium 125% — the default — / larger steps up).
        val fontScale: Float
            get() = when (this) {
                Small -> 1.00f
                Medium -> 1.25f
                Large -> 1.35f
                Xlarge -> 1.65f
            }
    }

    @Serializable(with = AppThemeSerializer::class)
    enum class AppTheme(val token: String) {
        Dark("dark"),
        Light("light");

        val label: String
            get() = token.replaceFirstChar { it.uppercase() }

        val isDark: Boolean
            get() = this == Dark
    }

    /// How the play button sources audio — the web's PlaybackPreference, same tokens.
    /// Embedded accounts are forced to StreamOnly (the media already lives on-device).
    @Serializable(with = PlaybackStrategySerializer::class)
    enum class PlaybackStrategy(val token: String) {
        DownloadOnly("DownloadOnly"),
        StreamFirstAndDownload("StreamFirstAndDownload"),
        StreamFallback("StreamFallback"),
        StreamOnly("StreamOnly");

        val label: String
            get() = when (this) {
                DownloadOnly -> "Download only (local-first)"
                StreamFirstAndDownload -> "Stream first, download in background"
                StreamFallback -> "Local first, stream as fallback"
                StreamOnly -> "Stream only"
            }
    }

    /// Snap stored values onto the web-parity option sets — pre-parity
    /// builds persisted chunk sizes that are no longer offered (an out-of-set
    /// value falls back to the default, the web's from_str_or_default).
    fun normalized(): ClientPrefs = copy(
        downloadChunkKiB =
            if (downloadChunkKiB in downloadChunkKiBOptions) downloadChunkKiB
            else default.downloadChunkKiB,
        downloadParallelism =
            if (downloadParallelism in downloadParallelisms) downloadParallelism
            else default.downloadParallelism,
    )

    companion object {
        val default = ClientPrefs()

        /// The web's PLAYBACK_RATES — shared by the settings picker and the
        /// player's speed menu so both offer the same set.
        val playbackRates: List<Float> = listOf(0.5f, 0.75f, 1.0f, 1.25f, 1.5f, 1.75f, 2.0f)

        /// The web's SLEEP_DURATIONS (minutes) for the settings picker.
        val sleepDurations: List<Int> = listOf(5, 10, 15, 20, 30, 45, 60, 90, 120)

        /// The web's DownloadChunkSize options in KiB (2–32 MB; 0 = no chunking).
        val downloadChunkKiBOptions: List<Int> = listOf(2048, 4096, 8192, 16384, 32768, 0)

        /// The web's DOWNLOAD_PARALLELISMS.
        val downloadParallelisms: List<Int> = listOf(1, 2, 4, 8)
    }
}

/// Reactive holder + per-account persistence.
class ClientPrefsModel(
    private val store: LocalStore?,
    private val scope: CoroutineScope = MainScope(),
) {
    var prefs: ClientPrefs by mutableStateOf(ClientPrefs.default)
        private set

    suspend fun load() {
        val saved = store?.load<ClientPrefs>(KEY) ?: return
        prefs = saved.normalized()
    }

    fun update(mutate: (ClientPrefs) -> ClientPrefs) {
        prefs = mutate(prefs)
        val snapshot = prefs
        scope.launch { store?.save(snapshot, KEY) }
    }

    private companion object {
        const val KEY = "client-prefs"
    }
}
