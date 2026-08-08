package org.fgsec.halogen.core

import java.io.IOException
import java.io.InterruptedIOException
import java.net.ConnectException
import java.net.NoRouteToHostException
import java.net.SocketException
import java.net.UnknownHostException
import javax.net.ssl.SSLException
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.serialization.SerializationException
import org.fgsec.halogen.networking.HalogenClient
import uniffi.halogen_mobile.CoreException

/// Human copy for every failure class — never the raw exception dump
/// (web: the classify funnel + auth error_message mapping). Every inline
/// error slot, toast, and the sync-failure surface routes through here;
/// screens with context-specific copy (ConnectView) override then delegate.
object FriendlyError {
    fun message(error: Throwable): String = when (error) {
        is HalogenClient.ClientError -> clientMessage(error)
        // OkHttp/IO failure classes — the URLError mapping's Android shape.
        is UnknownHostException ->
            "Can't find the server — check the address."
        is SSLException ->
            "Secure connection failed — check the server's certificate."
        is ConnectException, is NoRouteToHostException, is SocketException ->
            if (error.message?.contains("unreachable", ignoreCase = true) == true)
                "You're offline — connect to a network first."
            else "Can't reach the server right now — try again."
        // SocketTimeoutException and OkHttp's callTimeout both land here.
        is InterruptedIOException ->
            "Can't reach the server right now — try again."
        is CancellationException -> "Cancelled."
        is IOException ->
            if (error.message == "Canceled") "Cancelled."
            else "Network request failed — check the connection and try again."
        is SerializationException ->
            "Couldn't read the server's response — app and server versions may not match."
        is HalogenCore.EmbeddedUserException ->
            error.message ?: "Something went wrong — try again."
        is CoreException.Server -> error.msg
        else -> "Something went wrong — try again."
    }

    private fun clientMessage(error: HalogenClient.ClientError): String = when (error) {
        is HalogenClient.ClientError.Api -> {
            val messages = error.errors.values.flatten().mapNotNull { it.message }
            if (messages.isEmpty()) "The server rejected the request."
            else messages.joinToString(" · ")
        }
        is HalogenClient.ClientError.Http -> when {
            error.code == 401 -> "Session expired — sign in again."
            error.code == 404 -> "Not found on the server — it may have been removed."
            error.code >= 500 -> "The server hit an error (${error.code}). Try again in a moment."
            else -> "The server refused the request (HTTP ${error.code})."
        }
        is HalogenClient.ClientError.Offline -> "You're offline — reconnect to do this."
        is HalogenClient.ClientError.SignedOut -> "Not signed in."
        is HalogenClient.ClientError.EmptyData ->
            "The server answered with an unexpected response."
    }
}
