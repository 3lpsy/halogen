package org.fgsec.halogen.networking

import java.io.File
import java.io.FileInputStream
import java.io.IOException
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.runBlocking
import okhttp3.Interceptor
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.ResponseBody
import okhttp3.ResponseBody.Companion.toResponseBody
import okio.Buffer
import okio.BufferedSource
import okio.ForwardingSource
import okio.buffer
import okio.source
import uniffi.halogen_mobile.LocalCore

/** Local requests terminate in Rust; the synthetic host never reaches DNS or a socket. */
object LocalTransport : Interceptor {
    private const val SUFFIX = ".halogen-local.invalid"
    private val sessions = ConcurrentHashMap<String, LocalCore>()

    fun install(core: LocalCore): String {
        val host = UUID.randomUUID().toString() + SUFFIX
        sessions[host] = core
        return "https://$host"
    }

    fun core(host: String): LocalCore = sessions[host] ?: throw IOException("local session is closed")

    fun remove(host: String?) { if (host != null) sessions.remove(host) }

    override fun intercept(chain: Interceptor.Chain): Response {
        val request = chain.request()
        if (!request.url.host.endsWith(SUFFIX)) return chain.proceed(request)
        val core = sessions[request.url.host] ?: throw IOException("local session is closed")
        return try { runBlocking {
            val parts = request.url.pathSegments
            if (request.url.encodedPath == "/healthz") {
                response(request, 200, ByteArray(0).toResponseBody())
            } else if (parts.size >= 5 && parts[4] in setOf("audio", "art")) {
                val id = parts[3].toIntOrNull()?.takeIf { it > 0 }
                    ?: throw IOException("invalid media ID")
                val path = when {
                    parts[2] == "episodes" && parts[4] == "audio" -> core.audioPath(id)
                    parts[2] in setOf("episodes", "podcasts") && parts[4] == "art" ->
                        core.artPath(id, parts[2] == "episodes", parts.last() == "small")
                    else -> throw IOException("invalid media path")
                }
                if (path == null) response(request, if (parts[4] == "art") 204 else 404, ByteArray(0).toResponseBody())
                else fileResponse(request, File(path))
            } else {
                val body = request.body?.let {
                    val limit = if (request.url.encodedPath == "/api/v1/admin/db/import") 256L else 16L
                    if (it.contentLength() !in 0..limit * 1024 * 1024) throw IOException("request exceeds limit")
                    val buffer = Buffer(); it.writeTo(buffer); buffer.readByteArray()
                }
                val result = core.invoke(request.method, request.url.encodedPath, request.url.encodedQuery, body)
                response(request, result.status.toInt(), result.body.toResponseBody("application/json".toMediaType()))
            }
        } } catch (error: Exception) {
            throw if (error is IOException) error else IOException("local request failed", error)
        }
    }

    private fun response(request: Request, status: Int, body: ResponseBody): Response =
        Response.Builder().request(request).protocol(Protocol.HTTP_1_1)
            .code(status).message("Local").body(body).build()

    /** Stream the confined file with byte ranges so seeking never buffers a whole episode. */
    private fun fileResponse(request: Request, file: File): Response {
        val size = file.length()
        val range = request.header("Range")
        var start = 0L
        var end = size - 1
        if (range != null) {
            val match = Regex("bytes=(\\d+)-(\\d*)").matchEntire(range)
            start = match?.groupValues?.get(1)?.toLongOrNull() ?: -1
            end = match?.groupValues?.get(2)?.takeIf { it.isNotEmpty() }?.toLongOrNull() ?: end
            if (start < 0 || start >= size || end < start) {
                return response(request, 416, ByteArray(0).toResponseBody()).newBuilder()
                    .header("Content-Range", "bytes */$size").build()
            }
            end = minOf(end, size - 1)
        }
        val length = end - start + 1
        val stream = FileInputStream(file)
        stream.channel.position(start)
        val source = object : ForwardingSource(stream.source()) {
            var remaining = length
            override fun read(sink: Buffer, byteCount: Long): Long {
                if (remaining == 0L) return -1
                val read = super.read(sink, minOf(byteCount, remaining))
                if (read > 0) remaining -= read
                return read
            }
        }.buffer()
        val body = object : ResponseBody() {
            override fun contentType() = (java.net.URLConnection.guessContentTypeFromName(file.name)
                ?: "application/octet-stream").toMediaType()
            override fun contentLength() = length
            override fun source(): BufferedSource = source
        }
        return response(request, if (range == null) 200 else 206, body).newBuilder()
            .header("Accept-Ranges", "bytes").header("Content-Length", length.toString())
            .apply { if (range != null) header("Content-Range", "bytes $start-$end/$size") }
            .build()
    }
}
