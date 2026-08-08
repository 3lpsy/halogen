package org.fgsec.halogen.features.settings

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore

/// Change the signed-in user's password (current + new, confirmed
/// server-side). The web's user-edit page, scoped to the password half.
@Composable
fun ChangePasswordView(core: HalogenCore, onBack: () -> Unit) {
    var current by remember { mutableStateOf("") }
    var new by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    suspend fun save() {
        saving = true
        try {
            core.changePassword(current = current, new = new)
            // The screen just closes otherwise — success must be explicit.
            ToastCenter.success("Password changed")
            onBack()
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            saving = false
        }
    }

    SettingsScaffold(title = "Change password", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            SettingsGroup {
                SettingsTextField(current, { current = it }, "Current password", secure = true)
                SettingsTextField(new, { new = it }, "New password", secure = true)
                SettingsTextField(confirm, { confirm = it }, "Confirm new password", secure = true)
            }
            error?.let {
                FootnoteText(
                    it,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
            Box(Modifier.padding(16.dp)) {
                // Local validation (web local_password_errors: 8-256) + offline
                // gate — a short password round-tripped to a raw server error.
                BusyButton(
                    title = "Change password",
                    busy = saving,
                    enabled = current.isNotEmpty() && new.length >= 8 && new.length <= 256 &&
                        new == confirm && !core.isOffline,
                ) {
                    scope.launch { save() }
                }
            }
            if (new.isNotEmpty() && new.length < 8) {
                FootnoteText(
                    "Password must be at least 8 characters.",
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
            if (core.isOffline) {
                FootnoteText(
                    "You're offline — reconnect to change the password.",
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
        }
    }
}
