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
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore

/// Edit the signed-in user's username (`PUT /users/{id}`, self-or-admin).
/// The stored session updates so the change survives relaunch.
@Composable
fun UsernameEditView(core: HalogenCore, onBack: () -> Unit) {
    var username by remember { mutableStateOf(core.account?.username ?: "") }
    var error by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val trimmed = username.trim()

    suspend fun save() {
        val account = core.account ?: return
        saving = true
        try {
            core.updateUsername(userId = account.userId, username = trimmed)
            core.renameActiveAccount(trimmed)
            ToastCenter.success("Username updated")
            onBack()
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            saving = false
        }
    }

    SettingsScaffold(title = "Edit username", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            SettingsGroup {
                SettingsTextField(
                    value = username,
                    onValueChange = { username = it },
                    label = "Username",
                    capitalization = KeyboardCapitalization.None,
                )
            }
            error?.let {
                FootnoteText(
                    it,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
            Box(Modifier.padding(16.dp)) {
                // Web rules: 3-64 chars; compare the TRIMMED value (a padded
                // copy of the same name used to pass the unchanged guard and
                // PUT an identical name); offline-gated (online-only endpoint).
                BusyButton(
                    title = "Save",
                    busy = saving,
                    enabled = trimmed.length >= 3 && trimmed.length <= 64 &&
                        trimmed != core.account?.username && !core.isOffline,
                ) {
                    scope.launch { save() }
                }
            }
            if (core.isOffline) {
                FootnoteText(
                    "You're offline — reconnect to rename the account.",
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
        }
    }
}
