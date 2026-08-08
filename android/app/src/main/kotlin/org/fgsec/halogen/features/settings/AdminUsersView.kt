package org.fgsec.halogen.features.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import org.fgsec.halogen.components.LoadErrorView
import org.fgsec.halogen.components.ToastCenter
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.wire.UserData

private sealed interface AdminUsersRoute {
    data class Edit(val user: UserData) : AdminUsersRoute
    data object Create : AdminUsersRoute
}

/// Admin-only user management — the web's AdminUsers page: every server user
/// (`GET /users`), edit one, delete one. Online-only (no outbox, no cache);
/// the server is the real authority for the admin gate and the self-delete
/// refusal. Entry point: Settings → Accounts → Manage server users.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AdminUsersView(core: HalogenCore, onBack: () -> Unit) {
    var users by remember { mutableStateOf(listOf<UserData>()) }
    var loadError by remember { mutableStateOf<String?>(null) }
    var pendingDelete by remember { mutableStateOf<UserData?>(null) }
    var deleting by remember { mutableStateOf(false) }
    var refreshing by remember { mutableStateOf(false) }
    var route by remember { mutableStateOf<AdminUsersRoute?>(null) }
    val scope = rememberCoroutineScope()

    fun isSelf(user: UserData): Boolean = user.id == core.account?.userId

    suspend fun load() {
        try {
            users = core.listUsers()
            loadError = null
        } catch (e: Exception) {
            DeviceLog.warn("AdminUsersView: refresh failed — ${e::class.simpleName}: ${e.message}")
            if (users.isEmpty()) loadError = FriendlyError.message(e)
        }
    }

    suspend fun delete(user: UserData) {
        deleting = true
        try {
            core.deleteUser(user.id)
            ToastCenter.success("User deleted")
            load()
        } catch (e: Exception) {
            ToastCenter.error("Delete failed: ${FriendlyError.message(e)}")
        } finally {
            deleting = false
            pendingDelete = null
        }
    }

    LaunchedEffect(Unit) { load() }

    when (val r = route) {
        is AdminUsersRoute.Edit -> AdminUserEditView(
            core = core,
            user = r.user,
            onSaved = { scope.launch { load() } },
            onBack = { route = null },
        )
        AdminUsersRoute.Create -> AdminUserCreateView(
            core = core,
            onCreated = { scope.launch { load() } },
            onBack = { route = null },
        )
        null -> SettingsScaffold(
            title = "Users",
            onBack = onBack,
            actions = {
                IconButton(
                    onClick = { route = AdminUsersRoute.Create },
                    enabled = !core.isOffline,
                ) {
                    Icon(halogenIcon("plus"), contentDescription = "Create user")
                }
            },
        ) { padding ->
            val failure = loadError
            if (failure != null) {
                Box(Modifier.fillMaxSize().padding(padding)) {
                    LoadErrorView(
                        title = "Couldn't load users",
                        message = failure,
                        retry = { load() },
                    )
                }
            } else {
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
                    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
                        if (core.isOffline) {
                            SettingsGroup {
                                SettingsRow {
                                    FootnoteText(
                                        "You're offline — reconnect to manage users.",
                                        color = MaterialTheme.colorScheme.tertiary,
                                    )
                                }
                            }
                        }
                        SettingsGroup(
                            footer = if (users.isNotEmpty())
                                "Swipe to delete a user. You can't delete your own account."
                            else null,
                        ) {
                            for (user in users) {
                                key(user.id) {
                                    UserRow(
                                        user = user,
                                        isSelf = isSelf(user),
                                        enabled = !core.isOffline,
                                        canDelete = !isSelf(user) && !core.isOffline && !deleting,
                                        onOpen = { route = AdminUsersRoute.Edit(user) },
                                        onDelete = { pendingDelete = user },
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pendingDelete?.let { user ->
        AlertDialog(
            onDismissRequest = { pendingDelete = null },
            title = { Text("Delete user?") },
            text = {
                Text(
                    "This permanently deletes the account \"${user.username}\". This can't be undone."
                )
            },
            confirmButton = {
                TextButton(onClick = { scope.launch { delete(user) } }) {
                    Text("Delete", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { pendingDelete = null }) { Text("Cancel") }
            },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun UserRow(
    user: UserData,
    isSelf: Boolean,
    enabled: Boolean,
    canDelete: Boolean,
    onOpen: () -> Unit,
    onDelete: () -> Unit,
) {
    // Can't delete your own account — another admin must do it (web parity).
    val dismissState = rememberSwipeToDismissBoxState(
        confirmValueChange = { value ->
            if (value == SwipeToDismissBoxValue.EndToStart && canDelete) onDelete()
            // Never dismiss — the confirm dialog owns the deletion.
            value == SwipeToDismissBoxValue.Settled
        },
    )
    SwipeToDismissBox(
        state = dismissState,
        enableDismissFromStartToEnd = false,
        gesturesEnabled = canDelete,
        backgroundContent = {
            Box(
                Modifier.fillMaxSize().background(MaterialTheme.colorScheme.error),
                contentAlignment = Alignment.CenterEnd,
            ) {
                Icon(
                    halogenIcon("trash"),
                    contentDescription = "Delete",
                    tint = MaterialTheme.colorScheme.onError,
                    modifier = Modifier.padding(end = 20.dp),
                )
            }
        },
    ) {
        Surface(color = MaterialTheme.colorScheme.surfaceContainerLow) {
            SettingsRow(onClick = onOpen, enabled = enabled) {
                Text(if (user.username.isEmpty()) "User ${user.id}" else user.username)
                if (user.is_admin) {
                    Capsule(
                        "Admin",
                        container = MaterialTheme.colorScheme.primary.copy(alpha = 0.15f),
                        content = MaterialTheme.colorScheme.primary,
                    )
                }
                if (isSelf) {
                    Capsule(
                        "You",
                        container = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.15f),
                        content = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                Box(Modifier.weight(1f))
                RowChevron()
            }
        }
    }
}

@Composable
private fun Capsule(text: String, container: Color, content: Color) {
    Surface(shape = RoundedCornerShape(50), color = container) {
        Text(
            text,
            style = MaterialTheme.typography.labelSmall.copy(fontWeight = FontWeight.SemiBold),
            color = content,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
        )
    }
}

/// Admin edit page for another user — username + the Is Admin toggle (`PUT
/// /users/{id}`), the web's AdminUserEdit / AccountDetailsForm with
/// `admin: Some(current)`. There's no password field (admins can't change
/// another user's password).
@Composable
fun AdminUserEditView(
    core: HalogenCore,
    user: UserData,
    onSaved: () -> Unit = {},
    onBack: () -> Unit,
) {
    var username by remember { mutableStateOf(user.username) }
    var isAdmin by remember { mutableStateOf(user.is_admin) }
    var error by remember { mutableStateOf<String?>(null) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    val trimmed = username.trim()
    val usernameChanged = trimmed != user.username.trim()
    val unchanged = !usernameChanged && isAdmin == user.is_admin

    suspend fun save() {
        saving = true
        try {
            // Only send the username when it actually changed (web parity:
            // an admin-only flag toggle must not re-validate a legacy name).
            core.updateUser(
                userId = user.id,
                username = if (usernameChanged) trimmed else null,
                isAdmin = isAdmin,
            )
            if (user.id == core.account?.userId && usernameChanged) {
                core.renameActiveAccount(trimmed)
            }
            ToastCenter.success("User updated")
            onSaved()
            onBack()
        } catch (e: Exception) {
            error = FriendlyError.message(e)
        } finally {
            saving = false
        }
    }

    SettingsScaffold(title = "Edit user", onBack = onBack) { padding ->
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
                SettingsRow {
                    Text("Administrator", Modifier.weight(1f))
                    Switch(checked = isAdmin, onCheckedChange = { isAdmin = it })
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
                    title = "Save",
                    busy = saving,
                    enabled = !unchanged && !core.isOffline &&
                        !(usernameChanged && trimmed.length < 3),
                ) {
                    scope.launch { save() }
                }
            }
        }
    }
}
