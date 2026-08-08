package org.fgsec.halogen.features.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.storage.Session

private enum class AccountsRoute { AddEmbeddedUser, AdminUsers }

/// Settings → Accounts: every saved session, switch/remove, add another
/// account (remote login or a new embedded user) — the web's accounts page.
@Composable
fun AccountsView(core: HalogenCore, onBack: () -> Unit) {
    var route by remember { mutableStateOf<AccountsRoute?>(null) }

    when (route) {
        AccountsRoute.AddEmbeddedUser -> AddEmbeddedUserView(core, onBack = { route = null })
        AccountsRoute.AdminUsers -> AdminUsersView(core, onBack = { route = null })
        null -> AccountsList(core, onBack, push = { route = it })
    }
}

@Composable
private fun AccountsList(core: HalogenCore, onBack: () -> Unit, push: (AccountsRoute) -> Unit) {
    var registry by remember { mutableStateOf(core.sessionStore.load()) }
    var confirmRemove by remember { mutableStateOf<Session?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) { registry = core.sessionStore.load() }

    fun remove(session: Session) {
        if (session.id == registry.activeId) {
            core.signOut()
        } else {
            core.sessionStore.remove(session.id)
            registry = core.sessionStore.load()
        }
    }

    SettingsScaffold(title = "Accounts", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            SettingsGroup(header = "Accounts on this device") {
                for (session in registry.sessions) {
                    key(session.id) {
                        // Destructive parity: every other destructive action
                        // in the app confirms first.
                        val dismissState = rememberSwipeToDismissBoxState(
                            confirmValueChange = { value ->
                                if (value == SwipeToDismissBoxValue.EndToStart) {
                                    confirmRemove = session
                                }
                                // Never dismiss — the confirm dialog owns the removal.
                                value == SwipeToDismissBoxValue.Settled
                            },
                        )
                        SwipeToDismissBox(
                            state = dismissState,
                            enableDismissFromStartToEnd = false,
                            backgroundContent = {
                                Box(
                                    Modifier.fillMaxSize()
                                        .background(MaterialTheme.colorScheme.error),
                                    contentAlignment = Alignment.CenterEnd,
                                ) {
                                    Icon(
                                        halogenIcon("trash"),
                                        contentDescription = "Remove",
                                        tint = MaterialTheme.colorScheme.onError,
                                        modifier = Modifier.padding(end = 20.dp),
                                    )
                                }
                            },
                        ) {
                            Surface(color = MaterialTheme.colorScheme.surfaceContainerLow) {
                                SettingsRow(onClick = {
                                    if (session.id != registry.activeId) {
                                        // App scope: the switch remounts the
                                        // tree, which would cancel a
                                        // composition-scoped launch mid-way.
                                        core.scope.launch { core.switchAccount(session) }
                                    }
                                }) {
                                    Icon(
                                        halogenIcon(
                                            if (session.kind == Session.Kind.EMBEDDED) "iphone"
                                            else "server.rack"),
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                    Column(
                                        Modifier.weight(1f),
                                        verticalArrangement = Arrangement.spacedBy(1.dp),
                                    ) {
                                        Text(session.username)
                                        Text(
                                            if (session.kind == Session.Kind.EMBEDDED) "This device"
                                            else session.serverUrl ?: "",
                                            style = MaterialTheme.typography.bodySmall,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                            maxLines = 1,
                                            overflow = TextOverflow.MiddleEllipsis,
                                        )
                                    }
                                    if (session.id == registry.activeId) {
                                        Icon(
                                            halogenIcon("checkmark"),
                                            contentDescription = "Active",
                                            tint = MaterialTheme.colorScheme.primary,
                                            modifier = Modifier.size(20.dp),
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
            }

            SettingsGroup(
                footer = "Switching accounts swaps the whole library view; each account keeps its own cache and settings. Removing an account signs it out on this device — nothing is deleted on the server.",
            ) {
                SettingsRow(onClick = { core.beginAddAccount() }) {
                    Icon(
                        halogenIcon("plus"),
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                    )
                    Text("Add account (server login)", Modifier.weight(1f))
                }
                if (core.isEmbeddedAccount) {
                    SettingsRow(onClick = { push(AccountsRoute.AddEmbeddedUser) }) {
                        Icon(
                            halogenIcon("person.badge.plus"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text("New user on this device", Modifier.weight(1f))
                        RowChevron()
                    }
                }
                // Admin-only: manage every server user (list / edit / delete).
                if (core.isAdmin) {
                    SettingsRow(onClick = { push(AccountsRoute.AdminUsers) }) {
                        Icon(
                            halogenIcon("person.2.badge.gearshape"),
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.primary,
                        )
                        Text("Manage server users", Modifier.weight(1f))
                        RowChevron()
                    }
                }
            }
        }
    }

    confirmRemove?.let { session ->
        AlertDialog(
            onDismissRequest = { confirmRemove = null },
            title = { Text("Remove this account?") },
            text = { Text("Signs the account out on this device — nothing is deleted on the server.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmRemove = null
                    remove(session)
                }) { Text("Remove ${session.username}", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = {
                TextButton(onClick = { confirmRemove = null }) { Text("Cancel") }
            },
        )
    }
}

/// Create + switch to a new user on the embedded server (username only — the
/// app generates and stores the password), the web's add-embedded-user page.
@Composable
fun AddEmbeddedUserView(core: HalogenCore, onBack: () -> Unit) {
    var username by remember { mutableStateOf("") }
    var error by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    suspend fun create() {
        saving = true
        try {
            core.addEmbeddedUser(username)
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            saving = false
        }
    }

    SettingsScaffold(title = "New device user", onBack = onBack) { padding ->
        Column(
            Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()),
        ) {
            SettingsGroup(
                footer = "Users on the embedded server are always administrators. No password needed — the app manages sign-in.",
            ) {
                SettingsTextField(
                    value = username,
                    onValueChange = { username = it },
                    label = "Username",
                    capitalization = KeyboardCapitalization.None,
                )
                // Visible but locked, like the web: every embedded user is an
                // admin of the on-device server (no lesser role to manage).
                SettingsRow {
                    Text("Administrator", Modifier.weight(1f))
                    Switch(checked = true, onCheckedChange = null, enabled = false)
                }
            }
            error?.let {
                FootnoteText(
                    it,
                    color = MaterialTheme.colorScheme.error,
                    modifier = Modifier.padding(horizontal = 32.dp),
                )
            }
            Box(Modifier.padding(16.dp)) {
                BusyButton(
                    title = "Create and switch",
                    busy = saving,
                    enabled = username.trim().isNotEmpty(),
                ) {
                    // App scope: success switches accounts and remounts the
                    // tree — a composition scope cancels the POST mid-flight.
                    core.scope.launch { create() }
                }
            }
        }
    }
}
