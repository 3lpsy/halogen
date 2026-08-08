package org.fgsec.halogen.features.settings

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore

/// Admin-only "create user with password" form — the counterpart to the web's
/// /admin/users/create page. Online-only, like the rest of user management:
/// the submit goes direct so server errors surface inline, and the server
/// independently enforces the admin gate on POST /admin/users.
@Composable
fun AdminUserCreateView(
    core: HalogenCore,
    onCreated: () -> Unit = {},
    onBack: () -> Unit,
) {
    var username by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var passwordConfirm by remember { mutableStateOf("") }
    var showPassword by remember { mutableStateOf(false) }
    var isAdmin by remember { mutableStateOf(false) }
    var submitted by remember { mutableStateOf(false) }
    var submitting by remember { mutableStateOf(false) }
    var serverError by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    // Mirrors wire UserStoreData's validators (username 3–64, password
    // 8–256, confirmation must match) so the form and the server agree.
    val trimmed = username.trim()
    val usernameError: String? =
        if (trimmed.length < 3 || trimmed.length > 64)
            "Username must be between 3 and 64 characters long"
        else null
    val passwordError: String? = when {
        password.length < 8 || password.length > 256 ->
            "Password must be between 8 and 256 characters long"
        passwordConfirm != password ->
            "Password confirmation must match the password"
        else -> null
    }

    suspend fun submit() {
        submitted = true
        serverError = null
        if (usernameError != null || passwordError != null) return
        submitting = true
        try {
            core.createUser(username = trimmed, password = password, isAdmin = isAdmin)
            ToastCenter.success("User created")
            onCreated()
            onBack()
        } catch (e: Exception) {
            serverError = FriendlyError.message(e)
        } finally {
            submitting = false
        }
    }

    SettingsScaffold(title = "Create user", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            SettingsGroup(footer = "3–64 characters.") {
                SettingsTextField(
                    value = username,
                    onValueChange = { username = it },
                    label = "Username",
                    capitalization = KeyboardCapitalization.None,
                )
                if (submitted && usernameError != null) {
                    FieldError(usernameError)
                }
            }

            SettingsGroup(footer = "8–256 characters.") {
                SettingsTextField(
                    value = password,
                    onValueChange = { password = it },
                    label = "Password",
                    secure = !showPassword,
                    trailingIcon = {
                        IconButton(
                            onClick = { showPassword = !showPassword },
                            modifier = Modifier.testTag("password-eye"),
                        ) {
                            Icon(
                                halogenIcon(if (showPassword) "eye.slash" else "eye"),
                                contentDescription =
                                    if (showPassword) "Hide password" else "Show password",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    },
                )
                SettingsTextField(
                    value = passwordConfirm,
                    onValueChange = { passwordConfirm = it },
                    label = "Confirm password",
                    secure = !showPassword,
                )
                if (submitted && passwordError != null) {
                    FieldError(passwordError)
                }
            }

            SettingsGroup(
                footer = "Administrators can manage users, server settings, and polling.",
            ) {
                SettingsRow {
                    Text("Administrator", Modifier.weight(1f))
                    Switch(checked = isAdmin, onCheckedChange = { isAdmin = it })
                }
            }

            serverError?.let { FieldError(it) }

            Box(Modifier.padding(16.dp)) {
                BusyButton(
                    title = "Create user",
                    busy = submitting,
                    enabled = !core.isOffline,
                ) {
                    scope.launch { submit() }
                }
            }
            if (core.isOffline) {
                FootnoteText(
                    "You're offline — reconnect to create users.",
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
        }
    }
}

@Composable
private fun FieldError(message: String) {
    FootnoteText(
        message,
        color = MaterialTheme.colorScheme.error,
        modifier = Modifier.padding(horizontal = 32.dp, vertical = 4.dp),
    )
}
