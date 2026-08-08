package org.fgsec.halogen.features.admin

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.HalogenNavbar
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.components.halogenExtras
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.WireJson
import org.fgsec.halogen.wire.PollJobData
import org.fgsec.halogen.wire.PollJobStatus

/// Polling history (admin): recent poll jobs with per-run totals, and a
/// "poll now" trigger. Online-only, like the web page.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PollingView(core: HalogenCore, onBack: (() -> Unit)? = null) {
    var jobs by remember { mutableStateOf(listOf<PollJobData>()) }
    var error by remember { mutableStateOf<String?>(null) }
    var polling by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    suspend fun load() {
        try {
            jobs = core.pollJobs()
            error = null
        } catch (e: Exception) {
            DeviceLog.warn("PollingView: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (jobs.isEmpty()) error = FriendlyError.message(e)
        }
    }

    suspend fun pollNow() {
        polling = true
        try {
            core.startPollJob()
            ToastCenter.success("Poll started")
        } catch (e: Exception) {
            // A silent failure here left no feedback at all (web surfaces
            // every non-form mutation outcome through the toast queue).
            ToastCenter.error("Couldn't start poll: $e")
        } finally {
            polling = false
        }
        load()
    }

    LaunchedEffect(Unit) { load() }

    Scaffold(
        topBar = {
            // Root screen: the shared navbar (online dot + account menu).
            HalogenNavbar(
                core = core,
                title = "Polling",
                leading = {
                    if (onBack != null) {
                        IconButton(onClick = onBack) {
                            Icon(halogenIcon("chevron.left"), contentDescription = "Back")
                        }
                    }
                },
                actions = {
                    IconButton(
                        onClick = { scope.launch { pollNow() } },
                        enabled = !polling,
                    ) {
                        if (polling) {
                            CircularProgressIndicator(
                                modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                        } else {
                            Icon(halogenIcon("arrow.clockwise"), contentDescription = "Poll now")
                        }
                    }
                },
            )
        },
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = {
                scope.launch {
                    refreshing = true
                    load()
                    refreshing = false
                }
            },
            modifier = Modifier.fillMaxSize().padding(padding),
        ) {
            val failure = error
            when {
                failure != null -> AdminUnavailableView(
                    icon = "wifi.exclamationmark",
                    title = "Couldn't load poll jobs",
                    description = failure,
                    monospaced = true,
                )

                jobs.isEmpty() -> AdminUnavailableView(
                    icon = "arrow.triangle.2.circlepath",
                    title = "No poll jobs yet",
                    description = "Trigger a poll to fetch new episodes.",
                )

                else -> LazyColumn(Modifier.fillMaxSize()) {
                    // ULong is a value class the saved-state Bundle rejects.
                    items(jobs, key = { it.id.toLong() }) { job -> PollJobRow(job) }
                }
            }
        }
    }
}

@Composable
private fun PollJobRow(job: PollJobData) {
    Column(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            StatusBadge(job.status)
            Text(
                formatJobDate(job.started_at),
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.weight(1f))
            Text(
                "#${job.id}",
                style = MaterialTheme.typography.labelSmall.copy(
                    fontFamily = FontFamily.Monospace),
                color = MaterialTheme.colorScheme.outline,
            )
        }
        Text(
            "${job.total_new} new · ${job.total_updated} updated · ${job.total_errors} errors",
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun StatusBadge(status: PollJobStatus) {
    val color = badgeColor(status)
    Text(
        status.string.replaceFirstChar { it.uppercase() },
        style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.SemiBold),
        color = color,
        modifier = Modifier
            .background(color.copy(alpha = 0.15f), RoundedCornerShape(50))
            .padding(horizontal = 6.dp, vertical = 2.dp),
    )
}

@Composable
private fun badgeColor(status: PollJobStatus): Color = when (status) {
    PollJobStatus.Completed -> halogenExtras.success
    PollJobStatus.Running -> MaterialTheme.colorScheme.primary
    PollJobStatus.Failed -> MaterialTheme.colorScheme.error
}

/// iOS `.dateTime.month().day().hour().minute()` shape ("Jul 29, 05:21").
private val jobDateFormatter =
    DateTimeFormatter.ofPattern("MMM d, HH:mm").withZone(ZoneId.systemDefault())

internal fun formatJobDate(raw: String): String =
    runCatching { jobDateFormatter.format(WireJson.parseInstant(raw)) }.getOrDefault(raw)

/// iOS ContentUnavailableView parity used by the admin screens: an icon,
/// title, and description centered in a scrollable (pull-to-refresh) column.
@Composable
internal fun AdminUnavailableView(
    icon: String,
    title: String,
    description: String,
    monospaced: Boolean = false,
) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(32.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(
            halogenIcon(icon),
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(48.dp),
        )
        Text(title, style = MaterialTheme.typography.titleMedium)
        Text(
            description,
            style = if (monospaced)
                MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace)
            else MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
    }
}
