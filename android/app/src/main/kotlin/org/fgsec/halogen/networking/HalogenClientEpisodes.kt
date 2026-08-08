package org.fgsec.halogen.networking

import okhttp3.Request
import okhttp3.Response
import org.fgsec.halogen.wire.DefaultDataType
import org.fgsec.halogen.wire.DownloadProgressData

// Episode download actions (server-side files).

/// Ask the server to download an episode's audio from its origin.
suspend fun HalogenClient.triggerDownload(episodeId: Int) {
    postEmpty("episodes/$episodeId/download", DefaultDataType, DefaultDataType.serializer())
}

/// Delete the server's stored file for an episode.
suspend fun HalogenClient.removeServerDownload(episodeId: Int) {
    delete("episodes/$episodeId/download")
}

/// In-flight server download progress; null once the tracker drops the entry
/// (terminal — the episode row's status carries the outcome).
suspend fun HalogenClient.downloadProgress(episodeId: Int): DownloadProgressData? =
    try {
        get(
            "episodes/$episodeId/download-progress",
            emptyList(),
            DownloadProgressData.serializer(),
        )
    } catch (e: HalogenClient.ClientError) {
        when {
            e is HalogenClient.ClientError.Http && e.code == 404 -> null
            e is HalogenClient.ClientError.Api -> null
            else -> throw e
        }
    }

/// Raw streaming GET of an episode's audio, bearer attached, optional resume
/// via `Range: bytes=<from>-`. Returns the open Response — the caller streams
/// `body.byteStream()`, checks 200/206/416 itself, and MUST close it.
suspend fun HalogenClient.downloadAudio(episodeId: Int, rangeStart: ULong? = null): Response {
    val builder = Request.Builder().url(audioUrl(episodeId)).get()
    token?.let { builder.header("Authorization", "Bearer $it") }
    rangeStart?.let { builder.header("Range", "bytes=$it-") }
    return Http.client.newCall(builder.build()).await()
}
