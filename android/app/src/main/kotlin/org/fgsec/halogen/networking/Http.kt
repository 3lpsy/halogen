package org.fgsec.halogen.networking

import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import okhttp3.Call
import okhttp3.Callback
import okhttp3.Cookie
import okhttp3.CookieJar
import okhttp3.HttpUrl
import okhttp3.OkHttpClient
import okhttp3.Response

/// In-memory cookie jar keyed by host so the server's `auth_media` cookie
/// persists across requests to the same base URL (client + Coil + Media3).
class MemoryCookieJar : CookieJar {
    private val store = mutableMapOf<String, MutableList<Cookie>>()

    @Synchronized
    override fun saveFromResponse(url: HttpUrl, cookies: List<Cookie>) {
        val list = store.getOrPut(url.host) { mutableListOf() }
        for (cookie in cookies) {
            list.removeAll { it.name == cookie.name && it.path == cookie.path }
            list.add(cookie)
        }
    }

    @Synchronized
    override fun loadForRequest(url: HttpUrl): List<Cookie> {
        val list = store[url.host] ?: return emptyList()
        val now = System.currentTimeMillis()
        list.removeAll { it.expiresAt < now }
        return list.filter { it.matches(url) }
    }
}

/// The ONE shared OkHttpClient — connection pool and cookie jar are shared by
/// the API client, image loading, and media playback.
object Http {
    val cookieJar = MemoryCookieJar()

    val client: OkHttpClient = OkHttpClient.Builder()
        .cookieJar(cookieJar)
        .connectTimeout(15, TimeUnit.SECONDS)
        .readTimeout(60, TimeUnit.SECONDS)
        .writeTimeout(60, TimeUnit.SECONDS)
        .build()

    /// For MUTATIONS. OkHttp's transparent retry re-sends a POST whose response was
    /// lost (one tap created three playlists) — never replay bodies (iOS parity);
    /// failures surface to app retry paths. Shares pool/dispatcher/cookies with `client`.
    val mutationClient: OkHttpClient =
        client.newBuilder().retryOnConnectionFailure(false).build()
}

/// Suspend bridge over OkHttp's async `enqueue`; cancelling the coroutine
/// cancels the call.
suspend fun Call.await(): Response = suspendCancellableCoroutine { cont ->
    enqueue(object : Callback {
        override fun onFailure(call: Call, e: IOException) {
            if (cont.isCancelled) return
            cont.resumeWithException(e)
        }

        override fun onResponse(call: Call, response: Response) {
            cont.resume(response) { _ -> response.close() }
        }
    })
    cont.invokeOnCancellation { cancel() }
}

/// Drain the body OFF the caller's dispatcher: the headers arrive before the
/// body, so `.string()` on a Main-dispatched coroutine is a live socket read
/// — Android throws NetworkOnMainThreadException, and only when the payload
/// didn't fit the socket buffer (why Latest failed intermittently).
suspend fun Response.readBody(): String =
    withContext(Dispatchers.IO) { use { it.body?.string() ?: "" } }
