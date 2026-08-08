package org.fgsec.halogen.core

import android.content.Context
import android.content.SharedPreferences
import android.content.pm.ApplicationInfo
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.io.File
import java.time.Instant
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import uniffi.halogen_mobile.DeviceLogLine
import uniffi.halogen_mobile.drainDeviceLog
import uniffi.halogen_mobile.setDeviceLogCapture

/// In-app diagnostic log ring — the native counterpart of the web's device-log
/// ring (halogen-ui-logging): app layers append notable events, the Device Logs page
/// renders it; bounded at 5000 lines, persisted, capture toggle + threshold applied live.
class DeviceLog private constructor(context: Context) {
    @Serializable
    data class Entry(
        val id: String = UUID.randomUUID().toString(),
        /// Unix epoch milliseconds (UTC).
        val at: Long,
        val level: Level,
        val message: String,
        /// Rust tracing target for core-originated lines (`halogen_server`,
        /// `halogen_rss`, …) — the app-vs-server tag the web shows as the
        /// target column on /logs/device. `null` for app-side lines.
        val source: String? = null,
    )

    /// Severity, most-severe first — a line is captured when its severity is
    /// at or above the threshold (web: `level as u8 <= threshold`).
    @Serializable
    enum class Level(val token: String) {
        @SerialName("error") Error("error"),
        @SerialName("warn") Warn("warn"),
        @SerialName("info") Info("info");

        val severity: Int
            get() = when (this) {
                Error -> 0
                Warn -> 1
                Info -> 2
            }

        /// Human label for the threshold picker (web: Level::label).
        val label: String
            get() = when (this) {
                Error -> "Error only"
                Warn -> "Warn and above"
                Info -> "Info and above (default)"
            }

        /// The FFI token ("Error" | "Warn" | "Info").
        val coreToken: String
            get() = token.replaceFirstChar { it.uppercase() }
    }

    private val scope: CoroutineScope = MainScope()
    private val flags: SharedPreferences =
        context.getSharedPreferences("device-log", Context.MODE_PRIVATE)
    private val file: File = File(context.filesDir, "device-log.json")
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }
    private val debuggable =
        (context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0

    var entries: List<Entry> by mutableStateOf(emptyList())
        private set

    private var enabledState by mutableStateOf(flags.getBoolean(ENABLED_KEY, true))
    private var thresholdState by mutableStateOf(
        flags.getString(LEVEL_KEY, null)?.let { raw ->
            Level.entries.firstOrNull { it.token == raw }
        } ?: Level.Info)

    /// Capture gate — when off, nothing new is recorded (web parity).
    var enabled: Boolean
        get() = enabledState
        set(value) {
            enabledState = value
            flags.edit().putBoolean(ENABLED_KEY, value).apply()
            pushCaptureToCore()
        }

    /// Minimum severity captured into the ring.
    var threshold: Level
        get() = thresholdState
        set(value) {
            thresholdState = value
            flags.edit().putString(LEVEL_KEY, value.token).apply()
            pushCaptureToCore()
        }

    private var saveJob: Job? = null
    /// Drains the Rust core's tracing ring (the embedded server's only log
    /// surface) into this one. Started once the embedded server boots.
    private var corePumpJob: Job? = null

    init {
        loadPersisted()
    }

    fun log(level: Level, message: String) {
        if (debuggable) Log.d("devicelog", "[${level.token}] $message")
        if (!enabled || level.severity > threshold.severity) return
        append(Entry(at = System.currentTimeMillis(), level = level, message = message))
    }

    private fun append(entry: Entry) {
        entries = (entries + entry).takeLast(CAPACITY)
        scheduleSave()
    }

    // Rust core capture (embedded-server tracing → this ring).

    /// Start polling the Rust core's device-log ring (2s cadence, matching
    /// the web's flush loop). Each drained line lands here tagged with its
    /// tracing target. Idempotent.
    fun startCorePump() {
        if (corePumpJob != null) return
        pushCaptureToCore()
        corePumpJob = scope.launch {
            while (isActive) {
                delay(2_000)
                val lines = withContext(Dispatchers.IO) { drainDeviceLog() }
                lines.forEach { ingest(it) }
            }
        }
    }

    /// Mirror the capture toggle + threshold into the Rust ring so filtered
    /// lines are never buffered core-side (the Rust gate is authoritative;
    /// `ingest` re-applies it only for lines already in flight).
    private fun pushCaptureToCore() {
        setDeviceLogCapture(enabled, threshold.coreToken)
    }

    private fun ingest(line: DeviceLogLine) {
        val level = when (line.level) {
            "Error" -> Level.Error
            "Warn" -> Level.Warn
            // Debug/Trace have no native tier — the Rust filter caps server
            // crates at info, so this is a defensive fold, not a hot path.
            else -> Level.Info
        }
        if (!enabled || level.severity > threshold.severity) return
        append(Entry(at = line.tsMs, level = level, message = line.msg, source = line.target))
    }

    /// Drop the ring AND the persisted copy (cleared lines must not return
    /// on relaunch — web: logging::clear + store::clear).
    fun clear() {
        entries = emptyList()
        saveJob?.cancel()
        scope.launch(Dispatchers.IO) { file.delete() }
    }

    /// Plain-text export, oldest first (web: logging::export_text).
    fun exportText(): String =
        entries.joinToString("\n") { entry ->
            val ts = DateTimeFormatter.ISO_INSTANT.format(
                Instant.ofEpochMilli(entry.at).truncatedTo(ChronoUnit.SECONDS))
            val source = entry.source?.let { "$it: " } ?: ""
            "$ts ${entry.level.token.uppercase()} $source${entry.message}"
        }

    // Persistence (2s debounced flush, web parity).

    private fun loadPersisted() {
        val saved = runCatching {
            json.decodeFromString<List<Entry>>(file.readText())
        }.getOrNull() ?: return
        entries = saved.takeLast(CAPACITY)
    }

    private fun scheduleSave() {
        saveJob?.cancel()
        saveJob = scope.launch {
            delay(2_000)
            persistRing()
        }
    }

    private suspend fun persistRing() {
        val snapshot = entries
        withContext(Dispatchers.IO) {
            runCatching { file.writeText(json.encodeToString(snapshot)) }
        }
    }

    companion object {
        /// Web: device_log::CAP.
        private const val CAPACITY = 5000
        private const val ENABLED_KEY = "device-log.enabled"
        private const val LEVEL_KEY = "device-log.level"

        @Volatile
        private var sharedOrNull: DeviceLog? = null

        /// Create the singleton — App.onCreate calls this once. Idempotent.
        fun initialize(context: Context): DeviceLog =
            sharedOrNull ?: DeviceLog(context.applicationContext).also { sharedOrNull = it }

        val shared: DeviceLog
            get() = requireNotNull(sharedOrNull) { "DeviceLog.initialize not called" }

        // Fire-and-forget helpers, callable from any thread (append on Main).
        fun info(message: String) = post(Level.Info, message)

        fun warn(message: String) = post(Level.Warn, message)

        fun error(message: String) = post(Level.Error, message)

        private fun post(level: Level, message: String) {
            val log = sharedOrNull ?: return
            log.scope.launch { log.log(level, message) }
        }
    }
}
