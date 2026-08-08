package org.fgsec.halogen.core

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.Request
import org.fgsec.halogen.networking.Http
import org.fgsec.halogen.networking.await

/// Server reachability for the navbar status dot: a periodic `/healthz` probe plus
/// one on foregrounding — the polling stand-in for the web's connectivity-WS RTT,
/// same three-state surface. `scope` must be Main: `status`/`manualOffline` are
/// Compose state read during composition.
class ConnectionMonitor(private val scope: CoroutineScope) {
    enum class Status { Unknown, Online, Offline }

    var status: Status by mutableStateOf(Status.Unknown)
        private set

    /// User-forced offline (the navbar toggle, like the web's). Probes pause,
    /// the dot shows offline, and the outbox holds until back online.
    var manualOffline: Boolean by mutableStateOf(false)
        private set

    /// Fired whenever a probe finds the server reachable after it wasn't —
    /// the outbox drain trigger.
    var onOnline: (() -> Unit)? = null

    private var baseUrl: String? = null
    private var probeJob: Job? = null

    // Shares the app's connection pool; 5s budget matching the iOS probe.
    private val client = Http.client.newBuilder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(5, TimeUnit.SECONDS)
        .callTimeout(5, TimeUnit.SECONDS)
        .build()

    /// (Re)start probing against a server. Called by the core once a session
    /// connects; safe to call again on account/server switch.
    fun start(baseUrl: String) {
        this.baseUrl = baseUrl
        probeJob?.cancel()
        probeJob = scope.launch {
            while (isActive) {
                probe()
                delay(INTERVAL_MS)
            }
        }
    }

    fun stop() {
        probeJob?.cancel()
        probeJob = null
        status = Status.Unknown
    }

    /// Flip the user-forced offline mode. Going online probes immediately
    /// (which also drains the outbox via onOnline).
    fun applyManualOffline(offline: Boolean) {
        manualOffline = offline
        if (offline) {
            status = Status.Offline
        } else {
            status = Status.Unknown
            scope.launch { probe() }
        }
    }

    /// One immediate probe — the foreground (ON_START) hook calls this so a
    /// return to the foreground doesn't wait out the interval.
    suspend fun probe() {
        if (manualOffline) return
        val base = baseUrl ?: return
        val url = "$base/healthz".toHttpUrlOrNull() ?: return
        val previous = status
        val ok = runCatching {
            client.newCall(Request.Builder().url(url).build()).await().use { it.code in 200..299 }
        }.getOrDefault(false)
        status = if (ok) Status.Online else Status.Offline
        if (status == Status.Online && previous != Status.Online) {
            onOnline?.invoke()
        }
    }

    private companion object {
        const val INTERVAL_MS = 15_000L
    }
}
