package org.fgsec.halogen.networking

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.SerializationException
import kotlinx.serialization.builtins.ListSerializer
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.fgsec.halogen.core.DeviceLog
import org.fgsec.halogen.wire.PasswordChangeData
import org.fgsec.halogen.wire.PasswordUpdateData
import org.fgsec.halogen.wire.PollJobData
import org.fgsec.halogen.wire.PollJobStartData
import org.fgsec.halogen.wire.ResponseData
import org.fgsec.halogen.wire.ServerErrorsData
import org.fgsec.halogen.wire.ServerLogsData

// Admin + account-maintenance slice (poll jobs, server logs/errors, password
// change). Non-admin callers get clean 403s from the server.

suspend fun HalogenClient.pollJobs(): List<PollJobData> =
    get("admin/poll-jobs", emptyList(), ListSerializer(PollJobData.serializer()))

/// Kick a poll of every feed (or one podcast). Returns the job id.
suspend fun HalogenClient.startPollJob(podcastId: Int? = null): ULong {
    val query = mutableListOf<Pair<String, String>>()
    podcastId?.let { query.add("podcast_id" to it.toString()) }
    val request = Request.Builder()
        .url(url("admin/poll-job", query))
        .post(ByteArray(0).toRequestBody(null))
        .build()
    val envelope = send(request, PollJobStartData.serializer())
    val data = envelope.data ?: throw HalogenClient.ClientError.EmptyData
    return data.job_id
}

suspend fun HalogenClient.serverLogs(): ServerLogsData =
    try {
        get("admin/server-logs", emptyList(), ServerLogsData.serializer())
    } catch (decodeError: SerializationException) {
        // Log payloads are the one endpoint whose bytes can arrive mangled
        // (huge bodies through proxies, raw log content): invalid UTF-8 can
        // reject the WHOLE body. Refetch raw, re-encode lossily (bad bytes →
        // U+FFFD) and retry; keep diagnostics either way.
        val builder = Request.Builder().url(url("admin/server-logs")).get()
        token?.let { builder.header("Authorization", "Bearer $it") }
        val response = Http.client.newCall(builder.build()).await()
        val status = response.code
        // Off the caller's dispatcher: draining a body is a live socket read
        // (NetworkOnMainThreadException on Main — the readBody() rule).
        val bytes = withContext(Dispatchers.IO) {
            response.use { it.body?.bytes() ?: ByteArray(0) }
        }
        if (status !in 200..299) throw HalogenClient.ClientError.Http(status)
        val lossy = String(bytes, Charsets.UTF_8)
        val payload = try {
            WireJson.json
                .decodeFromString(ResponseData.serializer(ServerLogsData.serializer()), lossy)
                .data
        } catch (e: SerializationException) {
            DeviceLog.warn("server-logs: lossy re-decode failed — ${e::class.simpleName}: ${e.message}")
            null
        }
        if (payload != null) {
            payload
        } else {
            val head = String(bytes.copyOfRange(0, minOf(160, bytes.size)), Charsets.UTF_8)
            DeviceLog.warn("server-logs decode failed: ${bytes.size} bytes, head: $head")
            throw decodeError
        }
    }

suspend fun HalogenClient.serverErrors(): ServerErrorsData =
    get("admin/server-errors", emptyList(), ServerErrorsData.serializer())

/// Change the caller's own password (requires the current one).
suspend fun HalogenClient.changePassword(current: String, new: String) {
    postEmpty(
        "auth/password",
        PasswordChangeData(
            current_password = current,
            new_password = PasswordUpdateData(password = new, password_confirm = new),
        ),
        PasswordChangeData.serializer(),
    )
}
