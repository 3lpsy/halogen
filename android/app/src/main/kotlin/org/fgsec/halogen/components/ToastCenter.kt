package org.fgsec.halogen.components

import androidx.compose.animation.animateContentSize
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/// Lightweight global toast queue — the native counterpart of the web's
/// ui-toast surface for actions that have no inline error/result slot
/// (background mutations, toolbar triggers, admin actions).
object ToastCenter {
    data class Toast(val id: Long, val message: String, val isError: Boolean)

    // All mutations hop to Main — callable from any thread (iOS @MainActor parity).
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var nextId = 0L

    var toasts by mutableStateOf(listOf<Toast>())
        private set

    fun success(message: String) = push(message, isError = false)
    fun error(message: String) = push(message, isError = true)

    fun dismiss(id: Long) {
        scope.launch { toasts = toasts.filterNot { it.id == id } }
    }

    private fun push(message: String, isError: Boolean) {
        scope.launch {
            // Dedupe + cap: a burst (e.g. a drained batch dead-lettering) must
            // not stack the screen full of identical capsules.
            if (toasts.any { it.message == message }) return@launch
            val toast = Toast(id = ++nextId, message = message, isError = isError)
            var next = toasts
            if (next.size >= 4) next = next.drop(1)
            toasts = next + toast
            scope.launch {
                delay(4_000)
                toasts = toasts.filterNot { it.id == toast.id }
            }
        }
    }
}

/// Bottom-stacked toast overlay; place once at the root, aligned BottomCenter
/// over the app content (iOS `.toastOverlay()`).
@Composable
fun ToastHost(modifier: Modifier = Modifier) {
    Column(
        modifier
            .navigationBarsPadding()
            .padding(horizontal = 24.dp)
            .padding(bottom = 72.dp)
            .animateContentSize(tween(200)),
        verticalArrangement = Arrangement.spacedBy(8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        for (toast in ToastCenter.toasts) {
            key(toast.id) {
                Surface(
                    onClick = { ToastCenter.dismiss(toast.id) },
                    shape = RoundedCornerShape(50),
                    color = if (toast.isError)
                        MaterialTheme.colorScheme.error.copy(alpha = 0.92f)
                    else
                        halogenExtras.toastBackground,
                    contentColor = if (toast.isError)
                        MaterialTheme.colorScheme.onError
                    else
                        MaterialTheme.colorScheme.onSurface,
                    shadowElevation = 4.dp,
                ) {
                    Text(
                        toast.message,
                        style = MaterialTheme.typography.bodySmall,
                        textAlign = TextAlign.Center,
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp),
                    )
                }
            }
        }
    }
}
