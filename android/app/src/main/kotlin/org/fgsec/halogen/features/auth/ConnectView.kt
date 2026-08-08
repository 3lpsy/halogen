package org.fgsec.halogen.features.auth

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch
import kotlinx.serialization.SerializationException
import org.fgsec.halogen.DebugHooks
import org.fgsec.halogen.components.halogenIcon
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.core.FriendlyError
import org.fgsec.halogen.core.HalogenCore
import org.fgsec.halogen.networking.HalogenClient
import java.net.MalformedURLException

/// The landing page: server + credentials on ONE page. "Use embedded server"
/// is the local-library escape hatch, mirroring the web's standalone footer.
@Composable
fun ConnectView(core: HalogenCore) {
    var serverUrl by rememberSaveable { mutableStateOf("") }
    var username by rememberSaveable { mutableStateOf("") }
    var password by rememberSaveable { mutableStateOf("") }
    var showPassword by rememberSaveable { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var connecting by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()

    fun connect() {
        error = null
        connecting = true
        // Launched on the core scope: the phase change unmounts this view,
        // which would cancel a composition-scoped login mid-flight.
        core.scope.launch {
            try {
                // Clipboard hygiene: pasted values routinely carry trailing
                // whitespace/newlines. Usernames can't contain them; passwords
                // only lose line breaks + edge spaces.
                core.connectRemote(
                    serverUrl = serverUrl.trim(),
                    username = username.trim(),
                    password = password.trim('\n', '\r'),
                )
            } catch (e: Exception) {
                error = connectFriendly(e)
                DeviceLog.warn("connect: failed — $e")
            } finally {
                connecting = false
            }
        }
    }

    LaunchedEffect(Unit) {
        // An expired remote session lands here with the server and username
        // already known — only the password is asked again.
        core.reauthHint?.let { hint ->
            if (serverUrl.isEmpty() && username.isEmpty()) {
                serverUrl = hint.serverUrl
                username = hint.username
            }
        }
        // DEBUG smoke-test hook: HALOGEN_AUTOCONNECT drives the landing page
        // headlessly ("url|user|pass", or "local" for the embedded button).
        DebugHooks.autoconnect?.let { spec ->
            if (spec == "local") {
                core.scope.launch { core.useLocalLibrary() }
            } else {
                val parts = spec.split("|")
                if (parts.size == 3) {
                    serverUrl = parts[0]; username = parts[1]; password = parts[2]
                    connect()
                }
            }
        }
    }

    Column(
        modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(
            modifier = Modifier.widthIn(max = 480.dp).padding(24.dp),
            verticalArrangement = Arrangement.spacedBy(24.dp),
        ) {
            // Add-account arrives here with the previous session intact —
            // give it a way back (an expired session has none).
            if (core.addAccountReturnSession != null) {
                Row {
                    TextButton(onClick = { scope.launch { core.cancelAddAccount() } }) {
                        Icon(halogenIcon("chevron.left"), contentDescription = null)
                        Text("Back")
                    }
                    Spacer(Modifier.weight(1f))
                }
            }

            Column(
                modifier = Modifier.fillMaxWidth().padding(top = 48.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Icon(
                    halogenIcon("waveform.circle.fill"), contentDescription = null,
                    modifier = Modifier.width(56.dp), tint = MaterialTheme.colorScheme.primary,
                )
                Text("Halogen", style = MaterialTheme.typography.headlineLarge)
                Text(
                    "Connect to your server",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(
                    value = serverUrl, onValueChange = { serverUrl = it },
                    label = { Text("Server URL (https://…)") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    singleLine = true, modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = username, onValueChange = { username = it },
                    label = { Text("Username") },
                    singleLine = true, modifier = Modifier.fillMaxWidth(),
                )
                OutlinedTextField(
                    value = password, onValueChange = { password = it },
                    label = { Text("Password") },
                    singleLine = true, modifier = Modifier.fillMaxWidth(),
                    visualTransformation =
                        if (showPassword) VisualTransformation.None else PasswordVisualTransformation(),
                    keyboardOptions = KeyboardOptions(
                        keyboardType = if (showPassword) KeyboardType.Ascii else KeyboardType.Password,
                    ),
                    trailingIcon = {
                        IconButton(onClick = { showPassword = !showPassword }) {
                            Icon(
                                halogenIcon(if (showPassword) "eye.slash" else "eye"),
                                contentDescription = if (showPassword) "Hide password" else "Show password",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    },
                )

                error?.let {
                    Text(
                        it, style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }

                Button(
                    onClick = ::connect,
                    enabled = !connecting && serverUrl.isNotEmpty() && username.isNotEmpty() && password.isNotEmpty(),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    if (connecting) CircularProgressIndicator(modifier = Modifier.width(20.dp))
                    else Text("Connect")
                }
            }

            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    HorizontalDivider(Modifier.weight(1f))
                    Text(
                        "or", style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(horizontal = 8.dp),
                    )
                    HorizontalDivider(Modifier.weight(1f))
                }
                OutlinedButton(
                    onClick = { core.scope.launch { core.useLocalLibrary() } },
                    enabled = !connecting,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(halogenIcon("iphone"), contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text("Use embedded server (this device)")
                }
            }
        }
    }
}

/// Connect-specific copy layered over the shared mapper: a 401 here means bad
/// credentials (not an expired session), and an undecodable/empty answer
/// usually means the URL isn't a Halogen server at all.
private fun connectFriendly(error: Exception): String = when {
    error is HalogenClient.ClientError.Http && error.code == 401 ->
        "Invalid username or password."
    error is HalogenClient.ClientError.EmptyData || error is SerializationException ->
        "The server answered with an unexpected response. Is the URL a Halogen server?"
    error is HalogenCore.ConnectException || error is MalformedURLException ->
        "That doesn't look like a valid server URL."
    error is javax.net.ssl.SSLException ->
        "Secure connection failed — check the server's certificate (or use http:// for a local server)."
    else -> FriendlyError.message(error)
}
