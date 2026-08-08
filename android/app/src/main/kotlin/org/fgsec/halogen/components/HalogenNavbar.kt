package org.fgsec.halogen.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MenuDefaults
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import org.fgsec.halogen.core.ConnectionMonitor
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.features.settings.AccountsView

/// The app's top navbar (title, reachability dot, user menu) — the `topBar` of each
/// tab's ROOT screen; pushed views keep their own titles. `leading` is the top-LEFT
/// slot (iOS `.topBarLeading` parity); `actions` trails, before the dot + user menu.
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HalogenNavbar(
    core: HalogenCore,
    title: String? = null,
    leading: @Composable () -> Unit = {},
    actions: @Composable RowScope.() -> Unit = {},
) {
    var showAccounts by remember { mutableStateOf(false) }
    var menuOpen by remember { mutableStateOf(false) }
    var confirmSignOut by remember { mutableStateOf(false) }

    CenterAlignedTopAppBar(
        // The default 64dp grows with the 125% font scale and ate a fifth of
        // the screen; iOS's bar is 44pt and this matches it.
        expandedHeight = 48.dp,
        title = { Text(title ?: "Halogen", style = MaterialTheme.typography.titleMedium) },
        navigationIcon = leading,
        actions = {
            actions()
            OnlineDot(status = core.connection.status)
            Spacer(Modifier.width(10.dp))
            Box {
                IconButton(onClick = { menuOpen = true }) {
                    Icon(
                        halogenIcon("person.circle"),
                        contentDescription = "Account menu",
                    )
                }
                DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                    // Section header — who's signed in (iOS Menu Section title).
                    Text(
                        core.account?.username ?: "Signed out",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 6.dp),
                    )
                    DropdownMenuItem(
                        text = { Text("Accounts") },
                        leadingIcon = {
                            Icon(halogenIcon("person.2"), contentDescription = null)
                        },
                        onClick = {
                            menuOpen = false
                            showAccounts = true
                        },
                    )
                    DropdownMenuItem(
                        text = { Text("Add account") },
                        leadingIcon = {
                            Icon(halogenIcon("person.badge.plus"), contentDescription = null)
                        },
                        onClick = {
                            menuOpen = false
                            core.beginAddAccount()
                        },
                    )
                    // Embedded accounts talk to the on-device server — "going
                    // offline" against it is meaningless and just suspends the
                    // outbox (web hides the toggle there too).
                    if (!core.isEmbeddedAccount) {
                        val offline = core.connection.manualOffline
                        DropdownMenuItem(
                            text = { Text(if (offline) "Go online" else "Go offline") },
                            leadingIcon = {
                                Icon(
                                    halogenIcon(if (offline) "wifi" else "wifi.slash"),
                                    contentDescription = null)
                            },
                            onClick = {
                                menuOpen = false
                                core.setManualOffline(!offline)
                            },
                        )
                    }
                    DropdownMenuItem(
                        text = { Text("Sign out") },
                        leadingIcon = {
                            Icon(
                                halogenIcon("rectangle.portrait.and.arrow.right"),
                                contentDescription = null)
                        },
                        colors = MenuDefaults.itemColors(
                            textColor = MaterialTheme.colorScheme.error,
                            leadingIconColor = MaterialTheme.colorScheme.error,
                        ),
                        onClick = {
                            menuOpen = false
                            confirmSignOut = true
                        },
                    )
                }
            }
        },
    )

    // Destructive confirm (the confirmationDialog → AlertDialog idiom).
    if (confirmSignOut) {
        AlertDialog(
            onDismissRequest = { confirmSignOut = false },
            title = { Text("Sign out?") },
            text = {
                Text(
                    core.account?.username?.let { "Sign out of '$it'? Cached data stays on this device." }
                        ?: "Cached data stays on this device.")
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmSignOut = false
                        core.signOut()
                    }
                ) {
                    Text("Sign out", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { confirmSignOut = false }) { Text("Cancel") }
            },
        )
    }

    // The Accounts sheet (iOS .sheet + NavigationStack with a Done button).
    if (showAccounts) {
        ModalBottomSheet(onDismissRequest = { showAccounts = false }) {
            Column(Modifier.fillMaxWidth()) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Spacer(Modifier.weight(1f))
                    TextButton(onClick = { showAccounts = false }) { Text("Done") }
                }
                HorizontalDivider()
                AccountsView(core = core, onBack = { showAccounts = false })
            }
        }
    }
}

/// Green = server reachable, red = not, gray = not probed yet.
@Composable
fun OnlineDot(status: ConnectionMonitor.Status, modifier: Modifier = Modifier) {
    val color = when (status) {
        ConnectionMonitor.Status.Online -> halogenExtras.success
        ConnectionMonitor.Status.Offline -> MaterialTheme.colorScheme.error
        ConnectionMonitor.Status.Unknown -> MaterialTheme.colorScheme.outline
    }
    val label = when (status) {
        ConnectionMonitor.Status.Online -> "Online"
        ConnectionMonitor.Status.Offline -> "Offline"
        ConnectionMonitor.Status.Unknown -> "Connection unknown"
    }
    Box(
        modifier
            .size(9.dp)
            .clip(CircleShape)
            .background(color)
            .semantics { contentDescription = label }
    )
}
